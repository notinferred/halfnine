use serde::{Deserialize, Serialize};
use specta::Type;

use crate::data::TableRef;
use crate::database::RowSink;
use crate::error::Error;

/// A table as a backup keeps it, to make it again: each part as Postgres itself printed it.
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TableDefinition {
    pub table: TableRef,
    pub unlogged: bool,
    pub comment: Option<String>,
    pub columns: Vec<ColumnDefinition>,
    /// Primary key, unique, exclusion, check and foreign key constraints, foreign keys last.
    pub constraints: Vec<NamedDefinition>,
    /// Indexes no constraint makes, each as Postgres writes its `CREATE INDEX`.
    pub indexes: Vec<NamedDefinition>,
    pub triggers: Vec<TriggerDefinition>,
    /// Sequences the table owns, as a `serial` column's, made again before it.
    pub sequences: Vec<SequenceDefinition>,
    pub primary_key: Vec<String>,
    /// What this backup doesn't keep, in words, such as row-level security policies.
    pub not_kept: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDefinition {
    pub name: String,
    pub type_name: String,
    /// A collation other than the type's own, already quoted.
    pub collation: Option<String>,
    pub not_null: bool,
    pub default: Option<String>,
    pub identity: Option<IdentityDefinition>,
    /// The expression of a generated column, and whether it is stored.
    pub generated: Option<GeneratedDefinition>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IdentityDefinition {
    /// Generated always, rather than by default.
    pub always: bool,
    /// Its sequence's options, such as `INCREMENT BY 1 MINVALUE 1 …`.
    pub options: String,
    /// The value it last gave, or `None` if it never gave one.
    pub last_value: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedDefinition {
    pub expression: String,
    pub stored: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NamedDefinition {
    pub name: String,
    pub definition: String,
    /// The columns it uses, so putting one column back adds only what dropping it took.
    pub columns: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDefinition {
    pub name: String,
    pub definition: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SequenceDefinition {
    pub schema: String,
    pub name: String,
    /// The column whose default it fills.
    pub column: String,
    pub options: String,
    pub last_value: Option<String>,
}

/// What a restore puts back.
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RestoreInto {
    /// The table was dropped: it is made again, with its rows.
    Recreate,
    /// The table was emptied: its rows go back in.
    Refill,
    /// Columns were dropped: they are added again, and each row gets its values back by primary key.
    Columns { names: Vec<String> },
}

/// Where a backup goes as it is taken: the definition first, then its columns and every row, then the end.
pub trait TableSink: RowSink {
    fn definition(&mut self, definition: &TableDefinition) -> Result<(), Error>;
    /// Called once every row is written: the backup must be safe on disk before the change commits.
    fn finish(&mut self) -> Result<(), Error>;
}
