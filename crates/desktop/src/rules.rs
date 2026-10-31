//! The rules the person set for each database, kept in `rules.json`: what correct data is. An agent's change must keep
//! them to commit, and the person's own commit warns when it breaks one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hn_core::{Checks, ColumnRef, Database, Error, ErrorKind, Rule, RuleCheck, RuleSource, TableRef};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::AppState;
use crate::commands::now_ms;
use crate::files;

const FILE: &str = "rules.json";
const VERSION: u32 = 1;
/// A rule that takes longer than this to check on the data as it is may make changes to its table wait, or be refused
/// for time, which the person is told when they add it.
const SLOW_MS: u32 = 1000;

/// Each database's rules, by connection id and then database.
#[derive(Default, Serialize, Deserialize)]
struct Saved {
    connections: BTreeMap<String, BTreeMap<String, Vec<Rule>>>,
}

pub struct Rules {
    path: PathBuf,
    saved: Mutex<Saved>,
}

impl Rules {
    pub fn open(dir: &Path) -> Result<Self, Error> {
        let path = dir.join(FILE);
        let saved = files::load::<Saved>(&path, VERSION, "the rules")?;
        Ok(Self {
            path,
            saved: Mutex::new(saved),
        })
    }

    pub fn in_database(&self, connection_id: &str, database: &str) -> Vec<Rule> {
        self.lock()
            .connections
            .get(connection_id)
            .and_then(|databases| databases.get(database))
            .cloned()
            .unwrap_or_default()
    }

    /// The rules a change on this database must keep, reported to a reader who can't see `hidden`.
    pub fn checks(&self, connection_id: &str, database: &str, hidden: Vec<ColumnRef>, dry_run: bool) -> Checks {
        Checks {
            rules: self.in_database(connection_id, database),
            hidden,
            dry_run,
        }
    }

    pub fn add(&self, connection_id: &str, database: &str, rule: Rule) -> Result<(), Error> {
        self.change(|saved| {
            saved
                .connections
                .entry(connection_id.to_string())
                .or_default()
                .entry(database.to_string())
                .or_default()
                .push(rule);
        })
    }

    pub fn remove(&self, connection_id: &str, database: &str, id: &str) -> Result<(), Error> {
        self.change(|saved| {
            if let Some(rules) = saved.connections.get_mut(connection_id).and_then(|d| d.get_mut(database)) {
                rules.retain(|r| r.id != id);
            }
        })
    }

    /// Gives a copy of a database the rules of the one it was copied from, as it gets its hidden columns.
    pub fn copy_connection(&self, from: &str, to: &str) -> Result<(), Error> {
        self.change(|saved| {
            if let Some(databases) = saved.connections.get(from).cloned() {
                saved.connections.insert(to.to_string(), databases);
            }
        })
    }

    fn change(&self, edit: impl FnOnce(&mut Saved)) -> Result<(), Error> {
        let mut saved = self.lock();
        let mut next = Saved {
            connections: saved.connections.clone(),
        };
        edit(&mut next);
        files::save_private(&self.path, VERSION, &next, "the rules")?;
        *saved = next;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Saved> {
        self.saved.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// A rule just added, and whether it takes long enough to check that changes to its table may wait for it, or be
/// refused for time.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuleAdded {
    pub rule: Rule,
    pub slow: bool,
}

/// The rules of the database a session is on.
#[tauri::command]
#[specta::specta]
pub async fn list_rules(state: State<'_, AppState>, session_id: String) -> Result<Vec<Rule>, Error> {
    let session = state.sessions.get(&session_id).await?;
    Ok(state.rules.in_database(&session.connection.id, &session.database))
}

/// Adds a rule after trying it on the data as it is: one that rows already break is refused, naming them. A custom
/// rule takes the person's own `words`; any other is put in words by Halfnine. `evidence` marks one Halfnine suggested.
#[tauri::command]
#[specta::specta]
pub async fn add_rule(
    state: State<'_, AppState>,
    session_id: String,
    table: TableRef,
    check: RuleCheck,
    words: Option<String>,
    evidence: Option<String>,
) -> Result<RuleAdded, Error> {
    let invalid = |why: &str| Err(Error::new(ErrorKind::Invalid, why));
    match &check {
        RuleCheck::OneOf { values, .. } if values.iter().all(|v| v.trim().is_empty()) => return invalid("Give at least one value."),
        RuleCheck::Between {
            at_least: None,
            at_most: None,
            ..
        } => return invalid("Give at least one bound."),
        RuleCheck::Custom { sql, .. } if sql.trim().is_empty() => return invalid("Write the query that finds the rows breaking the rule."),
        _ => {}
    }
    let words = match (check.sentence(&table, true), words.map(|w| w.trim().to_string())) {
        (Some(sentence), _) => sentence,
        (None, Some(words)) if !words.is_empty() => words,
        (None, _) => return invalid("Say the rule in words, as agents will read it."),
    };
    let session = state.sessions.get(&session_id).await?;
    let mut rule = Rule {
        id: uuid::Uuid::new_v4().simple().to_string(),
        table,
        check,
        words,
        source: evidence.map_or(RuleSource::Written, |evidence| RuleSource::Suggested { evidence }),
        added_ms: now_ms(),
    };
    let tried = session.db.try_rule(&rule).await?;
    if !tried.breaking.is_empty() {
        return Err(Error::new(
            ErrorKind::Invalid,
            format!(
                "Rows break this rule already ({}), so it isn't added. Fix them first, or change the rule.",
                tried.breaking.join("; ")
            ),
        ));
    }
    if let RuleCheck::Custom { reads, .. } = &mut rule.check {
        *reads = tried.reads;
    }
    state.rules.add(&session.connection.id, &session.database, rule.clone())?;
    Ok(RuleAdded {
        rule,
        slow: tried.elapsed_ms > SLOW_MS,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn delete_rule(state: State<'_, AppState>, session_id: String, id: String) -> Result<(), Error> {
    let session = state.sessions.get(&session_id).await?;
    state.rules.remove(&session.connection.id, &session.database, &id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_each_databases_rules_apart_and_on_disk() {
        let dir = std::env::temp_dir().join(format!("hn-rules-{}", uuid::Uuid::new_v4().simple()));
        let rules = Rules::open(&dir).unwrap();
        let rule = Rule {
            id: "r1".into(),
            table: TableRef {
                schema: "public".into(),
                name: "invoices".into(),
            },
            check: RuleCheck::OnlyGrows,
            words: "invoices only grows".into(),
            source: RuleSource::Written,
            added_ms: 0.0,
        };
        rules.add("c1", "app", rule.clone()).unwrap();
        assert_eq!(Rules::open(&dir).unwrap().in_database("c1", "app"), std::slice::from_ref(&rule));
        assert!(rules.in_database("c1", "other").is_empty() && rules.in_database("c2", "app").is_empty());
        rules.copy_connection("c1", "docker:copy").unwrap();
        assert_eq!(rules.in_database("docker:copy", "app").len(), 1);
        rules.remove("c1", "app", "r1").unwrap();
        assert!(rules.in_database("c1", "app").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
