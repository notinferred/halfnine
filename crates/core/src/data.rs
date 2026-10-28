use serde::{Deserialize, Serialize};
use specta::Type;

/// A value exactly as Postgres prints it as text, or `None` for NULL, which is never confused with an empty string.
pub type Cell = Option<String>;

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct TableRef {
    pub schema: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    pub version: String,
    pub database: String,
    pub user: String,
    /// Every database on the server that accepts connections, to switch between.
    pub databases: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SchemaNode {
    pub name: String,
    pub relations: Vec<Relation>,
    pub functions: Vec<String>,
    pub sequences: Vec<String>,
    pub enums: Vec<EnumType>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Relation {
    pub name: String,
    pub kind: RelationKind,
    /// Postgres's planner estimate; `None` when the table has never been analyzed.
    pub estimated_rows: Option<f64>,
}

#[derive(Clone, Copy, Debug, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RelationKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    ForeignTable,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct EnumType {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Structure {
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Vec<String>,
    pub indexes: Vec<Index>,
    /// Check, unique and exclusion constraints. The primary key is in `primary_key` and foreign keys have their own lists.
    pub constraints: Vec<Constraint>,
    pub foreign_keys: Vec<ForeignKey>,
    /// Foreign keys in other tables that point at this one.
    pub referenced_by: Vec<IncomingKey>,
    pub estimated_rows: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    /// The type as Postgres writes it, such as `numeric(12,2)` or `timestamp with time zone`.
    pub type_name: String,
    pub nullable: bool,
    /// The default expression, or how an identity or generated column is filled.
    pub default: Option<String>,
    pub comment: Option<String>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Index {
    pub name: String,
    pub definition: String,
    pub unique: bool,
    pub primary: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Constraint {
    pub name: String,
    pub definition: String,
}

/// A foreign key on this table: `columns` here point at `target_columns` in `target`.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKey {
    pub name: String,
    pub columns: Vec<String>,
    pub target: TableRef,
    pub target_columns: Vec<String>,
}

/// A foreign key elsewhere: `source_columns` in `source` point at `columns` here.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct IncomingKey {
    pub name: String,
    pub source: TableRef,
    pub source_columns: Vec<String>,
    pub columns: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    pub table: TableRef,
    pub sort: Option<Sort>,
    pub filter: Option<Filter>,
    pub offset: u32,
    pub limit: u32,
}

#[derive(Clone, Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Sort {
    pub column: String,
    pub descending: bool,
}

#[derive(Clone, Debug, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Filter {
    /// A condition you typed, used as the query's `WHERE`.
    Where { sql: String },
    /// Rows whose columns equal these values, as when following a foreign key.
    Equals { values: Vec<ColumnValue> },
}

#[derive(Clone, Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ColumnValue {
    pub column: String,
    pub value: Cell,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub columns: Vec<PageColumn>,
    pub rows: Vec<Vec<Cell>>,
    /// Cells longer than the page's limit, which were cut and need the value viewer for the rest.
    pub cut: Vec<CutCell>,
    pub has_more: bool,
    pub elapsed_ms: f64,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PageColumn {
    pub name: String,
    pub type_name: String,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CutCell {
    pub row: u32,
    pub column: u32,
}
