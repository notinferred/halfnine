//! The Docker engine on this computer, as Halfnine uses it: the Postgres containers in it, and making, starting,
//! stopping and removing them, running commands inside them, and copying files into them.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::{ContainerCreateBody, ContainerInspectResponse, HostConfig, Mount, MountType, PortBinding, VolumeCreateRequest};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, EventsOptionsBuilder, ListContainersOptionsBuilder, RemoveContainerOptionsBuilder,
    RemoveVolumeOptionsBuilder, StopContainerOptionsBuilder, UploadToContainerOptionsBuilder,
};
use bollard::{API_DEFAULT_VERSION, Docker};
use bytes::Bytes;
use futures_util::{Stream, StreamExt, TryStreamExt};
use hn_core::{Error, ErrorKind};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::discover::Found;
use crate::postgres::{self, Credentials};

/// The port Postgres listens on inside its container.
const POSTGRES_PORT: &str = "5432/tcp";
/// How long a request may wait for the engine's answer to begin. Streamed output, as from a dump, can run longer.
const REQUEST_TIMEOUT: u64 = 120;
/// The most of a command's error output kept, to show its first lines.
const STDERR_LIMIT: usize = 64 * 1024;

pub struct Engine {
    docker: Docker,
    /// Where it was found, to show people which engine Halfnine uses.
    pub found: Found,
}

/// A Postgres container, with what Halfnine needs to connect to it and to manage it.
#[derive(Clone, Debug)]
pub struct Container {
    pub id: String,
    pub name: String,
    /// The image as it was asked for, such as `postgres:18`.
    pub image: String,
    /// The exact image it runs, to make a copy from the same one.
    pub image_id: String,
    pub major: Option<u16>,
    pub running: bool,
    /// The port on this computer that reaches Postgres inside, when it publishes one.
    pub port: Option<u16>,
    pub labels: HashMap<String, String>,
    /// The named volume its data lives in, if it has one.
    pub volume: Option<String>,
    pub credentials: Credentials,
}

/// Whether Docker couldn't start a container because something else took its port first.
pub fn port_taken(error: &Error) -> bool {
    error.message.contains("port is already allocated") || error.message.contains("address already in use")
}

/// A new Postgres container to make.
#[derive(Clone, Debug)]
pub struct NewContainer {
    pub name: String,
    /// The image to run: a tag such as `postgres:18`, or an image ID to copy one exactly.
    pub image: String,
    pub major: u16,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
    pub volume: String,
    pub labels: HashMap<String, String>,
}

/// How a command inside a container ended.
#[derive(Clone, Debug)]
pub struct Ran {
    pub code: i64,
    /// Its error output, cut to the first 64 KB.
    pub stderr: String,
}

impl Ran {
    pub fn succeeded(&self) -> bool {
        self.code == 0
    }
}

fn docker_error(what: &str, e: bollard::errors::Error) -> Error {
    match e {
        bollard::errors::Error::DockerResponseServerError { status_code: 404, message } => {
            Error::new(ErrorKind::NotFound, format!("{what}: {message}"))
        }
        bollard::errors::Error::DockerResponseServerError { message, .. } => Error::new(ErrorKind::Docker, format!("{what}: {message}")),
        other => Error::new(ErrorKind::Docker, format!("{what}: {other}")),
    }
}

impl Engine {
    /// The first candidate that answers within three seconds.
    pub async fn find(candidates: &[Found]) -> Result<Self, Error> {
        for found in candidates {
            let Ok(docker) = Docker::connect_with_socket(&found.endpoint.address(), REQUEST_TIMEOUT, API_DEFAULT_VERSION) else {
                continue;
            };
            let answered = tokio::time::timeout(Duration::from_secs(3), async {
                let docker = docker.negotiate_version().await?;
                docker.ping().await.map(|_| docker)
            })
            .await;
            if let Ok(Ok(docker)) = answered {
                return Ok(Self {
                    docker,
                    found: found.clone(),
                });
            }
        }
        Err(Error::new(
            ErrorKind::Docker,
            "Halfnine can't find Docker. Start Docker Desktop, OrbStack or Colima.",
        ))
    }

    /// Every Postgres container, running or not.
    pub async fn containers(&self) -> Result<Vec<Container>, Error> {
        let options = ListContainersOptionsBuilder::new().all(true).build();
        let summaries = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|e| docker_error("Docker couldn't list its containers", e))?;
        let mut containers = Vec::new();
        for summary in summaries {
            let Some(id) = summary.id else { continue };
            // A container removed since the list was made is simply left out.
            if let Ok(inspected) = self.docker.inspect_container(&id, None).await
                && let Some(container) = postgres_container(inspected)
            {
                containers.push(container);
            }
        }
        containers.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(containers)
    }

    pub async fn container(&self, name: &str) -> Result<Container, Error> {
        let inspected = self
            .docker
            .inspect_container(name, None)
            .await
            .map_err(|e| docker_error(&format!("There is no container named {name}"), e))?;
        postgres_container(inspected).ok_or_else(|| Error::new(ErrorKind::NotFound, format!("{name} doesn't run Postgres.")))
    }

    pub async fn has_image(&self, image: &str) -> Result<bool, Error> {
        match self.docker.inspect_image(image).await {
            Ok(_) => Ok(true),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => Ok(false),
            Err(e) => Err(docker_error("Docker couldn't look up the image", e)),
        }
    }

    /// Downloads an image, saying how much of it has arrived, from 0 to 1, as it goes.
    pub async fn pull(&self, image: &str, progress: &mut (dyn FnMut(f64) + Send)) -> Result<(), Error> {
        let (name, tag) = image.rsplit_once(':').unwrap_or((image, "latest"));
        let options = CreateImageOptionsBuilder::new().from_image(name).tag(tag).build();
        let mut layers = Layers::default();
        let mut stream = self.docker.create_image(Some(options), None, None);
        while let Some(info) = stream.next().await {
            let info = info.map_err(|e| docker_error(&format!("Docker couldn't download {image}"), e))?;
            if let Some(error) = info.error_detail.and_then(|d| d.message) {
                return Err(Error::new(ErrorKind::Docker, format!("Docker couldn't download {image}: {error}")));
            }
            if let (Some(layer), Some(detail)) = (info.id, info.progress_detail) {
                layers.update(layer, detail.current.unwrap_or(0), detail.total.unwrap_or(0));
                progress(layers.fraction());
            }
        }
        progress(1.0);
        Ok(())
    }

    /// Makes the volume and the container, without starting it. Postgres is published on this computer only.
    pub async fn create(&self, new: &NewContainer) -> Result<String, Error> {
        self.docker
            .create_volume(VolumeCreateRequest {
                name: Some(new.volume.clone()),
                labels: Some(new.labels.clone()),
                ..Default::default()
            })
            .await
            .map_err(|e| docker_error("Docker couldn't make the volume", e))?;
        let binding = PortBinding {
            host_ip: Some("127.0.0.1".into()),
            host_port: Some(new.port.to_string()),
        };
        let body = ContainerCreateBody {
            image: Some(new.image.clone()),
            env: Some(vec![
                format!("POSTGRES_USER={}", new.user),
                format!("POSTGRES_PASSWORD={}", new.password),
                format!("POSTGRES_DB={}", new.database),
            ]),
            labels: Some(new.labels.clone()),
            exposed_ports: Some(vec![POSTGRES_PORT.into()]),
            host_config: Some(HostConfig {
                port_bindings: Some(HashMap::from([(POSTGRES_PORT.to_string(), Some(vec![binding]))])),
                mounts: Some(vec![Mount {
                    target: Some(postgres::data_path(new.major).into()),
                    source: Some(new.volume.clone()),
                    typ: Some(MountType::VOLUME),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        let options = CreateContainerOptionsBuilder::new().name(&new.name).build();
        let made = self.docker.create_container(Some(options), body).await;
        match made {
            Ok(made) => Ok(made.id),
            Err(e) => {
                let _ = self
                    .docker
                    .remove_volume(&new.volume, None::<bollard::query_parameters::RemoveVolumeOptions>)
                    .await;
                Err(docker_error(&format!("Docker couldn't make {}", new.name), e))
            }
        }
    }

    pub async fn start(&self, name: &str) -> Result<(), Error> {
        self.docker
            .start_container(name, None)
            .await
            .map_err(|e| docker_error(&format!("Docker couldn't start {name}"), e))
    }

    /// Stops a container, giving Postgres ten seconds to shut down cleanly.
    pub async fn stop(&self, name: &str) -> Result<(), Error> {
        let options = StopContainerOptionsBuilder::new().t(10).build();
        match self.docker.stop_container(name, Some(options)).await {
            // 304: it was already stopped.
            Ok(()) | Err(bollard::errors::Error::DockerResponseServerError { status_code: 304, .. }) => Ok(()),
            Err(e) => Err(docker_error(&format!("Docker couldn't stop {name}"), e)),
        }
    }

    /// Removes a container, and the named volume when one is given.
    pub async fn remove(&self, name: &str, volume: Option<&str>) -> Result<(), Error> {
        let options = RemoveContainerOptionsBuilder::new().force(true).build();
        self.docker
            .remove_container(name, Some(options))
            .await
            .map_err(|e| docker_error(&format!("Docker couldn't remove {name}"), e))?;
        if let Some(volume) = volume {
            let options = RemoveVolumeOptionsBuilder::new().force(true).build();
            self.docker
                .remove_volume(volume, Some(options))
                .await
                .map_err(|e| docker_error(&format!("Docker couldn't remove the volume {volume}"), e))?;
        }
        Ok(())
    }

    /// Runs a command inside a container as its `postgres` user, which signs in to Postgres there without a password,
    /// streaming what it prints to `out`.
    pub async fn run_to(&self, name: &str, cmd: &[&str], out: &mut (dyn AsyncWrite + Send + Unpin)) -> Result<Ran, Error> {
        let options = CreateExecOptions {
            cmd: Some(cmd.iter().map(|c| c.to_string()).collect()),
            user: Some("postgres".to_string()),
            attach_stdout: Some(true),
            attach_stderr: Some(true),
            ..Default::default()
        };
        let failed = |e| docker_error(&format!("Docker couldn't run {} in {name}", cmd[0]), e);
        let exec = self.docker.create_exec(name, options).await.map_err(failed)?;
        let mut stderr = String::new();
        if let StartExecResults::Attached { mut output, .. } = self.docker.start_exec(&exec.id, None).await.map_err(failed)? {
            while let Some(chunk) = output.next().await {
                match chunk.map_err(failed)? {
                    bollard::container::LogOutput::StdOut { message } => {
                        out.write_all(&message)
                            .await
                            .map_err(|e| Error::new(ErrorKind::Storage, format!("Couldn't write what {} printed: {e}.", cmd[0])))?;
                    }
                    bollard::container::LogOutput::StdErr { message } if stderr.len() < STDERR_LIMIT => {
                        stderr.push_str(&String::from_utf8_lossy(&message));
                    }
                    _ => {}
                }
            }
        }
        out.flush().await.map_err(|e| Error::new(ErrorKind::Storage, e.to_string()))?;
        let inspected = self.docker.inspect_exec(&exec.id).await.map_err(failed)?;
        Ok(Ran {
            code: inspected.exit_code.unwrap_or(-1),
            stderr,
        })
    }

    /// As [`run_to`](Self::run_to), keeping what it prints.
    pub async fn run(&self, name: &str, cmd: &[&str]) -> Result<(Ran, Vec<u8>), Error> {
        let mut out = Vec::new();
        let ran = self.run_to(name, cmd, &mut out).await?;
        Ok((ran, out))
    }

    /// Copies a file into a directory inside a container, streamed as a one-file tar, so a big dump is never held in
    /// memory.
    pub async fn upload(&self, name: &str, dir: &str, file_name: &str, file: &Path) -> Result<(), Error> {
        let read_failed = |e: std::io::Error| Error::new(ErrorKind::Storage, format!("Couldn't read {}: {e}.", file.display()));
        let source = tokio::fs::File::open(file).await.map_err(read_failed)?;
        let size = source.metadata().await.map_err(read_failed)?.len();
        let mut header = tar::Header::new_gnu();
        header
            .set_path(file_name)
            .map_err(|e| Error::new(ErrorKind::Invalid, format!("Couldn't name the file {file_name}: {e}.")))?;
        header.set_size(size);
        header.set_mode(0o644);
        header.set_cksum();
        let head = Bytes::copy_from_slice(header.as_bytes());
        let padding = (512 - (size % 512) as usize) % 512;
        let tail = Bytes::from(vec![0u8; padding + 1024]);
        let chunks = futures_util::stream::try_unfold(source, |mut source| async move {
            let mut buffer = vec![0u8; 256 * 1024];
            let read = source.read(&mut buffer).await?;
            if read == 0 {
                return Ok(None);
            }
            buffer.truncate(read);
            Ok(Some((Bytes::from(buffer), source)))
        });
        let body = futures_util::stream::once(async { Ok(head) })
            .chain(chunks)
            .chain(futures_util::stream::once(async { Ok(tail) }));
        let options = UploadToContainerOptionsBuilder::new().path(dir).build();
        self.docker
            .upload_to_container(name, Some(options), bollard::body_try_stream(body))
            .await
            .map_err(|e| docker_error(&format!("Docker couldn't copy the file into {name}"), e))
    }

    /// Something happened to a container: made, started, stopped, renamed or removed. Ends if the engine goes away.
    pub fn changes(&self) -> impl Stream<Item = Result<(), Error>> + use<> {
        let filters = HashMap::from([("type", vec!["container"])]);
        let options = EventsOptionsBuilder::new().filters(&filters).build();
        self.docker
            .events(Some(options))
            .map_ok(|_| ())
            .map_err(|e| docker_error("Docker stopped sending its events", e))
    }
}

/// A container as Halfnine sees it, or `None` when it doesn't run Postgres.
fn postgres_container(inspected: ContainerInspectResponse) -> Option<Container> {
    let config = inspected.config.unwrap_or_default();
    let env = config.env.unwrap_or_default();
    let image = config.image.unwrap_or_default();
    if !postgres::is_postgres(&image, &env) {
        return None;
    }
    let running = inspected.state.and_then(|s| s.running).unwrap_or(false);
    // A running container reports the port it got; a stopped one, the port it asked for.
    let bindings = inspected
        .network_settings
        .and_then(|n| n.ports)
        .filter(|_| running)
        .or_else(|| inspected.host_config.and_then(|h| h.port_bindings));
    let port = bindings
        .and_then(|ports| ports.get(POSTGRES_PORT).cloned().flatten())
        .and_then(|bindings| {
            let ipv4 = bindings.iter().find(|b| !b.host_ip.as_deref().unwrap_or_default().contains(':'));
            ipv4.or(bindings.first())?.host_port.as_deref()?.parse().ok()
        });
    let volume = inspected.mounts.unwrap_or_default().into_iter().find_map(|m| {
        let data = m.destination.as_deref().is_some_and(|d| d.starts_with("/var/lib/postgresql"));
        (m.typ.as_deref() == Some("volume") && data).then_some(m.name).flatten()
    });
    Some(Container {
        id: inspected.id.unwrap_or_default(),
        name: inspected.name.unwrap_or_default().trim_start_matches('/').to_string(),
        major: postgres::major(&image, &env),
        image_id: inspected.image.unwrap_or_default(),
        image,
        running,
        port,
        labels: config.labels.unwrap_or_default(),
        volume,
        credentials: postgres::credentials(&env),
    })
}

/// Download progress by layer, so the whole is one number.
#[derive(Default)]
pub struct Layers(HashMap<String, (i64, i64)>);

impl Layers {
    pub fn update(&mut self, layer: String, current: i64, total: i64) {
        let seen = self.0.entry(layer).or_default();
        *seen = (current.max(seen.0), total.max(seen.1));
    }

    pub fn fraction(&self) -> f64 {
        let (current, total) = self.0.values().fold((0, 0), |(c, t), (lc, lt)| (c + lc, t + lt));
        if total == 0 { 0.0 } else { (current as f64 / total as f64).min(1.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_when_something_took_the_port_first() {
        let taken = "Docker couldn't start hn: Bind for 127.0.0.1:5433 failed: port is already allocated";
        assert!(port_taken(&Error::new(ErrorKind::Docker, taken)));
        assert!(!port_taken(&Error::new(ErrorKind::Docker, "Docker couldn't start hn: no such image")));
    }

    #[test]
    fn counts_download_progress_across_layers() {
        let mut layers = Layers::default();
        layers.update("a".into(), 50, 100);
        layers.update("b".into(), 0, 300);
        assert_eq!(layers.fraction(), 0.125);
        layers.update("a".into(), 100, 100);
        layers.update("b".into(), 300, 300);
        assert_eq!(layers.fraction(), 1.0);
    }
}
