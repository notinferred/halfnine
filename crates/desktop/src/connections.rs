//! Every connection the app can open, wherever it comes from, and the secrets each needs. The window and agents both
//! find connections here, so they open them the same way.

use std::sync::Arc;

use hn_core::{ConnectionSpec, Error, ErrorKind, HiddenColumn};
use hn_pg::Secrets;

use crate::secrets::Passwords;
use crate::store::Store;

pub struct Directory {
    pub store: Arc<Store>,
    pub passwords: Arc<Passwords>,
}

impl Directory {
    pub fn new(store: Arc<Store>, passwords: Arc<Passwords>) -> Self {
        Self { store, passwords }
    }

    pub fn list(&self) -> Vec<ConnectionSpec> {
        self.store.list()
    }

    pub fn get(&self, id: &str) -> Result<ConnectionSpec, Error> {
        self.store.get(id)
    }

    /// The secrets to open `spec` with: what was just typed, else what was typed earlier this launch or is saved. A
    /// connection that asks each time, with nothing typed yet, needs its password first.
    pub async fn secrets(&self, spec: &ConnectionSpec, typed: Option<Secrets>) -> Result<Secrets, Error> {
        let asks = !spec.save_password;
        match typed {
            Some(typed) => {
                if asks {
                    self.passwords.remember(&spec.id, &typed);
                }
                Ok(typed)
            }
            None if asks => self
                .passwords
                .remembered(&spec.id)
                .ok_or_else(|| Error::new(ErrorKind::PasswordNeeded, format!("Type the password for {} to connect.", spec.name))),
            None => self.passwords.saved(&spec.id),
        }
    }

    /// The server refused a password typed this launch, so it is asked for again next time.
    pub fn refused(&self, spec: &ConnectionSpec) {
        if !spec.save_password {
            self.passwords.forget_typed(&spec.id);
        }
    }

    /// Hides a column of a connection from agents, or shows it to them again.
    pub fn set_hidden(&self, connection_id: &str, column: HiddenColumn, hidden: bool) -> Result<ConnectionSpec, Error> {
        let mut spec = self.store.get(connection_id)?;
        spec.hidden_columns.retain(|h| h != &column);
        if hidden {
            spec.hidden_columns.push(column);
        }
        self.store.save(spec.clone())?;
        Ok(spec)
    }
}
