//! Undo for an agent's row changes. When `change_rows` commits, the reverse is kept here: the rows it changed set back,
//! the rows it deleted put back as they were, and the rows it added deleted. Undoing stages that reverse in the table
//! like the person's own edits, so it is reviewed and committed the usual way, and refused if a row moved on since.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hn_core::{CellChange, Changes, ColumnValue, Error, Filter, RowUpdate, TableRef};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::files;

const FILE: &str = "agent-undo.json";
const VERSION: u32 = 1;
/// The most changes kept to undo; older ones drop off.
const KEEP: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UndoRecord {
    /// The agent call that made the change, as the activity log names it.
    pub call: String,
    pub agent: String,
    pub connection_id: String,
    pub database: String,
    pub table: TableRef,
    /// The reverse of the change, to stage.
    pub changes: Changes,
    /// The rows it touches that exist now, so the table shows those.
    pub filter: Filter,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    records: Vec<UndoRecord>,
}

pub struct Undos {
    path: PathBuf,
    records: Mutex<Vec<UndoRecord>>,
}

impl Undos {
    pub fn open(dir: &Path) -> Self {
        let path = dir.join(FILE);
        let records = files::load::<Saved>(&path, VERSION, "the changes agents made").map_or_else(|_| Vec::new(), |s| s.records);
        Self {
            path,
            records: Mutex::new(records),
        }
    }

    fn change(&self, edit: impl FnOnce(&mut Vec<UndoRecord>)) -> Result<(), Error> {
        let mut records = self.records.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut next = records.clone();
        edit(&mut next);
        files::save(&self.path, VERSION, &Saved { records: next.clone() }, "the changes agents made")?;
        *records = next;
        Ok(())
    }

    pub fn keep(&self, record: UndoRecord) -> Result<(), Error> {
        self.change(|records| {
            records.push(record);
            let extra = records.len().saturating_sub(KEEP);
            records.drain(..extra);
        })
    }

    pub fn get(&self, call: &str) -> Option<UndoRecord> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .find(|r| r.call == call)
            .cloned()
    }

    /// Once the reverse is committed, there is nothing left to undo.
    pub fn forget(&self, call: &str) -> Result<(), Error> {
        self.change(|records| records.retain(|r| r.call != call))
    }
}

/// The reverse of committed row changes, and the rows it touches that exist now. `restore` is each deleted row whole,
/// and `inserted` the key of each row added.
pub fn reverse(changes: &Changes, restore: &[Vec<ColumnValue>], inserted: &[Vec<ColumnValue>]) -> Option<(Changes, Filter)> {
    let Changes::Rows { table, updates, .. } = changes else {
        return None;
    };
    let updates: Vec<RowUpdate> = updates
        .iter()
        .map(|u| RowUpdate {
            // A changed key column names the row by its new value now.
            key: u
                .key
                .iter()
                .map(|k| ColumnValue {
                    column: k.column.clone(),
                    value: u
                        .cells
                        .iter()
                        .find(|c| c.column == k.column)
                        .map_or_else(|| k.value.clone(), |c| c.to.clone()),
                })
                .collect(),
            cells: u
                .cells
                .iter()
                .map(|c| CellChange {
                    column: c.column.clone(),
                    from: c.to.clone(),
                    to: c.from.clone(),
                })
                .collect(),
        })
        .collect();
    let keys = updates.iter().map(|u| u.key.clone()).chain(inserted.iter().cloned()).collect();
    Some((
        Changes::Rows {
            table: table.clone(),
            deletes: inserted.to_vec(),
            updates,
            inserts: vec![],
            restores: restore.to_vec(),
        },
        Filter::Keys { keys },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(column: &str, value: &str) -> ColumnValue {
        ColumnValue {
            column: column.into(),
            value: Some(value.into()),
        }
    }

    #[test]
    fn reverses_each_kind_of_row_change() {
        let table = TableRef {
            schema: "public".into(),
            name: "users".into(),
        };
        let change = Changes::Rows {
            table: table.clone(),
            deletes: vec![vec![value("id", "2")]],
            updates: vec![RowUpdate {
                key: vec![value("id", "1")],
                cells: vec![
                    CellChange {
                        column: "id".into(),
                        from: Some("1".into()),
                        to: Some("10".into()),
                    },
                    CellChange {
                        column: "name".into(),
                        from: None,
                        to: Some("Ana".into()),
                    },
                ],
            }],
            inserts: vec![vec![value("name", "Bo")]],
            restores: vec![],
        };
        let restore = vec![vec![value("id", "2"), value("name", "Cy")]];
        let inserted = vec![vec![value("id", "3")]];
        let (reversed, filter) = reverse(&change, &restore, &inserted).unwrap();
        let Changes::Rows {
            deletes,
            updates,
            inserts,
            restores,
            ..
        } = reversed
        else {
            panic!("rows")
        };
        assert_eq!(deletes, inserted);
        assert_eq!((inserts, restores), (vec![], restore));
        assert_eq!(updates[0].key, [value("id", "10")]);
        assert_eq!(
            (updates[0].cells[1].from.as_deref(), updates[0].cells[1].to.as_deref()),
            (Some("Ana"), None)
        );
        assert_eq!(
            filter,
            Filter::Keys {
                keys: vec![vec![value("id", "10")], vec![value("id", "3")]]
            }
        );
        assert!(reverse(&Changes::Structure { table, changes: vec![] }, &[], &[]).is_none());
    }
}
