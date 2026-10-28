//! The editor's history (the last 500 runs) and its saved queries, each a small file beside the saved connections.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use specta::Type;
use tu_core::Error;

use crate::files;

const VERSION: u32 = 1;
const HISTORY_LIMIT: usize = 500;

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub sql: String,
    pub connection_id: String,
    pub database: String,
    /// When it ran, in milliseconds since 1970 (UTC).
    pub ran_at_ms: f64,
    pub elapsed_ms: u32,
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub sql: String,
}

#[derive(Default, Serialize, Deserialize)]
struct History {
    entries: Vec<HistoryEntry>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    queries: Vec<SavedQuery>,
}

pub struct Queries {
    history_path: PathBuf,
    saved_path: PathBuf,
    history: Mutex<Vec<HistoryEntry>>,
    saved: Mutex<Vec<SavedQuery>>,
}

impl Queries {
    pub fn open(dir: PathBuf) -> Result<Self, Error> {
        let history_path = dir.join("history.json");
        let saved_path = dir.join("saved-queries.json");
        let history: History = files::load(&history_path, VERSION, "the query history")?;
        let saved: Saved = files::load(&saved_path, VERSION, "the saved queries")?;
        Ok(Self {
            history_path,
            saved_path,
            history: Mutex::new(history.entries),
            saved: Mutex::new(saved.queries),
        })
    }

    /// Newest first, keeping the last [`HISTORY_LIMIT`].
    pub fn history(&self) -> Vec<HistoryEntry> {
        lock(&self.history).clone()
    }

    pub fn record(&self, entry: HistoryEntry) -> Result<(), Error> {
        let mut entries = lock(&self.history);
        let mut next = Vec::with_capacity(HISTORY_LIMIT);
        next.push(entry);
        next.extend(entries.iter().take(HISTORY_LIMIT - 1).cloned());
        files::save(&self.history_path, VERSION, &History { entries: next.clone() }, "the query history")?;
        *entries = next;
        Ok(())
    }

    pub fn saved(&self) -> Vec<SavedQuery> {
        lock(&self.saved).clone()
    }

    pub fn save(&self, query: SavedQuery) -> Result<(), Error> {
        let mut queries = lock(&self.saved);
        let mut next = queries.clone();
        match next.iter_mut().find(|q| q.id == query.id) {
            Some(existing) => *existing = query,
            None => next.push(query),
        }
        next.sort_by_key(|q| q.name.to_lowercase());
        files::save(&self.saved_path, VERSION, &Saved { queries: next.clone() }, "the saved queries")?;
        *queries = next;
        Ok(())
    }

    pub fn forget(&self, id: &str) -> Result<(), Error> {
        let mut queries = lock(&self.saved);
        let next: Vec<_> = queries.iter().filter(|q| q.id != id).cloned().collect();
        files::save(&self.saved_path, VERSION, &Saved { queries: next.clone() }, "the saved queries")?;
        *queries = next;
        Ok(())
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: usize) -> HistoryEntry {
        HistoryEntry {
            id: n.to_string(),
            sql: format!("SELECT {n}"),
            connection_id: "c".into(),
            database: "d".into(),
            ran_at_ms: n as f64,
            elapsed_ms: 1,
            ok: true,
        }
    }

    #[test]
    fn keeps_the_newest_runs_and_the_saved_queries_across_reopening() {
        let dir = std::env::temp_dir().join(format!("tableultra-queries-{}", uuid::Uuid::new_v4()));
        let queries = Queries::open(dir.clone()).unwrap();
        for n in 0..HISTORY_LIMIT + 5 {
            queries.record(entry(n)).unwrap();
        }
        queries
            .save(SavedQuery {
                id: "b".into(),
                name: "Overdue".into(),
                sql: "SELECT 2".into(),
            })
            .unwrap();
        queries
            .save(SavedQuery {
                id: "a".into(),
                name: "accounts".into(),
                sql: "SELECT 1".into(),
            })
            .unwrap();
        queries
            .save(SavedQuery {
                id: "b".into(),
                name: "Overdue".into(),
                sql: "SELECT 3".into(),
            })
            .unwrap();

        let reopened = Queries::open(dir.clone()).unwrap();
        let history = reopened.history();
        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history[0], entry(HISTORY_LIMIT + 4));
        let saved: Vec<_> = reopened.saved().into_iter().map(|q| (q.id, q.sql)).collect();
        assert_eq!(
            saved,
            [("a".to_string(), "SELECT 1".to_string()), ("b".to_string(), "SELECT 3".to_string())]
        );
        reopened.forget("a").unwrap();
        assert_eq!(Queries::open(dir.clone()).unwrap().saved().len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
