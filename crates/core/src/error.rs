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
    /// The connection asks for its password each time, and none has been typed this launch.
    PasswordNeeded,
    Tls,
    Ssh,
    Query,
    ReadOnly,
    /// A row changed after it was loaded, so a commit wrote nothing.
    Conflict,
    NotFound,
    Invalid,
    Storage,
    Keychain,
    /// The Docker engine on this computer couldn't be reached, or refused.
    Docker,
    /// Nothing was dropped, emptied or deleted, because Halfnine couldn't back it up first. The message says why.
    NoBackup,
    /// Nothing was written, because the change broke a rule the person set, or Halfnine couldn't check one in time.
    BrokenRule,
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
