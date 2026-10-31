//! Docker databases for agents: making, copying, starting and deleting them. Making and copying are jobs the Docker
//! page shows, and asking again for the same name joins the job already running, so an agent can't make two by
//! retrying. An agent deletes a database an agent of its name made at once; any other only once the person says yes.

use std::sync::Arc;
use std::time::Duration;

use hn_core::{AgentAccess, AgentMark, ConnectionSpec, Error, ErrorKind, Made, Source};
use serde::Serialize;
use specta::Type;
use tokio::sync::watch;

use super::results::{Reply, Status};
use super::{Agent, Agents, Ask, Verdict, lock, new_id, verdict};
use crate::backups::DeleteDone;
use crate::docker::make::{NewDatabase, Origin, Outcome};

/// The most Docker databases one agent may have made, so a loop can't fill the disk.
pub const DATABASES_LIMIT: usize = 10;
/// How long a call waits for a database before it answers WAITING.
const JOB_WAIT: Duration = Duration::from_secs(45);

impl Agent {
    fn mark(&self) -> AgentMark {
        AgentMark {
            key: self.key.clone(),
            title: self.title.clone(),
        }
    }

    /// Whether an agent of this name made the database.
    pub fn made(&self, spec: &ConnectionSpec) -> bool {
        matches!(&spec.source, Source::Docker { made: Some(Made { agent: Some(a), .. }), .. } if a.key == self.key)
    }
}

impl Agents {
    /// Docker databases are on and the engine answers; otherwise why not.
    fn docker_ready(&self) -> Result<(), Reply> {
        let state = self.directory.docker.state();
        if !state.on {
            return Err(Reply::new(
                Status::Refused,
                "Docker databases are off in Halfnine. The person can turn them on in Settings.",
            ));
        }
        match state.problem {
            Some(problem) => Err(Reply::new(Status::Refused, problem)),
            None => Ok(()),
        }
    }

    /// A Docker database by its connection's name, or why it isn't one.
    pub(super) fn docker_database(&self, connection: &str) -> Result<(ConnectionSpec, String), Reply> {
        let spec = self.connection(connection)?;
        match &spec.source {
            Source::Docker { container, .. } => {
                let container = container.clone();
                Ok((spec, container))
            }
            Source::Saved => Err(Reply::new(
                Status::Refused,
                format!("{connection} isn't a Docker database; list_connections marks those."),
            )),
        }
    }

    /// The first of `base`, `base-2`, `base-3` and on that no container or job has.
    fn unused_name(&self, base: &str) -> String {
        let docker = &self.directory.docker;
        let taken = |name: &String| docker.container(name).is_some() || docker.jobs().iter().any(|j| &j.name == name);
        std::iter::once(base.to_string())
            .chain((2..).map(|n| format!("{base}-{n}")))
            .find(|name| !taken(name))
            .unwrap_or_default()
    }

    /// Checks a database an agent asks to make by name: an agent asking again for one it has, or is making, gets that
    /// one; a name someone else has is refused; and an agent with the most databases it may have is refused.
    fn may_make(&self, agent: &Agent, name: &str) -> Result<Option<ConnectionSpec>, Reply> {
        let docker = &self.directory.docker;
        if docker.jobs().iter().any(|j| j.name == name && j.failed.is_none()) {
            return Ok(None);
        }
        if docker.container(name).is_some() {
            return match self.directory.list().into_iter().find(|c| c.name == name && agent.made(c)) {
                Some(spec) => Ok(Some(spec)),
                None => Err(Reply::new(
                    Status::Refused,
                    format!("There's already a container named {name}. Choose another name."),
                )),
            };
        }
        hn_docker::valid_name(name).map_err(Reply::from)?;
        let made = docker.state().databases.iter().filter(|d| agent.made(&d.connection)).count();
        if made >= DATABASES_LIMIT {
            return Err(Reply::new(
                Status::Refused,
                format!("You have {DATABASES_LIMIT} Docker databases already, the most an agent may have. Delete one with delete_database first."),
            ));
        }
        Ok(None)
    }

    pub(super) async fn create_database(&self, agent: &Agent, name: Option<String>, version: Option<u16>) -> Reply {
        if let Err(reply) = self.docker_ready() {
            return reply;
        }
        let version = version.unwrap_or(18);
        if !hn_docker::VERSIONS.contains(&version) {
            return Reply::new(Status::Refused, "Choose PostgreSQL 14, 15, 16, 17 or 18.");
        }
        let name = name.unwrap_or_else(|| self.unused_name("scratch"));
        match self.may_make(agent, &name) {
            Ok(Some(spec)) => return self.made(&spec).await,
            Ok(None) => {}
            Err(reply) => return reply,
        }
        let new = NewDatabase {
            name: name.clone(),
            major: version,
            image: None,
            load: None,
            origin: Origin::New,
            copy_of: None,
            agent: Some(agent.mark()),
            user: None,
            database: None,
            port: None,
            password: None,
        };
        let docker = self.directory.docker.clone();
        let job = docker.clone();
        let waiting = docker.job(&name, "Starting…", async move { job.make(new).await });
        self.finished(waiting, &name, "create_database").await
    }

    pub(super) async fn copy_database(&self, agent: &Agent, connection: &str, name: Option<String>) -> Reply {
        if let Err(reply) = self.docker_ready() {
            return reply;
        }
        let (spec, source) = match self.docker_database(connection) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        if let Verdict::Refuse(why) = verdict(&spec, Ask::Read, false) {
            return Reply::new(Status::Refused, why);
        }
        let name = name.unwrap_or_else(|| self.unused_name(&format!("{source}-copy")));
        match self.may_make(agent, &name) {
            Ok(Some(spec)) => return self.made(&spec).await,
            Ok(None) => {}
            Err(reply) => return reply,
        }
        let new = NewDatabase {
            name: name.clone(),
            major: 18,
            image: None,
            load: None,
            origin: Origin::Copy,
            copy_of: Some(source.clone()),
            agent: Some(agent.mark()),
            user: None,
            database: None,
            port: None,
            password: None,
        };
        let docker = self.directory.docker.clone();
        let (job, rules) = (docker.clone(), self.rules.clone());
        let from = spec.id.clone();
        let waiting = docker.job(&name, &format!("Copying {source}…"), async move {
            let copy = job.copy(&source, new).await?;
            // A copy keeps the rules of the database it copies, as it keeps its hidden columns.
            rules.copy_connection(&from, &copy.id)?;
            Ok(copy)
        });
        self.finished(waiting, &name, "copy_database").await
    }

    pub(super) async fn start_database(&self, connection: &str) -> Reply {
        if let Err(reply) = self.docker_ready() {
            return reply;
        }
        let (spec, container) = match self.docker_database(connection) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        if let Verdict::Refuse(why) = verdict(&spec, Ask::StartDatabase, false) {
            return Reply::new(Status::Refused, why);
        }
        let docker = &self.directory.docker;
        if docker.state().databases.iter().any(|d| d.connection.id == spec.id && d.running) {
            return Reply::new(Status::Ok, format!("{connection} is already running."));
        }
        match docker.start(&container).await {
            Ok(()) => Reply::new(
                Status::Ok,
                format!("Started {connection}. Postgres takes a few seconds to accept connections, and describe and query wait for it."),
            ),
            Err(error) => Reply::from(error),
        }
    }

    /// Waits for a job up to [`JOB_WAIT`], then answers WAITING with how to keep waiting.
    async fn finished(&self, mut waiting: watch::Receiver<Outcome>, name: &str, tool: &str) -> Reply {
        let outcome = match tokio::time::timeout(JOB_WAIT, waiting.wait_for(Option::is_some)).await {
            Ok(Ok(outcome)) => outcome.clone(),
            Ok(Err(_)) => None,
            Err(_) => {
                let doing = self.directory.docker.jobs().into_iter().find(|j| j.name == name).map(|j| j.doing);
                return Reply::new(
                    Status::Waiting,
                    format!(
                        "{name} isn't ready yet: {}. Call {tool} again with the name {name} to keep waiting; it won't make a second one.",
                        doing.unwrap_or_else(|| "still working".into())
                    ),
                );
            }
        };
        match outcome {
            Some(Ok(spec)) => self.made(&spec).await,
            Some(Err(error)) => Reply::from(error),
            None => Reply::new(
                Status::Error,
                format!("Halfnine lost track of making {name}. list_connections shows whether it's there."),
            ),
        }
    }

    /// What an agent needs to use a database made for it: its name, its access, and its URL where that may be given.
    async fn made(&self, spec: &ConnectionSpec) -> Reply {
        let access = match spec.agent_access() {
            AgentAccess::Full => "you may read and change everything in it",
            AgentAccess::ApprovedWrites => "the person approves what you change in it",
            AgentAccess::Read => "you may read it",
            AgentAccess::Off => "the person has turned agents off for it",
        };
        let mut text = format!(
            "{} is ready, and {access}. Use the connection name {} with describe, query, change_rows, change_structure and run_sql.",
            spec.name, spec.name
        );
        let Source::Docker { container, .. } = &spec.source else {
            return Reply::new(Status::Ok, text);
        };
        if verdict(spec, Ask::Credentials, false) == Verdict::Run
            && let Ok(secrets) = self.directory.docker.secrets(container).await
        {
            text.push_str(&format!(
                " Its URL, for a program you run: {}",
                hn_core::url(spec, secrets.password.as_deref())
            ));
        }
        Reply::new(Status::Ok, text)
    }
}

/// Where an agent's request to delete a Docker database it didn't make stands.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DeletionState {
    Pending,
    Deleting,
    Deleted { backed_up: bool, kept_volume: bool },
    Declined,
    Failed { error: Error },
    Withdrawn { why: String },
}

impl DeletionState {
    fn settled(&self) -> bool {
        !matches!(self, DeletionState::Pending | DeletionState::Deleting)
    }
}

pub(super) struct DeletionRequest {
    id: String,
    agent: Agent,
    mcp_session: String,
    connection: ConnectionSpec,
    container: String,
    state: Arc<watch::Sender<DeletionState>>,
}

/// A request to delete a database as the window asks it, with only Yes and No: "don't ask again" never covers a delete.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeletionRequestView {
    pub id: String,
    pub agent: String,
    pub connection: ConnectionSpec,
    /// Whether Halfnine backs it up first, so it can be put back.
    pub backed_up: bool,
    pub state: DeletionState,
}

impl Agents {
    /// Deletes a Docker database for an agent: one an agent of its name made at once, any other once the person says
    /// yes. Asking again while the person decides waits on the same request.
    pub(super) async fn delete_database(&self, agent: &Agent, mcp_session: &str, connection: &str) -> Reply {
        let asked = {
            let mut deletions = lock(&self.deletions);
            // One withdrawn, as when the agent that asked left, answers nobody now.
            deletions.retain(|_, r| !matches!(*r.state.borrow(), DeletionState::Withdrawn { .. }));
            deletions.get(connection).map(|r| r.state.subscribe())
        };
        if let Some(state) = asked {
            return self.deletion_answer(connection, state).await;
        }
        if let Err(reply) = self.docker_ready() {
            return reply;
        }
        let (spec, container) = match self.docker_database(connection) {
            Ok(found) => found,
            Err(reply) => return reply,
        };
        match verdict(&spec, Ask::DeleteDatabase { own: agent.made(&spec) }, false) {
            Verdict::Refuse(why) => Reply::new(Status::Refused, why),
            Verdict::Run => match self.delete_now(agent, &spec, &container).await {
                Ok(done) => Reply::new(Status::Ok, deleted(connection, done.backup.is_some(), done.kept_volume)),
                Err(error) => Reply::from(error),
            },
            Verdict::Propose => {
                let state = Arc::new(watch::channel(DeletionState::Pending).0);
                let waiting = state.subscribe();
                lock(&self.deletions).insert(
                    connection.to_string(),
                    DeletionRequest {
                        id: new_id("d"),
                        agent: agent.clone(),
                        mcp_session: mcp_session.to_string(),
                        connection: spec,
                        container,
                        state,
                    },
                );
                self.desk.changed();
                self.desk.come_forward();
                self.deletion_answer(connection, waiting).await
            }
        }
    }

    async fn deletion_answer(&self, name: &str, mut state: watch::Receiver<DeletionState>) -> Reply {
        let settled = match tokio::time::timeout(self.wait, state.wait_for(DeletionState::settled)).await {
            Ok(Ok(settled)) => settled.clone(),
            Ok(Err(_)) => return Reply::new(Status::Error, "Halfnine forgot this request."),
            Err(_) => {
                return Reply::new(
                    Status::Waiting,
                    format!("Halfnine is asking the person whether to delete {name}. Call delete_database again to keep waiting."),
                );
            }
        };
        lock(&self.deletions).remove(name);
        match settled {
            DeletionState::Deleted { backed_up, kept_volume } => Reply::new(Status::Approved, deleted(name, backed_up, kept_volume)),
            DeletionState::Declined => Reply::new(
                Status::Declined,
                format!("The person chose not to delete {name}. Don't ask again unless they do."),
            ),
            DeletionState::Failed { error } => Reply::from(error),
            DeletionState::Withdrawn { why } => Reply::new(Status::Refused, format!("Withdrawn: {why} Nothing was deleted.")),
            DeletionState::Pending | DeletionState::Deleting => Reply::new(Status::Error, "Halfnine lost track of this request."),
        }
    }

    /// Deletes a database for `agent`, backed up first while backups are on, closing what anyone has open on it.
    async fn delete_now(&self, agent: &Agent, spec: &ConnectionSpec, container: &str) -> Result<DeleteDone, Error> {
        let choices = self.settings.get();
        let delete = async || {
            self.forget_connection(&spec.id).await;
            self.sessions.close_connection(&spec.id).await;
            self.directory.docker.delete(container).await
        };
        self.backups
            .delete_database(&choices, &self.directory.docker, container, Some(agent.mark()), !choices.backups, delete)
            .await
    }

    /// The person answered a request to delete a database. Yes deletes it here, so it happens even if the agent
    /// stopped waiting, and gives what the window shows.
    pub async fn answer_deletion(&self, id: &str, yes: bool) -> Result<Option<DeleteDone>, Error> {
        let found = lock(&self.deletions)
            .values()
            .find(|r| r.id == id && matches!(*r.state.borrow(), DeletionState::Pending))
            .map(|r| (r.state.clone(), r.agent.clone(), r.connection.clone(), r.container.clone()));
        let (state, agent, spec, container) = found.ok_or_else(|| Error::new(ErrorKind::NotFound, "The agent stopped asking."))?;
        if !yes {
            state.send_replace(DeletionState::Declined);
            self.desk.changed();
            return Ok(None);
        }
        state.send_replace(DeletionState::Deleting);
        self.desk.changed();
        let done = self.delete_now(&agent, &spec, &container).await;
        state.send_replace(match &done {
            Ok(done) => DeletionState::Deleted {
                backed_up: done.backup.is_some(),
                kept_volume: done.kept_volume,
            },
            Err(error) => DeletionState::Failed { error: error.clone() },
        });
        self.desk.changed();
        done.map(Some)
    }

    /// Withdraws the requests to delete that `which` picks, as when agents are paused or the asking one leaves.
    pub(super) fn withdraw_deletions(&self, which: impl Fn(&str) -> bool, why: &str) {
        for request in lock(&self.deletions).values() {
            if which(&request.mcp_session) && matches!(*request.state.borrow(), DeletionState::Pending) {
                request.state.send_replace(DeletionState::Withdrawn { why: why.to_string() });
            }
        }
    }

    pub(super) fn deletion_views(&self) -> Vec<DeletionRequestView> {
        let backed_up = self.settings.get().backups;
        lock(&self.deletions)
            .values()
            .filter(|r| !r.state.borrow().settled())
            .map(|r| DeletionRequestView {
                id: r.id.clone(),
                agent: r.agent.title.clone(),
                connection: r.connection.clone(),
                backed_up,
                state: r.state.borrow().clone(),
            })
            .collect()
    }
}

fn deleted(name: &str, backed_up: bool, kept_volume: bool) -> String {
    let backup = if backed_up {
        " Halfnine backed it up first, so the person can put it back."
    } else {
        " Backups are off in Halfnine, so it can't be put back."
    };
    let volume = if kept_volume {
        " Its data volume wasn't made by Halfnine, so it was kept."
    } else {
        ""
    };
    format!("Deleted {name}.{backup}{volume}")
}
