//! The SQL that commits staged changes. Each statement is written twice in one pass: to run, with identifiers through
//! [`ident`] and values as bound parameters, and to show, with the values written in and identifiers quoted only where
//! Postgres needs it. Both come from the same calls, so what someone reviews is what runs.

use hn_core::{Cell, ColumnValue, Error, ErrorKind, NewColumn, RelationKind, RowUpdate, StructureChange, TableRef};

use crate::sql::{Built, ColumnMeta, find, ident};

/// One statement of a commit.
#[derive(Debug)]
pub(crate) struct Step {
    pub run: Built,
    pub shown: String,
    /// What it does, in words, for an error that names it.
    pub what: String,
    /// Set when the statement must change exactly one row: the error to give when it doesn't.
    pub conflict: Option<String>,
    /// Set when the statement returns the primary key of the row it added.
    pub returns_key: bool,
}

#[derive(Default)]
struct Writer {
    sql: String,
    shown: String,
    params: Vec<Cell>,
}

impl Writer {
    fn text(&mut self, text: &str) -> &mut Self {
        self.sql.push_str(text);
        self.shown.push_str(text);
        self
    }

    fn name(&mut self, name: &str) -> &mut Self {
        self.sql.push_str(&ident(name));
        self.shown.push_str(&shown_ident(name));
        self
    }

    fn table(&mut self, table: &TableRef) -> &mut Self {
        self.name(&table.schema).text(".").name(&table.name)
    }

    fn names<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) -> &mut Self {
        for (i, name) in names.into_iter().enumerate() {
            if i > 0 {
                self.text(", ");
            }
            self.name(name);
        }
        self
    }

    /// A value Postgres reads with the column's own type, so it does every conversion itself.
    fn value(&mut self, value: &Cell, column: &ColumnMeta) -> &mut Self {
        let Some(text) = value else { return self.text("NULL") };
        self.params.push(Some(text.clone()));
        self.sql.push_str(&format!("${}::text::{}", self.params.len(), column.type_name));
        self.shown.push_str(&literal(text, &column.type_name));
        self
    }

    /// Whether the column still prints as `value`. Comparing the text works for every type, even those without `=`.
    fn holds(&mut self, column: &ColumnMeta, value: &Cell) -> &mut Self {
        let Some(text) = value else {
            return self.name(&column.name).text(" IS NULL");
        };
        self.params.push(Some(text.clone()));
        self.sql.push_str(&format!("{}::text = ${}", ident(&column.name), self.params.len()));
        self.shown.push_str(&format!("{}::text = {}", shown_ident(&column.name), quote(text)));
        self
    }

    /// Picks out one row by its primary key, and returns how to name that row in words.
    fn key(&mut self, key: &[&ColumnMeta], values: &[ColumnValue]) -> Result<String, Error> {
        let mut words = Vec::new();
        for (i, column) in key.iter().enumerate() {
            let value = values
                .iter()
                .find(|v| v.column == column.name)
                .and_then(|v| v.value.as_ref())
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::Invalid,
                        format!("A row to change has no value for its key column {}.", column.name),
                    )
                })?;
            if i > 0 {
                self.text(" AND ");
            }
            self.name(&column.name).text(" = ").value(&Some(value.clone()), column);
            words.push(format!("{} = {}", column.name, literal(value, &column.type_name)));
        }
        Ok(words.join(" and "))
    }

    /// SQL someone typed for their own table, such as a type or a default. Postgres reads it as part of one statement.
    fn typed(&mut self, sql: &str) -> &mut Self {
        self.text(sql.trim())
    }

    fn column(&mut self, column: &NewColumn) -> Result<&mut Self, Error> {
        self.name(required(&column.name, "Name every new column.")?)
            .text(" ")
            .typed(required(&column.type_name, &format!("Give {} a type.", column.name.trim()))?);
        if !column.nullable && !column.key {
            self.text(" NOT NULL");
        }
        match column.default.as_deref().map(str::trim) {
            None | Some("") => {}
            Some(fill) if fill.to_lowercase().starts_with("generated ") => {
                self.text(" ").typed(fill);
            }
            Some(default) => {
                self.text(" DEFAULT ").typed(default);
            }
        }
        Ok(self)
    }

    fn alter(&mut self, table: &TableRef) -> &mut Self {
        self.text("ALTER TABLE ").table(table).text(" ")
    }

    fn step(self, what: String, conflict: Option<String>) -> Step {
        Step {
            run: Built {
                sql: self.sql,
                params: self.params,
            },
            shown: format!("{};", self.shown),
            what,
            conflict,
            returns_key: false,
        }
    }
}

/// The statements for staged row changes: deletes, then updates, then inserts, so a new row can take a deleted one's place.
pub(crate) fn rows(
    table: &TableRef,
    deletes: &[Vec<ColumnValue>],
    updates: &[RowUpdate],
    inserts: &[Vec<ColumnValue>],
    columns: &[ColumnMeta],
) -> Result<Vec<Step>, Error> {
    let mut key: Vec<&ColumnMeta> = columns.iter().filter(|c| c.key_position.is_some()).collect();
    key.sort_by_key(|c| c.key_position);
    if key.is_empty() && (!deletes.is_empty() || !updates.is_empty()) {
        return Err(Error::new(
            ErrorKind::Invalid,
            format!("{} has no primary key, so a row can't be told apart from its twins.", table.name),
        ));
    }

    let mut steps = Vec::new();
    for row in deletes {
        let mut w = Writer::default();
        w.text("DELETE FROM ").table(table).text("\n WHERE ");
        let row = w.key(&key, row)?;
        let gone = format!("The row where {row} was already gone, so nothing was written. Refresh to see the table as it is now.");
        steps.push(w.step(format!("Deleting the row where {row}"), Some(gone)));
    }
    for update in updates.iter().filter(|u| !u.cells.is_empty()) {
        let mut w = Writer::default();
        w.text("UPDATE ").table(table).text("\n   SET ");
        for (i, cell) in update.cells.iter().enumerate() {
            let column = writable(columns, &cell.column)?;
            if i > 0 {
                w.text(", ");
            }
            w.name(&column.name).text(" = ").value(&cell.to, column);
        }
        w.text("\n WHERE ");
        let row = w.key(&key, &update.key)?;
        for cell in &update.cells {
            w.text(" AND ").holds(find(columns, &cell.column)?, &cell.from);
        }
        let changed =
            format!("The row where {row} changed after you loaded it, so nothing was written. Refresh to see it, then make the change again.");
        steps.push(w.step(format!("Updating the row where {row}"), Some(changed)));
    }
    for (n, row) in inserts.iter().enumerate() {
        let mut w = Writer::default();
        w.text("INSERT INTO ").table(table);
        if row.is_empty() {
            w.text(" DEFAULT VALUES");
        } else {
            let targets = row.iter().map(|v| insertable(columns, &v.column)).collect::<Result<Vec<_>, _>>()?;
            w.text(" (").names(targets.iter().map(|c| c.name.as_str())).text(")");
            // Only a row being put back, as when an agent's delete is undone, names an identity column.
            if targets.iter().any(|c| c.identity) {
                w.text("\nOVERRIDING SYSTEM VALUE");
            }
            w.text("\nVALUES (");
            for (i, (value, column)) in row.iter().zip(&targets).enumerate() {
                if i > 0 {
                    w.text(", ");
                }
                w.value(&value.value, column);
            }
            w.text(")");
        }
        let returns_key = !key.is_empty();
        if returns_key {
            w.text("\nRETURNING ");
            for (i, column) in key.iter().enumerate() {
                if i > 0 {
                    w.text(", ");
                }
                w.name(&column.name).text("::text");
            }
        }
        let mut step = w.step(format!("Adding new row {}", n + 1), None);
        step.returns_key = returns_key;
        steps.push(step);
    }
    Ok(steps)
}

/// The statements for structure changes, in an order that works whatever order they were made in: removals first,
/// then changes to existing columns under their old names, then renames, then additions that may use the new names.
pub(crate) fn structure(table: &TableRef, kind: Option<RelationKind>, changes: &[StructureChange]) -> Result<Vec<Step>, Error> {
    let mut ordered: Vec<&StructureChange> = changes.iter().collect();
    ordered.sort_by_key(|change| match change {
        StructureChange::CreateTable { .. } | StructureChange::Truncate | StructureChange::Drop => 0,
        StructureChange::DropIndex { .. } => 1,
        StructureChange::DropColumn { .. } => 2,
        StructureChange::SetType { .. } => 3,
        StructureChange::SetNullable { .. } => 4,
        StructureChange::SetDefault { .. } => 5,
        StructureChange::RenameColumn { .. } => 6,
        StructureChange::AddColumn { .. } => 7,
        StructureChange::CreateIndex { .. } => 8,
    });
    ordered.into_iter().map(|change| structure_step(table, kind, change)).collect()
}

fn structure_step(table: &TableRef, kind: Option<RelationKind>, change: &StructureChange) -> Result<Step, Error> {
    let mut w = Writer::default();
    let what = match change {
        StructureChange::CreateTable { columns } => {
            required(&table.name, "Name the table.")?;
            w.text("CREATE TABLE ").table(table).text(" (");
            for (i, column) in columns.iter().enumerate() {
                w.text(if i == 0 { "\n  " } else { ",\n  " }).column(column)?;
            }
            let key: Vec<&str> = columns.iter().filter(|c| c.key).map(|c| c.name.trim()).collect();
            if !key.is_empty() {
                w.text(",\n  PRIMARY KEY (").names(key).text(")");
            }
            w.text("\n)");
            format!("Creating {}", table.name)
        }
        StructureChange::AddColumn { column } => {
            w.alter(table).text("ADD COLUMN ").column(column)?;
            format!("Adding the column {}", column.name.trim())
        }
        StructureChange::RenameColumn { column, to } => {
            w.alter(table)
                .text("RENAME COLUMN ")
                .name(column)
                .text(" TO ")
                .name(required(to, "Name every column.")?);
            format!("Renaming {column}")
        }
        StructureChange::SetType { column, to } => {
            let to = required(to, &format!("Give {column} a type."))?;
            w.alter(table).text("ALTER COLUMN ").name(column).text(" TYPE ").typed(to);
            w.text(" USING ").name(column).text("::").typed(to);
            format!("Changing the type of {column}")
        }
        StructureChange::SetNullable { column, nullable } => {
            w.alter(table)
                .text("ALTER COLUMN ")
                .name(column)
                .text(if *nullable { " DROP NOT NULL" } else { " SET NOT NULL" });
            format!("Changing whether {column} may be NULL")
        }
        StructureChange::SetDefault { column, to } => {
            w.alter(table).text("ALTER COLUMN ").name(column);
            match to.as_deref().map(str::trim) {
                None | Some("") => w.text(" DROP DEFAULT"),
                Some(default) => w.text(" SET DEFAULT ").typed(default),
            };
            format!("Changing the default of {column}")
        }
        StructureChange::DropColumn { column } => {
            w.alter(table).text("DROP COLUMN ").name(column);
            format!("Dropping the column {column}")
        }
        StructureChange::CreateIndex { columns, unique } => {
            if columns.is_empty() {
                return Err(Error::new(ErrorKind::Invalid, "Choose the columns of each new index."));
            }
            w.text(if *unique { "CREATE UNIQUE INDEX ON " } else { "CREATE INDEX ON " })
                .table(table)
                .text(" (")
                .names(columns.iter().map(String::as_str))
                .text(")");
            format!("Adding the index on {}", columns.join(", "))
        }
        StructureChange::DropIndex { name } => {
            w.text("DROP INDEX ").name(&table.schema).text(".").name(name);
            format!("Dropping the index {name}")
        }
        StructureChange::Truncate => {
            w.text("TRUNCATE ").table(table);
            format!("Emptying {}", table.name)
        }
        StructureChange::Drop => {
            let object = match kind {
                Some(RelationKind::Table | RelationKind::PartitionedTable) => "TABLE",
                Some(RelationKind::View) => "VIEW",
                Some(RelationKind::MaterializedView) => "MATERIALIZED VIEW",
                Some(RelationKind::ForeignTable) => "FOREIGN TABLE",
                None => {
                    return Err(Error::new(
                        ErrorKind::NotFound,
                        format!("There is no table or view named {}.{}.", table.schema, table.name),
                    ));
                }
            };
            w.text("DROP ").text(object).text(" ").table(table);
            format!("Dropping {}", table.name)
        }
    };
    Ok(w.step(what, None))
}

fn required<'a>(text: &'a str, otherwise: &str) -> Result<&'a str, Error> {
    match text.trim() {
        "" => Err(Error::new(ErrorKind::Invalid, otherwise)),
        text => Ok(text),
    }
}

fn writable<'a>(columns: &'a [ColumnMeta], name: &str) -> Result<&'a ColumnMeta, Error> {
    let column = find(columns, name)?;
    if column.generated {
        return Err(Error::new(
            ErrorKind::Invalid,
            format!("Postgres fills {name} itself, so it can't be written."),
        ));
    }
    Ok(column)
}

/// A column a new row may name: any but one Postgres computes itself. An identity column can be written, which puts
/// a deleted row back as it was.
fn insertable<'a>(columns: &'a [ColumnMeta], name: &str) -> Result<&'a ColumnMeta, Error> {
    let column = find(columns, name)?;
    if column.generated && !column.identity {
        return Err(Error::new(
            ErrorKind::Invalid,
            format!("Postgres computes {name} itself, so it can't be written."),
        ));
    }
    Ok(column)
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// A value as it reads in SQL: numbers bare in a numeric column, everything else quoted.
fn literal(text: &str, type_name: &str) -> String {
    let numeric = ["smallint", "integer", "bigint", "numeric", "real", "double precision"]
        .iter()
        .any(|t| type_name.starts_with(t));
    let number = text.chars().any(|c| c.is_ascii_digit()) && text.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'));
    if numeric && number { text.to_string() } else { quote(text) }
}

/// A name as Postgres's own `quote_ident` writes it: bare when Postgres would read it back unchanged, quoted otherwise.
pub(crate) fn shown_ident(name: &str) -> String {
    let mut chars = name.chars();
    let plain = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !KEYWORDS.contains(&name);
    if plain { name.to_string() } else { ident(name) }
}

/// Postgres's keywords that a bare name can't be: reserved, type and function names, and column names. A test
/// checks this list against `pg_get_keywords()` on a real server.
const KEYWORDS: &[&str] = &[
    "all",
    "analyse",
    "analyze",
    "and",
    "any",
    "array",
    "as",
    "asc",
    "asymmetric",
    "authorization",
    "between",
    "bigint",
    "binary",
    "bit",
    "boolean",
    "both",
    "case",
    "cast",
    "char",
    "character",
    "check",
    "coalesce",
    "collate",
    "collation",
    "column",
    "concurrently",
    "constraint",
    "create",
    "cross",
    "current_catalog",
    "current_date",
    "current_role",
    "current_schema",
    "current_time",
    "current_timestamp",
    "current_user",
    "dec",
    "decimal",
    "default",
    "deferrable",
    "desc",
    "distinct",
    "do",
    "else",
    "end",
    "except",
    "exists",
    "extract",
    "false",
    "fetch",
    "float",
    "for",
    "foreign",
    "freeze",
    "from",
    "full",
    "grant",
    "greatest",
    "group",
    "grouping",
    "having",
    "ilike",
    "in",
    "initially",
    "inner",
    "inout",
    "int",
    "integer",
    "intersect",
    "interval",
    "into",
    "is",
    "isnull",
    "join",
    "json",
    "json_array",
    "json_arrayagg",
    "json_exists",
    "json_object",
    "json_objectagg",
    "json_query",
    "json_scalar",
    "json_serialize",
    "json_table",
    "json_value",
    "lateral",
    "leading",
    "least",
    "left",
    "like",
    "limit",
    "localtime",
    "localtimestamp",
    "merge_action",
    "national",
    "natural",
    "nchar",
    "none",
    "normalize",
    "not",
    "notnull",
    "null",
    "nullif",
    "numeric",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "out",
    "outer",
    "overlaps",
    "overlay",
    "placing",
    "position",
    "precision",
    "primary",
    "real",
    "references",
    "returning",
    "right",
    "row",
    "select",
    "session_user",
    "setof",
    "similar",
    "smallint",
    "some",
    "substring",
    "symmetric",
    "system_user",
    "table",
    "tablesample",
    "then",
    "time",
    "timestamp",
    "to",
    "trailing",
    "treat",
    "trim",
    "true",
    "union",
    "unique",
    "user",
    "using",
    "values",
    "varchar",
    "variadic",
    "verbose",
    "when",
    "where",
    "window",
    "with",
    "xmlattributes",
    "xmlconcat",
    "xmlelement",
    "xmlexists",
    "xmlforest",
    "xmlnamespaces",
    "xmlparse",
    "xmlpi",
    "xmlroot",
    "xmlserialize",
    "xmltable",
];

#[cfg(test)]
mod tests {
    use super::*;
    use hn_core::CellChange;

    fn table() -> TableRef {
        TableRef {
            schema: "public".into(),
            name: "invoices".into(),
        }
    }

    fn columns() -> Vec<ColumnMeta> {
        let column = |name: &str, type_name: &str, key_position: Option<i32>, generated: bool| ColumnMeta {
            name: name.into(),
            type_name: type_name.into(),
            key_position,
            generated,
            identity: name == "id",
        };
        vec![
            column("id", "bigint", Some(1), true),
            column("status", "text", None, false),
            column("Note", "text", None, false),
            column("doubled", "numeric", None, true),
        ]
    }

    fn value(column: &str, value: Option<&str>) -> ColumnValue {
        ColumnValue {
            column: column.into(),
            value: value.map(str::to_string),
        }
    }

    #[test]
    fn updates_by_key_and_checks_the_loaded_values() {
        let update = RowUpdate {
            key: vec![value("id", Some("1044"))],
            cells: vec![
                CellChange {
                    column: "status".into(),
                    from: Some("open".into()),
                    to: Some("paid".into()),
                },
                CellChange {
                    column: "Note".into(),
                    from: None,
                    to: Some("it's done".into()),
                },
            ],
        };
        let steps = rows(&table(), &[], &[update], &[], &columns()).unwrap();
        let step = &steps[0];
        assert_eq!(
            step.run.sql,
            "UPDATE \"public\".\"invoices\"\n   SET \"status\" = $1::text::text, \"Note\" = $2::text::text\n WHERE \"id\" = $3::text::bigint \
             AND \"status\"::text = $4 AND \"Note\" IS NULL"
        );
        assert_eq!(
            step.run.params,
            vec![Some("paid".into()), Some("it's done".into()), Some("1044".into()), Some("open".into())]
        );
        assert_eq!(
            step.shown,
            "UPDATE public.invoices\n   SET status = 'paid', \"Note\" = 'it''s done'\n WHERE id = 1044 AND status::text = 'open' AND \"Note\" IS NULL;"
        );
        assert!(step.conflict.as_deref().unwrap().starts_with("The row where id = 1044 changed"));
    }

    #[test]
    fn deletes_first_and_inserts_last() {
        let inserts = vec![vec![value("status", Some("open")), value("Note", None)], vec![]];
        let steps = rows(&table(), &[vec![value("id", Some("7"))]], &[], &inserts, &columns()).unwrap();
        let shown: Vec<&str> = steps.iter().map(|s| s.shown.as_str()).collect();
        assert_eq!(
            shown,
            [
                "DELETE FROM public.invoices\n WHERE id = 7;",
                "INSERT INTO public.invoices (status, \"Note\")\nVALUES ('open', NULL)\nRETURNING id::text;",
                "INSERT INTO public.invoices DEFAULT VALUES\nRETURNING id::text;",
            ]
        );
        assert!(steps[1].conflict.is_none() && steps[1].returns_key);
    }

    #[test]
    fn puts_a_deleted_row_back_with_its_own_identity() {
        let restore = vec![vec![value("id", Some("7")), value("status", Some("open"))]];
        let steps = rows(&table(), &[], &[], &restore, &columns()).unwrap();
        assert_eq!(
            steps[0].shown,
            "INSERT INTO public.invoices (id, status)\nOVERRIDING SYSTEM VALUE\nVALUES (7, 'open')\nRETURNING id::text;"
        );
    }

    #[test]
    fn refuses_generated_columns_and_tables_without_a_key() {
        let insert = vec![vec![value("doubled", Some("2"))]];
        assert_eq!(rows(&table(), &[], &[], &insert, &columns()).unwrap_err().kind, ErrorKind::Invalid);
        let keyless: Vec<ColumnMeta> = columns().into_iter().map(|c| ColumnMeta { key_position: None, ..c }).collect();
        let delete = vec![vec![value("id", Some("1"))]];
        assert_eq!(rows(&table(), &delete, &[], &[], &keyless).unwrap_err().kind, ErrorKind::Invalid);
    }

    #[test]
    fn orders_structure_changes_so_each_finds_what_it_needs() {
        let changes = vec![
            StructureChange::CreateIndex {
                columns: vec!["state".into()],
                unique: false,
            },
            StructureChange::RenameColumn {
                column: "status".into(),
                to: "state".into(),
            },
            StructureChange::SetType {
                column: "status".into(),
                to: "varchar(20)".into(),
            },
            StructureChange::DropIndex { name: "old_idx".into() },
        ];
        let shown: Vec<String> = structure(&table(), None, &changes).unwrap().into_iter().map(|s| s.shown).collect();
        assert_eq!(
            shown,
            [
                "DROP INDEX public.old_idx;",
                "ALTER TABLE public.invoices ALTER COLUMN status TYPE varchar(20) USING status::varchar(20);",
                "ALTER TABLE public.invoices RENAME COLUMN status TO state;",
                "CREATE INDEX ON public.invoices (state);",
            ]
        );
    }

    #[test]
    fn creates_tables_with_identity_defaults_and_keys() {
        let column = |name: &str, type_name: &str, default: Option<&str>, key: bool| NewColumn {
            name: name.into(),
            type_name: type_name.into(),
            nullable: !key,
            default: default.map(str::to_string),
            key,
        };
        let create = StructureChange::CreateTable {
            columns: vec![
                column("id", "bigint", Some("generated always as identity"), true),
                column("made", "timestamptz", Some("now()"), false),
            ],
        };
        let steps = structure(&table(), None, &[create]).unwrap();
        assert_eq!(
            steps[0].shown,
            "CREATE TABLE public.invoices (\n  id bigint generated always as identity,\n  made timestamptz DEFAULT now(),\n  PRIMARY KEY (id)\n);"
        );
    }

    #[test]
    fn drops_each_kind_with_its_own_statement() {
        let drop = |kind| structure(&table(), kind, &[StructureChange::Drop]).map(|s| s[0].shown.clone());
        assert_eq!(
            drop(Some(RelationKind::MaterializedView)).unwrap(),
            "DROP MATERIALIZED VIEW public.invoices;"
        );
        assert_eq!(drop(None).unwrap_err().kind, ErrorKind::NotFound);
    }

    #[test]
    fn shows_names_bare_only_when_postgres_would() {
        assert_eq!(shown_ident("status"), "status");
        assert_eq!(shown_ident("order"), "\"order\"");
        assert_eq!(shown_ident("Note"), "\"Note\"");
        assert_eq!(shown_ident("2fa"), "\"2fa\"");
        assert_eq!(shown_ident("a\"b"), "\"a\"\"b\"");
    }
}
