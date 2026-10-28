use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use specta::Type;
use url::Url;

use crate::error::{Error, ErrorKind};

/// A saved connection, without its secrets: passwords live only in the OS keychain.
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSpec {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: String,
    pub tls: TlsMode,
    pub label: Label,
    /// The server refuses writes: every session starts with `default_transaction_read_only = on`.
    pub read_only: bool,
    pub ssh: Option<SshSpec>,
}

/// The same choices as libpq's `sslmode`, named the same way.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    Disable,
    Prefer,
    Require,
    VerifyFull,
}

/// The colour that marks an environment, so production always looks different.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Label {
    None,
    Green,
    Amber,
    Red,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SshSpec {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
}

/// How the tunnel authenticates. A key's passphrase or the password is kept in the keychain, never here.
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum SshAuth {
    Agent,
    Key { path: String },
    Password,
}

/// What a pasted `postgres://` URL says, ready to fill in a connection form.
#[derive(Clone, Debug, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UrlParts {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<String>,
    pub database: String,
    pub tls: Option<TlsMode>,
}

/// Parses a libpq-style URL with one host, such as `postgres://user:secret@db.example.com:5432/app?sslmode=require`.
pub fn parse_url(input: &str) -> Result<UrlParts, Error> {
    let invalid = |message: &str| Error::new(ErrorKind::Invalid, message);
    let url = Url::parse(input.trim()).map_err(|_| invalid("That isn't a URL. It should look like postgres://user@host:5432/database."))?;
    if !matches!(url.scheme(), "postgres" | "postgresql") {
        return Err(invalid("The URL should start with postgres:// or postgresql://."));
    }
    let host = url.host_str().filter(|h| !h.is_empty()).ok_or_else(|| invalid("The URL has no host."))?;
    if host.contains(',') {
        return Err(invalid("Only one host is supported."));
    }
    let decode = |s: &str| {
        percent_decode_str(s)
            .decode_utf8()
            .map(|d| d.into_owned())
            .map_err(|_| invalid("The URL isn't valid UTF-8."))
    };
    let user = decode(url.username())?;
    let database = decode(url.path().trim_start_matches('/'))?;
    let password = url.password().map(decode).transpose()?;
    let tls = match url.query_pairs().find(|(k, _)| k == "sslmode").map(|(_, v)| v.into_owned()) {
        None => None,
        Some(mode) => Some(match mode.as_str() {
            "disable" | "allow" => TlsMode::Disable,
            "prefer" => TlsMode::Prefer,
            "require" | "verify-ca" => TlsMode::Require,
            "verify-full" => TlsMode::VerifyFull,
            _ => return Err(invalid("The URL's sslmode isn't one Postgres knows.")),
        }),
    };
    Ok(UrlParts {
        host: host.trim_matches(['[', ']']).to_string(),
        port: url.port().unwrap_or(5432),
        user,
        password,
        database,
        tls,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_url() {
        let parts = parse_url("postgres://app:s%40cret@db.example.com:6543/my%20db?sslmode=verify-full").unwrap();
        assert_eq!(
            parts,
            UrlParts {
                host: "db.example.com".into(),
                port: 6543,
                user: "app".into(),
                password: Some("s@cret".into()),
                database: "my db".into(),
                tls: Some(TlsMode::VerifyFull),
            }
        );
    }

    #[test]
    fn fills_defaults_and_accepts_postgresql() {
        let parts = parse_url("postgresql://localhost").unwrap();
        assert_eq!(
            (parts.port, parts.user.as_str(), parts.database.as_str(), parts.tls),
            (5432, "", "", None)
        );
    }

    #[test]
    fn rejects_other_schemes_and_hosts_lists() {
        assert!(parse_url("mysql://localhost/app").is_err());
        assert!(parse_url("not a url").is_err());
        assert!(parse_url("postgres://a,b/app").is_err());
        assert!(parse_url("postgres://localhost/app?sslmode=sometimes").is_err());
    }
}
