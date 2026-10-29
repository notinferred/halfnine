//! The saved connections: one JSON file in the app's config folder, written whole and atomically, with no secrets in it.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use hn_core::{ConnectionSpec, Error, ErrorKind};

use crate::files;

/// Raised only with a migration from the previous version, which this app is the only one to write.
const VERSION: u32 = 1;
const WHAT: &str = "the saved connections";

#[derive(Default, Serialize, Deserialize)]
struct Connections {
    connections: Vec<ConnectionSpec>,
}

pub struct Store {
    path: PathBuf,
    connections: Mutex<Vec<ConnectionSpec>>,
}

impl Store {
    pub fn open(dir: PathBuf) -> Result<Self, Error> {
        let path = dir.join("connections.json");
        let file: Connections = files::load(&path, VERSION, WHAT)?;
        Ok(Self {
            path,
            connections: Mutex::new(file.connections),
        })
    }

    pub fn list(&self) -> Vec<ConnectionSpec> {
        self.lock().clone()
    }

    pub fn get(&self, id: &str) -> Result<ConnectionSpec, Error> {
        self.lock()
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "That connection no longer exists."))
    }

    pub fn save(&self, spec: ConnectionSpec) -> Result<(), Error> {
        let mut connections = self.lock();
        let mut next = connections.clone();
        match next.iter_mut().find(|c| c.id == spec.id) {
            Some(existing) => *existing = spec,
            None => next.push(spec),
        }
        self.write(next.clone())?;
        *connections = next;
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<(), Error> {
        let mut connections = self.lock();
        let next: Vec<_> = connections.iter().filter(|c| c.id != id).cloned().collect();
        self.write(next.clone())?;
        *connections = next;
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<ConnectionSpec>> {
        self.connections.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write(&self, connections: Vec<ConnectionSpec>) -> Result<(), Error> {
        files::save(&self.path, VERSION, &Connections { connections }, WHAT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hn_core::{Label, TlsMode};

    fn spec(id: &str, name: &str) -> ConnectionSpec {
        ConnectionSpec {
            id: id.into(),
            name: name.into(),
            host: "localhost".into(),
            port: 5432,
            user: "postgres".into(),
            database: "postgres".into(),
            tls: TlsMode::Prefer,
            label: Label::None,
            read_only: false,
            ssh: None,
            save_password: true,
        }
    }

    use std::fs;

    #[test]
    fn saves_replaces_and_removes_across_reopening() {
        let dir = std::env::temp_dir().join(format!("halfnine-store-{}", uuid::Uuid::new_v4()));
        let store = Store::open(dir.clone()).unwrap();
        store.save(spec("a", "First")).unwrap();
        store.save(spec("b", "Second")).unwrap();
        store.save(spec("a", "Renamed")).unwrap();
        store.remove("b").unwrap();

        let reopened = Store::open(dir.clone()).unwrap();
        assert_eq!(reopened.list(), vec![spec("a", "Renamed")]);
        assert!(!dir.join("connections.json.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refuses_a_file_from_a_newer_version() {
        let dir = std::env::temp_dir().join(format!("halfnine-store-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("connections.json"), r#"{"version":99,"connections":[]}"#).unwrap();
        assert_eq!(Store::open(dir.clone()).err().unwrap().kind, ErrorKind::Storage);
        fs::remove_dir_all(dir).unwrap();
    }
}
