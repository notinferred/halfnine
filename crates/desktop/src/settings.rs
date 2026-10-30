//! The few choices the app keeps: whether Docker databases are on, whether a backup is made before something is
//! dropped, emptied or deleted and up to what size, whether agents are paused, and whether the first-launch setup was
//! seen. One file, `settings.json`, written whole.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use hn_core::Error;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::files;

const FILE: &str = "settings.json";
const VERSION: u32 = 1;
/// Tables bigger than this aren't backed up before a drop unless someone raises it, so a drop never holds writers up
/// for long.
pub const BACKUP_LIMIT: f64 = 1_000_000_000.0;

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Choices {
    pub setup_seen: bool,
    pub docker: bool,
    pub backups: bool,
    /// The biggest table backed up before it is dropped or emptied, in bytes. `None` is no limit.
    pub backup_limit_bytes: Option<f64>,
    pub agents_paused: bool,
}

impl Default for Choices {
    fn default() -> Self {
        Self {
            setup_seen: false,
            docker: false,
            backups: true,
            backup_limit_bytes: Some(BACKUP_LIMIT),
            agents_paused: false,
        }
    }
}

/// The pause switch's old home, read once when `settings.json` doesn't exist yet.
#[derive(Default, Deserialize)]
struct AgentsFile {
    paused: bool,
}

pub struct Settings {
    path: PathBuf,
    choices: Mutex<Choices>,
}

impl Settings {
    pub fn open(dir: &Path) -> Result<Self, Error> {
        let path = dir.join(FILE);
        let choices = if path.exists() {
            files::load(&path, VERSION, "the settings")?
        } else {
            let paused = files::load::<AgentsFile>(&dir.join("agents.json"), 1, "the agent settings").is_ok_and(|a| a.paused);
            Choices {
                agents_paused: paused,
                ..Choices::default()
            }
        };
        Ok(Self {
            path,
            choices: Mutex::new(choices),
        })
    }

    pub fn get(&self) -> Choices {
        self.lock().clone()
    }

    /// Changes the choices and writes them, or changes nothing if they can't be written.
    pub fn change(&self, edit: impl FnOnce(&mut Choices)) -> Result<Choices, Error> {
        let mut choices = self.lock();
        let mut next = choices.clone();
        edit(&mut next);
        files::save(&self.path, VERSION, &next, "the settings")?;
        *choices = next.clone();
        Ok(next)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Choices> {
        self.choices.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_with_backups_on_and_keeps_an_earlier_pause() {
        let dir = std::env::temp_dir().join(format!("hn-settings-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("agents.json"), r#"{"version":1,"paused":true}"#).unwrap();
        let settings = Settings::open(&dir).unwrap();
        let choices = settings.get();
        assert!(choices.agents_paused && choices.backups && !choices.docker && !choices.setup_seen);
        assert_eq!(choices.backup_limit_bytes, Some(BACKUP_LIMIT));
        settings.change(|c| c.agents_paused = false).unwrap();
        assert!(!Settings::open(&dir).unwrap().get().agents_paused, "settings.json wins once it exists");
        let _ = std::fs::remove_dir_all(dir);
    }
}
