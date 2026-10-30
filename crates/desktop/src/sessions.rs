//! Open connections, each with its own small pool, looked up by the id the UI holds.

use std::collections::HashMap;
use std::sync::Arc;

use hn_core::{ConnectionSpec, Error, ErrorKind};
use hn_pg::{PgDatabase, Secrets};
use tokio::sync::RwLock;

pub struct Session {
    pub connection: ConnectionSpec,
    pub database: String,
    pub db: PgDatabase,
    /// Kept in memory while the session is open, so switching databases never asks for them again.
    pub secrets: Secrets,
}

#[derive(Default)]
pub struct Sessions {
    open: RwLock<HashMap<String, Arc<Session>>>,
}

impl Sessions {
    pub async fn insert(&self, session: Session) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.open.write().await.insert(id.clone(), Arc::new(session));
        id
    }

    pub async fn get(&self, id: &str) -> Result<Arc<Session>, Error> {
        self.open
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "This connection was closed. Open it again."))
    }

    /// Closes every session on a connection, as before its database is deleted.
    pub async fn close_connection(&self, connection_id: &str) {
        let ids: Vec<String> = self
            .open
            .read()
            .await
            .iter()
            .filter(|(_, s)| s.connection.id == connection_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.close(&id).await;
        }
    }

    pub async fn close(&self, id: &str) {
        let removed = self.open.write().await.remove(id);
        if let Some(session) = removed {
            session.db.close().await;
        }
    }
}
