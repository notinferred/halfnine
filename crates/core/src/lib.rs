//! The types every part of Halfnine shares, and the [`Database`] trait each database adapter implements.
//! Nothing here does I/O: values are parsed into these types once, at the edge, and trusted from then on.

mod backup;
mod changes;
mod connection;
mod data;
mod database;
mod error;
mod rules;

pub use backup::{
    ColumnDefinition, GeneratedDefinition, IdentityDefinition, NamedDefinition, RestoreInto, SequenceDefinition, TableDefinition, TableSink,
    TriggerDefinition,
};
pub use changes::{CellChange, Changes, Choice, Committed, NewColumn, RowUpdate, StructureChange};
pub use connection::{
    AgentAccess, AgentMark, ConnectionSpec, HiddenColumn, Label, Made, MadeHow, Source, SshAuth, SshSpec, TlsMode, UrlParts, parse_url, url,
};
pub use data::{
    Activity, Cell, ColumnInfo, ColumnRef, ColumnValue, Constraint, CutCell, EnumType, ExportSource, Filter, ForeignKey, IncomingKey, Index, Page,
    PageColumn, PageRequest, Reference, Relation, RelationKind, RunFailure, RunOutcome, SchemaNode, ServerInfo, Sort, StatementResult, Structure,
    TableRef,
};
pub use database::{ByteSource, Database, RowSink};
pub use error::{Error, ErrorKind};
pub use rules::{Checks, Rule, RuleCheck, RuleSource, Suggestion};

/// One line of CSV as Postgres's own CSV reader reads it back: NULL as an empty field and empty text as `""`. A value is
/// quoted when it must be, including `\.` alone, which Postgres would otherwise take for the end of the data.
pub fn csv_line<'a>(values: impl IntoIterator<Item = Option<&'a str>>) -> String {
    let fields: Vec<String> = values
        .into_iter()
        .map(|value| match value {
            None => String::new(),
            Some(text) if text.is_empty() || text == "\\." || text.contains([',', '"', '\n', '\r']) => {
                format!("\"{}\"", text.replace('"', "\"\""))
            }
            Some(text) => text.to_string(),
        })
        .collect();
    format!("{}\n", fields.join(","))
}

/// A duration as the whole milliseconds the UI shows, saturating at `u32::MAX` (49 days).
pub fn millis(elapsed: std::time::Duration) -> u32 {
    u32::try_from(elapsed.as_millis()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_null_apart_from_empty_text() {
        let line = super::csv_line([Some("a"), None, Some(""), Some("say \"hi\"\nthere"), Some("\\."), Some("b c")]);
        assert_eq!(line, "a,,\"\",\"say \"\"hi\"\"\nthere\",\"\\.\",b c\n");
    }
}
