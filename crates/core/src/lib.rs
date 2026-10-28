//! The types every part of TableUltra shares, and the [`Database`] trait each database adapter implements.
//! Nothing here does I/O: values are parsed into these types once, at the edge, and trusted from then on.

mod connection;
mod data;
mod database;
mod error;

pub use connection::{ConnectionSpec, Label, SshAuth, SshSpec, TlsMode, UrlParts, parse_url};
pub use data::{
    Cell, ColumnInfo, ColumnValue, Constraint, CutCell, EnumType, Filter, ForeignKey, IncomingKey, Index, Page, PageColumn, PageRequest, Relation,
    RelationKind, SchemaNode, ServerInfo, Sort, Structure, TableRef,
};
pub use database::Database;
pub use error::{Error, ErrorKind};
