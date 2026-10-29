//! What AI agents may do through `halfnine mcp`, decided here for every call, and what the app keeps about them: their
//! own database sessions, the changes they propose, the allowances given until quit, and the pause switch. Approval
//! happens only through the window's commands; nothing an agent sends can approve, allow or unpause anything.

pub mod activity;
pub mod commands;
pub mod desk;
pub mod listen;
mod results;
mod server;
mod staging;
mod tools;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use hn_core::{AgentAccess, Changes, ConnectionSpec, Database, Error, ErrorKind, Filter, Label, TableRef};
use hn_pg::{PgDatabase, Secrets};
use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::watch;

use crate::commands::{SessionInfo, now_ms, open_with};
use crate::files;
use crate::secrets::Passwords;
use crate::sessions::{Session, Sessions};
use crate::store::Store;
use activity::{ActivityEntry, ActivityLog, Call, Done, Outcome};
use results::{Reply, Status, plural};

pub use desk::Desk;
pub use server::Server;

/// How long a call waits for the person before answering WAITING, under the 60 seconds some clients allow a call.
const WAIT: Duration = Duration::from_secs(45);
const SETTINGS_VERSION: u32 = 1;
/// Settled proposals are kept this long, so an agent that comes back late still learns what happened.
const KEEP_SETTLED: f64 = 60.0 * 60.0 * 1000.0;

/// An agent as its MCP client names itself. The name is the client's own claim, so it labels things and keys the
/// allowances given until quit, and never grants anything by itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agent {
    pub key: String,
    pub title: String,
}

impl Agent {
    pub fn from_client(name: &str, title: Option<&str>) -> Self {
        let known = match name {
            "claude-code" => Some("Claude Code"),
            "claude-ai" => Some("Claude Desktop"),
            "codex-mcp-client" | "codex" => Some("Codex"),
            "cursor-vscode" => Some("Cursor"),
            "Visual Studio Code" => Some("VS Code"),
            _ => None,
        };
        let shown = known.map(str::to_string).unwrap_or_else(|| clean(title.unwrap_or(name)));
        Self {
            key: clean(name),
            title: if shown.is_empty() { "An agent".to_string() } else { shown },
        }
    }
}

/// A name fit to show: printable, on one line, and short.
fn clean(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(40).collect::<String>().trim().to_string()
}

/// What a call asks of a connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Read,
    /// A write, and whether it drops or empties a table, which always asks at "Read with approved writes".
    Write {
        drops: bool,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Run,
    Propose,
    Refuse(String),
}

/// The whole access rule. `allowed` is "Yes, don't ask again here" given to this agent on this connection.
pub fn verdict(spec: &ConnectionSpec, ask: Ask, allowed: bool) -> Verdict {
    let name = &spec.name;
    let change = "The person you're working with can change that in Halfnine, in the connection's settings.";
    match (spec.agent_access(), ask) {
        (AgentAccess::Off, _) => Verdict::Refuse(format!("Agents are off for {name}. {change}")),
        (_, Ask::Read) => Verdict::Run,
        (_, Ask::Write { .. }) if spec.read_only => Verdict::Refuse(format!("{name} is a read-only connection, so agents can only read it.")),
        (AgentAccess::Read, Ask::Write { .. }) => Verdict::Refuse(format!("Agents may read {name} but not change it. {change}")),
        (AgentAccess::ApprovedWrites, Ask::Write { drops }) if allowed && !drops => Verdict::Run,
        (AgentAccess::ApprovedWrites, _) => Verdict::Propose,
        (AgentAccess::Full, _) => Verdict::Run,
    }
}

/// What an approved change runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Work {
    Changes(Changes),
    Statements(Vec<String>),
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ProposalState {
    Pending,
    Committing,
    Approved { statements: u32, rows: f64, elapsed_ms: u32 },
    Declined { reason: String },
    Failed { error: Error },
    Withdrawn { why: String },
}

impl ProposalState {
    fn settled(&self) -> bool {
        !matches!(self, ProposalState::Pending | ProposalState::Committing)
    }
}

/// A change staged for the person, in the form the window shows and the commit runs.
pub struct Staged {
    pub work: Work,
    pub shown: Vec<String>,
    pub table: Option<TableRef>,
    /// The rows it touches, so the window shows exactly those.
    pub filter: Option<Filter>,
    pub rows: usize,
    /// What it changes, as in "wants to change 3 rows in users".
    pub summary: String,
    pub drops: bool,
}

struct Proposal {
    id: String,
    agent: Agent,
    mcp_session: String,
    call: String,
    session: SessionInfo,
    staged: Staged,
    created_ms: f64,
    state: Arc<watch::Sender<ProposalState>>,
    dismissed: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ProposalKind {
    Rows,
    Structure,
    Statements,
}

/// A proposal as the window shows it. Approving it commits what is kept here, never anything the window sends back.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProposalView {
    pub id: String,
    pub agent: String,
    /// The agent's own session on that database, which the window pages through to show the change.
    pub session: SessionInfo,
    pub kind: ProposalKind,
    pub table: Option<TableRef>,
    pub changes: Option<Changes>,
    pub shown: Vec<String>,
    pub filter: Option<Filter>,
    pub summary: String,
    pub state: ProposalState,
    pub created_ms: f64,
}

struct PasswordRequest {
    id: String,
    agent: String,
    connection: ConnectionSpec,
    answered: Arc<watch::Sender<Option<bool>>>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PasswordRequestView {
    pub id: String,
    pub agent: String,
    pub connection: ConnectionSpec,
}

struct Client {
    agent: Option<Agent>,
    since_ms: f64,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ClientView {
    pub id: String,
    pub agent: String,
    pub since_ms: f64,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentState {
    pub paused: bool,
    pub clients: Vec<ClientView>,
    pub proposals: Vec<ProposalView>,
    pub passwords: Vec<PasswordRequestView>,
}

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    paused: bool,
}

pub struct Agents {
    store: Arc<Store>,
    passwords: Arc<Passwords>,
    sessions: Arc<Sessions>,
    desk: Arc<dyn Desk>,
    log: ActivityLog,
    settings: PathBuf,
    paused: AtomicBool,
    /// Each agent database session by (connection id, database), shared by every agent and apart from the window's.
    databases: tokio::sync::Mutex<HashMap<(String, String), SessionInfo>>,
    proposals: Mutex<HashMap<String, Proposal>>,
    /// "Yes, don't ask again here": (agent, connection id), until quit and never saved.
    allowed: Mutex<HashSet<(String, String)>>,
    clients: Mutex<HashMap<String, Client>>,
    /// Password requests by connection id, one at a time for each.
    asked: Mutex<HashMap<String, PasswordRequest>>,
    wait: Duration,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn new_id(prefix: &str) -> String {
    format!("{prefix}-{}", &uuid::Uuid::new_v4().simple().to_string()[..12])
}

impl Agents {
    pub fn new(dir: &Path, store: Arc<Store>, passwords: Arc<Passwords>, sessions: Arc<Sessions>, desk: Arc<dyn Desk>) -> Self {
        let settings = dir.join("agents.json");
        let paused = files::load::<Settings>(&settings, SETTINGS_VERSION, "the agent settings").is_ok_and(|s| s.paused);
        Self {
            store,
            passwords,
            sessions,
            desk,
            log: ActivityLog::open(dir.to_path_buf()),
            settings,
            paused: AtomicBool::new(paused),
            databases: tokio::sync::Mutex::new(HashMap::new()),
            proposals: Mutex::new(HashMap::new()),
            allowed: Mutex::new(HashSet::new()),
            clients: Mutex::new(HashMap::new()),
            asked: Mutex::new(HashMap::new()),
            wait: WAIT,
        }
    }

    #[cfg(test)]
    pub fn waiting_at_most(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    pub fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    /// Pausing withdraws whatever agents are waiting on and closes their database sessions.
    pub async fn set_paused(&self, paused: bool) -> Result<(), Error> {
        files::save(&self.settings, SETTINGS_VERSION, &Settings { paused }, "the agent settings")?;
        self.paused.store(paused, Ordering::SeqCst);
        if paused {
            self.withdraw_where(|_| true, "The person paused agents in Halfnine.");
            for request in lock(&self.asked).drain().map(|(_, r)| r) {
                request.answered.send_replace(Some(false));
            }
            let open: Vec<SessionInfo> = self.databases.lock().await.drain().map(|(_, info)| info).collect();
            for info in open {
                self.sessions.close(&info.session_id).await;
            }
        }
        self.desk.changed();
        Ok(())
    }

    pub fn connected(&self, mcp_session: &str) {
        lock(&self.clients).insert(
            mcp_session.to_string(),
            Client {
                agent: None,
                since_ms: now_ms(),
            },
        );
        self.desk.changed();
    }

    pub fn disconnected(&self, mcp_session: &str) {
        lock(&self.clients).remove(mcp_session);
        self.withdraw_where(|p| p.mcp_session == mcp_session, "The agent disconnected.");
        self.desk.changed();
    }

    /// Records which agent a session is, from its first call.
    fn named(&self, mcp_session: &str, agent: &Agent) {
        let named = match lock(&self.clients).get_mut(mcp_session) {
            Some(client) if client.agent.as_ref() != Some(agent) => {
                client.agent = Some(agent.clone());
                true
            }
            _ => false,
        };
        if named {
            self.desk.changed();
        }
    }

    /// A connection was edited or deleted: what agents had open or waiting on it ends, and so do its allowances.
    pub async fn forget_connection(&self, id: &str) {
        self.withdraw_where(|p| p.session.connection.id == id, "The connection's settings changed in Halfnine.");
        lock(&self.allowed).retain(|(_, connection)| connection != id);
        let closing: Vec<SessionInfo> = {
            let mut databases = self.databases.lock().await;
            let keys: Vec<_> = databases.keys().filter(|(connection, _)| connection == id).cloned().collect();
            keys.iter().filter_map(|k| databases.remove(k)).collect()
        };
        for info in closing {
            self.sessions.close(&info.session_id).await;
        }
        self.desk.changed();
    }

    /// The connection an agent named, by the name the person gave it.
    fn connection(&self, name: &str) -> Result<ConnectionSpec, Reply> {
        let matching: Vec<ConnectionSpec> = self.store.list().into_iter().filter(|c| c.name == name).collect();
        match matching.len() {
            1 => Ok(matching.into_iter().next().expect("one connection")),
            0 => Err(Reply::new(
                Status::Refused,
                format!("Halfnine has no connection named \"{name}\". list_connections shows them."),
            )),
            _ => Err(Reply::new(
                Status::Refused,
                format!("Halfnine has more than one connection named \"{name}\". Ask the person to rename one."),
            )),
        }
    }

    fn allowed(&self, agent: &Agent, spec: &ConnectionSpec) -> bool {
        lock(&self.allowed).contains(&(agent.key.clone(), spec.id.clone()))
    }

    /// Writes a call's first line. A call that can't be logged is refused, so nothing an agent does goes unrecorded.
    fn begin(&self, agent: &Agent, tool: &str, spec: Option<&ConnectionSpec>, database: Option<&str>, sql: Vec<String>) -> Result<String, Reply> {
        let call = new_id("c");
        self.log
            .call(Call {
                call: call.clone(),
                at_ms: now_ms(),
                agent: agent.key.clone(),
                agent_title: agent.title.clone(),
                tool: tool.to_string(),
                connection_id: spec.map(|s| s.id.clone()),
                connection: spec.map(|s| s.name.clone()),
                database: database.map(str::to_string).or_else(|| spec.map(|s| s.database.clone())),
                proposal: None,
                sql,
            })
            .map_err(|e| {
                Reply::new(
                    Status::Error,
                    format!("Halfnine refused the call because it couldn't log it: {}", e.message),
                )
            })?;
        self.desk.activity();
        Ok(call)
    }

    fn end(&self, call: &str, outcome: Outcome, rows: Option<f64>, elapsed_ms: Option<u32>, error: Option<&Error>, reason: Option<String>) {
        let _ = self.log.done(Done {
            call: call.to_string(),
            at_ms: now_ms(),
            outcome,
            rows,
            elapsed_ms,
            code: error.and_then(|e| e.code.clone()),
            reason,
        });
        self.desk.activity();
    }

    /// Logs how a call ended, from the reply it gets.
    fn ended(&self, call: &str, reply: &Reply) {
        let outcome = match reply.status {
            Status::Ok => Outcome::Ok,
            Status::Waiting => Outcome::Waiting,
            Status::Approved => Outcome::Approved,
            Status::Declined => Outcome::Declined,
            Status::Conflict => Outcome::Conflict,
            Status::Refused => Outcome::Refused,
            Status::Paused => Outcome::Paused,
            Status::Busy => Outcome::Busy,
            Status::Error => Outcome::Error,
        };
        self.end(call, outcome, None, None, None, None);
    }

    fn secrets(&self, spec: &ConnectionSpec) -> Result<Secrets, Error> {
        if spec.save_password {
            self.passwords.saved(&spec.id)
        } else {
            self.passwords
                .remembered(&spec.id)
                .ok_or_else(|| Error::new(ErrorKind::PasswordNeeded, format!("Halfnine needs the password for {} first.", spec.name)))
        }
    }

    /// The agents' own session on a database, opened on first use. For a connection that asks for its password each
    /// time, Halfnine asks the person, once, and the agent waits.
    async fn database(&self, agent: &Agent, spec: &ConnectionSpec, database: Option<&str>) -> Result<(Arc<Session>, SessionInfo), Reply> {
        let name = database.unwrap_or(&spec.database).to_string();
        let key = (spec.id.clone(), name.clone());
        let known = self.databases.lock().await.get(&key).cloned();
        if let Some(info) = known
            && let Ok(session) = self.sessions.get(&info.session_id).await
        {
            return Ok((session, info));
        }
        let secrets = match self.secrets(spec) {
            Ok(secrets) => secrets,
            Err(e) if e.kind == ErrorKind::PasswordNeeded => {
                self.ask_password(agent, spec).await?;
                self.secrets(spec)?
            }
            Err(e) => return Err(e.into()),
        };
        let opened = open_with(&self.sessions, spec.clone(), Some(&name), secrets).await;
        if !spec.save_password && opened.as_ref().is_err_and(|e| e.kind == ErrorKind::Auth) {
            self.passwords.forget_typed(&spec.id);
        }
        let info = opened?;
        let mut databases = self.databases.lock().await;
        let info = match databases.get(&key) {
            // Another call opened it meanwhile; that one is kept.
            Some(existing) => {
                self.sessions.close(&info.session_id).await;
                existing.clone()
            }
            None => {
                databases.insert(key, info.clone());
                info
            }
        };
        drop(databases);
        Ok((self.sessions.get(&info.session_id).await?, info))
    }

    async fn ask_password(&self, agent: &Agent, spec: &ConnectionSpec) -> Result<(), Reply> {
        let mut answered = lock(&self.asked)
            .entry(spec.id.clone())
            .or_insert_with(|| PasswordRequest {
                id: new_id("w"),
                agent: agent.title.clone(),
                connection: spec.clone(),
                answered: Arc::new(watch::channel(None).0),
            })
            .answered
            .subscribe();
        self.desk.changed();
        self.desk.come_forward();
        let given = tokio::time::timeout(self.wait, answered.wait_for(Option::is_some))
            .await
            .map(|answer| answer.map(|a| *a == Some(true)));
        match given {
            Ok(Ok(true)) => Ok(()),
            Ok(_) => Err(Reply::new(
                Status::Refused,
                format!("The person didn't give Halfnine the password for {}.", spec.name),
            )),
            Err(_) => Err(Reply::new(
                Status::Waiting,
                format!(
                    "Halfnine is asking the person for the password of {}. Call this tool again in a moment.",
                    spec.name
                ),
            )),
        }
    }

    /// The person typed the password an agent needed. It is remembered until quit, as when they connect themselves.
    pub async fn answer_password(&self, id: &str, secrets: Secrets) -> Result<(), Error> {
        let spec = lock(&self.asked)
            .values()
            .find(|r| r.id == id)
            .map(|r| r.connection.clone())
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "The agent stopped waiting for this password."))?;
        let db = PgDatabase::connect(&spec, None, &secrets).await?;
        db.close().await;
        self.passwords.remember(&spec.id, &secrets);
        if let Some(request) = lock(&self.asked).remove(&spec.id) {
            request.answered.send_replace(Some(true));
        }
        self.desk.changed();
        Ok(())
    }

    pub fn cancel_password(&self, id: &str) {
        let mut asked = lock(&self.asked);
        let found = asked.iter().find(|(_, r)| r.id == id).map(|(k, _)| k.clone());
        if let Some(request) = found.and_then(|k| asked.remove(&k)) {
            request.answered.send_replace(Some(false));
        }
        drop(asked);
        self.desk.changed();
    }

    /// Stages `staged` for the person, unless this agent session already waits on another change.
    fn propose(&self, agent: &Agent, mcp_session: &str, call: &str, session: SessionInfo, staged: Staged) -> Result<String, Reply> {
        let mut proposals = lock(&self.proposals);
        let now = now_ms();
        proposals.retain(|_, p| !p.state.borrow().settled() || now - p.created_ms < KEEP_SETTLED);
        let waiting = proposals.values().find(|p| p.mcp_session == mcp_session && !p.state.borrow().settled());
        if let Some(waiting) = waiting {
            return if waiting.staged.work == staged.work && waiting.session.connection.id == session.connection.id {
                Ok(waiting.id.clone())
            } else {
                Err(Reply::new(
                    Status::Busy,
                    format!(
                        "Proposal {} is still waiting for the person in Halfnine. Wait for it with wait_for_approval before proposing another.",
                        waiting.id
                    ),
                ))
            };
        }
        let id = new_id("p");
        proposals.insert(
            id.clone(),
            Proposal {
                id: id.clone(),
                agent: agent.clone(),
                mcp_session: mcp_session.to_string(),
                call: call.to_string(),
                session,
                staged,
                created_ms: now,
                state: Arc::new(watch::channel(ProposalState::Pending).0),
                dismissed: false,
            },
        );
        drop(proposals);
        self.desk.changed();
        self.desk.come_forward();
        Ok(id)
    }

    /// Waits up to the budget for the person to settle a proposal, and says how it ended or that it still waits.
    pub async fn wait_for(&self, id: &str, mcp_session: &str) -> Reply {
        let state = {
            let proposals = lock(&self.proposals);
            match proposals.get(id) {
                Some(p) if p.mcp_session == mcp_session || p.agent.key == self.agent_of(mcp_session).unwrap_or_default() => p.state.subscribe(),
                _ => return Reply::new(Status::Refused, format!("There is no proposal {id} from this agent.")),
            }
        };
        let mut state = state;
        let settled = tokio::time::timeout(self.wait, state.wait_for(ProposalState::settled))
            .await
            .map(|s| s.map(|s| s.clone()));
        match settled {
            Ok(Ok(state)) => settled_reply(&state),
            Ok(Err(_)) => Reply::new(Status::Error, "Halfnine forgot this proposal."),
            Err(_) => Reply::new(
                Status::Waiting,
                format!(
                    "Proposal {id} is staged in Halfnine for the person to review. Call wait_for_approval with this id to keep waiting. Don't propose it again."
                ),
            ),
        }
    }

    fn agent_of(&self, mcp_session: &str) -> Option<String> {
        lock(&self.clients).get(mcp_session).and_then(|c| c.agent.as_ref().map(|a| a.key.clone()))
    }

    fn withdraw_where(&self, which: impl Fn(&Proposal) -> bool, why: &str) {
        let withdrawn: Vec<String> = {
            let proposals = lock(&self.proposals);
            proposals
                .values()
                .filter(|p| which(p) && matches!(*p.state.borrow(), ProposalState::Pending))
                .map(|p| {
                    p.state.send_replace(ProposalState::Withdrawn { why: why.to_string() });
                    p.call.clone()
                })
                .collect()
        };
        for call in &withdrawn {
            self.end(call, Outcome::Withdrawn, None, None, None, Some(why.to_string()));
        }
        if !withdrawn.is_empty() {
            self.desk.changed();
        }
    }

    /// The agent cancelled a call that waited on the person, so what it proposed goes away.
    pub fn withdraw_waiting(&self, mcp_session: &str, proposal: &str) {
        self.withdraw_where(
            |p| p.mcp_session == mcp_session && (proposal.is_empty() || p.id == proposal),
            "The agent stopped waiting.",
        );
    }

    /// A call that touches no database: logged before and after, like every other.
    async fn logged<F, Fut>(&self, agent: &Agent, tool: &str, spec: Option<&ConnectionSpec>, database: Option<&str>, work: F) -> Reply
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Reply>,
    {
        let call = match self.begin(agent, tool, spec, database, Vec::new()) {
            Ok(call) => call,
            Err(reply) => return reply,
        };
        let reply = work().await;
        self.ended(&call, &reply);
        reply
    }

    /// The person approved a proposal in the window. It commits exactly what was staged.
    pub async fn approve(&self, id: &str, remember: bool) -> Result<(), Error> {
        let (state, call, session_id, work, rows, allowance) = {
            let proposals = lock(&self.proposals);
            let p = proposals
                .get(id)
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "This change is no longer waiting."))?;
            if !matches!(*p.state.borrow(), ProposalState::Pending) {
                return Ok(());
            }
            p.state.send_replace(ProposalState::Committing);
            (
                p.state.clone(),
                p.call.clone(),
                p.session.session_id.clone(),
                p.staged.work.clone(),
                p.staged.rows,
                (p.agent.key.clone(), p.session.connection.id.clone()),
            )
        };
        self.desk.changed();
        if remember {
            lock(&self.allowed).insert(allowance);
        }
        let ran = match self.sessions.get(&session_id).await {
            Ok(session) => run(&session.db, &work, rows).await,
            Err(e) => Err(e),
        };
        let result = match ran {
            Ok((statements, rows, elapsed_ms)) => {
                self.end(&call, Outcome::Approved, Some(rows), Some(elapsed_ms), None, None);
                state.send_replace(ProposalState::Approved {
                    statements,
                    rows,
                    elapsed_ms,
                });
                Ok(())
            }
            Err(error) => {
                let outcome = if error.kind == ErrorKind::Conflict {
                    Outcome::Conflict
                } else {
                    Outcome::Error
                };
                self.end(&call, outcome, None, None, Some(&error), None);
                state.send_replace(ProposalState::Failed { error: error.clone() });
                Err(error)
            }
        };
        self.desk.changed();
        result
    }

    pub fn decline(&self, id: &str, reason: &str) {
        let call = {
            let proposals = lock(&self.proposals);
            match proposals.get(id) {
                Some(p) if matches!(*p.state.borrow(), ProposalState::Pending) => {
                    p.state.send_replace(ProposalState::Declined {
                        reason: reason.trim().to_string(),
                    });
                    Some(p.call.clone())
                }
                _ => None,
            }
        };
        if let Some(call) = call {
            let reason = Some(reason.trim().to_string()).filter(|r| !r.is_empty());
            self.end(&call, Outcome::Declined, None, None, None, reason);
        }
        self.desk.changed();
    }

    /// The person closed a failed proposal's tab.
    pub fn dismiss(&self, id: &str) {
        if let Some(p) = lock(&self.proposals).get_mut(id) {
            p.dismissed = true;
        }
        self.desk.changed();
    }

    pub fn state(&self) -> AgentState {
        let mut proposals: Vec<ProposalView> = lock(&self.proposals)
            .values()
            .filter(|p| match &*p.state.borrow() {
                ProposalState::Pending | ProposalState::Committing => true,
                ProposalState::Failed { .. } => !p.dismissed,
                _ => false,
            })
            .map(|p| ProposalView {
                id: p.id.clone(),
                agent: p.agent.title.clone(),
                session: p.session.clone(),
                kind: match &p.staged.work {
                    Work::Changes(Changes::Rows { .. }) => ProposalKind::Rows,
                    Work::Changes(Changes::Structure { .. }) => ProposalKind::Structure,
                    Work::Statements(_) => ProposalKind::Statements,
                },
                table: p.staged.table.clone(),
                changes: match &p.staged.work {
                    Work::Changes(changes) => Some(changes.clone()),
                    Work::Statements(_) => None,
                },
                shown: p.staged.shown.clone(),
                filter: p.staged.filter.clone(),
                summary: p.staged.summary.clone(),
                state: p.state.borrow().clone(),
                created_ms: p.created_ms,
            })
            .collect();
        proposals.sort_by(|a, b| a.created_ms.total_cmp(&b.created_ms));
        let mut clients: Vec<ClientView> = lock(&self.clients)
            .iter()
            .map(|(id, c)| ClientView {
                id: id.clone(),
                agent: c.agent.as_ref().map_or_else(|| "An agent".to_string(), |a| a.title.clone()),
                since_ms: c.since_ms,
            })
            .collect();
        clients.sort_by(|a, b| a.since_ms.total_cmp(&b.since_ms));
        AgentState {
            paused: self.paused(),
            clients,
            proposals,
            passwords: lock(&self.asked)
                .values()
                .map(|r| PasswordRequestView {
                    id: r.id.clone(),
                    agent: r.agent.clone(),
                    connection: r.connection.clone(),
                })
                .collect(),
        }
    }

    pub fn activity(&self, limit: usize) -> Vec<ActivityEntry> {
        self.log.recent(limit)
    }

    /// The write path every changing tool shares: the access rule, then either running it now or staging it for the
    /// person and waiting for their answer.
    async fn write(&self, agent: &Agent, mcp_session: &str, tool: &str, request: staging::Request) -> Reply {
        let (spec, ask_reply) = match self.connection(&request.connection) {
            Ok(spec) => (Some(spec), None),
            Err(reply) => (None, Some(reply)),
        };
        let quick = ask_reply.or_else(|| {
            if self.paused() {
                return Some(paused());
            }
            let spec = spec.as_ref()?;
            match verdict(spec, Ask::Write { drops: request.drops() }, self.allowed(agent, spec)) {
                Verdict::Refuse(why) => Some(Reply::new(Status::Refused, why)),
                _ => None,
            }
        });
        if let Some(reply) = quick {
            if let Ok(call) = self.begin(agent, tool, spec.as_ref(), request.database.as_deref(), Vec::new()) {
                self.ended(&call, &reply);
            }
            return reply;
        }
        let spec = spec.expect("a connection was found");
        let (session, info) = match self.database(agent, &spec, request.database.as_deref()).await {
            Ok(opened) => opened,
            Err(reply) => return reply,
        };
        let staged = match staging::stage(&session.db, request.work).await {
            Ok(Some(staged)) => staged,
            Ok(None) => return Reply::new(Status::Ok, "Nothing to change: the rows already hold those values."),
            Err(reply) => {
                if let Ok(call) = self.begin(agent, tool, Some(&spec), Some(&info.server.database), Vec::new()) {
                    self.ended(&call, &reply);
                }
                return reply;
            }
        };
        let call = match self.begin(agent, tool, Some(&spec), Some(&info.server.database), staged.shown.clone()) {
            Ok(call) => call,
            Err(reply) => return reply,
        };
        if verdict(&spec, Ask::Write { drops: staged.drops }, self.allowed(agent, &spec)) == Verdict::Run {
            let started = Instant::now();
            return match run(&session.db, &staged.work, staged.rows).await {
                Ok((statements, rows, elapsed_ms)) => {
                    self.end(&call, Outcome::Ok, Some(rows), Some(elapsed_ms), None, None);
                    Reply::new(
                        Status::Ok,
                        format!(
                            "Committed {}, {}, in {} ms.",
                            plural(statements as usize, "statement"),
                            plural(rows as usize, "row"),
                            hn_core::millis(started.elapsed())
                        ),
                    )
                }
                Err(error) => {
                    let reply = Reply::from(error.clone());
                    self.end(&call, Outcome::Error, None, None, Some(&error), None);
                    reply
                }
            };
        }
        let id = match self.propose(agent, mcp_session, &call, info, staged) {
            Ok(id) => id,
            Err(reply) => {
                self.ended(&call, &reply);
                return reply;
            }
        };
        let reply = self.wait_for(&id, mcp_session).await;
        if reply.status == Status::Waiting {
            self.end(&call, Outcome::Waiting, None, None, None, None);
        }
        reply
    }
}

fn paused() -> Reply {
    Reply::new(
        Status::Paused,
        "The person has paused all agents in Halfnine. Ask them to resume agents there.",
    )
}

/// Runs approved work: a commit of staged changes, or statements in one transaction. Returns the statements, rows
/// and milliseconds it took.
async fn run(db: &PgDatabase, work: &Work, rows: usize) -> Result<(u32, f64, u32), Error> {
    match work {
        Work::Changes(changes) => db.commit(changes).await.map(|c| (c.statements, rows as f64, c.elapsed_ms)),
        Work::Statements(statements) => {
            let started = Instant::now();
            let results = db.execute(statements).await?;
            let affected = results.iter().map(|r| f64::from(r.rows_affected)).sum();
            Ok((results.len() as u32, affected, hn_core::millis(started.elapsed())))
        }
    }
}

fn settled_reply(state: &ProposalState) -> Reply {
    match state {
        ProposalState::Approved {
            statements,
            rows,
            elapsed_ms,
        } => Reply::new(
            Status::Approved,
            format!(
                "The person approved it in Halfnine, and it was committed: {}, {}, in {elapsed_ms} ms.",
                plural(*statements as usize, "statement"),
                plural(*rows as usize, "row")
            ),
        ),
        ProposalState::Declined { reason } if reason.is_empty() => Reply::new(
            Status::Declined,
            "The person declined it in Halfnine. Nothing was written. Don't make this change again unless they ask.",
        ),
        ProposalState::Declined { reason } => Reply::new(
            Status::Declined,
            format!("The person declined it in Halfnine and said: \"{reason}\". Nothing was written. Don't make this change again unless they ask."),
        ),
        ProposalState::Failed { error } => {
            let reply = Reply::from(error.clone());
            Reply::new(reply.status, format!("The person approved it, but it failed: {}", reply.text))
        }
        ProposalState::Withdrawn { why } => Reply::new(Status::Refused, format!("Withdrawn: {why} Nothing was written.")),
        ProposalState::Pending | ProposalState::Committing => Reply::new(Status::Waiting, "Still waiting."),
    }
}

/// Labels as agents read them.
fn label_name(label: Label) -> &'static str {
    match label {
        Label::None => "no label",
        Label::Green => "green label",
        Label::Amber => "amber label",
        Label::Red => "red label",
    }
}
