//! What `halfnine mcp` answers while the app is closed: the session's opening, pings and the list of tools, from the
//! same definitions the app serves. An agent session that never calls a tool never opens the app. The first message
//! only the app can answer ends the lobby, and the session so far is handed to the app.

use std::collections::HashSet;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResponse, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, tool_handler};
use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

use crate::agents::Server;

/// The requests answered without the app. Every notification stays here too, since none needs an answer.
const ANSWERED_HERE: &[&str] = &["initialize", "ping", "tools/list", "server/discover"];

/// How long the lobby may take to send the answers it still owes once the agent needs the app.
const LAST_ANSWERS: Duration = Duration::from_secs(5);

/// The app's server without the app. It never receives a call: the bridge hands those to the app.
struct Lobby;

#[tool_handler(router = Server::tool_router())]
impl ServerHandler for Lobby {
    fn get_info(&self) -> ServerConfig {
        Server::config()
    }

    async fn call_tool(&self, _request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, ErrorData> {
        Err(ErrorData::internal_error("Halfnine's app answers tool calls, not the lobby", None))
    }
}

/// The agent's session so far, for the app to take over.
#[derive(Default)]
pub struct Opening {
    /// The agent's `initialize` request and `initialized` notification, when its protocol version uses them. The app
    /// learns the agent's name and protocol version from them.
    replay: Vec<String>,
    /// The id of the replayed `initialize`. The agent already has its answer, so the app's is dropped.
    initialize_id: Option<Value>,
    /// The message that needed the app.
    first_call: Option<String>,
}

impl Opening {
    /// Replays the opening to the app, sends it the message that needed it, and drops the app's answer to the replayed
    /// `initialize`. Anything else the app sends meanwhile goes on to the agent.
    pub async fn hand_over<A, R, W>(&self, to_app: &mut A, from_app: &mut R, agent_out: &mut W) -> Result<(), String>
    where
        A: AsyncWrite + Unpin,
        R: AsyncBufRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        for line in self.replay.iter().chain(&self.first_call) {
            write_line(to_app, line).await?;
        }
        let Some(initialize_id) = &self.initialize_id else {
            return Ok(());
        };
        let mut line = String::new();
        loop {
            line.clear();
            if from_app.read_line(&mut line).await.map_err(|e| e.to_string())? == 0 {
                return Ok(());
            }
            if Message::read(&line).answer() == Some(initialize_id.to_string()) {
                return Ok(());
            }
            write_line(agent_out, line.trim_end()).await?;
        }
    }
}

/// Answers the agent until it sends something only the app can answer, and returns the session so far. `None` when the
/// agent hung up first, so the app was never needed.
pub async fn receive<R, W>(agent_in: &mut R, agent_out: &mut W) -> Result<Option<Opening>, String>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (lobby_end, bridge_end) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        if let Ok(running) = Lobby.serve(lobby_end).await {
            let _ = running.waiting().await;
        }
    });
    let (from_lobby, mut to_lobby) = tokio::io::split(bridge_end);
    let mut lobby_lines = BufReader::new(from_lobby).lines();
    let mut agent_lines = agent_in.lines();
    let mut opening = Opening::default();
    // Requests sent to the lobby that it hasn't answered yet, by id.
    let mut unanswered = HashSet::new();
    let first_call = loop {
        tokio::select! {
            line = agent_lines.next_line() => {
                let Some(line) = line.map_err(|e| e.to_string())? else {
                    return Ok(None);
                };
                if line.trim().is_empty() {
                    continue;
                }
                let message = Message::read(&line);
                if !message.answered_here() {
                    break line;
                }
                if message.opens_session() {
                    opening.replay.push(line.clone());
                }
                if message.method.as_deref() == Some("initialize") {
                    opening.initialize_id = message.id.clone();
                }
                if let Some(id) = message.request() {
                    unanswered.insert(id);
                }
                write_line(&mut to_lobby, &line).await?;
            }
            line = lobby_lines.next_line() => {
                let Some(line) = line.map_err(|e| e.to_string())? else {
                    return Err("the session ended before the agent called a tool".to_string());
                };
                if let Some(id) = Message::read(&line).answer() {
                    unanswered.remove(&id);
                }
                write_line(agent_out, &line).await?;
            }
        }
    };
    // The answers the lobby still owes go out before the app takes over.
    while !unanswered.is_empty() {
        let Ok(Ok(Some(line))) = tokio::time::timeout(LAST_ANSWERS, lobby_lines.next_line()).await else {
            break;
        };
        if let Some(id) = Message::read(&line).answer() {
            unanswered.remove(&id);
        }
        write_line(agent_out, &line).await?;
    }
    opening.first_call = Some(first_call);
    Ok(Some(opening))
}

/// What the bridge reads of one message: its method, and its id when it is a request or an answer.
struct Message {
    method: Option<String>,
    id: Option<Value>,
}

impl Message {
    fn read(line: &str) -> Self {
        let value = serde_json::from_str::<Value>(line).unwrap_or_default();
        Self {
            method: value.get("method").and_then(Value::as_str).map(str::to_owned),
            id: value.get("id").cloned(),
        }
    }

    /// Requests the lobby answers, and every notification. A message without a method is an answer, and the lobby
    /// asks the agent nothing, so that stays here too.
    fn answered_here(&self) -> bool {
        match self.method.as_deref() {
            None => true,
            Some(method) => method.starts_with("notifications/") || ANSWERED_HERE.contains(&method),
        }
    }

    fn opens_session(&self) -> bool {
        matches!(self.method.as_deref(), Some("initialize" | "notifications/initialized"))
    }

    /// The id of the request this message asks, written as JSON.
    fn request(&self) -> Option<String> {
        self.method.as_ref().and(self.id.as_ref()).map(Value::to_string)
    }

    /// The id of the request this message answers, written as JSON.
    fn answer(&self) -> Option<String> {
        if self.method.is_some() {
            return None;
        }
        self.id.as_ref().map(Value::to_string)
    }
}

/// Sends one message, which MCP over stdio ends with a newline.
async fn write_line<W: AsyncWrite + Unpin>(to: &mut W, line: &str) -> Result<(), String> {
    to.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
    to.write_all(b"\n").await.map_err(|e| e.to_string())?;
    to.flush().await.map_err(|e| e.to_string())
}
