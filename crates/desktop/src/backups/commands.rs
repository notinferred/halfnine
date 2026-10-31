//! The window's commands for backups: listing them, putting one back, deleting one, and the backup settings.

use hn_core::{ConnectionSpec, Error};
use serde::Serialize;
use specta::Type;
use tauri::State;

use super::Backup;
use crate::AppState;
use crate::commands::{now_ms, record};
use crate::settings::Choices;

#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BackupsState {
    /// Newest first.
    pub backups: Vec<Backup>,
    /// Their size together.
    pub bytes: f64,
}

#[tauri::command]
#[specta::specta]
pub async fn list_backups(state: State<'_, AppState>) -> Result<BackupsState, Error> {
    Ok(BackupsState {
        backups: state.backups.list(),
        bytes: state.backups.bytes(),
    })
}

/// What putting a backup back would run, to read first.
#[tauri::command]
#[specta::specta]
pub async fn review_restore(state: State<'_, AppState>, session_id: String, id: String) -> Result<Vec<String>, Error> {
    let session = state.sessions.get(&session_id).await?;
    state.backups.review_restore(&session, &id).await
}

/// Puts a table backup back in one transaction, adds what ran to the history, and gives how many rows went back.
#[tauri::command]
#[specta::specta]
pub async fn restore_backup(state: State<'_, AppState>, session_id: String, id: String) -> Result<f64, Error> {
    let session = state.sessions.get(&session_id).await?;
    let shown = state.backups.review_restore(&session, &id).await?;
    let started = now_ms();
    let restored = state.backups.restore(&session, &id).await;
    log::info!(
        "restore {id}: {} statements{}",
        shown.len(),
        if restored.is_err() { ", then an error" } else { "" }
    );
    record(&state, &session, shown, (now_ms() - started) as u32, restored.is_ok())?;
    restored
}

/// Makes a deleted Docker database again from its backup, and waits until it's back. Its progress shows on the Docker
/// page meanwhile.
#[tauri::command]
#[specta::specta]
pub async fn restore_database_backup(state: State<'_, AppState>, id: String) -> Result<ConnectionSpec, Error> {
    let backup = state.backups.get(&id)?;
    let spec = state.backups.restore_database(&state.docker, &id).await?;
    // Rules are kept by connection, so one put back under a new name gets those of the one deleted.
    if let super::Holds::Database { container } = backup.holds {
        let deleted = format!("{}{}", crate::docker::ID_PREFIX, container.name);
        if deleted != spec.id {
            state.rules.copy_connection(&deleted, &spec.id)?;
        }
    }
    Ok(spec)
}

#[tauri::command]
#[specta::specta]
pub async fn delete_backup(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    state.backups.delete(&id)
}

#[tauri::command]
#[specta::specta]
pub async fn set_backups_enabled(state: State<'_, AppState>, on: bool) -> Result<Choices, Error> {
    state.settings.change(|c| c.backups = on)
}

/// The biggest table backed up before a drop, in bytes, or `None` for no limit.
#[tauri::command]
#[specta::specta]
pub async fn set_backup_limit(state: State<'_, AppState>, bytes: Option<f64>) -> Result<Choices, Error> {
    state.settings.change(|c| c.backup_limit_bytes = bytes)
}
