use serde::Serialize;
use specta::Type;

/// The one error the UI ever sees: a kind it can act on, a sentence for people, and Postgres's SQLSTATE when there is one.
#[derive(Clone, Debug, Serialize, Type, thiserror::Error, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[error("{message}")]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub code: Option<String>,
}

/// Transient kinds (`Network`, `Timeout`) are worth retrying; the rest need someone to change something first.
#[derive(Clone, Copy, Debug, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    Network,
    Timeout,
    Auth,
    Tls,
    Ssh,
    Query,
    ReadOnly,
    NotFound,
    Invalid,
    Storage,
    Keychain,
}

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            code: None,
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}
