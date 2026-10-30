//! Backing a table up before it is dropped or emptied, and putting it back. The definition is read from the catalog,
//! each part as Postgres prints it. Putting it back runs in one transaction, every statement written by [`Writer`], so
//! what someone reviews is what runs.

use hn_core::{
    ColumnDefinition, Error, ErrorKind, GeneratedDefinition, IdentityDefinition, NamedDefinition, RestoreInto, SequenceDefinition, TableDefinition,
    TableRef, TriggerDefinition,
};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};

use crate::PgDatabase;
use crate::changes::{Step, Writer, shown_ident};
use crate::errors::from_sqlx;
use crate::sql::{Built, ColumnMeta};

/// A relation's oid and kind, how big it is, and what a backup of it can't keep.
const FACTS: &str = "SELECT c.oid::int8, c.relkind::text, pg_total_relation_size(c.oid)::float8, c.relpersistence = 'u', \
     obj_description(c.oid, 'pg_class'), \
     c.relrowsecurity OR EXISTS (SELECT 1 FROM pg_policy p WHERE p.polrelid = c.oid), c.relacl IS NOT NULL, c.reloptions IS NOT NULL, \
     EXISTS (SELECT 1 FROM pg_rewrite r WHERE r.ev_class = c.oid AND r.rulename <> '_RETURN'), \
     EXISTS (SELECT 1 FROM pg_publication_rel p WHERE p.prrelid = c.oid), \
     EXISTS (SELECT 1 FROM pg_statistic_ext s WHERE s.stxrelid = c.oid), \
     EXISTS (SELECT 1 FROM pg_inherits i WHERE i.inhrelid = c.oid OR i.inhparent = c.oid) \
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = $2";

/// A sequence's options as `CREATE SEQUENCE` and an identity column take them, from `pg_sequence q`.
macro_rules! sequence_options {
    () => {
        "format('INCREMENT BY %s MINVALUE %s MAXVALUE %s START WITH %s CACHE %s %s', q.seqincrement, q.seqmin, q.seqmax, \
         q.seqstart, q.seqcache, CASE WHEN q.seqcycle THEN 'CYCLE' ELSE 'NO CYCLE' END)"
    };
}

const COLUMNS: &str = concat!(
    "SELECT a.attname::text, format_type(a.atttypid, a.atttypmod), \
     CASE WHEN a.attcollation <> 0 AND a.attcollation <> t.typcollation \
       THEN quote_ident(cn.nspname) || '.' || quote_ident(co.collname) END, \
     a.attnotnull, CASE WHEN a.attidentity = '' AND a.attgenerated = '' THEN pg_get_expr(d.adbin, d.adrelid) END, \
     a.attidentity::text, CASE WHEN a.attgenerated <> '' THEN pg_get_expr(d.adbin, d.adrelid) END, a.attgenerated::text, \
     col_description(a.attrelid, a.attnum), s.options, s.last_value \
     FROM pg_attribute a JOIN pg_type t ON t.oid = a.atttypid \
     LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
     LEFT JOIN pg_collation co ON co.oid = a.attcollation LEFT JOIN pg_namespace cn ON cn.oid = co.collnamespace \
     LEFT JOIN LATERAL (SELECT ",
    sequence_options!(),
    " AS options, pg_sequence_last_value(q.seqrelid)::text AS last_value \
       FROM pg_depend dep JOIN pg_sequence q ON q.seqrelid = dep.objid \
       WHERE dep.classid = 'pg_class'::regclass AND dep.refobjid = a.attrelid AND dep.refobjsubid = a.attnum \
       AND dep.deptype = 'i') s ON a.attidentity <> '' \
     WHERE a.attrelid = $1::oid AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum"
);

/// Sequences the table owns, as a `serial` column's.
const SEQUENCES: &str = concat!(
    "SELECT n.nspname::text, s.relname::text, a.attname::text, 'AS ' || format_type(q.seqtypid, NULL) || ' ' || ",
    sequence_options!(),
    ", pg_sequence_last_value(s.oid)::text \
     FROM pg_depend dep JOIN pg_class s ON s.oid = dep.objid AND s.relkind = 'S' \
     JOIN pg_namespace n ON n.oid = s.relnamespace JOIN pg_sequence q ON q.seqrelid = s.oid \
     JOIN pg_attribute a ON a.attrelid = dep.refobjid AND a.attnum = dep.refobjsubid \
     WHERE dep.classid = 'pg_class'::regclass AND dep.refobjid = $1::oid AND dep.deptype = 'a' ORDER BY s.relname"
);

const CONSTRAINTS: &str = "SELECT k.conname::text, pg_get_constraintdef(k.oid), \
     ARRAY(SELECT a.attname::text FROM pg_attribute a WHERE a.attrelid = k.conrelid AND a.attnum = ANY (k.conkey) ORDER BY a.attnum) \
     FROM pg_constraint k WHERE k.conrelid = $1::oid AND k.contype IN ('p', 'u', 'x', 'c', 'f') ORDER BY k.contype = 'f', k.conname";

/// An index's columns are those it depends on, which include those in its expressions and its WHERE.
const INDEXES: &str = "SELECT c.relname::text, pg_get_indexdef(i.indexrelid), \
     ARRAY(SELECT a.attname::text FROM pg_depend d JOIN pg_attribute a ON a.attrelid = d.refobjid AND a.attnum = d.refobjsubid \
       WHERE d.classid = 'pg_class'::regclass AND d.objid = i.indexrelid AND d.refobjid = i.indrelid ORDER BY a.attnum) \
     FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid \
     WHERE i.indrelid = $1::oid AND NOT EXISTS (SELECT 1 FROM pg_constraint k WHERE k.conindid = i.indexrelid AND k.conrelid = $1::oid) \
     ORDER BY c.relname";

const TRIGGERS: &str = "SELECT tgname::text, pg_get_triggerdef(oid), tgenabled <> 'D' FROM pg_trigger \
     WHERE tgrelid = $1::oid AND NOT tgisinternal ORDER BY tgname";

const FIRING: &str = "SELECT tgname::text FROM pg_trigger WHERE tgrelid = $1::oid AND NOT tgisinternal AND tgenabled <> 'D' ORDER BY 1";

const HAS_COLUMN: &str = "SELECT EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = $1::oid AND attname = $2 AND attnum > 0 AND NOT attisdropped)";

/// What a backup of a table would keep, and how big it is.
pub(crate) struct Facts {
    pub oid: i64,
    pub size: f64,
    unlogged: bool,
    comment: Option<String>,
    not_kept: Vec<String>,
}

/// Reads a table's facts, or refuses one a backup can't make again, saying why.
pub(crate) async fn facts(conn: &mut PgConnection, table: &TableRef) -> Result<Facts, Error> {
    let row = sqlx::query(FACTS)
        .bind(&table.schema)
        .bind(&table.name)
        .fetch_optional(&mut *conn)
        .await
        .map_err(from_sqlx)?
        .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("There is no table named {}.{}.", table.schema, table.name)))?;
    let name = &table.name;
    let kind: String = row.try_get(1).map_err(from_sqlx)?;
    let refused = match kind.as_str() {
        "r" if row.try_get::<bool, _>(11).map_err(from_sqlx)? => Some(format!("{name} inherits from another table or is inherited")),
        "r" => None,
        "p" => Some(format!("{name} is partitioned")),
        "f" => Some(format!("{name} is a foreign table, whose rows live elsewhere")),
        _ => Some(format!("{name} isn't a table")),
    };
    if let Some(why) = refused {
        return Err(Error::new(ErrorKind::Invalid, format!("{why}, so Halfnine can't back it up.")));
    }
    let mut not_kept = Vec::new();
    for (at, what) in [
        (5, "row-level security and its policies"),
        (6, "grants"),
        (7, "storage options"),
        (8, "rules"),
        (9, "publications"),
        (10, "extended statistics"),
    ] {
        if row.try_get::<bool, _>(at).map_err(from_sqlx)? {
            not_kept.push(what.to_string());
        }
    }
    Ok(Facts {
        oid: row.try_get(0).map_err(from_sqlx)?,
        size: row.try_get(2).map_err(from_sqlx)?,
        unlogged: row.try_get(3).map_err(from_sqlx)?,
        comment: row.try_get(4).map_err(from_sqlx)?,
        not_kept,
    })
}

async fn fetch(conn: &mut PgConnection, sql: &'static str, oid: i64) -> Result<Vec<PgRow>, Error> {
    sqlx::query(sql).bind(oid).fetch_all(&mut *conn).await.map_err(from_sqlx)
}

/// A table's whole definition, read in the transaction that backs it up.
pub(crate) async fn definition(conn: &mut PgConnection, table: &TableRef) -> Result<TableDefinition, Error> {
    let facts = facts(conn, table).await?;
    let oid = facts.oid;
    let columns = fetch(conn, COLUMNS, oid).await?;
    let sequences = fetch(conn, SEQUENCES, oid).await?;
    let constraints = fetch(conn, CONSTRAINTS, oid).await?;
    let indexes = fetch(conn, INDEXES, oid).await?;
    let triggers = fetch(conn, TRIGGERS, oid).await?;
    let key = sqlx::query(crate::catalog::PRIMARY_KEY)
        .bind(oid)
        .fetch_optional(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    read_definition(table, facts, &columns, &sequences, &constraints, &indexes, &triggers, key.as_ref()).map_err(from_sqlx)
}

#[allow(clippy::too_many_arguments)]
fn read_definition(
    table: &TableRef,
    facts: Facts,
    columns: &[PgRow],
    sequences: &[PgRow],
    constraints: &[PgRow],
    indexes: &[PgRow],
    triggers: &[PgRow],
    key: Option<&PgRow>,
) -> Result<TableDefinition, sqlx::Error> {
    let named = |r: &PgRow| -> Result<NamedDefinition, sqlx::Error> {
        Ok(NamedDefinition {
            name: r.try_get(0)?,
            definition: r.try_get(1)?,
            columns: r.try_get(2)?,
        })
    };
    let column = |r: &PgRow| -> Result<ColumnDefinition, sqlx::Error> {
        let identity: String = r.try_get(5)?;
        let generated: String = r.try_get(7)?;
        Ok(ColumnDefinition {
            name: r.try_get(0)?,
            type_name: r.try_get(1)?,
            collation: r.try_get(2)?,
            not_null: r.try_get(3)?,
            default: r.try_get(4)?,
            identity: match identity.as_str() {
                "" => None,
                _ => Some(IdentityDefinition {
                    always: identity == "a",
                    options: r.try_get::<Option<String>, _>(9)?.unwrap_or_default(),
                    last_value: r.try_get(10)?,
                }),
            },
            generated: match generated.as_str() {
                "" => None,
                _ => Some(GeneratedDefinition {
                    expression: r.try_get::<Option<String>, _>(6)?.unwrap_or_default(),
                    stored: generated == "s",
                }),
            },
            comment: r.try_get(8)?,
        })
    };
    Ok(TableDefinition {
        table: table.clone(),
        unlogged: facts.unlogged,
        comment: facts.comment,
        columns: columns.iter().map(column).collect::<Result<_, _>>()?,
        constraints: constraints.iter().map(named).collect::<Result<_, _>>()?,
        indexes: indexes.iter().map(named).collect::<Result<_, _>>()?,
        triggers: triggers
            .iter()
            .map(|r| {
                Ok(TriggerDefinition {
                    name: r.try_get(0)?,
                    definition: r.try_get(1)?,
                    enabled: r.try_get(2)?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()?,
        sequences: sequences
            .iter()
            .map(|r| {
                Ok(SequenceDefinition {
                    schema: r.try_get(0)?,
                    name: r.try_get(1)?,
                    column: r.try_get(2)?,
                    options: r.try_get(3)?,
                    last_value: r.try_get(4)?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()?,
        primary_key: key.map(|k| k.try_get(0)).transpose()?.unwrap_or_default(),
        not_kept: facts.not_kept,
    })
}

/// The columns to read a backup's rows by, in the definition's order.
pub(crate) fn columns(definition: &TableDefinition) -> Vec<ColumnMeta> {
    definition
        .columns
        .iter()
        .map(|c| ColumnMeta {
            name: c.name.clone(),
            type_name: c.type_name.clone(),
            key_position: definition.primary_key.iter().position(|k| k == &c.name).map(|at| at as i32 + 1),
            generated: c.generated.is_some() || c.identity.as_ref().is_some_and(|i| i.always),
            identity: c.identity.as_ref().is_some_and(|i| i.always),
        })
        .collect()
}

/// What the table has now that a restore must work around.
#[derive(Default)]
pub(crate) struct Present {
    /// Its triggers that fire, paused while the rows go back.
    firing: Vec<String>,
}

/// Reads what the table has now, refusing to refill a table that has rows or to add a column it has again.
pub(crate) async fn present(
    conn: &mut PgConnection,
    oid: i64,
    definition: &TableDefinition,
    into: &RestoreInto,
    db: &PgDatabase,
) -> Result<Present, Error> {
    let table = &definition.table;
    match into {
        RestoreInto::Recreate => return Ok(Present::default()),
        RestoreInto::Refill => {
            let has_rows = Built {
                sql: format!("SELECT EXISTS (SELECT 1 FROM {})", crate::sql::table(table)),
                params: Vec::new(),
            };
            let row = db.query(has_rows).fetch_one(&mut *conn).await.map_err(from_sqlx)?;
            if row.try_get::<bool, _>(0).map_err(from_sqlx)? {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    format!(
                        "{} has rows again, so Halfnine won't mix the backup's rows in. Empty it first.",
                        table.name
                    ),
                ));
            }
        }
        RestoreInto::Column { name } => {
            let found: bool = sqlx::query_scalar(HAS_COLUMN)
                .bind(oid)
                .bind(name)
                .fetch_one(&mut *conn)
                .await
                .map_err(from_sqlx)?;
            if found {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    format!("{} has a column named {name} again. Rename or drop it first.", table.name),
                ));
            }
        }
    }
    let firing = fetch(conn, FIRING, oid).await?;
    Ok(Present {
        firing: firing.iter().map(|r| r.try_get(0)).collect::<Result<_, _>>().map_err(from_sqlx)?,
    })
}

/// The statements that put a backup back: those before its rows, how its rows come back, and those after.
pub(crate) struct Plan {
    pub before: Vec<Step>,
    /// A temporary table of text, one column per column of the backup, that the rows are copied into first.
    pub stage: String,
    pub copy: String,
    /// How the rows go from there into the table, or `None` when Postgres computes the column being put back.
    pub rows: Option<Step>,
    pub after: Vec<Step>,
}

impl Plan {
    pub(crate) fn shown(&self) -> Vec<String> {
        let mut shown: Vec<String> = self.before.iter().map(|s| s.shown.clone()).collect();
        if let Some(rows) = &self.rows {
            shown.push(format!("{};", self.stage));
            shown.push(format!("{};", self.copy));
            shown.push(rows.shown.clone());
        }
        shown.extend(self.after.iter().map(|s| s.shown.clone()));
        shown
    }
}

fn alter(table: &TableRef) -> Writer {
    let mut w = Writer::default();
    w.text("ALTER TABLE ").table(table).text(" ");
    w
}

fn sequence(s: &SequenceDefinition) -> TableRef {
    TableRef {
        schema: s.schema.clone(),
        name: s.name.clone(),
    }
}

/// A column as `CREATE TABLE` and `ADD COLUMN` take it. An identity column generated always is added generated by
/// default when its values go back with `UPDATE`, which can't write it otherwise.
fn column_sql(w: &mut Writer, c: &ColumnDefinition, adding: bool) {
    w.name(&c.name).text(" ").typed(&c.type_name);
    if let Some(collation) = &c.collation {
        w.text(" COLLATE ").typed(collation);
    }
    if let Some(identity) = &c.identity {
        let how = if identity.always && !adding { "ALWAYS" } else { "BY DEFAULT" };
        w.text(&format!(" GENERATED {how} AS IDENTITY (")).typed(&identity.options).text(")");
    }
    if let Some(generated) = &c.generated {
        let how = if generated.stored { "STORED" } else { "VIRTUAL" };
        w.text(" GENERATED ALWAYS AS (").typed(&generated.expression).text(&format!(") {how}"));
    }
    if let Some(default) = &c.default {
        w.text(" DEFAULT ").typed(default);
    }
    if c.not_null && c.identity.is_none() && !adding {
        w.text(" NOT NULL");
    }
}

/// The plan for putting `definition` back as `into` asks, given what the table has now.
pub(crate) fn plan(definition: &TableDefinition, into: &RestoreInto, present: &Present) -> Result<Plan, Error> {
    let table = &definition.table;
    let restoring = |c: &str| match into {
        RestoreInto::Recreate | RestoreInto::Refill => true,
        RestoreInto::Column { name } => c == name,
    };
    let remaking = !matches!(into, RestoreInto::Refill);
    let sequences: Vec<&SequenceDefinition> = definition.sequences.iter().filter(|s| restoring(&s.column)).collect();
    let mut before = Vec::new();
    let mut after = Vec::new();

    if remaking {
        for s in &sequences {
            let mut w = Writer::default();
            w.text("CREATE SEQUENCE ").table(&sequence(s)).text(" ").typed(&s.options);
            before.push(w.step(format!("Making the sequence {}", s.name), None));
        }
    }
    let column = match into {
        RestoreInto::Recreate => {
            let mut w = Writer::default();
            w.text(if definition.unlogged {
                "CREATE UNLOGGED TABLE "
            } else {
                "CREATE TABLE "
            })
            .table(table)
            .text(" (");
            for (i, c) in definition.columns.iter().enumerate() {
                w.text(if i == 0 { "\n  " } else { ",\n  " });
                column_sql(&mut w, c, false);
            }
            w.text("\n)");
            before.push(w.step(format!("Making the table {}", table.name), None));
            None
        }
        RestoreInto::Refill => None,
        RestoreInto::Column { name } => {
            let at = definition
                .columns
                .iter()
                .position(|c| &c.name == name)
                .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("The backup has no column named {name}.")))?;
            if definition.primary_key.is_empty() || definition.primary_key.contains(name) {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    format!(
                        "Halfnine puts a column back by the primary key, and {} has none apart from {name}. Restore the whole table instead.",
                        table.name
                    ),
                ));
            }
            let mut w = alter(table);
            w.text("ADD COLUMN ");
            column_sql(&mut w, &definition.columns[at], true);
            before.push(w.step(format!("Adding the column {name} again"), None));
            Some((at, &definition.columns[at]))
        }
    };
    // Rows go back into a table that keeps its triggers, which would otherwise fire as if the rows were new.
    let computed = column.is_some_and(|(_, c)| c.generated.is_some());
    let pausing: Vec<&String> = match into {
        RestoreInto::Recreate => Vec::new(),
        _ if computed => Vec::new(),
        _ => present.firing.iter().collect(),
    };
    for trigger in &pausing {
        let mut w = alter(table);
        w.text("DISABLE TRIGGER ").name(trigger);
        before.push(w.step(format!("Pausing the trigger {trigger} while the rows go back"), None));
    }

    let stage = format!(
        "CREATE TEMPORARY TABLE hn_restore ({}) ON COMMIT DROP",
        (1..=definition.columns.len())
            .map(|i| format!("c{i} text"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let copy = "COPY hn_restore FROM STDIN WITH (FORMAT csv, HEADER true)".to_string();
    let rows = match column {
        Some(_) if computed => None,
        Some((at, c)) => {
            let mut w = Writer::default();
            w.text("UPDATE ")
                .table(table)
                .text(" AS t SET ")
                .name(&c.name)
                .text(&format!(" = h.c{}::", at + 1))
                .typed(&c.type_name)
                .text(" FROM hn_restore h WHERE ");
            for (i, key) in definition.primary_key.iter().enumerate() {
                let k = definition.columns.iter().position(|c| &c.name == key).unwrap_or_default();
                if i > 0 {
                    w.text(" AND ");
                }
                w.text("t.")
                    .name(key)
                    .text(&format!(" = h.c{}::", k + 1))
                    .typed(&definition.columns[k].type_name);
            }
            Some(w.step(format!("Putting back the values of {}", c.name), None))
        }
        None => {
            let written: Vec<(usize, &ColumnDefinition)> = definition.columns.iter().enumerate().filter(|(_, c)| c.generated.is_none()).collect();
            let mut w = Writer::default();
            w.text("INSERT INTO ").table(table).text(" (");
            for (n, (_, c)) in written.iter().enumerate() {
                if n > 0 {
                    w.text(", ");
                }
                w.name(&c.name);
            }
            w.text(")");
            if written.iter().any(|(_, c)| c.identity.as_ref().is_some_and(|i| i.always)) {
                w.text(" OVERRIDING SYSTEM VALUE");
            }
            w.text(" SELECT ");
            for (n, (i, c)) in written.iter().enumerate() {
                if n > 0 {
                    w.text(", ");
                }
                w.text(&format!("c{}::", i + 1)).typed(&c.type_name);
            }
            w.text(" FROM hn_restore");
            Some(w.step(format!("Putting back the rows of {}", table.name), None))
        }
    };

    for trigger in &pausing {
        let mut w = alter(table);
        w.text("ENABLE TRIGGER ").name(trigger);
        after.push(w.step(format!("Turning the trigger {trigger} back on"), None));
    }
    if let Some((_, c)) = column {
        if c.identity.as_ref().is_some_and(|i| i.always) {
            let mut w = alter(table);
            w.text("ALTER COLUMN ").name(&c.name).text(" SET GENERATED ALWAYS");
            after.push(w.step(format!("Generating {} always again", c.name), None));
        }
        if c.not_null && c.identity.is_none() {
            let mut w = alter(table);
            w.text("ALTER COLUMN ").name(&c.name).text(" SET NOT NULL");
            after.push(w.step(format!("Making {} required again", c.name), None));
        }
    }
    // Sequences carry on from where they were, so new rows don't collide with those put back.
    for s in &sequences {
        if remaking {
            let mut w = Writer::default();
            w.text("ALTER SEQUENCE ")
                .table(&sequence(s))
                .text(" OWNED BY ")
                .table(table)
                .text(".")
                .name(&s.column);
            after.push(w.step(format!("Tying {} to its column", s.name), None));
        }
        if let Some(last) = &s.last_value {
            let mut w = Writer::default();
            let name = format!("{}.{}", shown_ident(&s.schema), shown_ident(&s.name));
            w.text("SELECT setval(")
                .param(&name, "regclass")
                .text(", ")
                .param(last, "bigint")
                .text(")");
            after.push(w.step(format!("Carrying {} on from {last}", s.name), None));
        }
    }
    for c in definition.columns.iter().filter(|c| restoring(&c.name)) {
        if let Some(last) = c.identity.as_ref().and_then(|i| i.last_value.as_ref()) {
            let mut w = Writer::default();
            let name = format!("{}.{}", shown_ident(&table.schema), shown_ident(&table.name));
            w.text("SELECT setval(pg_get_serial_sequence(")
                .param(&name, "text")
                .text(", ")
                .param(&c.name, "text")
                .text("), ")
                .param(last, "bigint")
                .text(")");
            after.push(w.step(format!("Carrying {} on from {last}", c.name), None));
        }
    }
    if remaking {
        let lost = |d: &&NamedDefinition| match into {
            RestoreInto::Column { name } => d.columns.contains(name),
            _ => true,
        };
        for k in definition.constraints.iter().filter(lost) {
            let mut w = alter(table);
            w.text("ADD CONSTRAINT ").name(&k.name).text(" ").typed(&k.definition);
            after.push(w.step(format!("Adding the constraint {}", k.name), None));
        }
        for i in definition.indexes.iter().filter(lost) {
            let mut w = Writer::default();
            w.typed(&i.definition);
            after.push(w.step(format!("Making the index {}", i.name), None));
        }
    }
    if matches!(into, RestoreInto::Recreate) {
        for t in &definition.triggers {
            let mut w = Writer::default();
            w.typed(&t.definition);
            after.push(w.step(format!("Making the trigger {}", t.name), None));
            if !t.enabled {
                let mut w = alter(table);
                w.text("DISABLE TRIGGER ").name(&t.name);
                after.push(w.step(format!("Turning {} off, as it was", t.name), None));
            }
        }
        if let Some(comment) = &definition.comment {
            let mut w = Writer::default();
            w.text("COMMENT ON TABLE ").table(table).text(" IS ").literal(comment);
            after.push(w.step(format!("Commenting {}", table.name), None));
        }
    }
    if remaking {
        for c in definition.columns.iter().filter(|c| restoring(&c.name)) {
            if let Some(comment) = &c.comment {
                let mut w = Writer::default();
                w.text("COMMENT ON COLUMN ")
                    .table(table)
                    .text(".")
                    .name(&c.name)
                    .text(" IS ")
                    .literal(comment);
                after.push(w.step(format!("Commenting {}", c.name), None));
            }
        }
    }
    Ok(Plan {
        before,
        stage,
        copy,
        rows,
        after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, type_name: &str) -> ColumnDefinition {
        ColumnDefinition {
            name: name.into(),
            type_name: type_name.into(),
            collation: None,
            not_null: false,
            default: None,
            identity: None,
            generated: None,
            comment: None,
        }
    }

    fn definition() -> TableDefinition {
        TableDefinition {
            table: TableRef {
                schema: "public".into(),
                name: "notes".into(),
            },
            unlogged: false,
            comment: Some("Kept notes".into()),
            columns: vec![
                ColumnDefinition {
                    identity: Some(IdentityDefinition {
                        always: true,
                        options: "INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 START WITH 1 CACHE 1 NO CYCLE".into(),
                        last_value: Some("41".into()),
                    }),
                    not_null: true,
                    ..column("id", "bigint")
                },
                ColumnDefinition {
                    not_null: true,
                    comment: Some("it's the body".into()),
                    ..column("body", "text")
                },
                ColumnDefinition {
                    generated: Some(GeneratedDefinition {
                        expression: "length(body)".into(),
                        stored: true,
                    }),
                    ..column("size", "integer")
                },
            ],
            constraints: vec![NamedDefinition {
                name: "notes_pkey".into(),
                definition: "PRIMARY KEY (id)".into(),
                columns: vec!["id".into()],
            }],
            indexes: vec![NamedDefinition {
                name: "notes_body_idx".into(),
                definition: "CREATE INDEX notes_body_idx ON public.notes USING btree (body)".into(),
                columns: vec!["body".into()],
            }],
            triggers: vec![],
            sequences: vec![],
            primary_key: vec!["id".into()],
            not_kept: vec![],
        }
    }

    #[test]
    fn makes_a_dropped_table_again_with_its_rows_identity_and_comments() {
        let plan = plan(&definition(), &RestoreInto::Recreate, &Present::default()).unwrap();
        assert_eq!(
            plan.shown(),
            [
                "CREATE TABLE public.notes (\n  id bigint GENERATED ALWAYS AS IDENTITY (INCREMENT BY 1 MINVALUE 1 MAXVALUE 9223372036854775807 START WITH 1 CACHE 1 NO CYCLE),\n  body text NOT NULL,\n  size integer GENERATED ALWAYS AS (length(body)) STORED\n);",
                "CREATE TEMPORARY TABLE hn_restore (c1 text, c2 text, c3 text) ON COMMIT DROP;",
                "COPY hn_restore FROM STDIN WITH (FORMAT csv, HEADER true);",
                "INSERT INTO public.notes (id, body) OVERRIDING SYSTEM VALUE SELECT c1::bigint, c2::text FROM hn_restore;",
                "SELECT setval(pg_get_serial_sequence('public.notes', 'id'), 41);",
                "ALTER TABLE public.notes ADD CONSTRAINT notes_pkey PRIMARY KEY (id);",
                "CREATE INDEX notes_body_idx ON public.notes USING btree (body);",
                "COMMENT ON TABLE public.notes IS 'Kept notes';",
                "COMMENT ON COLUMN public.notes.body IS 'it''s the body';",
            ]
        );
        assert_eq!(
            plan.after[0].run.params,
            [Some("public.notes".into()), Some("id".into()), Some("41".into())]
        );
    }

    #[test]
    fn refills_an_emptied_table_with_its_triggers_paused() {
        let present = Present {
            firing: vec!["stamp".into()],
        };
        let plan = plan(&definition(), &RestoreInto::Refill, &present).unwrap();
        assert_eq!(
            plan.shown(),
            [
                "ALTER TABLE public.notes DISABLE TRIGGER stamp;",
                "CREATE TEMPORARY TABLE hn_restore (c1 text, c2 text, c3 text) ON COMMIT DROP;",
                "COPY hn_restore FROM STDIN WITH (FORMAT csv, HEADER true);",
                "INSERT INTO public.notes (id, body) OVERRIDING SYSTEM VALUE SELECT c1::bigint, c2::text FROM hn_restore;",
                "ALTER TABLE public.notes ENABLE TRIGGER stamp;",
                "SELECT setval(pg_get_serial_sequence('public.notes', 'id'), 41);",
            ]
        );
    }

    #[test]
    fn puts_a_dropped_column_back_by_key_with_what_it_lost() {
        let present = Present::default();
        let into = RestoreInto::Column { name: "body".into() };
        assert_eq!(
            plan(&definition(), &into, &present).unwrap().shown(),
            [
                "ALTER TABLE public.notes ADD COLUMN body text;",
                "CREATE TEMPORARY TABLE hn_restore (c1 text, c2 text, c3 text) ON COMMIT DROP;",
                "COPY hn_restore FROM STDIN WITH (FORMAT csv, HEADER true);",
                "UPDATE public.notes AS t SET body = h.c2::text FROM hn_restore h WHERE t.id = h.c1::bigint;",
                "ALTER TABLE public.notes ALTER COLUMN body SET NOT NULL;",
                "CREATE INDEX notes_body_idx ON public.notes USING btree (body);",
                "COMMENT ON COLUMN public.notes.body IS 'it''s the body';",
            ]
        );
        let generated = RestoreInto::Column { name: "size".into() };
        assert_eq!(
            plan(&definition(), &generated, &present).unwrap().shown(),
            ["ALTER TABLE public.notes ADD COLUMN size integer GENERATED ALWAYS AS (length(body)) STORED;"]
        );
        let key = RestoreInto::Column { name: "id".into() };
        assert_eq!(plan(&definition(), &key, &present).err().map(|e| e.kind), Some(ErrorKind::Invalid));
    }
}
