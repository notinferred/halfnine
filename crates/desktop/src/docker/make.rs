//! Making and deleting Docker databases, as jobs the window can follow: downloading the image, starting Postgres,
//! loading a file. A job keeps its failure for the window until someone dismisses it.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use hn_core::{AgentMark, ConnectionSpec, Error, ErrorKind};
use hn_docker::{Engine, NewContainer, label, pick_port, postgres, valid_name};
use hn_pg::{PgDatabase, Secrets};
use serde::Serialize;
use specta::Type;
use tokio::sync::watch;

use super::{Docker, lock};

/// What a Docker database job is doing, for the window to show beside its row.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub name: String,
    pub doing: String,
    pub failed: Option<String>,
}

pub type Outcome = Option<Result<ConnectionSpec, Error>>;

pub(super) struct Running {
    pub job: Job,
    pub done: watch::Sender<Outcome>,
}

/// How a new database comes to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    New,
    Copy,
    Restored,
}

impl Origin {
    fn label(self) -> &'static str {
        match self {
            Origin::New => "new",
            Origin::Copy => "copy",
            Origin::Restored => "restored",
        }
    }
}

/// A database to make.
#[derive(Clone, Debug)]
pub struct NewDatabase {
    pub name: String,
    pub major: u16,
    /// An exact image to run, as for a copy; else the official image for `major`.
    pub image: Option<String>,
    /// A dump or SQL file to load once Postgres is up.
    pub load: Option<PathBuf>,
    pub origin: Origin,
    pub copy_of: Option<String>,
    /// The agent that asked for it, if one did.
    pub agent: Option<AgentMark>,
}

/// How long a new database may take to accept connections, past downloading its image.
const READY: Duration = Duration::from_secs(90);

impl Docker {
    /// Runs `work` as the job for `name`, or joins the job already running for it, so asking twice makes one database.
    pub fn job(
        self: &Arc<Self>,
        name: &str,
        doing: &str,
        work: impl Future<Output = Result<ConnectionSpec, Error>> + Send + 'static,
    ) -> watch::Receiver<Outcome> {
        let mut jobs = lock(&self.jobs);
        if let Some(running) = jobs.get(name).filter(|r| r.job.failed.is_none()) {
            return running.done.subscribe();
        }
        let (done, waiting) = watch::channel(None);
        jobs.insert(
            name.to_string(),
            Running {
                job: Job {
                    name: name.to_string(),
                    doing: doing.to_string(),
                    failed: None,
                },
                done,
            },
        );
        drop(jobs);
        (self.changed)();
        let docker = Arc::clone(self);
        let name = name.to_string();
        tauri::async_runtime::spawn(async move {
            let outcome = work.await;
            {
                let mut jobs = lock(&docker.jobs);
                match &outcome {
                    Ok(_) => {
                        if let Some(running) = jobs.remove(&name) {
                            running.done.send_replace(Some(outcome.clone()));
                        }
                    }
                    Err(error) => {
                        if let Some(running) = jobs.get_mut(&name) {
                            running.job.failed = Some(error.message.clone());
                            running.done.send_replace(Some(outcome.clone()));
                        }
                    }
                }
            }
            let _ = docker.refresh().await;
            (docker.changed)();
        });
        waiting
    }

    /// Says what a job is doing now. Frequent progress, as from a download, reaches the window at most every 200 ms.
    pub fn progress(&self, name: &str, doing: String) {
        let changed = {
            let mut jobs = lock(&self.jobs);
            let mut last = lock(&self.last_progress);
            match jobs.get_mut(name) {
                Some(running) if running.job.doing != doing => {
                    running.job.doing = doing;
                    let due = last.is_none_or(|at| at.elapsed() > Duration::from_millis(200));
                    if due {
                        *last = Some(Instant::now());
                    }
                    due
                }
                _ => false,
            }
        };
        if changed {
            (self.changed)();
        }
    }

    pub fn jobs(&self) -> Vec<super::make::Job> {
        let mut jobs: Vec<Job> = lock(&self.jobs).values().map(|r| r.job.clone()).collect();
        jobs.sort_by(|a, b| a.name.cmp(&b.name));
        jobs
    }

    /// The person closed a failed job's message.
    pub fn dismiss(&self, name: &str) {
        let mut jobs = lock(&self.jobs);
        if jobs.get(name).is_some_and(|r| r.job.failed.is_some()) {
            jobs.remove(name);
        }
        drop(jobs);
        (self.changed)();
    }

    /// Makes a database: its image downloaded if missing, a labelled volume of its own, Postgres published on this
    /// computer only on a free port with a new password, then waits until it accepts connections, and loads a file.
    pub async fn make(&self, new: NewDatabase) -> Result<ConnectionSpec, Error> {
        valid_name(&new.name)?;
        let engine = self.engine().await?;
        if engine.container(&new.name).await.is_ok() {
            return Err(Error::new(ErrorKind::Invalid, format!("There's already a container named {}.", new.name)));
        }
        let image = new.image.clone().unwrap_or_else(|| postgres::image(new.major));
        if !engine.has_image(&image).await? {
            let (name, major) = (new.name.clone(), new.major);
            engine
                .pull(&image, &mut |fraction| {
                    self.progress(&name, format!("Downloading PostgreSQL {major} · {}%", (fraction * 100.0).round()));
                })
                .await?;
        }
        self.progress(&new.name, "Starting…".into());
        let taken: HashSet<u16> = engine.containers().await?.iter().filter_map(|c| c.port).collect();
        let port = pick_port(&taken, |p| std::net::TcpListener::bind(("127.0.0.1", p)).is_ok())
            .ok_or_else(|| Error::new(ErrorKind::Docker, "Every port from 5433 to 5532 is taken."))?;
        let password = uuid::Uuid::new_v4().simple().to_string();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut labels = HashMap::from([
            (label::MANAGED.to_string(), "true".to_string()),
            (label::ID.to_string(), id.clone()),
            (label::ORIGIN.to_string(), new.origin.label().to_string()),
        ]);
        if let Some(copy_of) = &new.copy_of {
            labels.insert(label::COPY_OF.into(), copy_of.clone());
        }
        if let Some(agent) = &new.agent {
            labels.insert(label::AGENT.into(), agent.title.clone());
            labels.insert(label::AGENT_KEY.into(), agent.key.clone());
        }
        if cfg!(test) {
            labels.insert(label::TEST.into(), "true".into());
        }
        let database = new.name.replace(['-', '.'], "_");
        engine
            .create(&NewContainer {
                name: new.name.clone(),
                image,
                major: new.major,
                port,
                user: "postgres".into(),
                password: password.clone(),
                database: database.clone(),
                volume: format!("halfnine-{}", &id[..12]),
                labels,
            })
            .await?;
        engine.start(&new.name).await?;
        self.refresh().await?;
        let spec = self
            .connections()
            .into_iter()
            .find(|c| c.name == new.name)
            .ok_or_else(|| Error::new(ErrorKind::Docker, format!("{} was made but Docker doesn't list it.", new.name)))?;
        ready(&spec, &password).await?;
        if let Some(file) = &new.load {
            let shown = file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            self.progress(&new.name, format!("Loading {shown}…"));
            load(&engine, &new.name, file, &spec).await?;
        }
        Ok(spec)
    }

    /// Removes a container, and its volume when Halfnine made that volume; anyone else's volume is kept. Returns
    /// whether a volume was kept.
    pub async fn delete(&self, name: &str) -> Result<bool, Error> {
        let engine = self.engine().await?;
        let container = engine.container(name).await?;
        let ours = container.labels.get(label::MANAGED).map(String::as_str) == Some("true");
        let volume = container.volume.as_deref().filter(|_| ours);
        engine.remove(name, volume).await?;
        self.forget(name)?;
        self.refresh().await?;
        Ok(container.volume.is_some() && !ours)
    }
}

/// Waits until Postgres in a new container accepts a real connection; Docker accepts one for it sooner.
async fn ready(spec: &ConnectionSpec, password: &str) -> Result<(), Error> {
    let secrets = Secrets {
        password: Some(password.to_string()),
        ssh: None,
    };
    let started = Instant::now();
    loop {
        match PgDatabase::connect(spec, None, &secrets).await {
            Ok(db) => {
                db.close().await;
                return Ok(());
            }
            Err(e) if e.kind == ErrorKind::Auth => return Err(e),
            Err(_) if started.elapsed() < READY => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(e) => {
                return Err(Error::new(
                    ErrorKind::Docker,
                    format!("{} didn't start accepting connections within 90 seconds: {}", spec.name, e.message),
                ));
            }
        }
    }
}

/// Loads a dump made by `pg_dump -Fc`, or an SQL file, into a new database, with the tools inside its container.
async fn load(engine: &Engine, name: &str, file: &std::path::Path, spec: &ConnectionSpec) -> Result<(), Error> {
    let mut head = [0u8; 5];
    let custom = std::fs::File::open(file)
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
        .is_ok_and(|()| &head == b"PGDMP");
    engine.run(name, &["mkdir", "-p", "/tmp/halfnine"]).await?;
    engine.upload(name, "/tmp/halfnine", "load", file).await?;
    let path = "/tmp/halfnine/load";
    let cmd: Vec<&str> = if custom {
        vec!["pg_restore", "--no-owner", "-U", &spec.user, "-d", &spec.database, path]
    } else {
        vec!["psql", "-v", "ON_ERROR_STOP=1", "-q", "-U", &spec.user, "-d", &spec.database, "-f", path]
    };
    let (ran, _) = engine.run(name, &cmd).await?;
    let _ = engine.run(name, &["rm", "-f", path]).await;
    if ran.succeeded() {
        Ok(())
    } else {
        let first: Vec<&str> = ran.stderr.lines().take(3).collect();
        Err(Error::new(
            ErrorKind::Query,
            format!("{name} was made, but loading the file failed: {}", first.join(" ")),
        ))
    }
}
