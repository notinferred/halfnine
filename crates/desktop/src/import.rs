//! CSV import. The file is chosen in a dialog opened from Rust, so a path never comes from the window: the window holds
//! only a token for it. A preview reads the first lines here; the import itself streams the file to Postgres's CSV reader.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use specta::Type;
use hn_core::{ByteSource, Error, ErrorKind};

use crate::files::storage;

/// Enough of the file to show its first rows.
const PREVIEW_BYTES: u64 = 64 * 1024;
const PREVIEW_ROWS: usize = 5;
const PIECE: usize = 256 * 1024;

#[derive(Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub token: String,
    pub file: String,
    /// The names on the file's first line.
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Clone)]
pub struct Chosen {
    pub path: PathBuf,
    pub delimiter: u8,
}

#[derive(Default)]
pub struct Imports {
    chosen: Mutex<HashMap<String, Chosen>>,
}

impl Imports {
    pub fn preview(&self, path: PathBuf) -> Result<ImportPreview, Error> {
        let mut head = Vec::new();
        File::open(&path)
            .and_then(|f| f.take(PREVIEW_BYTES).read_to_end(&mut head))
            .map_err(|e| storage("Couldn't read the file", e))?;
        let text = match std::str::from_utf8(&head) {
            Ok(text) => text,
            // Only the last character was cut by the preview's limit, which is fine.
            Err(e) if e.error_len().is_none() => std::str::from_utf8(&head[..e.valid_up_to()]).unwrap_or_default(),
            Err(_) => {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    "Halfnine reads CSV files saved as UTF-8. Save it as UTF-8, then import it again.",
                ));
            }
        };
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let delimiter = delimiter(text.lines().next().unwrap_or_default());
        let mut reader = csv::ReaderBuilder::new().delimiter(delimiter).flexible(true).from_reader(text.as_bytes());
        let columns: Vec<String> = reader
            .headers()
            .map_err(|e| Error::new(ErrorKind::Invalid, format!("The file's first line couldn't be read as CSV: {e}.")))?
            .iter()
            .map(str::to_string)
            .collect();
        if columns.iter().all(String::is_empty) {
            return Err(Error::new(
                ErrorKind::Invalid,
                "The file is empty. Its first line should name its columns.",
            ));
        }
        let rows = reader
            .records()
            .take(PREVIEW_ROWS)
            .filter_map(Result::ok)
            .map(|record| record.iter().map(str::to_string).collect())
            .collect();
        let token = uuid::Uuid::new_v4().to_string();
        let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        self.chosen.lock().map_err(|_| busy())?.insert(token.clone(), Chosen { path, delimiter });
        Ok(ImportPreview { token, file, columns, rows })
    }

    pub fn get(&self, token: &str) -> Result<Chosen, Error> {
        self.chosen
            .lock()
            .map_err(|_| busy())?
            .get(token)
            .cloned()
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "Choose the file again."))
    }

    pub fn forget(&self, token: &str) {
        if let Ok(mut chosen) = self.chosen.lock() {
            chosen.remove(token);
        }
    }
}

fn busy() -> Error {
    Error::new(ErrorKind::Storage, "The list of chosen files couldn't be read. Restart Halfnine.")
}

/// The separator the first line uses most: a comma, a semicolon (common where the comma is a decimal point), a tab or a bar.
fn delimiter(first_line: &str) -> u8 {
    b",;\t|"
        .iter()
        .copied()
        .max_by_key(|d| first_line.bytes().filter(|b| b == d).count())
        .filter(|d| first_line.as_bytes().contains(d))
        .unwrap_or(b',')
}

/// The file, read a piece at a time.
pub struct FileSource(File);

impl FileSource {
    pub fn open(path: &Path) -> Result<Self, Error> {
        File::open(path).map(Self).map_err(|e| storage("Couldn't open the file", e))
    }
}

impl ByteSource for FileSource {
    fn next(&mut self) -> Result<Option<Vec<u8>>, Error> {
        let mut piece = vec![0; PIECE];
        let read = self.0.read(&mut piece).map_err(|e| storage("Couldn't read the file", e))?;
        piece.truncate(read);
        Ok((read > 0).then_some(piece))
    }
}

#[cfg(test)]
mod tests {
    use super::delimiter;

    #[test]
    fn picks_the_separator_the_header_uses() {
        assert_eq!(delimiter("id,name,email"), b',');
        assert_eq!(delimiter("id;amount;note, with comma"), b';');
        assert_eq!(delimiter("id\tname"), b'\t');
        assert_eq!(delimiter("single"), b',');
    }
}
