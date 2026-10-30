//! What Halfnine knows about the official `postgres` image: which containers run it, where its data lives, and the
//! credentials its environment gives. Pure, so it is tested without an engine.

use std::collections::HashSet;

use hn_core::{Error, ErrorKind};

/// The versions offered when creating a database, newest first. Each is a tag of the official image.
pub const VERSIONS: [u16; 5] = [18, 17, 16, 15, 14];

/// Labels Halfnine puts on what it makes, so it knows its own.
pub mod label {
    pub const MANAGED: &str = "com.notinferred.halfnine.managed";
    pub const ID: &str = "com.notinferred.halfnine.id";
    pub const ORIGIN: &str = "com.notinferred.halfnine.origin";
    pub const COPY_OF: &str = "com.notinferred.halfnine.copy-of";
    pub const AGENT: &str = "com.notinferred.halfnine.agent";
    pub const AGENT_KEY: &str = "com.notinferred.halfnine.agent-key";
    /// Only on what the tests make, so CI can remove it whatever happens.
    pub const TEST: &str = "com.notinferred.halfnine.test";
}

pub fn image(major: u16) -> String {
    format!("postgres:{major}")
}

/// Where a version keeps its data, which its volume is mounted on. From 18 the image keeps it a level up, and a volume
/// at the old place stops the container from starting.
pub fn data_path(major: u16) -> &'static str {
    if major >= 18 {
        "/var/lib/postgresql"
    } else {
        "/var/lib/postgresql/data"
    }
}

/// A container runs Postgres when its image is `postgres`, or one built on it, such as PostGIS, which sets `PG_MAJOR`.
pub fn is_postgres(image: &str, env: &[String]) -> bool {
    let name = image.rsplit('/').next().unwrap_or(image);
    name.starts_with("postgres") || env.iter().any(|e| e.starts_with("PG_MAJOR="))
}

/// The major version, from `PG_MAJOR` when the image sets it, else from the image's tag.
pub fn major(image: &str, env: &[String]) -> Option<u16> {
    let from_env = env.iter().find_map(|e| e.strip_prefix("PG_MAJOR=")).and_then(|v| v.parse().ok());
    from_env.or_else(|| {
        let tag = image.rsplit_once(':')?.1;
        tag.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
    })
}

/// How to sign in to a container's database, as its environment says. The environment applies only when the data was
/// first made, so it can be out of date.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub user: String,
    pub password: Option<String>,
    /// A file inside the container that holds the password, read when connecting.
    pub password_file: Option<String>,
    pub database: String,
    /// The container lets anyone in without a password.
    pub trust: bool,
}

pub fn credentials(env: &[String]) -> Credentials {
    let get = |key: &str| env.iter().find_map(|e| e.strip_prefix(key)?.strip_prefix('=')).map(str::to_string);
    let user = get("POSTGRES_USER").filter(|u| !u.is_empty()).unwrap_or_else(|| "postgres".to_string());
    Credentials {
        database: get("POSTGRES_DB").filter(|d| !d.is_empty()).unwrap_or_else(|| user.clone()),
        password: get("POSTGRES_PASSWORD"),
        password_file: get("POSTGRES_PASSWORD_FILE"),
        trust: get("POSTGRES_HOST_AUTH_METHOD").as_deref() == Some("trust"),
        user,
    }
}

/// A container name Docker accepts and people can type.
pub fn valid_name(name: &str) -> Result<(), Error> {
    let mut chars = name.chars();
    let starts = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    let rest = chars.all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
    if starts && rest && name.len() <= 63 {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::Invalid,
            "Name it with letters, digits, dots, dashes and underscores, starting with a letter or digit.",
        ))
    }
}

/// A host port for a new database: the first from 5433 that no container publishes and nothing else listens on.
pub fn pick_port(taken: &HashSet<u16>, free: impl Fn(u16) -> bool) -> Option<u16> {
    (5433..=5532).find(|port| !taken.contains(port) && free(*port))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(vars: &[&str]) -> Vec<String> {
        vars.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn keeps_data_where_each_version_expects_it() {
        assert_eq!(data_path(18), "/var/lib/postgresql");
        assert_eq!(data_path(19), "/var/lib/postgresql");
        for major in 14..=17 {
            assert_eq!(data_path(major), "/var/lib/postgresql/data");
        }
    }

    #[test]
    fn knows_postgres_containers_and_their_version() {
        assert!(is_postgres("postgres:18", &[]));
        assert!(is_postgres("docker.io/library/postgres:16-alpine", &[]));
        assert!(is_postgres("postgis/postgis:17-3.5", &env(&["PG_MAJOR=17"])));
        assert!(!is_postgres("redis:7", &env(&["REDIS_VERSION=7"])));
        assert_eq!(major("postgres:16-alpine", &[]), Some(16));
        assert_eq!(major("postgis/postgis:latest", &env(&["PG_MAJOR=17"])), Some(17));
        assert_eq!(major("postgres", &[]), None);
    }

    #[test]
    fn reads_credentials_with_the_image_defaults() {
        let plain = credentials(&env(&["POSTGRES_PASSWORD=s3cret", "PG_MAJOR=18"]));
        assert_eq!(
            plain,
            Credentials {
                user: "postgres".into(),
                password: Some("s3cret".into()),
                password_file: None,
                database: "postgres".into(),
                trust: false,
            }
        );
        let named = credentials(&env(&[
            "POSTGRES_USER=app",
            "POSTGRES_PASSWORD_FILE=/run/secrets/pg",
            "POSTGRES_HOST_AUTH_METHOD=trust",
        ]));
        assert_eq!(
            (named.database.as_str(), named.password_file.as_deref(), named.trust),
            ("app", Some("/run/secrets/pg"), true)
        );
    }

    #[test]
    fn refuses_names_docker_wouldnt_take() {
        assert!(valid_name("shop-dev_2.1").is_ok());
        for bad in ["", "-x", "has space", "ünïcödé", &"x".repeat(64)] {
            assert!(valid_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn picks_the_first_free_port_from_5433() {
        let taken: HashSet<u16> = [5433, 5434].into();
        assert_eq!(pick_port(&taken, |p| p != 5435), Some(5436));
        assert_eq!(pick_port(&HashSet::new(), |_| false), None);
    }
}
