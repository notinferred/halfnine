use hn_core::{Error, ErrorKind};

/// Turns a driver error into words for people, keeping Postgres's SQLSTATE so the cause can be looked up.
pub(crate) fn from_sqlx(error: sqlx::Error) -> Error {
    match error {
        sqlx::Error::Database(db) => {
            let code = db.code().map(|c| c.into_owned());
            let (kind, message) = match code.as_deref() {
                Some("28P01" | "28000") => (ErrorKind::Auth, db.message().to_string()),
                Some("25006") => (
                    ErrorKind::ReadOnly,
                    "This connection is read-only, so Postgres refused the change.".to_string(),
                ),
                Some("57014") => (
                    ErrorKind::Timeout,
                    "Postgres stopped the statement: it was cancelled, or ran past its time limit.".to_string(),
                ),
                Some("3D000") => (ErrorKind::NotFound, db.message().to_string()),
                Some(c) if c.starts_with("08") || c.starts_with("57P") => (ErrorKind::Network, db.message().to_string()),
                _ => (ErrorKind::Query, db.message().to_string()),
            };
            let error = Error::new(kind, message);
            match code {
                Some(code) => error.with_code(code),
                None => error,
            }
        }
        sqlx::Error::Io(e) => Error::new(ErrorKind::Network, format!("Couldn't reach the server: {e}.")),
        sqlx::Error::Tls(e) => Error::new(ErrorKind::Tls, format!("The secure connection failed: {e}.")),
        sqlx::Error::PoolTimedOut => Error::new(ErrorKind::Timeout, "The server took too long to answer."),
        sqlx::Error::PoolClosed => Error::new(ErrorKind::Network, "This connection was closed."),
        sqlx::Error::Configuration(e) => Error::new(ErrorKind::Invalid, e.to_string()),
        other => Error::new(ErrorKind::Query, other.to_string()),
    }
}
