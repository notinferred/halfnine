//! The small JSON files the app keeps in its config folder: each read with a version check and written whole and atomically.

use std::fs;
use std::path::Path;

use hn_core::{Error, ErrorKind};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct Versioned<T> {
    version: u32,
    #[serde(flatten)]
    body: T,
}

/// Reads `path`, or gives the default when it doesn't exist yet. A file from a newer version is refused, never guessed at.
pub fn load<T: DeserializeOwned + Default>(path: &Path, version: u32, what: &str) -> Result<T, Error> {
    match fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(storage(&format!("Couldn't read {what}"), e)),
        Ok(bytes) => {
            let file: Versioned<T> = serde_json::from_slice(&bytes).map_err(|e| storage(&format!("Couldn't read {what}"), e))?;
            if file.version > version {
                return Err(Error::new(
                    ErrorKind::Storage,
                    format!("A newer Halfnine wrote {what}, so this one won't change them."),
                ));
            }
            Ok(file.body)
        }
    }
}

/// Writes beside the file and renames over it, so a crash leaves either the old contents or the new, never half of them.
pub fn save<T: Serialize>(path: &Path, version: u32, body: &T, what: &str) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(&Versioned { version, body }).map_err(|e| storage(&format!("Couldn't save {what}"), e))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| storage("Couldn't create the settings folder", e))?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, bytes).map_err(|e| storage(&format!("Couldn't save {what}"), e))?;
    fs::rename(&temporary, path).map_err(|e| storage(&format!("Couldn't save {what}"), e))
}

pub fn storage(what: &str, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Storage, format!("{what}: {e}."))
}
