use std::future::Future;

use crate::data::{Activity, Cell, ExportSource, Filter, Page, PageColumn, PageRequest, RunOutcome, SchemaNode, ServerInfo, Structure, TableRef};
use crate::error::Error;

/// What the app needs from a database. Each adapter (Postgres today) implements it; the app depends only on this.
pub trait Database: Send + Sync {
    fn server(&self) -> impl Future<Output = Result<ServerInfo, Error>> + Send;
    fn schema(&self) -> impl Future<Output = Result<Vec<SchemaNode>, Error>> + Send;
    fn structure(&self, table: &TableRef) -> impl Future<Output = Result<Structure, Error>> + Send;
    /// A page of a table. Browsing only ever reads: it runs in a read-only transaction on every connection.
    fn page(&self, request: &PageRequest) -> impl Future<Output = Result<Page, Error>> + Send;
    /// The exact number of rows, which can be slow on big tables, so only on request.
    fn count(&self, table: &TableRef, filter: Option<&Filter>) -> impl Future<Output = Result<f64, Error>> + Send;

    /// Runs the editor's statements one by one on its own connection, which keeps its session between runs.
    /// Each must be exactly one statement, as the database itself parses it.
    fn run(&self, statements: &[String]) -> impl Future<Output = Result<RunOutcome, Error>> + Send;
    /// Stops whatever the editor or an explain is running now. `false` when nothing was running.
    fn cancel(&self) -> impl Future<Output = Result<bool, Error>> + Send;
    /// The plan for one statement as the database's own JSON. With `analyze`, it runs the statement and then rolls it back.
    fn explain(&self, statement: &str, analyze: bool) -> impl Future<Output = Result<String, Error>> + Send;
    /// Streams every row of `source` into `sink` from a read-only transaction, and returns how many there were.
    fn export(&self, source: &ExportSource, sink: &mut dyn RowSink) -> impl Future<Output = Result<f64, Error>> + Send;
    fn activity(&self) -> impl Future<Output = Result<Vec<Activity>, Error>> + Send;
    /// Cancels another session's statement, or ends the session when `terminate`. `false` when it no longer existed.
    fn stop(&self, pid: i32, terminate: bool) -> impl Future<Output = Result<bool, Error>> + Send;
}

/// Where an export's rows go, one at a time, so a table of any size streams without being held in memory.
pub trait RowSink: Send {
    fn columns(&mut self, columns: &[PageColumn]) -> Result<(), Error>;
    fn row(&mut self, row: &[Cell]) -> Result<(), Error>;
}
