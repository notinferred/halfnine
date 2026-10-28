//! The commands the UI calls. Each argument arrives as a typed value; passwords go into the keychain and never come back out.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;
use tu_core::{
    ConnectionSpec, Database, Error, ErrorKind, Filter, Label, Page, PageRequest, SchemaNode, ServerInfo, SshAuth, SshSpec, Structure, TableRef,
    TlsMode, UrlParts,
};
use tu_pg::{PgDatabase, Secrets};

use crate::sessions::Session;
use crate::{AppState, secrets};

/// A saved connection, and whether the keychain holds its secrets. The secrets themselves never leave Rust.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SavedConnection {
    pub spec: ConnectionSpec,
    pub has_password: bool,
    pub has_ssh_secret: bool,
}

/// A connection as the form holds it: without an id until it is first saved, and with what to do about each secret.
#[derive(Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionDraft {
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub tls: TlsMode,
    pub label: Label,
    pub read_only: bool,
    pub ssh: Option<SshSpec>,
    pub password: SecretChange,
    pub ssh_secret: SecretChange,
}

#[derive(Deserialize, Type)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum SecretChange {
    Keep,
    Clear,
    Set { value: String },
}

#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub elapsed_ms: f64,
    pub server_version: String,
}

#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: String,
    pub connection: ConnectionSpec,
    pub server: ServerInfo,
}

#[tauri::command]
#[specta::specta]
pub async fn list_connections(state: State<'_, AppState>) -> Result<Vec<SavedConnection>, Error> {
    state.store.list().into_iter().map(saved).collect()
}

#[tauri::command]
#[specta::specta]
pub async fn save_connection(state: State<'_, AppState>, draft: ConnectionDraft) -> Result<SavedConnection, Error> {
    let spec = spec_from(&draft)?;
    apply(&secrets::password_account(&spec.id), &draft.password)?;
    apply(&secrets::ssh_account(&spec.id), &draft.ssh_secret)?;
    state.store.save(spec.clone())?;
    saved(spec)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_connection(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    state.store.remove(&id)?;
    secrets::delete(&secrets::password_account(&id))?;
    secrets::delete(&secrets::ssh_account(&id))
}

#[tauri::command]
#[specta::specta]
pub async fn test_connection(draft: ConnectionDraft) -> Result<TestResult, Error> {
    let spec = spec_from(&draft)?;
    let secrets = Secrets {
        password: resolve(&secrets::password_account(&spec.id), &draft.password)?,
        ssh: resolve(&secrets::ssh_account(&spec.id), &draft.ssh_secret)?,
    };
    let started = Instant::now();
    let db = PgDatabase::connect(&spec, None, &secrets).await?;
    let server = db.server().await;
    db.close().await;
    Ok(TestResult {
        elapsed_ms: started.elapsed().as_secs_f64() * 1000.0,
        server_version: server?.version,
    })
}

#[tauri::command]
#[specta::specta]
pub fn parse_connection_url(url: String) -> Result<UrlParts, Error> {
    tu_core::parse_url(&url)
}

#[tauri::command]
#[specta::specta]
pub async fn open_session(state: State<'_, AppState>, connection_id: String, database: Option<String>) -> Result<SessionInfo, Error> {
    let connection = state.store.get(&connection_id)?;
    let secrets = Secrets {
        password: secrets::get(&secrets::password_account(&connection.id))?,
        ssh: secrets::get(&secrets::ssh_account(&connection.id))?,
    };
    let db = PgDatabase::connect(&connection, database.as_deref(), &secrets).await?;
    let server = db.server().await?;
    let session_id = state
        .sessions
        .insert(Session {
            connection: connection.clone(),
            db,
        })
        .await;
    Ok(SessionInfo {
        session_id,
        connection,
        server,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn close_session(state: State<'_, AppState>, session_id: String) -> Result<(), Error> {
    state.sessions.close(&session_id).await;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn schema(state: State<'_, AppState>, session_id: String) -> Result<Vec<SchemaNode>, Error> {
    state.sessions.get(&session_id).await?.db.schema().await
}

#[tauri::command]
#[specta::specta]
pub async fn structure(state: State<'_, AppState>, session_id: String, table: TableRef) -> Result<Structure, Error> {
    state.sessions.get(&session_id).await?.db.structure(&table).await
}

#[tauri::command]
#[specta::specta]
pub async fn page(state: State<'_, AppState>, session_id: String, request: PageRequest) -> Result<Page, Error> {
    state.sessions.get(&session_id).await?.db.page(&request).await
}

#[tauri::command]
#[specta::specta]
pub async fn count_rows(state: State<'_, AppState>, session_id: String, table: TableRef, filter: Option<Filter>) -> Result<f64, Error> {
    state.sessions.get(&session_id).await?.db.count(&table, filter.as_ref()).await
}

fn saved(spec: ConnectionSpec) -> Result<SavedConnection, Error> {
    Ok(SavedConnection {
        has_password: secrets::get(&secrets::password_account(&spec.id))?.is_some(),
        has_ssh_secret: secrets::get(&secrets::ssh_account(&spec.id))?.is_some(),
        spec,
    })
}

/// Checks a draft once, here, so everything past this point can trust it.
fn spec_from(draft: &ConnectionDraft) -> Result<ConnectionSpec, Error> {
    let required = |value: &str, what: &str| {
        if value.trim().is_empty() {
            Err(Error::new(ErrorKind::Invalid, format!("Enter a {what}.")))
        } else {
            Ok(value.trim().to_string())
        }
    };
    let host = required(&draft.host, "host")?;
    let ssh = match &draft.ssh {
        None => None,
        Some(ssh) => Some(SshSpec {
            host: required(&ssh.host, "SSH host")?,
            port: if ssh.port == 0 { 22 } else { ssh.port },
            user: required(&ssh.user, "SSH user")?,
            auth: match &ssh.auth {
                SshAuth::Key { path } => SshAuth::Key {
                    path: required(path, "path to the SSH key")?,
                },
                other => other.clone(),
            },
        }),
    };
    Ok(ConnectionSpec {
        id: draft.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name: if draft.name.trim().is_empty() {
            host.clone()
        } else {
            draft.name.trim().to_string()
        },
        host,
        port: if draft.port == 0 { 5432 } else { draft.port },
        user: required(&draft.user, "user")?,
        database: if draft.database.trim().is_empty() {
            "postgres".to_string()
        } else {
            draft.database.trim().to_string()
        },
        tls: draft.tls,
        label: draft.label,
        read_only: draft.read_only,
        ssh,
    })
}

fn apply(account: &str, change: &SecretChange) -> Result<(), Error> {
    match change {
        SecretChange::Keep => Ok(()),
        SecretChange::Clear => secrets::delete(account),
        SecretChange::Set { value } => secrets::set(account, value),
    }
}

fn resolve(account: &str, change: &SecretChange) -> Result<Option<String>, Error> {
    match change {
        SecretChange::Keep => secrets::get(account),
        SecretChange::Clear => Ok(None),
        SecretChange::Set { value } => Ok(Some(value.clone())),
    }
}
