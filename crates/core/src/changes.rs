use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::data::{Cell, ColumnValue, TableRef};

/// Changes someone staged in the app, or an agent proposed. They are shown as SQL first, then committed together in one transaction.
#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Changes {
    /// Rows to delete, change and add, in that order. Rows are named by their primary key.
    Rows {
        table: TableRef,
        deletes: Vec<Vec<ColumnValue>>,
        updates: Vec<RowUpdate>,
        /// Each new row's values. A column left out takes its default.
        inserts: Vec<Vec<ColumnValue>>,
    },
    /// Changes to a table's structure, or a new table.
    Structure { table: TableRef, changes: Vec<StructureChange> },
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RowUpdate {
    /// The row's primary key, as it was loaded.
    pub key: Vec<ColumnValue>,
    pub cells: Vec<CellChange>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CellChange {
    pub column: String,
    /// The value as it was loaded. If the row holds something else by the time of the commit, nothing is written.
    pub from: Cell,
    pub to: Cell,
}

/// One change to a table's structure. Types and defaults are SQL someone typed, as Postgres would read them.
#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum StructureChange {
    CreateTable { columns: Vec<NewColumn> },
    AddColumn { column: NewColumn },
    RenameColumn { column: String, to: String },
    SetType { column: String, to: String },
    SetNullable { column: String, nullable: bool },
    SetDefault { column: String, to: Option<String> },
    DropColumn { column: String },
    CreateIndex { columns: Vec<String>, unique: bool },
    DropIndex { name: String },
    Truncate,
    Drop,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NewColumn {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    /// A default expression, or how Postgres fills the column, such as `generated always as identity`.
    pub default: Option<String>,
    /// Part of the new table's primary key.
    pub key: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Committed {
    pub statements: u32,
    pub elapsed_ms: u32,
}

/// A value to pick for a foreign key: the key itself, and a column of the same row to recognise it by.
#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: String,
    pub label: Option<String>,
}
