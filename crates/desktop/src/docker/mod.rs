//! Docker databases: the Postgres containers on this computer, shown as connections while Docker databases are on in
//! Settings. Nothing talks to the engine while they're off. A container's own credentials are read from it when it
//! connects; what someone sets for it (label, read-only, agent access, hidden columns) is kept in `docker.json`, by
//! container name, and holds no credentials.

pub mod commands;
pub mod dumps;
pub mod make;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::StreamExt;
use hn_core::{AgentAccess, AgentMark, ConnectionSpec, Error, ErrorKind, HiddenColumn, Label, Made, MadeHow, Source, TlsMode};
use hn_docker::{Container, Engine, discover, label};
use hn_pg::Secrets;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::async_runtime::JoinHandle;
use tokio::sync::RwLock;

use crate::files;
use crate::settings::Settings;

const FILE: &str = "docker.json";
const VERSION: u32 = 1;
/// Docker connections' ids start with this, followed by the container's name.
pub const ID_PREFIX: &str = "docker:";

/// What someone set for one container. The defaults are those of a new saved connection.
#[derive(Clone, Debug, Default, Serialize, Deserialize, Type, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Overrides {
    pub label: Option<Label>,
    pub read_only: bool,
    pub agent_access: Option<AgentAccess>,
    pub hidden_columns: Vec<HiddenColumn>,
}

#[derive(Default, Serialize, Deserialize)]
struct Overlay {
    containers: HashMap<String, Overrides>,
}

/// One Docker database as the window lists it.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DockerDatabase {
    pub connection: ConnectionSpec,
    pub running: bool,
    pub image: String,
    pub major: Option<u16>,
    /// The port on this computer, or `None` when it publishes none and can't be connected to.
    pub port: Option<u16>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DockerState {
    pub on: bool,
    /// Which engine Halfnine found, such as "the desktop-linux context".
    pub engine: Option<String>,
    /// Why there is no engine, when Docker databases are on.
    pub problem: Option<String>,
    pub databases: Vec<DockerDatabase>,
    /// Databases being made, copied, backed up or deleted, and those whose job failed.
    pub jobs: Vec<make::Job>,
}

pub struct Docker {
    settings: Arc<Settings>,
    overlay_path: PathBuf,
    overlay: Mutex<Overlay>,
    engine: RwLock<Option<Arc<Engine>>>,
    containers: Mutex<Vec<Container>>,
    problem: Mutex<Option<String>>,
    watcher: Mutex<Option<JoinHandle<()>>>,
    jobs: Mutex<HashMap<String, make::Running>>,
    last_progress: Mutex<Option<std::time::Instant>>,
    /// Files chosen in the open dialog, by the token the window holds.
    files: Mutex<HashMap<String, PathBuf>>,
    changed: Box<dyn Fn() + Send + Sync>,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Docker {
    pub fn new(dir: &Path, settings: Arc<Settings>, changed: impl Fn() + Send + Sync + 'static) -> Self {
        let overlay_path = dir.join(FILE);
        let overlay = files::load::<Overlay>(&overlay_path, VERSION, "the Docker databases' settings").unwrap_or_default();
        Self {
            settings,
            overlay_path,
            overlay: Mutex::new(overlay),
            engine: RwLock::new(None),
            containers: Mutex::new(Vec::new()),
            problem: Mutex::new(None),
            watcher: Mutex::new(None),
            jobs: Mutex::new(HashMap::new()),
            last_progress: Mutex::new(None),
            files: Mutex::new(HashMap::new()),
            changed: Box::new(changed),
        }
    }

    pub fn on(&self) -> bool {
        self.settings.get().docker
    }

    /// Follows the engine while Docker databases are on: finds it, lists its containers, and lists them again whenever
    /// one changes. If the engine goes away, it looks for it again, waiting longer each time up to 30 seconds.
    pub fn watch(self: &Arc<Self>) {
        let mut watcher = lock(&self.watcher);
        if !self.on() || watcher.as_ref().is_some_and(|w| !w.inner().is_finished()) {
            return;
        }
        let docker = Arc::clone(self);
        *watcher = Some(tauri::async_runtime::spawn(async move {
            let mut wait = Duration::from_secs(1);
            while docker.on() {
                match docker.attach().await {
                    Ok(engine) => {
                        wait = Duration::from_secs(1);
                        let mut changes = engine.changes();
                        while let Some(Ok(())) = changes.next().await {
                            let _ = docker.refresh().await;
                        }
                        *docker.engine.write().await = None;
                        *lock(&docker.problem) = Some("Docker stopped. Halfnine will find it again when it's back.".into());
                    }
                    Err(e) => *lock(&docker.problem) = Some(e.message),
                }
                lock(&docker.containers).clear();
                (docker.changed)();
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(30));
            }
        }));
    }

    /// Finds the engine and lists its containers.
    pub async fn attach(&self) -> Result<Arc<Engine>, Error> {
        let engine = Arc::new(Engine::find(&discover::here()).await?);
        *self.engine.write().await = Some(engine.clone());
        *lock(&self.problem) = None;
        let _ = self.refresh().await;
        Ok(engine)
    }

    /// Turns Docker databases on or off. Off, the engine is let go and nothing talks to it.
    pub async fn set_on(self: &Arc<Self>, on: bool) -> Result<(), Error> {
        self.settings.change(|choices| choices.docker = on)?;
        if on {
            self.watch();
        } else {
            if let Some(watcher) = lock(&self.watcher).take() {
                watcher.abort();
            }
            *self.engine.write().await = None;
            lock(&self.containers).clear();
            *lock(&self.problem) = None;
        }
        (self.changed)();
        Ok(())
    }

    pub async fn engine(&self) -> Result<Arc<Engine>, Error> {
        if !self.on() {
            return Err(Error::new(ErrorKind::Docker, "Docker databases are off in Halfnine's Settings."));
        }
        self.engine.read().await.clone().ok_or_else(|| {
            let problem = lock(&self.problem).clone();
            Error::new(
                ErrorKind::Docker,
                problem.unwrap_or_else(|| "Halfnine is still looking for Docker.".into()),
            )
        })
    }

    /// Lists the containers again and tells the window.
    pub async fn refresh(&self) -> Result<(), Error> {
        let containers = self.engine().await?.containers().await?;
        *lock(&self.containers) = containers;
        (self.changed)();
        Ok(())
    }

    pub fn container(&self, name: &str) -> Option<Container> {
        lock(&self.containers).iter().find(|c| c.name == name).cloned()
    }

    pub fn state(&self) -> DockerState {
        let overlay = lock(&self.overlay);
        let databases = lock(&self.containers)
            .iter()
            .map(|c| DockerDatabase {
                connection: connection(c, overlay.containers.get(&c.name)),
                running: c.running,
                image: c.image.clone(),
                major: c.major,
                port: c.port,
            })
            .collect();
        DockerState {
            on: self.on(),
            engine: self.engine.try_read().ok().and_then(|e| e.as_ref().map(|e| e.found.by.clone())),
            problem: lock(&self.problem).clone(),
            databases,
            jobs: self.jobs(),
        }
    }

    /// The Docker databases as connections.
    pub fn connections(&self) -> Vec<ConnectionSpec> {
        let overlay = lock(&self.overlay);
        lock(&self.containers)
            .iter()
            .map(|c| connection(c, overlay.containers.get(&c.name)))
            .collect()
    }

    /// What the container's environment says to sign in with, read now. A password kept in a file inside the
    /// container is read from it.
    pub async fn secrets(&self, name: &str) -> Result<Secrets, Error> {
        let container = self.container(name).ok_or_else(|| gone(name))?;
        let credentials = &container.credentials;
        let password = match (&credentials.password, &credentials.password_file) {
            (Some(password), _) => Some(password.clone()),
            (None, Some(file)) => {
                let (ran, out) = self.engine().await?.run(name, &["cat", file]).await?;
                ran.succeeded().then(|| String::from_utf8_lossy(&out).trim_end_matches('\n').to_string())
            }
            (None, None) => None,
        };
        Ok(Secrets { password, ssh: None })
    }

    /// Starts a stopped container and waits for its listing to say so, so its port is known.
    pub async fn start(&self, name: &str) -> Result<(), Error> {
        self.engine().await?.start(name).await?;
        self.refresh().await
    }

    pub async fn stop(&self, name: &str) -> Result<(), Error> {
        self.engine().await?.stop(name).await?;
        self.refresh().await
    }

    pub fn overrides(&self, name: &str) -> Overrides {
        lock(&self.overlay).containers.get(name).cloned().unwrap_or_default()
    }

    /// Changes what is kept for a container, writing `docker.json`, or changes nothing if it can't be written.
    pub fn set_overrides(&self, name: &str, edit: impl FnOnce(&mut Overrides)) -> Result<ConnectionSpec, Error> {
        let mut overlay = lock(&self.overlay);
        let mut next = overlay.containers.clone();
        edit(next.entry(name.to_string()).or_default());
        let saved = Overlay { containers: next };
        files::save(&self.overlay_path, VERSION, &saved, "the Docker databases' settings")?;
        *overlay = saved;
        drop(overlay);
        (self.changed)();
        let container = self.container(name).ok_or_else(|| gone(name))?;
        Ok(connection(&container, Some(&self.overrides(name))))
    }

    /// Keeps a file the person chose in the open dialog, and gives the window a token for it.
    pub fn keep_file(&self, path: PathBuf) -> String {
        let token = uuid::Uuid::new_v4().simple().to_string();
        lock(&self.files).insert(token.clone(), path);
        token
    }

    pub fn file(&self, token: &str) -> Result<PathBuf, Error> {
        lock(&self.files)
            .get(token)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "Choose the file again."))
    }

    /// Forgets what was kept for a container Halfnine deleted, so a new one by that name starts afresh.
    pub fn forget(&self, name: &str) -> Result<(), Error> {
        let mut overlay = lock(&self.overlay);
        if overlay.containers.remove(name).is_some() {
            files::save(&self.overlay_path, VERSION, &*overlay, "the Docker databases' settings")?;
        }
        Ok(())
    }
}

fn gone(name: &str) -> Error {
    Error::new(ErrorKind::NotFound, format!("There is no Docker database named {name} any more."))
}

/// A container as a connection: reached on this computer at its published port, with the user and database its
/// environment names.
pub fn connection(container: &Container, overrides: Option<&Overrides>) -> ConnectionSpec {
    let overrides = overrides.cloned().unwrap_or_default();
    let labels = &container.labels;
    let made = (labels.get(label::MANAGED).map(String::as_str) == Some("true")).then(|| Made {
        how: match labels.get(label::ORIGIN).map(String::as_str) {
            Some("copy") => MadeHow::Copy,
            Some("restored") => MadeHow::Restored,
            _ => MadeHow::New,
        },
        agent: labels.get(label::AGENT_KEY).map(|key| AgentMark {
            key: key.clone(),
            title: labels.get(label::AGENT).cloned().unwrap_or_else(|| key.clone()),
        }),
    });
    ConnectionSpec {
        id: format!("{ID_PREFIX}{}", container.name),
        name: container.name.clone(),
        host: "127.0.0.1".into(),
        port: container.port.unwrap_or(5432),
        user: container.credentials.user.clone(),
        database: container.credentials.database.clone(),
        tls: TlsMode::Disable,
        label: overrides.label.unwrap_or(Label::None),
        read_only: overrides.read_only,
        ssh: None,
        save_password: false,
        agent_access: overrides.agent_access,
        hidden_columns: overrides.hidden_columns,
        source: Source::Docker {
            container: container.name.clone(),
            made,
        },
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod unit {
    use super::*;
    use hn_docker::Credentials;

    fn container(labels: &[(&str, &str)]) -> Container {
        Container {
            id: "abc".into(),
            name: "scratch".into(),
            image: "postgres:18".into(),
            image_id: "sha256:1".into(),
            major: Some(18),
            running: true,
            port: Some(5433),
            labels: labels.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            volume: Some("scratch-data".into()),
            credentials: Credentials {
                user: "app".into(),
                password: Some("pw".into()),
                password_file: None,
                database: "shop".into(),
                trust: false,
            },
        }
    }

    #[test]
    fn turns_a_container_into_a_connection_without_its_password() {
        let spec = connection(&container(&[]), None);
        assert_eq!(
            (spec.id.as_str(), spec.host.as_str(), spec.port, spec.user.as_str()),
            ("docker:scratch", "127.0.0.1", 5433, "app")
        );
        assert_eq!(spec.agent_access(), AgentAccess::Read, "someone else's container starts at Read");
        assert!(!format!("{spec:?}").contains("pw"));
        let made = connection(
            &container(&[(label::MANAGED, "true"), (label::AGENT, "Claude Code"), (label::AGENT_KEY, "claude-code")]),
            None,
        );
        assert_eq!(made.agent_access(), AgentAccess::Full);
        let Source::Docker { made: Some(made), .. } = made.source else {
            panic!("made by Halfnine")
        };
        assert_eq!(made.agent.map(|a| a.title).as_deref(), Some("Claude Code"));
        let restored = connection(&container(&[(label::MANAGED, "true"), (label::ORIGIN, "restored")]), None);
        assert_eq!(restored.agent_access(), AgentAccess::Read);
    }
}
