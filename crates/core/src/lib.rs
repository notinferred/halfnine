//! The types every part of TableUltra shares, and the [`Database`] trait each database adapter implements.
//! Nothing here does I/O: values are parsed into these types once, at the edge, and trusted from then on.

mod connection;
mod data;
mod database;
mod error;

pub use connection::{ConnectionSpec, Label, SshAuth, SshSpec, TlsMode, UrlParts, parse_url};
pub use data::{
    Activity, Cell, ColumnInfo, ColumnValue, Constraint, CutCell, EnumType, ExportSource, Filter, ForeignKey, IncomingKey, Index, Page, PageColumn,
    PageRequest, Relation, RelationKind, RunFailure, RunOutcome, SchemaNode, ServerInfo, Sort, StatementResult, Structure, TableRef,
};
pub use database::{Database, RowSink};
pub use error::{Error, ErrorKind};

/// A duration as the whole milliseconds the UI shows, saturating at `u32::MAX` (49 days).
pub fn millis(elapsed: std::time::Duration) -> u32 {
    u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX)
}
