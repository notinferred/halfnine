//! What the window asks of Docker databases.

use hn_core::{AgentAccess, ConnectionSpec, Error, ErrorKind, Label};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;
use tauri_specta::Event;

use super::DockerState;
use super::make::{NewDatabase, Origin};
use crate::AppState;
use crate::backups::DeleteDone;
use crate::files::storage;

/// The window should fetch `docker_state` again. It carries nothing, so a missed one loses nothing.
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
pub struct DockerChanged;

#[tauri::command]
#[specta::specta]
pub async fn docker_state(state: State<'_, AppState>) -> Result<DockerState, Error> {
    Ok(state.docker.state())
}

/// Turns Docker databases on or off in Settings. Off, Halfnine stops talking to the engine.
#[tauri::command]
#[specta::specta]
pub async fn set_docker_enabled(state: State<'_, AppState>, on: bool) -> Result<(), Error> {
    state.docker.set_on(on).await
}

#[tauri::command]
#[specta::specta]
pub async fn start_docker_database(state: State<'_, AppState>, name: String) -> Result<(), Error> {
    state.docker.start(&name).await
}

/// Stops a container, closing what the window and agents have open on it first.
#[tauri::command]
#[specta::specta]
pub async fn stop_docker_database(state: State<'_, AppState>, name: String) -> Result<(), Error> {
    state.agents.forget_connection(&format!("{}{name}", super::ID_PREFIX)).await;
    state.docker.stop(&name).await
}

/// What someone sets for a Docker database, as the connection form does for a saved one.
#[derive(Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DockerSettings {
    pub label: Label,
    pub read_only: bool,
    pub agent_access: Option<AgentAccess>,
}

#[tauri::command]
#[specta::specta]
pub async fn set_docker_settings(state: State<'_, AppState>, name: String, settings: DockerSettings) -> Result<ConnectionSpec, Error> {
    let spec = state.docker.set_overrides(&name, |overrides| {
        overrides.label = Some(settings.label);
        overrides.read_only = settings.read_only;
        overrides.agent_access = settings.agent_access;
    })?;
    // A lowered access level applies to agents at once.
    state.agents.forget_connection(&spec.id).await;
    Ok(spec)
}

/// A file chosen to load into a new database: the token the window holds for it, and its name to show.
#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChosenFile {
    pub token: String,
    pub name: String,
}

/// Opens the dialog for a dump or SQL file to load into a new database. A path never comes from the window.
#[tauri::command]
#[specta::specta]
pub async fn choose_database_file(app: AppHandle, state: State<'_, AppState>) -> Result<Option<ChosenFile>, Error> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .add_filter("Dump or SQL", &["sql", "dump", "backup", "pgdump"])
            .blocking_pick_file()
    })
    .await
    .map_err(|e| storage("The open dialog failed", e))?;
    let Some(path) = picked.as_ref().and_then(|p| p.as_path()).map(|p| p.to_path_buf()) else {
        return Ok(None);
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Some(ChosenFile {
        token: state.docker.keep_file(path),
        name,
    }))
}

/// Starts making a database. It returns at once; its progress shows on the Docker page.
#[tauri::command]
#[specta::specta]
pub async fn create_docker_database(state: State<'_, AppState>, name: String, version: u16, file: Option<String>) -> Result<(), Error> {
    if !hn_docker::VERSIONS.contains(&version) {
        return Err(Error::new(ErrorKind::Invalid, "Choose PostgreSQL 14 to 18."));
    }
    hn_docker::valid_name(&name)?;
    let load = file.map(|token| state.docker.file(&token)).transpose()?;
    let new = NewDatabase {
        name: name.clone(),
        major: version,
        image: None,
        load,
        origin: Origin::New,
        copy_of: None,
        agent: None,
        user: None,
        database: None,
        port: None,
    };
    let docker = state.docker.clone();
    let _ = state.docker.job(&name, "Starting…", async move { docker.make(new).await });
    Ok(())
}

/// Deletes a Docker database, backing it up first when backups are on, and refused with `NoBackup` when it can't be
/// backed up, unless `without_backup` says someone confirmed going ahead without one. What the window and agents have
/// open on it is closed once the backup is made.
#[tauri::command]
#[specta::specta]
pub async fn delete_docker_database(state: State<'_, AppState>, name: String, without_backup: bool) -> Result<DeleteDone, Error> {
    let id = format!("{}{name}", super::ID_PREFIX);
    let delete = async || {
        state.agents.forget_connection(&id).await;
        state.sessions.close_connection(&id).await;
        state.docker.delete(&name).await
    };
    state
        .backups
        .delete_database(&state.settings.get(), &state.docker, &name, None, without_backup, delete)
        .await
}

#[tauri::command]
#[specta::specta]
pub async fn dismiss_docker_job(state: State<'_, AppState>, name: String) -> Result<(), Error> {
    state.docker.dismiss(&name);
    Ok(())
}

/// Starts copying a Docker database into a new one. It returns at once; its progress shows on the Docker page.
#[tauri::command]
#[specta::specta]
pub async fn copy_docker_database(state: State<'_, AppState>, source: String, name: String) -> Result<(), Error> {
    hn_docker::valid_name(&name)?;
    let new = NewDatabase {
        name: name.clone(),
        major: 18,
        image: None,
        load: None,
        origin: Origin::Copy,
        copy_of: Some(source.clone()),
        agent: None,
        user: None,
        database: None,
        port: None,
    };
    let docker = state.docker.clone();
    let _ = state
        .docker
        .job(&name, &format!("Copying {source}…"), async move { docker.copy(&source, new).await });
    Ok(())
}
