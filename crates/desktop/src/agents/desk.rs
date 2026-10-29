//! What agents need from the window: to say that something changed, and to come forward when only a person can answer.

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Manager, UserAttentionType};
use tauri_specta::Event;

pub trait Desk: Send + Sync + 'static {
    /// Something agents wait on or hold changed: a proposal, a password request, a connected agent, the pause switch.
    fn changed(&self);
    /// A line was added to the activity log.
    fn activity(&self);
    /// Brings the window forward, for an approval or a password only the person can give.
    fn come_forward(&self);
}

/// The window should fetch `agent_state` again. It carries nothing, so a missed one loses nothing.
#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
pub struct AgentsChanged;

#[derive(Clone, Debug, Serialize, Deserialize, Type, Event)]
pub struct AgentActivityAdded;

pub struct WindowDesk(pub AppHandle);

impl Desk for WindowDesk {
    fn changed(&self) {
        let _ = AgentsChanged.emit(&self.0);
    }

    fn activity(&self) {
        let _ = AgentActivityAdded.emit(&self.0);
    }

    fn come_forward(&self) {
        if let Some(window) = self.0.get_webview_window("main") {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
            // Where the system won't move focus to an app in the background, the Dock icon or taskbar button asks instead.
            let _ = window.request_user_attention(Some(UserAttentionType::Critical));
        }
    }
}
