//! Rules checked in the transaction that makes a change, just before it commits. Only the rules on tables the change
//! touched run. Postgres counts each transaction's writes per table, but a pooled connection can still hold counts from
//! the transaction before, and TRUNCATE sets them back to zero, so they are read when the transaction begins and again
//! before it commits, and only a difference means the change touched a table.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use hn_core::{Checks, ColumnRef, Error, ErrorKind, Rule, RuleCheck, RuleTried, TableRef};
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};

use crate::PgDatabase;
use crate::changes::Writer;
use crate::errors::from_sqlx;
use crate::sql::{Built, ColumnMeta};

/// How long the rules a change touched may take to check, together.
pub(crate) const TIME: Duration = Duration::from_secs(5);
/// How many rows that break a rule it names.
const EXAMPLES: usize = 3;

/// Each table's writes so far in this transaction, summed over the table and its partitions, and the files that hold
/// them. `pg_partition_tree` lists nothing for a table that isn't partitioned, so the table itself is always added.
const TABLES: &str = "SELECT t.schema, t.name, c.oid IS NOT NULL, \
     coalesce(sum(s.n_tup_ins + s.n_tup_upd + s.n_tup_del), 0)::int8, coalesce(sum(s.n_tup_upd + s.n_tup_del), 0)::int8, \
     coalesce(string_agg(p.relfilenode::text, ',' ORDER BY p.oid), '') \
     FROM unnest($1::text[], $2::text[]) AS t(schema, name) \
     LEFT JOIN pg_namespace n ON n.nspname = t.schema \
     LEFT JOIN pg_class c ON c.relnamespace = n.oid AND c.relname = t.name \
     LEFT JOIN LATERAL (SELECT c.oid AS relid UNION SELECT relid FROM pg_partition_tree(c.oid)) AS tree ON c.oid IS NOT NULL \
     LEFT JOIN pg_class p ON p.oid = tree.relid \
     LEFT JOIN pg_stat_xact_user_tables s ON s.relid = tree.relid \
     GROUP BY t.schema, t.name, c.oid";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Seen {
    exists: bool,
    /// Rows added, changed and removed.
    written: i64,
    /// Rows changed and removed.
    changed: i64,
    /// The files the table's rows live in, which TRUNCATE and a rewrite replace.
    files: String,
}

type Key = (String, String);

/// What the transaction looked like when it began.
#[derive(Default)]
pub(crate) struct Before {
    /// Postgres counts writes, so an untouched table can be told apart. Without it, every rule runs.
    counted: bool,
    seen: HashMap<Key, Seen>,
}

fn key(table: &TableRef) -> Key {
    (table.schema.clone(), table.name.clone())
}

/// Every table a change to which could break one of the rules.
fn tables(checks: &Checks) -> Vec<TableRef> {
    let mut all: Vec<TableRef> = Vec::new();
    for rule in &checks.rules {
        for table in rule.check.tables(&rule.table) {
            if !all.contains(table) {
                all.push(table.clone());
            }
        }
    }
    all
}

async fn read(conn: &mut PgConnection, tables: &[TableRef]) -> Result<HashMap<Key, Seen>, Error> {
    let schemas: Vec<&str> = tables.iter().map(|t| t.schema.as_str()).collect();
    let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
    let rows = sqlx::query(TABLES)
        .bind(&schemas)
        .bind(&names)
        .fetch_all(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    rows.iter()
        .map(|r| {
            Ok((
                (r.try_get(0)?, r.try_get(1)?),
                Seen {
                    exists: r.try_get(2)?,
                    written: r.try_get(3)?,
                    changed: r.try_get(4)?,
                    files: r.try_get(5)?,
                },
            ))
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(from_sqlx)
}

/// Reads the tables the rules depend on as the transaction begins. With no rules it reads nothing.
pub(crate) async fn baseline(conn: &mut PgConnection, checks: &Checks) -> Result<Before, Error> {
    if checks.rules.is_empty() {
        return Ok(Before::default());
    }
    let counted: bool = sqlx::query_scalar("SELECT current_setting('track_counts')::bool")
        .fetch_one(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    Ok(Before {
        counted,
        seen: read(conn, &tables(checks)).await?,
    })
}

/// A rule the change broke, and the rows that show it.
struct Broken<'a> {
    rule: &'a Rule,
    /// Up to three rows, each named by its primary key, or why the rule couldn't be checked.
    detail: String,
}

/// Checks the rules on the tables the change touched, in a read-only savepoint so a rule's own query can write
/// nothing, and refuses with [`ErrorKind::BrokenRule`] if one is broken or can't be checked in time.
pub(crate) async fn after(conn: &mut PgConnection, checks: &Checks, before: Before, db: &PgDatabase) -> Result<(), Error> {
    if checks.rules.is_empty() {
        return Ok(());
    }
    let now = read(conn, &tables(checks)).await?;
    let touched = |table: &TableRef| !before.counted || before.seen.get(&key(table)) != now.get(&key(table));
    let due: Vec<&Rule> = checks
        .rules
        .iter()
        .filter(|rule| rule.check.tables(&rule.table).into_iter().any(touched))
        .collect();
    let mut broken = Vec::new();
    for rule in due.iter().filter(|r| matches!(r.check, RuleCheck::OnlyGrows)) {
        let was = before.seen.get(&key(&rule.table)).cloned().unwrap_or_default();
        let is = now.get(&key(&rule.table)).cloned().unwrap_or_default();
        let detail = if !before.counted {
            Some("Postgres isn't counting writes here (track_counts is off), so Halfnine can't tell".to_string())
        } else if was.exists && !is.exists {
            Some("it removes the table".to_string())
        } else if is.files != was.files {
            Some("it empties or rewrites the table".to_string())
        } else if is.changed != was.changed {
            Some(format!("it changes or removes {}", rows(is.changed - was.changed)))
        } else {
            None
        };
        if let Some(detail) = detail {
            broken.push(Broken { rule, detail });
        }
    }
    let queried: Vec<&Rule> = due.into_iter().filter(|r| !matches!(r.check, RuleCheck::OnlyGrows)).collect();
    if !queried.is_empty() {
        broken.extend(check(conn, &queried, &checks.hidden, &now, db).await?);
    }
    match broken.as_slice() {
        [] => Ok(()),
        _ => Err(refusal(&broken, &checks.hidden)),
    }
}

/// Runs each rule's query in a savepoint made read-only, with what's left of [`TIME`], then rolls the savepoint back.
async fn check<'a>(
    conn: &mut PgConnection,
    rules: &[&'a Rule],
    hidden: &[ColumnRef],
    now: &HashMap<Key, Seen>,
    db: &PgDatabase,
) -> Result<Vec<Broken<'a>>, Error> {
    sqlx::raw_sql("SAVEPOINT hn_rules; SET TRANSACTION READ ONLY")
        .execute(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    sqlx::query("SELECT set_config('statement_timeout', $1, true)")
        .bind(TIME.as_millis().to_string())
        .execute(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    let mut broken = Vec::new();
    for rule in rules {
        let gone = |table: &TableRef| !now.get(&key(table)).is_some_and(|s| s.exists);
        if let Some(missing) = rule.check.tables(&rule.table).into_iter().find(|t| gone(t)) {
            broken.push(Broken {
                rule,
                detail: format!("it removes {}, which the rule needs", missing.name),
            });
            continue;
        }
        let columns = crate::table_columns(&mut *conn, &rule.table).await?;
        let keys: Vec<String> = keys(&columns, &rule.table, hidden);
        let built = match query(rule, &columns, &keys) {
            Ok(built) => built,
            Err(why) => {
                broken.push(Broken { rule, detail: why });
                continue;
            }
        };
        match db.query(built).persistent(false).fetch_all(&mut *conn).await {
            Ok(found) if found.is_empty() => {}
            Ok(found) => {
                let named = named(&found, &keys);
                let detail = if named.is_empty() {
                    "some rows break it".to_string()
                } else {
                    format!("rows that break it: {}", named.join("; "))
                };
                broken.push(Broken { rule, detail });
            }
            Err(error) => {
                let error = from_sqlx(error);
                let detail = if error.code.as_deref() == Some("57014") {
                    format!("Halfnine couldn't check it within {} seconds", TIME.as_secs())
                } else {
                    format!("Halfnine couldn't check it: {}", error.message)
                };
                broken.push(Broken { rule, detail });
                // The savepoint is aborted now, so nothing more can run in it.
                break;
            }
        }
    }
    sqlx::raw_sql("ROLLBACK TO SAVEPOINT hn_rules; RELEASE SAVEPOINT hn_rules")
        .execute(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    Ok(broken)
}

/// Each row found by its key, as "id = 1042", or none when the rows were found without one.
fn named(found: &[PgRow], keys: &[String]) -> Vec<String> {
    found
        .iter()
        .map(|row| {
            keys.iter()
                .enumerate()
                .map(|(i, k)| format!("{k} = {}", row.try_get::<Option<String>, _>(i).ok().flatten().unwrap_or_default()))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|n| !n.is_empty())
        .collect()
}

/// Tries a rule on the data as it is, before it's added: the rows that break it already, how long it takes, and for
/// custom SQL, which must be one statement that reads, the tables its plan reads.
pub(crate) async fn try_now(conn: &mut PgConnection, rule: &Rule, db: &PgDatabase) -> Result<RuleTried, Error> {
    let reads = match &rule.check {
        RuleCheck::Custom { sql, .. } => {
            let word = crate::editor::first_word(sql);
            if !["SELECT", "WITH", "VALUES", "TABLE"].contains(&word.as_str()) {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    "A custom rule is one query that reads, starting with SELECT, WITH, VALUES or TABLE.",
                ));
            }
            // Postgres describes it first, which refuses a second statement, so the plan is of this one only.
            crate::editor::describe(conn, sql).await?;
            relations(&crate::plan(conn, sql).await?)
        }
        _ => Vec::new(),
    };
    if matches!(rule.check, RuleCheck::OnlyGrows) {
        return Ok(RuleTried {
            breaking: Vec::new(),
            elapsed_ms: 0,
            reads,
        });
    }
    let columns = crate::table_columns(&mut *conn, &rule.table).await?;
    let keys = keys(&columns, &rule.table, &[]);
    let built = query(rule, &columns, &keys).map_err(|why| Error::new(ErrorKind::Invalid, why))?;
    let started = Instant::now();
    let found = db.query(built).persistent(false).fetch_all(&mut *conn).await.map_err(from_sqlx)?;
    let mut breaking = named(&found, &keys);
    if breaking.is_empty() && !found.is_empty() {
        breaking.push("some rows".into());
    }
    Ok(RuleTried {
        breaking,
        elapsed_ms: hn_core::millis(started.elapsed()),
        reads,
    })
}

/// Every table a plan reads, as Postgres names it in each node.
fn relations(plan: &serde_json::Value) -> Vec<TableRef> {
    let mut found = Vec::new();
    let mut stack = vec![plan];
    while let Some(node) = stack.pop() {
        match node {
            serde_json::Value::Object(map) => {
                if let (Some(serde_json::Value::String(schema)), Some(serde_json::Value::String(name))) =
                    (map.get("Schema"), map.get("Relation Name"))
                {
                    let table = TableRef {
                        schema: schema.clone(),
                        name: name.clone(),
                    };
                    if !found.contains(&table) {
                        found.push(table);
                    }
                }
                stack.extend(map.values());
            }
            serde_json::Value::Array(items) => stack.extend(items),
            _ => {}
        }
    }
    found
}

/// The primary key columns a report names rows by, unless one is hidden from the reader, when it names none.
fn keys(columns: &[ColumnMeta], table: &TableRef, hidden: &[ColumnRef]) -> Vec<String> {
    let mut key: Vec<&ColumnMeta> = columns.iter().filter(|c| c.key_position.is_some()).collect();
    key.sort_by_key(|c| c.key_position);
    let is_hidden = |c: &ColumnMeta| {
        hidden
            .iter()
            .any(|h| h.schema == table.schema && h.table == table.name && h.column == c.name)
    };
    if key.iter().any(|c| is_hidden(c)) {
        return Vec::new();
    }
    key.into_iter().map(|c| c.name.clone()).collect()
}

fn rows(n: i64) -> String {
    if n == 1 { "1 row".into() } else { format!("{n} rows") }
}

/// The query that finds up to three rows breaking a rule, each named by `keys`. Every name goes through
/// [`Writer::name`] and every value is a bound parameter, read with the column's own type. Custom SQL is the person's
/// own, as typed, inside a query that only counts its rows.
pub(crate) fn query(rule: &Rule, columns: &[ColumnMeta], keys: &[String]) -> Result<Built, String> {
    let column_type = |name: &str| {
        columns
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.type_name.clone())
            .ok_or_else(|| format!("it removes {name}, which the rule checks"))
    };
    let mut w = Writer::default();
    let select = |w: &mut Writer, alias: Option<&str>| {
        w.text("SELECT ");
        if keys.is_empty() {
            w.text("NULL");
        }
        for (i, k) in keys.iter().enumerate() {
            if i > 0 {
                w.text(", ");
            }
            if let Some(alias) = alias {
                w.text(alias).text(".");
            }
            w.name(k).text("::text");
        }
    };
    match &rule.check {
        RuleCheck::AlwaysFilled { column } => {
            column_type(column)?;
            select(&mut w, None);
            w.text(" FROM ").table(&rule.table).text(" WHERE ").name(column).text(" IS NULL OR ");
            w.name(column).text("::text = ''");
        }
        RuleCheck::OneOf { column, values } => {
            let type_name = column_type(column)?;
            select(&mut w, None);
            w.text(" FROM ").table(&rule.table).text(" WHERE ").name(column).text(" IS NOT NULL");
            if !values.is_empty() {
                w.text(" AND ").name(column).text(" NOT IN (");
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        w.text(", ");
                    }
                    w.param(value, "text").text("::").typed(&type_name);
                }
                w.text(")");
            }
        }
        RuleCheck::Between { column, at_least, at_most } => {
            let type_name = column_type(column)?;
            select(&mut w, None);
            w.text(" FROM ").table(&rule.table).text(" WHERE false");
            if let Some(low) = at_least {
                w.text(" OR ").name(column).text(" < ").param(low, "text").text("::").typed(&type_name);
            }
            if let Some(high) = at_most {
                w.text(" OR ").name(column).text(" > ").param(high, "text").text("::").typed(&type_name);
            }
        }
        RuleCheck::HasMatch {
            column,
            target,
            target_column,
        } => {
            column_type(column)?;
            select(&mut w, Some("s"));
            w.text(" FROM ")
                .table(&rule.table)
                .text(" AS s WHERE s.")
                .name(column)
                .text(" IS NOT NULL AND NOT EXISTS (SELECT FROM ");
            w.table(target)
                .text(" AS m WHERE m.")
                .name(target_column)
                .text(" = s.")
                .name(column)
                .text(")");
        }
        RuleCheck::Custom { sql, .. } => {
            w.text("SELECT NULL FROM (").typed(sql.trim().trim_end_matches(';')).text(") AS hn_rule");
        }
        RuleCheck::OnlyGrows => return Err("Halfnine checks this rule from what the change wrote".into()),
    }
    w.text(&format!(" LIMIT {EXAMPLES}"));
    Ok(w.step(rule.words.clone(), None).run)
}

/// The refusal a reader sees, each rule in the words they may see.
fn refusal(broken: &[Broken], hidden: &[ColumnRef]) -> Error {
    let each: Vec<String> = broken
        .iter()
        .map(|b| format!("“{}” on {}: {}", b.rule.shown(hidden), b.rule.table.name, b.detail))
        .collect();
    let rules = if broken.len() == 1 { "a rule" } else { "rules" };
    Error::new(
        ErrorKind::BrokenRule,
        format!(
            "Nothing was written: this change breaks {rules} set for this database. {}.",
            each.join(". ")
        ),
    )
}

#[cfg(test)]
mod tests {
    use hn_core::RuleSource;

    use super::*;

    fn column(name: &str, type_name: &str, key: Option<i32>) -> ColumnMeta {
        ColumnMeta {
            name: name.into(),
            type_name: type_name.into(),
            key_position: key,
            generated: false,
            identity: false,
        }
    }

    fn rule(check: RuleCheck) -> Rule {
        let table = TableRef {
            schema: "public".into(),
            name: "say \"hi\"".into(),
        };
        Rule {
            id: "r".into(),
            words: check.sentence(&table, true).unwrap_or_else(|| "custom".into()),
            table,
            check,
            source: RuleSource::Written,
            added_ms: 0.0,
        }
    }

    fn columns() -> Vec<ColumnMeta> {
        vec![
            column("id", "bigint", Some(1)),
            column("status", "text", None),
            column("amount", "numeric(12,2)", None),
        ]
    }

    #[test]
    fn finds_the_rows_that_break_each_rule_with_values_bound() {
        let keys = ["id".to_string()];
        let one_of = rule(RuleCheck::OneOf {
            column: "status".into(),
            values: vec!["draft".into(), "it's".into()],
        });
        let built = query(&one_of, &columns(), &keys).unwrap();
        assert_eq!(
            built.sql,
            "SELECT \"id\"::text FROM \"public\".\"say \"\"hi\"\"\" WHERE \"status\" IS NOT NULL AND \"status\" NOT IN ($1::text::text, $2::text::text) LIMIT 3"
        );
        assert_eq!(built.params, [Some("draft".to_string()), Some("it's".to_string())]);

        let between = rule(RuleCheck::Between {
            column: "amount".into(),
            at_least: Some("0".into()),
            at_most: None,
        });
        let built = query(&between, &columns(), &keys).unwrap();
        assert!(
            built.sql.contains("WHERE false OR \"amount\" < $1::text::numeric(12,2) LIMIT 3"),
            "{}",
            built.sql
        );

        let matched = rule(RuleCheck::HasMatch {
            column: "status".into(),
            target: TableRef {
                schema: "public".into(),
                name: "statuses".into(),
            },
            target_column: "name".into(),
        });
        let built = query(&matched, &columns(), &[]).unwrap();
        assert!(built.sql.starts_with("SELECT NULL FROM"), "a hidden key names no rows: {}", built.sql);
        assert!(
            built
                .sql
                .contains("NOT EXISTS (SELECT FROM \"public\".\"statuses\" AS m WHERE m.\"name\" = s.\"status\")")
        );
    }

    #[test]
    fn refuses_a_rule_whose_column_the_change_removed() {
        let filled = rule(RuleCheck::AlwaysFilled { column: "gone".into() });
        assert_eq!(query(&filled, &columns(), &[]).unwrap_err(), "it removes gone, which the rule checks");
    }

    #[test]
    fn finds_every_table_a_plan_reads_once() {
        let plan = serde_json::json!([{"Plan": {"Node Type": "Hash Join", "Plans": [
            {"Node Type": "Seq Scan", "Schema": "public", "Relation Name": "invoices"},
            {"Node Type": "Hash", "Plans": [{"Node Type": "Seq Scan", "Schema": "public", "Relation Name": "customers"}]},
            {"Node Type": "Seq Scan", "Schema": "public", "Relation Name": "invoices"}
        ]}}]);
        let mut names: Vec<String> = relations(&plan).into_iter().map(|t| t.name).collect();
        names.sort();
        assert_eq!(names, ["customers", "invoices"]);
    }

    #[test]
    fn names_rows_by_a_key_only_when_the_reader_may_see_it() {
        let table = TableRef {
            schema: "public".into(),
            name: "t".into(),
        };
        assert_eq!(keys(&columns(), &table, &[]), ["id"]);
        let hidden = [ColumnRef {
            schema: "public".into(),
            table: "t".into(),
            column: "id".into(),
        }];
        assert!(keys(&columns(), &table, &hidden).is_empty());
    }
}
