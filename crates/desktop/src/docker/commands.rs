//! What the window asks of Docker databases.

use hn_core::{AgentAccess, ConnectionSpec, Error, Label};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;
use tauri_specta::Event;

use super::DockerState;
use crate::AppState;

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
