//! Passwords and SSH secrets. Saved ones live together in one entry of the macOS Keychain or the Windows Credential
//! Manager, read once per launch and kept in memory until the app quits. macOS asks before an app that didn't save an
//! entry reads it, so one entry means at most one question per launch. Passwords typed for a connection that asks
//! each time are kept in memory only, until the app quits.

use std::collections::HashMap;
use std::sync::Mutex;

use hn_core::{Error, ErrorKind};
use hn_pg::Secrets;
use keyring_core::Entry;
use serde::{Deserialize, Serialize};

use crate::commands::SecretChange;

const SERVICE: &str = "com.notinferred.halfnine";
const ACCOUNT: &str = "passwords";
const VERSION: u32 = 1;

pub fn init() -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    keyring_core::set_default_store(apple_native_keyring_store::keychain::Store::new().map_err(keychain)?);
    #[cfg(target_os = "windows")]
    keyring_core::set_default_store(windows_native_keyring_store::Store::new().map_err(keychain)?);
    Ok(())
}

/// A connection's secrets as the keychain entry holds them.
#[derive(Clone, Default, Serialize, Deserialize)]
struct Pair {
    password: Option<String>,
    ssh: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    version: u32,
    connections: HashMap<String, Pair>,
}

#[derive(Default)]
pub struct Passwords {
    /// The keychain entry, once it has been read this launch.
    saved: Mutex<Option<Saved>>,
    /// Typed this launch, for connections that ask each time.
    typed: Mutex<HashMap<String, Pair>>,
}

impl Passwords {
    /// The saved secrets of a connection. The first call of a launch reads the keychain; the rest read memory.
    pub fn saved(&self, id: &str) -> Result<Secrets, Error> {
        self.with_saved(|saved| Ok(secrets(saved.connections.get(id))))
    }

    /// Applies a form's changes to a connection's saved secrets, writing the keychain only when something changed.
    pub fn save(&self, id: &str, password: &SecretChange, ssh: &SecretChange) -> Result<(), Error> {
        // A password typed this launch, while the connection asked each time, is what it saves when switched to saving.
        let typed = self.typed.lock().map_err(|_| busy())?.get(id).cloned().unwrap_or_default();
        let password = match password {
            SecretChange::Keep => typed.password.map(|p| SecretChange::Set { value: p }),
            other => Some(other.clone()),
        };
        let ssh = match ssh {
            SecretChange::Keep => typed.ssh.map(|p| SecretChange::Set { value: p }),
            other => Some(other.clone()),
        };
        if password.is_none() && ssh.is_none() {
            return Ok(());
        }
        self.forget_typed(id);
        self.with_saved(|saved| {
            let pair = saved.connections.entry(id.to_string()).or_default();
            if let Some(change) = &password {
                pair.password = after(change);
            }
            if let Some(change) = &ssh {
                pair.ssh = after(change);
            }
            if pair.password.is_none() && pair.ssh.is_none() {
                saved.connections.remove(id);
            }
            write(saved)
        })
    }

    /// Removes a connection's saved secrets, as when it stops saving them or is deleted.
    pub fn forget(&self, id: &str) -> Result<(), Error> {
        self.with_saved(|saved| match saved.connections.remove(id) {
            Some(_) => write(saved),
            None => Ok(()),
        })
    }

    pub fn remember(&self, id: &str, secrets: &Secrets) {
        if let Ok(mut typed) = self.typed.lock() {
            typed.insert(
                id.to_string(),
                Pair {
                    password: secrets.password.clone(),
                    ssh: secrets.ssh.clone(),
                },
            );
        }
    }

    /// What was typed this launch for a connection that asks each time, if anything.
    pub fn remembered(&self, id: &str) -> Option<Secrets> {
        self.typed.lock().ok()?.get(id).map(|pair| secrets(Some(pair)))
    }

    pub fn forget_typed(&self, id: &str) {
        if let Ok(mut typed) = self.typed.lock() {
            typed.remove(id);
        }
    }

    fn with_saved<T>(&self, work: impl FnOnce(&mut Saved) -> Result<T, Error>) -> Result<T, Error> {
        let mut slot = self.saved.lock().map_err(|_| busy())?;
        if slot.is_none() {
            *slot = Some(read()?);
        }
        match slot.as_mut() {
            Some(saved) => work(saved),
            None => Err(busy()),
        }
    }
}

fn read() -> Result<Saved, Error> {
    match Entry::new(SERVICE, ACCOUNT).and_then(|e| e.get_password()) {
        Ok(text) => {
            let saved: Saved = serde_json::from_str(&text).map_err(|e| keychain_text(&e))?;
            if saved.version > VERSION {
                return Err(Error::new(
                    ErrorKind::Keychain,
                    "A newer Halfnine saved these passwords, so this one won't change them.",
                ));
            }
            Ok(saved)
        }
        Err(keyring_core::Error::NoEntry) => Ok(Saved {
            version: VERSION,
            connections: HashMap::new(),
        }),
        Err(e) => Err(keychain(e)),
    }
}

fn write(saved: &mut Saved) -> Result<(), Error> {
    saved.version = VERSION;
    let entry = Entry::new(SERVICE, ACCOUNT).map_err(keychain)?;
    if saved.connections.is_empty() {
        return match entry.delete_credential() {
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain(e)),
        };
    }
    let text = serde_json::to_string(saved).map_err(|e| keychain_text(&e))?;
    entry.set_password(&text).map_err(keychain)
}

fn after(change: &SecretChange) -> Option<String> {
    match change {
        SecretChange::Set { value } if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

fn secrets(pair: Option<&Pair>) -> Secrets {
    Secrets {
        password: pair.and_then(|p| p.password.clone()),
        ssh: pair.and_then(|p| p.ssh.clone()),
    }
}

fn busy() -> Error {
    Error::new(ErrorKind::Keychain, "The saved passwords couldn't be read. Restart Halfnine.")
}

fn keychain(e: keyring_core::Error) -> Error {
    Error::new(ErrorKind::Keychain, format!("The system keychain refused: {e}."))
}

fn keychain_text(e: &serde_json::Error) -> Error {
    Error::new(ErrorKind::Keychain, format!("Halfnine's keychain entry couldn't be read: {e}."))
}
