//! Writing a result to a file, row by row as the database sends it, so a table of any size exports in constant memory.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;
use tu_core::{Cell, Error, PageColumn, RowSink};

use crate::files::storage;

#[derive(Clone, Copy, Debug, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    Csv,
    Json,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Json => "json",
        }
    }
}

#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExportDone {
    pub rows: f64,
    pub file: String,
}

pub enum Sink {
    /// NULL becomes an empty field, as CSV has no other way to write it.
    Csv(Box<csv::Writer<BufWriter<File>>>),
    /// An array of objects keyed by column name, with each value as Postgres printed it, or null.
    Json { out: BufWriter<File>, names: Vec<String>, rows: u64 },
}

impl Sink {
    pub fn create(path: &Path, format: ExportFormat) -> Result<Self, Error> {
        let file = File::create(path).map_err(|e| storage("Couldn't create the file", e))?;
        Ok(match format {
            ExportFormat::Csv => Self::Csv(Box::new(csv::Writer::from_writer(BufWriter::new(file)))),
            ExportFormat::Json => Self::Json {
                out: BufWriter::new(file),
                names: Vec::new(),
                rows: 0,
            },
        })
    }

    pub fn finish(self) -> Result<(), Error> {
        match self {
            Self::Csv(mut writer) => writer.flush().map_err(|e| storage("Couldn't finish the file", e)),
            Self::Json { mut out, rows, .. } => {
                let end = if rows == 0 { "]\n" } else { "\n]\n" };
                out.write_all(end.as_bytes())
                    .and_then(|()| out.flush())
                    .map_err(|e| storage("Couldn't finish the file", e))
            }
        }
    }
}

impl RowSink for Sink {
    fn columns(&mut self, columns: &[PageColumn]) -> Result<(), Error> {
        match self {
            Self::Csv(writer) => writer
                .write_record(columns.iter().map(|c| &c.name))
                .map_err(|e| storage("Couldn't write the file", e)),
            Self::Json { out, names, .. } => {
                *names = columns.iter().map(|c| c.name.clone()).collect();
                out.write_all(b"[").map_err(|e| storage("Couldn't write the file", e))
            }
        }
    }

    fn row(&mut self, row: &[Cell]) -> Result<(), Error> {
        match self {
            Self::Csv(writer) => writer
                .write_record(row.iter().map(|v| v.as_deref().unwrap_or("")))
                .map_err(|e| storage("Couldn't write the file", e)),
            Self::Json { out, names, rows } => {
                let object: serde_json::Map<String, serde_json::Value> = names
                    .iter()
                    .zip(row)
                    .map(|(name, value)| (name.clone(), value.clone().map_or(serde_json::Value::Null, serde_json::Value::String)))
                    .collect();
                let separator: &[u8] = if *rows == 0 { b"\n  " } else { b",\n  " };
                *rows += 1;
                out.write_all(separator)
                    .map_err(|e| storage("Couldn't write the file", e))
                    .and_then(|()| serde_json::to_writer(&mut *out, &object).map_err(|e| storage("Couldn't write the file", e)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(format: ExportFormat) -> String {
        let path = std::env::temp_dir().join(format!("tableultra-export-{}.{}", uuid::Uuid::new_v4(), format.extension()));
        let mut sink = Sink::create(&path, format).unwrap();
        let columns = [
            PageColumn {
                name: "id".into(),
                type_name: "int8".into(),
            },
            PageColumn {
                name: "note, with comma".into(),
                type_name: "text".into(),
            },
        ];
        sink.columns(&columns).unwrap();
        sink.row(&[Some("1".into()), Some("say \"hi\"".into())]).unwrap();
        sink.row(&[Some("2".into()), None]).unwrap();
        sink.finish().unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        written
    }

    #[test]
    fn writes_csv_with_quoting() {
        assert_eq!(write(ExportFormat::Csv), "id,\"note, with comma\"\n1,\"say \"\"hi\"\"\"\n2,\n");
    }

    #[test]
    fn writes_json_with_nulls() {
        let parsed: serde_json::Value = serde_json::from_str(&write(ExportFormat::Json)).unwrap();
        assert_eq!(
            parsed,
            serde_json::json!([{ "id": "1", "note, with comma": "say \"hi\"" }, { "id": "2", "note, with comma": null }])
        );
    }
}
