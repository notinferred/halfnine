//! What each tool takes, as agents send it, and how it becomes the app's own types once, here at the edge.

use hn_core::{Cell, ColumnValue, StructureChange, TableRef};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

/// The most rows one `change_rows` call may touch; bulk changes go through `run_sql`.
pub const ROWS_LIMIT: usize = 500;
/// The most statements one `run_sql` call may hold.
pub const STATEMENTS_LIMIT: usize = 20;
pub const QUERY_LIMIT: u32 = 1_000;
const QUERY_DEFAULT: u32 = 100;

#[derive(Deserialize, JsonSchema)]
pub struct DescribeInput {
    /// The connection's name, exactly as list_connections shows it.
    pub connection: String,
    /// A database on that server. Leave it out for the connection's own database.
    pub database: Option<String>,
    /// Only this schema. Leave it out for every schema.
    pub schema: Option<String>,
    /// One table or view to describe in full: its columns, keys, indexes, constraints and foreign keys both ways.
    pub table: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct QueryInput {
    /// The connection's name, exactly as list_connections shows it.
    pub connection: String,
    /// A database on that server. Leave it out for the connection's own database.
    pub database: Option<String>,
    /// One statement that reads: SELECT, WITH, VALUES, TABLE, SHOW or EXPLAIN.
    pub sql: String,
    /// The most rows to return, 100 unless given, at most 1000.
    pub limit: Option<u32>,
}

impl QueryInput {
    pub fn limit(&self) -> u32 {
        self.limit.unwrap_or(QUERY_DEFAULT).clamp(1, QUERY_LIMIT)
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct TableName {
    /// The table's schema; `public` unless given.
    pub schema: Option<String>,
    pub name: String,
}

impl TableName {
    pub fn table_ref(&self) -> TableRef {
        TableRef {
            schema: self.schema.clone().unwrap_or_else(|| "public".to_string()),
            name: self.name.clone(),
        }
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct ChangeRowsInput {
    /// The connection's name, exactly as list_connections shows it.
    pub connection: String,
    /// A database on that server. Leave it out for the connection's own database.
    pub database: Option<String>,
    pub table: TableName,
    /// New rows, each as {"column": value}. A column left out takes its default.
    #[serde(default)]
    pub insert: Vec<Map<String, Value>>,
    /// Rows to change, each named by its whole primary key.
    #[serde(default)]
    pub update: Vec<RowChange>,
    /// Rows to delete, each as its whole primary key, such as {"id": 42}.
    #[serde(default)]
    pub delete: Vec<Map<String, Value>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RowChange {
    /// The row's whole primary key, such as {"id": 42}.
    pub key: Map<String, Value>,
    /// The new values, such as {"status": "paid"}. null is SQL NULL.
    pub set: Map<String, Value>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ChangeStructureInput {
    /// The connection's name, exactly as list_connections shows it.
    pub connection: String,
    /// A database on that server. Leave it out for the connection's own database.
    pub database: Option<String>,
    pub table: TableName,
    /// The changes, in any order: Halfnine runs them in a safe order in one transaction.
    pub changes: Vec<StructureChange>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RunSqlInput {
    /// The connection's name, exactly as list_connections shows it.
    pub connection: String,
    /// A database on that server. Leave it out for the connection's own database.
    pub database: Option<String>,
    /// Up to 20 statements, each exactly one, run in order in one transaction. No BEGIN, COMMIT or COPY.
    pub statements: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct WaitInput {
    /// The proposal's id, as a WAITING result gave it.
    pub proposal: String,
}

/// A JSON value as Postgres text: strings as they are, numbers and booleans as written, objects and arrays as JSON,
/// for json and jsonb columns, and null as NULL.
pub fn cell(value: &Value) -> Cell {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Number(n) => Some(n.to_string()),
        other => Some(other.to_string()),
    }
}

pub fn column_values(map: &Map<String, Value>) -> Vec<ColumnValue> {
    map.iter()
        .map(|(column, value)| ColumnValue {
            column: column.clone(),
            value: cell(value),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_json_values_as_postgres_text() {
        let values: Vec<Value> = serde_json::from_str(r#"["a", 1.5, true, null, {"k": [1]}]"#).unwrap();
        let cells: Vec<Cell> = values.iter().map(cell).collect();
        assert_eq!(
            cells,
            [
                Some("a".to_string()),
                Some("1.5".to_string()),
                Some("true".to_string()),
                None,
                Some(r#"{"k":[1]}"#.to_string())
            ]
        );
    }
}
