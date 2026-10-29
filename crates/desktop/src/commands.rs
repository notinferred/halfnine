//! The commands the UI calls. Each argument arrives as a typed value; passwords go into the keychain and never come back out.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use hn_core::{
    Activity, Changes, Choice, Committed, ConnectionSpec, Database, Error, ErrorKind, ExportSource, Filter, Label, Page, PageRequest, RunOutcome,
    SchemaNode, ServerInfo, SshAuth, SshSpec, Structure, TableRef, TlsMode, UrlParts,
};
use hn_pg::{PgDatabase, Secrets};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::AppState;
use crate::export::{ExportDone, ExportFormat, Sink};
use crate::files::storage;
use crate::import::{FileSource, ImportPreview};
use crate::queries::{HistoryEntry, SavedQuery};
use crate::sessions::Session;

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
    pub save_password: bool,
    pub password: SecretChange,
    pub ssh_secret: SecretChange,
}

/// Secrets typed to connect once, for a connection that doesn't save its own.
#[derive(Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Typed {
    pub password: Option<String>,
    pub ssh_secret: Option<String>,
}

#[derive(Clone, Deserialize, Type)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum SecretChange {
    Keep,
    Clear,
    Set { value: String },
}

#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub elapsed_ms: u32,
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
/// The saved connections. Their secrets stay in the keychain, which is read only to connect: macOS asks before an app
/// that didn't save a secret reads it, and to macOS every new build is a different app.
pub async fn list_connections(state: State<'_, AppState>) -> Result<Vec<ConnectionSpec>, Error> {
    Ok(state.store.list())
}

#[tauri::command]
#[specta::specta]
pub async fn save_connection(state: State<'_, AppState>, draft: ConnectionDraft) -> Result<ConnectionSpec, Error> {
    let spec = spec_from(&draft)?;
    let was_saving = state.store.get(&spec.id).is_ok_and(|before| before.save_password);
    if spec.save_password {
        state.passwords.save(&spec.id, &draft.password, &draft.ssh_secret)?;
    } else if was_saving {
        // Switching to Ask each time removes what was saved, but only now, when the form is saved.
        state.passwords.forget(&spec.id)?;
    }
    state.store.save(spec.clone())?;
    Ok(spec)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_connection(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    let was_saving = state.store.get(&id).is_ok_and(|c| c.save_password);
    state.store.remove(&id)?;
    state.passwords.forget_typed(&id);
    if was_saving { state.passwords.forget(&id) } else { Ok(()) }
}

#[tauri::command]
#[specta::specta]
pub async fn test_connection(state: State<'_, AppState>, draft: ConnectionDraft) -> Result<TestResult, Error> {
    let spec = spec_from(&draft)?;
    // What the form leaves untouched comes from what is already known, and the keychain is read only if it must be.
    let untouched = matches!(draft.password, SecretChange::Keep) || matches!(draft.ssh_secret, SecretChange::Keep);
    let known = match draft.id {
        Some(_) if untouched && spec.save_password => state.passwords.saved(&spec.id)?,
        Some(_) if untouched => state.passwords.remembered(&spec.id).unwrap_or_default(),
        _ => Secrets::default(),
    };
    let secrets = Secrets {
        password: resolve(&draft.password, known.password),
        ssh: resolve(&draft.ssh_secret, known.ssh),
    };
    let started = Instant::now();
    let db = PgDatabase::connect(&spec, None, &secrets).await?;
    let server = db.server().await;
    db.close().await;
    Ok(TestResult {
        elapsed_ms: hn_core::millis(started.elapsed()),
        server_version: server?.version,
    })
}

#[tauri::command]
#[specta::specta]
pub fn parse_connection_url(url: String) -> Result<UrlParts, Error> {
    hn_core::parse_url(&url)
}

/// Opens a connection with what was just typed, or else with what is saved for it, or typed earlier this launch.
#[tauri::command]
#[specta::specta]
pub async fn open_session(
    state: State<'_, AppState>,
    connection_id: String,
    database: Option<String>,
    typed: Option<Typed>,
) -> Result<SessionInfo, Error> {
    let connection = state.store.get(&connection_id)?;
    let id = connection.id.clone();
    let asks = !connection.save_password;
    let secrets = match typed {
        Some(typed) => {
            let secrets = Secrets {
                password: typed.password.filter(|p| !p.is_empty()),
                ssh: typed.ssh_secret.filter(|p| !p.is_empty()),
            };
            if asks {
                state.passwords.remember(&id, &secrets);
            }
            secrets
        }
        None if asks => state.passwords.remembered(&id).ok_or_else(|| {
            Error::new(
                ErrorKind::PasswordNeeded,
                format!("Type the password for {} to connect.", connection.name),
            )
        })?,
        None => state.passwords.saved(&id)?,
    };
    let opened = open(&state, connection, database.as_deref(), secrets).await;
    // A password typed this launch that the server refused is asked for again next time.
    if asks && opened.as_ref().is_err_and(|e| e.kind == ErrorKind::Auth) {
        state.passwords.forget_typed(&id);
    }
    opened
}

/// Opens the same connection on another database, with the secrets it already holds, then closes the old session.
#[tauri::command]
#[specta::specta]
pub async fn switch_database(state: State<'_, AppState>, session_id: String, database: String) -> Result<SessionInfo, Error> {
    let current = state.sessions.get(&session_id).await?;
    let next = open(&state, current.connection.clone(), Some(&database), current.secrets.clone()).await?;
    state.sessions.close(&session_id).await;
    Ok(next)
}

async fn open(state: &AppState, connection: ConnectionSpec, database: Option<&str>, secrets: Secrets) -> Result<SessionInfo, Error> {
    let db = PgDatabase::connect(&connection, database, &secrets).await?;
    let server = db.server().await?;
    let session_id = state
        .sessions
        .insert(Session {
            connection: connection.clone(),
            database: server.database.clone(),
            db,
            secrets,
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

/// Runs the editor's statements, each exactly one statement, and adds the run to the history.
#[tauri::command]
#[specta::specta]
pub async fn run_statements(state: State<'_, AppState>, session_id: String, statements: Vec<String>) -> Result<RunOutcome, Error> {
    let session = state.sessions.get(&session_id).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let outcome = session.db.run(&statements).await?;
    // The log names the run, never its SQL, which can hold data.
    log::info!(
        "run {id}: {} of {} statements in {} ms{}",
        outcome.results.len(),
        statements.len(),
        outcome.elapsed_ms,
        if outcome.failed.is_some() { ", then an error" } else { "" }
    );
    state.queries.record(HistoryEntry {
        id,
        sql: statements.join(";\n"),
        connection_id: session.connection.id.clone(),
        database: session.database.clone(),
        ran_at_ms: now_ms(),
        elapsed_ms: outcome.elapsed_ms,
        ok: outcome.failed.is_none(),
    })?;
    Ok(outcome)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_run(state: State<'_, AppState>, session_id: String) -> Result<bool, Error> {
    state.sessions.get(&session_id).await?.db.cancel().await
}

#[tauri::command]
#[specta::specta]
pub async fn explain(state: State<'_, AppState>, session_id: String, statement: String, analyze: bool) -> Result<String, Error> {
    state.sessions.get(&session_id).await?.db.explain(&statement, analyze).await
}

/// Asks where to save, then streams every row of `source` into the file. `None` when the dialog was dismissed.
#[tauri::command]
#[specta::specta]
pub async fn export(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    source: ExportSource,
    format: ExportFormat,
    name: String,
) -> Result<Option<ExportDone>, Error> {
    let session = state.sessions.get(&session_id).await?;
    let extension = format.extension();
    let suggested = format!("{name}.{extension}");
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_file_name(suggested)
            .add_filter(extension.to_uppercase(), &[extension])
            .blocking_save_file()
    })
    .await
    .map_err(|e| storage("The save dialog failed", e))?;
    let Some(path) = picked.as_ref().and_then(|p| p.as_path()).map(|p| p.to_path_buf()) else {
        return Ok(None);
    };
    let mut sink = Sink::create(&path, format)?;
    let written = session.db.export(&source, &mut sink).await;
    match written.and_then(|rows| sink.finish().map(|()| rows)) {
        Ok(rows) => Ok(Some(ExportDone {
            rows,
            file: path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        })),
        Err(error) => {
            // Half a file would look like a whole one, so it goes.
            let _ = std::fs::remove_file(&path);
            Err(error)
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn activity(state: State<'_, AppState>, session_id: String) -> Result<Vec<Activity>, Error> {
    state.sessions.get(&session_id).await?.db.activity().await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_session(state: State<'_, AppState>, session_id: String, pid: i32, terminate: bool) -> Result<bool, Error> {
    state.sessions.get(&session_id).await?.db.stop(pid, terminate).await
}

/// The SQL that staged changes would run, to read before committing them.
#[tauri::command]
#[specta::specta]
pub async fn review_changes(state: State<'_, AppState>, session_id: String, changes: Changes) -> Result<Vec<String>, Error> {
    state.sessions.get(&session_id).await?.db.review(&changes).await
}

/// Commits staged changes in one transaction, and adds what ran to the history.
#[tauri::command]
#[specta::specta]
pub async fn commit_changes(state: State<'_, AppState>, session_id: String, changes: Changes) -> Result<Committed, Error> {
    let session = state.sessions.get(&session_id).await?;
    let shown = session.db.review(&changes).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let committed = session.db.commit(&changes).await;
    log::info!(
        "commit {id}: {} statements{}",
        shown.len(),
        if committed.is_err() { ", then an error" } else { "" }
    );
    state.queries.record(HistoryEntry {
        id,
        sql: shown.join("\n"),
        connection_id: session.connection.id.clone(),
        database: session.database.clone(),
        ran_at_ms: now_ms(),
        elapsed_ms: committed.as_ref().map_or(0, |c| c.elapsed_ms),
        ok: committed.is_ok(),
    })?;
    committed
}

/// Values to pick for a foreign key that points at `column` of `table`.
#[tauri::command]
#[specta::specta]
pub async fn choices(state: State<'_, AppState>, session_id: String, table: TableRef, column: String, search: String) -> Result<Vec<Choice>, Error> {
    state.sessions.get(&session_id).await?.db.choices(&table, &column, &search).await
}

/// Asks which CSV file to import and previews its first rows. `None` when the dialog was dismissed.
#[tauri::command]
#[specta::specta]
pub async fn choose_import(app: AppHandle, state: State<'_, AppState>) -> Result<Option<ImportPreview>, Error> {
    let picked = tauri::async_runtime::spawn_blocking(move || app.dialog().file().add_filter("CSV", &["csv", "tsv", "txt"]).blocking_pick_file())
        .await
        .map_err(|e| storage("The open dialog failed", e))?;
    let Some(path) = picked.as_ref().and_then(|p| p.as_path()).map(|p| p.to_path_buf()) else {
        return Ok(None);
    };
    state.imports.preview(path).map(Some)
}

/// Adds every row of the chosen file to `table` in one transaction. `columns` holds, for each of the file's columns,
/// the table column it goes into, or `None` to skip it.
#[tauri::command]
#[specta::specta]
pub async fn import_csv(
    state: State<'_, AppState>,
    session_id: String,
    table: TableRef,
    token: String,
    columns: Vec<Option<String>>,
) -> Result<f64, Error> {
    let session = state.sessions.get(&session_id).await?;
    let chosen = state.imports.get(&token)?;
    let mut source = FileSource::open(&chosen.path)?;
    let rows = session.db.import(&table, &columns, chosen.delimiter, &mut source).await?;
    log::info!("import: {rows} rows");
    state.imports.forget(&token);
    Ok(rows)
}

#[tauri::command]
#[specta::specta]
pub async fn list_history(state: State<'_, AppState>) -> Result<Vec<HistoryEntry>, Error> {
    Ok(state.queries.history())
}

#[tauri::command]
#[specta::specta]
pub async fn list_saved_queries(state: State<'_, AppState>) -> Result<Vec<SavedQuery>, Error> {
    Ok(state.queries.saved())
}

#[tauri::command]
#[specta::specta]
pub async fn save_query(state: State<'_, AppState>, id: Option<String>, name: String, sql: String) -> Result<SavedQuery, Error> {
    let name = name.trim();
    if name.is_empty() {
        return Err(Error::new(ErrorKind::Invalid, "Give the query a name."));
    }
    let query = SavedQuery {
        id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name: name.to_string(),
        sql,
    };
    state.queries.save(query.clone())?;
    Ok(query)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_saved_query(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    state.queries.forget(&id)
}

/// Now, in milliseconds since 1970 (UTC), as the history records it.
fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or_default()
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
        save_password: draft.save_password,
    })
}

fn resolve(change: &SecretChange, known: Option<String>) -> Option<String> {
    match change {
        SecretChange::Keep => known,
        SecretChange::Clear => None,
        SecretChange::Set { value } => Some(value.clone()).filter(|v| !v.is_empty()),
    }
}
