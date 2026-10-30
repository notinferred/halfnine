//! Every connection the app can open, saved or a Docker database, and the secrets each needs. The window and agents
//! both find connections here, so they open them the same way.

use std::sync::Arc;

use hn_core::{ConnectionSpec, Error, ErrorKind, HiddenColumn, Source};
use hn_pg::Secrets;

use crate::docker::{Docker, ID_PREFIX};
use crate::secrets::Passwords;
use crate::store::Store;

pub struct Directory {
    pub store: Arc<Store>,
    pub passwords: Arc<Passwords>,
    pub docker: Arc<Docker>,
}

impl Directory {
    pub fn new(store: Arc<Store>, passwords: Arc<Passwords>, docker: Arc<Docker>) -> Self {
        Self { store, passwords, docker }
    }

    /// The saved connections, then the Docker databases while they're on.
    pub fn list(&self) -> Vec<ConnectionSpec> {
        let mut all = self.store.list();
        all.extend(self.docker.connections());
        all
    }

    pub fn get(&self, id: &str) -> Result<ConnectionSpec, Error> {
        match id.strip_prefix(ID_PREFIX) {
            Some(name) => self
                .docker
                .connections()
                .into_iter()
                .find(|c| c.name == name)
                .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("There is no Docker database named {name} any more."))),
            None => self.store.get(id),
        }
    }

    /// A Docker database that is stopped is started, so it can be opened. Others are ready as they are.
    pub async fn ready(&self, spec: ConnectionSpec) -> Result<ConnectionSpec, Error> {
        match &spec.source {
            Source::Docker { container, .. } if self.docker.container(container).is_some_and(|c| !c.running) => {
                self.docker.start(container).await?;
                self.get(&spec.id)
            }
            _ => Ok(spec),
        }
    }

    /// The secrets to open `spec` with: what was just typed, else what was typed earlier this launch or is saved. A
    /// connection that asks each time, with nothing typed yet, needs its password first.
    pub async fn secrets(&self, spec: &ConnectionSpec, typed: Option<Secrets>) -> Result<Secrets, Error> {
        // A Docker database's credentials are read from its container each time, unless the person typed a password
        // because the container's had changed since.
        if let Source::Docker { container, .. } = &spec.source {
            if let Some(typed) = typed {
                self.passwords.remember(&spec.id, &typed);
                return Ok(typed);
            }
            return match self.passwords.remembered(&spec.id) {
                Some(typed) => Ok(typed),
                None => self.docker.secrets(container).await,
            };
        }
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
        if let Some(name) = connection_id.strip_prefix(ID_PREFIX) {
            return self.docker.set_overrides(name, |overrides| {
                overrides.hidden_columns.retain(|h| h != &column);
                if hidden {
                    overrides.hidden_columns.push(column);
                }
            });
        }
        let mut spec = self.store.get(connection_id)?;
        spec.hidden_columns.retain(|h| h != &column);
        if hidden {
            spec.hidden_columns.push(column);
        }
        self.store.save(spec.clone())?;
        Ok(spec)
    }
}
