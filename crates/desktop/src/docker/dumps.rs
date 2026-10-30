//! Whole Docker databases saved to a folder and made again from it, with the tools inside the container: the roles
//! (`pg_dumpall --globals-only`, without their password hashes) and each database (`pg_dump -Fc`). A copy is made
//! this way, and a backup before a delete.

use std::path::Path;
use std::time::{Duration, Instant};

use hn_core::{Error, ErrorKind};
use hn_docker::Engine;
use serde::{Deserialize, Serialize};

use crate::files::private_dir;

const GLOBALS: &str = "globals.sql";
/// How long a container just started may take before Postgres in it accepts connections.
const STARTING: Duration = Duration::from_secs(60);

/// What a dump folder holds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Dumped {
    pub user: String,
    /// Each database, dumped to `<n>.dump` in this order.
    pub databases: Vec<String>,
}

fn failed(what: &str, stderr: &str) -> Error {
    let first: Vec<&str> = stderr.lines().take(3).collect();
    Error::new(ErrorKind::Docker, format!("{what}: {}", first.join(" ")))
}

fn write_failed(e: std::io::Error) -> Error {
    Error::new(ErrorKind::Storage, format!("Couldn't write the dump: {e}."))
}

/// A file only this account can read, as a dump holds every row.
async fn create(path: &Path) -> Result<tokio::fs::File, Error> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    options.open(path).await.map_err(write_failed)
}

/// How big every database in a running container is together, in bytes.
pub async fn size(engine: &Engine, container: &str, user: &str) -> Result<f64, Error> {
    accepting(engine, container, user).await?;
    let sum = "SELECT coalesce(sum(pg_database_size(datname)), 0) FROM pg_database WHERE datallowconn AND NOT datistemplate";
    let (ran, out) = engine.run(container, &["psql", "-U", user, "-d", "postgres", "-tAc", sum]).await?;
    if !ran.succeeded() {
        return Err(failed("Halfnine couldn't measure the databases", &ran.stderr));
    }
    String::from_utf8_lossy(&out)
        .trim()
        .parse()
        .map_err(|_| Error::new(ErrorKind::Docker, "Halfnine couldn't measure the databases."))
}

/// Waits until Postgres in a running container accepts connections, which it doesn't at once after starting.
async fn accepting(engine: &Engine, container: &str, user: &str) -> Result<(), Error> {
    let started = Instant::now();
    loop {
        let (ran, _) = engine.run(container, &["pg_isready", "-q", "-U", user]).await?;
        if ran.succeeded() {
            return Ok(());
        }
        if started.elapsed() > STARTING {
            return Err(Error::new(
                ErrorKind::Docker,
                format!("{container} didn't start accepting connections within a minute."),
            ));
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Saves every database of a running container, and the roles, to `dir`, streamed to disk.
pub async fn dump(engine: &Engine, container: &str, user: &str, dir: &Path) -> Result<Dumped, Error> {
    private_dir(dir)?;
    accepting(engine, container, user).await?;
    let list = "SELECT datname FROM pg_database WHERE datallowconn AND NOT datistemplate ORDER BY datname";
    let (ran, out) = engine.run(container, &["psql", "-U", user, "-d", "postgres", "-tAc", list]).await?;
    if !ran.succeeded() {
        return Err(failed("Halfnine couldn't list the databases", &ran.stderr));
    }
    let databases: Vec<String> = String::from_utf8_lossy(&out)
        .lines()
        .map(str::to_string)
        .filter(|d| !d.is_empty())
        .collect();
    let mut globals = create(&dir.join(GLOBALS)).await?;
    let ran = engine
        .run_to(
            container,
            &["pg_dumpall", "-U", user, "--globals-only", "--no-role-passwords"],
            &mut globals,
        )
        .await?;
    if !ran.succeeded() {
        return Err(failed("Halfnine couldn't save the roles", &ran.stderr));
    }
    for (n, database) in databases.iter().enumerate() {
        let path = dir.join(format!("{n}.dump"));
        let mut file = create(&path).await?;
        let ran = engine
            .run_to(container, &["pg_dump", "-Fc", "-U", user, "-d", database], &mut file)
            .await?;
        if !ran.succeeded() {
            let _ = std::fs::remove_file(&path);
            return Err(failed(&format!("Halfnine couldn't save {database}"), &ran.stderr));
        }
        file.sync_all().await.map_err(write_failed)?;
    }
    let dumped = Dumped {
        user: user.to_string(),
        databases,
    };
    globals.sync_all().await.map_err(write_failed)?;
    Ok(dumped)
}

/// Makes every database of a dump folder in a running container: the roles first, then each database, made new
/// unless it already exists, as the one the container was made with does.
pub async fn restore(engine: &Engine, container: &str, dir: &Path, dumped: &Dumped) -> Result<(), Error> {
    engine.run(container, &["mkdir", "-p", "/tmp/halfnine"]).await?;
    engine.upload(container, "/tmp/halfnine", GLOBALS, &dir.join(GLOBALS)).await?;
    // Roles that already exist, such as the container's own user, only draw errors here, which are fine.
    let _ = engine
        .run(
            container,
            &["psql", "-q", "-U", &dumped.user, "-d", "postgres", "-f", "/tmp/halfnine/globals.sql"],
        )
        .await?;
    let (ran, out) = engine
        .run(
            container,
            &["psql", "-U", &dumped.user, "-d", "postgres", "-tAc", "SELECT datname FROM pg_database"],
        )
        .await?;
    if !ran.succeeded() {
        return Err(failed("Halfnine couldn't list the databases", &ran.stderr));
    }
    let existing = String::from_utf8_lossy(&out).to_string();
    for (n, database) in dumped.databases.iter().enumerate() {
        let file = format!("{n}.dump");
        engine.upload(container, "/tmp/halfnine", &file, &dir.join(&file)).await?;
        let path = format!("/tmp/halfnine/{file}");
        let exists = existing.lines().any(|d| d == database);
        let cmd: Vec<&str> = if exists {
            vec!["pg_restore", "-U", &dumped.user, "-d", database, &path]
        } else {
            vec!["pg_restore", "-U", &dumped.user, "-C", "-d", "postgres", &path]
        };
        let (ran, _) = engine.run(container, &cmd).await?;
        let _ = engine.run(container, &["rm", "-f", &path]).await;
        if !ran.succeeded() {
            return Err(failed(&format!("Halfnine couldn't restore {database}"), &ran.stderr));
        }
    }
    let _ = engine.run(container, &["rm", "-rf", "/tmp/halfnine"]).await;
    Ok(())
}
