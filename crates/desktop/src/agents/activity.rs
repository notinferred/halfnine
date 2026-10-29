//! Every agent call, appended to `agent-activity.jsonl` in the config folder and never rewritten. A call's line is
//! written before Halfnine touches a database for it, and a call that can't be logged is refused. Lines hold the SQL as
//! written, but never rows a database returned, an error's own words, which can quote values, or any secret.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use hn_core::Error;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::files::storage;

const FILE: &str = "agent-activity.jsonl";
const OLDER: &str = "agent-activity.1.jsonl";
/// Past this size at launch, the log starts afresh and the previous one is kept beside it.
const ROTATE_AT: u64 = 10 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    Ok,
    Waiting,
    Approved,
    Declined,
    Conflict,
    Refused,
    Paused,
    Busy,
    Withdrawn,
    Error,
}

/// What an agent asked for, written before any work, and again with the exact SQL once it is known.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    pub call: String,
    pub at_ms: f64,
    pub agent: String,
    pub agent_title: String,
    pub tool: String,
    pub connection_id: Option<String>,
    pub connection: Option<String>,
    pub database: Option<String>,
    pub proposal: Option<String>,
    pub sql: Vec<String>,
}

/// How a call ended. A proposal's call ends twice: waiting, then with what the person decided.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Done {
    pub call: String,
    pub at_ms: f64,
    pub outcome: Outcome,
    pub rows: Option<f64>,
    pub elapsed_ms: Option<u32>,
    /// Postgres's SQLSTATE, when an error had one.
    pub code: Option<String>,
    /// Why the person declined, in their words.
    pub reason: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
enum Line {
    Call(Call),
    Done(Done),
}

#[derive(Serialize, Deserialize)]
struct Versioned {
    v: u32,
    #[serde(flatten)]
    line: Line,
}

/// One call as the Agents page lists it, with its latest outcome.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub call: String,
    pub at_ms: f64,
    pub agent: String,
    pub tool: String,
    pub connection: Option<String>,
    pub database: Option<String>,
    pub sql: Vec<String>,
    pub outcome: Option<Outcome>,
    pub rows: Option<f64>,
    pub elapsed_ms: Option<u32>,
    pub code: Option<String>,
    pub reason: Option<String>,
    /// Its row changes can still be undone.
    pub undo: bool,
}

pub struct ActivityLog {
    path: PathBuf,
    writing: Mutex<()>,
}

impl ActivityLog {
    pub fn open(dir: PathBuf) -> Self {
        let path = dir.join(FILE);
        if fs::metadata(&path).is_ok_and(|m| m.len() > ROTATE_AT) {
            let _ = fs::rename(&path, dir.join(OLDER));
        }
        Self {
            path,
            writing: Mutex::new(()),
        }
    }

    pub fn call(&self, call: Call) -> Result<(), Error> {
        self.append(Line::Call(call))
    }

    pub fn done(&self, done: Done) -> Result<(), Error> {
        self.append(Line::Done(done))
    }

    fn append(&self, line: Line) -> Result<(), Error> {
        let mut text = serde_json::to_string(&Versioned { v: 1, line }).map_err(|e| storage("Couldn't write the agent activity log", e))?;
        text.push('\n');
        let _writing = self.writing.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| storage("Couldn't write the agent activity log", e))?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| file.write_all(text.as_bytes()))
            .map_err(|e| storage("Couldn't write the agent activity log", e))
    }

    /// The latest `limit` calls, newest first.
    pub fn recent(&self, limit: usize) -> Vec<ActivityEntry> {
        let Ok(text) = fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let mut order = Vec::new();
        let mut entries: HashMap<String, ActivityEntry> = HashMap::new();
        for line in text.lines() {
            match serde_json::from_str::<Versioned>(line).map(|v| v.line) {
                Ok(Line::Call(call)) => {
                    if let Some(entry) = entries.get_mut(&call.call) {
                        entry.sql = call.sql;
                        entry.database = call.database;
                        continue;
                    }
                    order.push(call.call.clone());
                    entries.insert(
                        call.call.clone(),
                        ActivityEntry {
                            call: call.call,
                            at_ms: call.at_ms,
                            agent: call.agent_title,
                            tool: call.tool,
                            connection: call.connection,
                            database: call.database,
                            sql: call.sql,
                            outcome: None,
                            rows: None,
                            elapsed_ms: None,
                            code: None,
                            reason: None,
                            undo: false,
                        },
                    );
                }
                Ok(Line::Done(done)) => {
                    if let Some(entry) = entries.get_mut(&done.call) {
                        entry.outcome = Some(done.outcome);
                        entry.rows = done.rows.or(entry.rows);
                        entry.elapsed_ms = done.elapsed_ms.or(entry.elapsed_ms);
                        entry.code = done.code;
                        entry.reason = done.reason;
                    }
                }
                // A line from a newer Halfnine, or one cut short by a crash, is passed over.
                Err(_) => {}
            }
        }
        order.iter().rev().take(limit).filter_map(|id| entries.remove(id)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_the_latest_outcome_of_each_call_newest_first() {
        let dir = std::env::temp_dir().join(format!("hn-activity-{}", uuid::Uuid::new_v4().simple()));
        let log = ActivityLog::open(dir.clone());
        let call = |id: &str| Call {
            call: id.into(),
            at_ms: 1.0,
            agent: "claude-code".into(),
            agent_title: "Claude Code".into(),
            tool: "change_rows".into(),
            connection_id: Some("c".into()),
            connection: Some("Staging".into()),
            database: Some("app".into()),
            proposal: None,
            sql: vec!["UPDATE users SET name = 'x'".into()],
        };
        let done = |id: &str, outcome| Done {
            call: id.into(),
            at_ms: 2.0,
            outcome,
            rows: None,
            elapsed_ms: None,
            code: None,
            reason: None,
        };
        log.call(call("a")).unwrap();
        log.call(Call {
            sql: vec!["DELETE FROM users WHERE id = 1".into()],
            ..call("a")
        })
        .unwrap();
        log.done(done("a", Outcome::Waiting)).unwrap();
        log.call(call("b")).unwrap();
        log.done(done("a", Outcome::Approved)).unwrap();
        let recent = log.recent(10);
        assert_eq!(
            recent.iter().map(|e| (e.call.as_str(), e.outcome)).collect::<Vec<_>>(),
            [("b", None), ("a", Some(Outcome::Approved))]
        );
        assert_eq!(
            recent[1].sql,
            ["DELETE FROM users WHERE id = 1"],
            "a later line for the same call replaces its SQL"
        );
        assert_eq!(log.recent(1).len(), 1);
        let _ = fs::remove_dir_all(dir);
    }
}
