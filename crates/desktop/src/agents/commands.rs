//! What the window asks of agents. Only these approve, decline, allow or pause, and only a person in the window calls them.

use hn_core::{ConnectionSpec, Error, HiddenColumn, TableRef};
use hn_pg::Secrets;
use tauri::State;

use super::AgentState;
use super::activity::ActivityEntry;
use super::undo::UndoRecord;
use crate::AppState;
use crate::commands::Typed;

#[tauri::command]
#[specta::specta]
pub async fn agent_state(state: State<'_, AppState>) -> Result<AgentState, Error> {
    Ok(state.agents.state())
}

/// Commits what the agent proposed. With `remember`, its next changes on that connection, short of dropping or
/// emptying a table, run without asking until Halfnine quits.
#[tauri::command]
#[specta::specta]
pub async fn approve_proposal(state: State<'_, AppState>, id: String, remember: bool) -> Result<(), Error> {
    state.agents.approve(&id, remember).await
}

#[tauri::command]
#[specta::specta]
pub async fn decline_proposal(state: State<'_, AppState>, id: String, reason: String) -> Result<(), Error> {
    state.agents.decline(&id, &reason);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn dismiss_proposal(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    state.agents.dismiss(&id);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn answer_password_request(state: State<'_, AppState>, id: String, typed: Typed) -> Result<(), Error> {
    let secrets = Secrets {
        password: typed.password.filter(|p| !p.is_empty()),
        ssh: typed.ssh_secret.filter(|p| !p.is_empty()),
    };
    state.agents.answer_password(&id, secrets).await
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_password_request(state: State<'_, AppState>, id: String) -> Result<(), Error> {
    state.agents.cancel_password(&id);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn set_agents_paused(state: State<'_, AppState>, paused: bool) -> Result<(), Error> {
    state.agents.set_paused(paused).await
}

#[tauri::command]
#[specta::specta]
pub async fn agent_activity(state: State<'_, AppState>, limit: u32) -> Result<Vec<ActivityEntry>, Error> {
    Ok(state.agents.activity(limit as usize))
}

/// The reverse of an agent's row change, to stage in its table and commit the usual way.
#[tauri::command]
#[specta::specta]
pub async fn undo_change(state: State<'_, AppState>, call: String) -> Result<UndoRecord, Error> {
    state.agents.undo(&call)
}

/// The reverse was committed, so the change has nothing left to undo.
#[tauri::command]
#[specta::specta]
pub async fn forget_undo(state: State<'_, AppState>, call: String) -> Result<(), Error> {
    state.agents.undone(&call)
}

/// Hides a column from agents, or shows it to them again. It applies from their next call.
#[tauri::command]
#[specta::specta]
pub async fn set_column_hidden(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: TableRef,
    column: String,
    hidden: bool,
) -> Result<ConnectionSpec, Error> {
    let mut spec = state.store.get(&connection_id)?;
    let entry = HiddenColumn {
        database,
        schema: table.schema,
        table: table.name,
        column,
    };
    spec.hidden_columns.retain(|h| h != &entry);
    if hidden {
        spec.hidden_columns.push(entry);
    }
    state.store.save(spec.clone())?;
    Ok(spec)
}
