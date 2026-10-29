//! Turns what an agent asked to change into the changes the window stages and the commit runs. Row changes are
//! matched to the rows as they are now, read by Halfnine itself, so the commit's own check refuses a row that changes
//! before the person approves.

use std::collections::HashSet;

use hn_core::{
    CellChange, Changes, ColumnValue, Database, Error, ErrorKind, Filter, Page, PageRequest, RelationKind, RowUpdate, StructureChange, TableRef,
};
use hn_pg::PgDatabase;
use serde_json::{Map, Value};

use super::Staged;
use super::Work;
use super::results::{Reply, Status, plural};
use super::tools::{ROWS_LIMIT, RowChange, STATEMENTS_LIMIT, cell, column_values};

pub struct Request {
    pub connection: String,
    pub database: Option<String>,
    pub work: Asked,
}

pub enum Asked {
    Rows {
        table: TableRef,
        insert: Vec<Map<String, Value>>,
        update: Vec<RowChange>,
        delete: Vec<Map<String, Value>>,
    },
    Structure {
        table: TableRef,
        changes: Vec<StructureChange>,
    },
    Statements(Vec<String>),
}

impl Request {
    /// Whether it drops or empties a table, which always asks the person.
    pub fn drops(&self) -> bool {
        match &self.work {
            Asked::Rows { .. } => false,
            Asked::Structure { changes, .. } => changes.iter().any(|c| matches!(c, StructureChange::Drop | StructureChange::Truncate)),
            Asked::Statements(statements) => statements.iter().any(|s| drops(s)),
        }
    }
}

fn drops(statement: &str) -> bool {
    matches!(hn_pg::first_word(statement).as_str(), "DROP" | "TRUNCATE")
}

fn refused(text: impl Into<String>) -> Reply {
    Reply::new(Status::Refused, text)
}

/// `None` when there is nothing to change.
pub async fn stage(db: &PgDatabase, asked: Asked) -> Result<Option<Staged>, Reply> {
    match asked {
        Asked::Rows {
            table,
            insert,
            update,
            delete,
        } => rows(db, table, insert, update, delete).await,
        Asked::Structure { table, changes } => {
            if changes.is_empty() {
                return Err(refused("Give at least one change."));
            }
            let drops = changes.iter().any(|c| matches!(c, StructureChange::Drop | StructureChange::Truncate));
            let summary = match changes.first() {
                Some(StructureChange::CreateTable { .. }) => format!("create the table {}", table.name),
                _ if changes.iter().any(|c| matches!(c, StructureChange::Drop)) => format!("drop the table {}", table.name),
                _ if changes.iter().any(|c| matches!(c, StructureChange::Truncate)) => format!("empty the table {}", table.name),
                _ => format!("change the structure of {}", table.name),
            };
            let work = Changes::Structure {
                table: table.clone(),
                changes,
            };
            let shown = db.review(&work).await?;
            Ok(Some(Staged {
                work: Work::Changes(work),
                shown,
                table: Some(table),
                filter: None,
                rows: 0,
                summary,
                drops,
                restore: None,
            }))
        }
        Asked::Statements(statements) => {
            let statements: Vec<String> = statements.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if statements.is_empty() || statements.len() > STATEMENTS_LIMIT {
                return Err(refused(format!("Give between 1 and {STATEMENTS_LIMIT} statements.")));
            }
            if let Some(word) = statements
                .iter()
                .map(|s| hn_pg::first_word(s))
                .find(|w| hn_pg::NOT_IN_A_CHANGE.contains(&w.as_str()))
            {
                return Err(refused(format!(
                    "{word} isn't allowed: Halfnine runs the statements as one transaction of its own."
                )));
            }
            Ok(Some(Staged {
                summary: format!("run {}", plural(statements.len(), "statement")),
                drops: statements.iter().any(|s| drops(s)),
                shown: statements.clone(),
                work: Work::Statements(statements),
                table: None,
                filter: None,
                rows: 0,
                restore: None,
            }))
        }
    }
}

/// A row as it is now, and which of its values were too long to compare.
struct Current {
    values: Vec<Option<String>>,
    cut: HashSet<usize>,
}

async fn rows(
    db: &PgDatabase,
    table: TableRef,
    insert: Vec<Map<String, Value>>,
    update: Vec<RowChange>,
    delete: Vec<Map<String, Value>>,
) -> Result<Option<Staged>, Reply> {
    let total = insert.len() + update.len() + delete.len();
    if total == 0 {
        return Err(refused("Nothing to change: give insert, update or delete."));
    }
    if total > ROWS_LIMIT {
        return Err(refused(format!(
            "change_rows takes at most {ROWS_LIMIT} rows at once. Use run_sql for bulk changes."
        )));
    }
    let structure = db.structure(&table).await?;
    if !matches!(structure.kind, RelationKind::Table | RelationKind::PartitionedTable) {
        return Err(refused(format!("{} isn't a table, so its rows can't be changed here.", table.name)));
    }
    let key = structure.primary_key.clone();
    if key.is_empty() && !(update.is_empty() && delete.is_empty()) {
        return Err(refused(format!(
            "{} has no primary key, so Halfnine can't tell its rows apart. Use run_sql.",
            table.name
        )));
    }
    let key_of = |map: &Map<String, Value>| -> Result<Vec<ColumnValue>, Reply> {
        let named: HashSet<&str> = map.keys().map(String::as_str).collect();
        if named != key.iter().map(String::as_str).collect() {
            return Err(refused(format!(
                "Name each row of {} by its whole primary key: {}.",
                table.name,
                key.join(", ")
            )));
        }
        Ok(key
            .iter()
            .map(|column| ColumnValue {
                column: column.clone(),
                value: cell(&map[column]),
            })
            .collect())
    };
    let update_keys = update.iter().map(|u| key_of(&u.key)).collect::<Result<Vec<_>, _>>()?;
    let delete_keys = delete.iter().map(key_of).collect::<Result<Vec<_>, _>>()?;
    let wanted: Vec<Vec<ColumnValue>> = update_keys.iter().chain(&delete_keys).cloned().collect();
    let found = if wanted.is_empty() {
        Vec::new()
    } else {
        current(db, &table, &key, &wanted).await?
    };
    let (found_updates, found_deletes) = found.split_at(update_keys.len());

    let generated: HashSet<&str> = structure.columns.iter().filter(|c| c.generated).map(|c| c.name.as_str()).collect();
    let mut updates = Vec::new();
    let mut touched = Vec::new();
    for (change, (row, printed_key, columns)) in update.iter().zip(found_updates) {
        let mut cells = Vec::new();
        for (column, value) in &change.set {
            let at = columns
                .iter()
                .position(|c| c == column)
                .ok_or_else(|| refused(format!("{} has no column named {column}.", table.name)))?;
            if generated.contains(column.as_str()) {
                return Err(refused(format!("Postgres fills {column} itself, so it can't be written.")));
            }
            if row.cut.contains(&at) {
                return Err(refused(format!(
                    "The current value of {column} is too long for Halfnine to compare. Use run_sql for this row."
                )));
            }
            let to = cell(value);
            if row.values[at] != to {
                cells.push(CellChange {
                    column: column.clone(),
                    from: row.values[at].clone(),
                    to,
                });
            }
        }
        if !cells.is_empty() {
            touched.push(printed_key.clone());
            updates.push(RowUpdate {
                key: printed_key.clone(),
                cells,
            });
        }
    }
    let deletes: Vec<Vec<ColumnValue>> = found_deletes.iter().map(|(_, printed_key, _)| printed_key.clone()).collect();
    // Each deleted row whole, to put back if the change is undone. A column Postgres computes is left for it to compute
    // again, and a row with a value too long to have been read whole can't be put back exactly.
    let computed: HashSet<&str> = structure
        .columns
        .iter()
        .filter(|c| c.generated && !c.default.as_deref().is_some_and(|d| d.starts_with("generated always as identity")))
        .map(|c| c.name.as_str())
        .collect();
    let restore = found_deletes
        .iter()
        .map(|(row, _, columns)| {
            row.cut.is_empty().then(|| {
                columns
                    .iter()
                    .zip(&row.values)
                    .filter(|(column, _)| !computed.contains(column.as_str()))
                    .map(|(column, value)| ColumnValue {
                        column: column.clone(),
                        value: value.clone(),
                    })
                    .collect()
            })
        })
        .collect::<Option<Vec<Vec<ColumnValue>>>>();
    touched.extend(deletes.iter().cloned());
    let inserts: Vec<Vec<ColumnValue>> = insert.iter().map(column_values).collect();

    let rows = updates.len() + deletes.len() + inserts.len();
    if rows == 0 {
        return Ok(None);
    }
    let summary = format!("change {} in {}", plural(rows, "row"), table.name);
    let work = Changes::Rows {
        table: table.clone(),
        deletes,
        updates,
        inserts,
    };
    let shown = db.review(&work).await?;
    Ok(Some(Staged {
        work: Work::Changes(work),
        shown,
        table: Some(table),
        filter: Some(Filter::Keys { keys: touched }),
        rows,
        summary,
        drops: false,
        restore,
    }))
}

/// Each wanted row as it is now, with its key as Postgres prints it and the page's column names, in the order asked.
async fn current(
    db: &PgDatabase,
    table: &TableRef,
    key: &[String],
    wanted: &[Vec<ColumnValue>],
) -> Result<Vec<(Current, Vec<ColumnValue>, Vec<String>)>, Reply> {
    let all = page(db, table, Filter::Keys { keys: wanted.to_vec() }, wanted.len()).await?;
    let mut found = Vec::with_capacity(wanted.len());
    for asked in wanted {
        if let Some(row) = matching(&all, key, asked) {
            found.push(extract(&all, row, key));
            continue;
        }
        // Postgres may print a key differently from how it was written, such as a uuid in capitals, so a key not
        // found by its text is looked up on its own.
        let alone = page(db, table, Filter::Equals { values: asked.clone() }, 1).await?;
        if alone.rows.len() != 1 {
            let named: Vec<String> = asked
                .iter()
                .map(|v| format!("{} = {}", v.column, v.value.as_deref().unwrap_or("NULL")))
                .collect();
            return Err(refused(format!("{} has no row where {}.", table.name, named.join(" and "))));
        }
        found.push(extract(&alone, 0, key));
    }
    Ok(found)
}

fn extract(page: &Page, row: usize, key: &[String]) -> (Current, Vec<ColumnValue>, Vec<String>) {
    let columns: Vec<String> = page.columns.iter().map(|c| c.name.clone()).collect();
    let values = page.rows[row].clone();
    let printed = key
        .iter()
        .filter_map(|k| {
            let at = columns.iter().position(|c| c == k)?;
            Some(ColumnValue {
                column: k.clone(),
                value: values[at].clone(),
            })
        })
        .collect();
    let cut = page.cut.iter().filter(|c| c.row as usize == row).map(|c| c.column as usize).collect();
    (Current { values, cut }, printed, columns)
}

async fn page(db: &PgDatabase, table: &TableRef, filter: Filter, rows: usize) -> Result<Page, Error> {
    db.page(&PageRequest {
        table: table.clone(),
        sort: None,
        filter: Some(filter),
        offset: 0,
        limit: (rows as u32 + 1).min(1_000),
    })
    .await
    .map_err(|e| match e.kind {
        ErrorKind::Query => Error {
            message: format!("Halfnine couldn't read the rows to change: {}", e.message),
            ..e
        },
        _ => e,
    })
}

fn matching(page: &Page, key: &[String], asked: &[ColumnValue]) -> Option<usize> {
    let at: Vec<usize> = key.iter().filter_map(|k| page.columns.iter().position(|c| &c.name == k)).collect();
    page.rows.iter().position(|row| at.iter().zip(asked).all(|(&i, v)| row[i] == v.value))
}
