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
    Ok(read(path, version, what)?.unwrap_or_default())
}

/// As [`load`], giving `None` when the file doesn't exist.
pub fn read<T: DeserializeOwned>(path: &Path, version: u32, what: &str) -> Result<Option<T>, Error> {
    match fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(storage(&format!("Couldn't read {what}"), e)),
        Ok(bytes) => {
            let file: Versioned<T> = serde_json::from_slice(&bytes).map_err(|e| storage(&format!("Couldn't read {what}"), e))?;
            if file.version > version {
                return Err(Error::new(
                    ErrorKind::Storage,
                    format!("A newer Halfnine wrote {what}, so this one won't change them."),
                ));
            }
            Ok(Some(file.body))
        }
    }
}

/// Writes beside the file and renames over it, so a crash leaves either the old contents or the new, never half of them.
pub fn save<T: Serialize>(path: &Path, version: u32, body: &T, what: &str) -> Result<(), Error> {
    write(path, version, body, what, false)
}

/// As [`save`], for a file only the person's own account may read. On Windows the config folder is already private.
pub fn save_private<T: Serialize>(path: &Path, version: u32, body: &T, what: &str) -> Result<(), Error> {
    write(path, version, body, what, true)
}

fn write<T: Serialize>(path: &Path, version: u32, body: &T, what: &str, private: bool) -> Result<(), Error> {
    let failed = |e: std::io::Error| storage(&format!("Couldn't save {what}"), e);
    let bytes = serde_json::to_vec_pretty(&Versioned { version, body }).map_err(|e| storage(&format!("Couldn't save {what}"), e))?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| storage("Couldn't create the settings folder", e))?;
    }
    let temporary = path.with_extension("json.tmp");
    // A mode applies only to a file being created, so whatever an earlier attempt left is removed first.
    let _ = fs::remove_file(&temporary);
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(&temporary).map_err(failed)?;
    std::io::Write::write_all(&mut file, &bytes).map_err(failed)?;
    drop(file);
    fs::rename(&temporary, path).map_err(failed)
}

/// Makes a folder, and any missing above it, that only this account can open. On Windows the config folder is
/// already private.
pub fn private_dir(path: &Path) -> Result<(), Error> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path).map_err(|e| storage("Couldn't create a folder", e))
}

pub fn storage(what: &str, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Storage, format!("{what}: {e}."))
}
