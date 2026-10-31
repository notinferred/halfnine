use std::future::Future;

use crate::backup::{RestoreInto, TableDefinition, TableSink};
use crate::changes::{Changes, Choice, Committed};
use crate::data::{
    Activity, Cell, ColumnRef, ExportSource, Filter, Page, PageColumn, PageRequest, Reference, RunOutcome, SchemaNode, ServerInfo, StatementResult,
    Structure, TableRef,
};
use crate::error::Error;
use crate::rules::{Checks, Rule, RuleTried};

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

    /// The statements `changes` would run, written out for someone to read before committing them.
    fn review(&self, changes: &Changes) -> impl Future<Output = Result<Vec<String>, Error>> + Send;
    /// Runs `changes` in one transaction. A row that changed after it was loaded rolls back everything, as does any error,
    /// and so does breaking one of the rules in `checks`, which are checked just before it commits.
    fn commit(&self, changes: &Changes, checks: &Checks) -> impl Future<Output = Result<Committed, Error>> + Send;
    /// Up to 50 values of `column` in `table` that match `search`, to pick one for a foreign key.
    fn choices(&self, table: &TableRef, column: &str, search: &str) -> impl Future<Output = Result<Vec<Choice>, Error>> + Send;
    /// Adds every row of a CSV file to `table` in one transaction, and returns how many there were. The file's first
    /// line names its columns; `columns` gives the table column each file column goes into, or `None` to skip it.
    fn import(
        &self,
        table: &TableRef,
        columns: &[Option<String>],
        delimiter: u8,
        source: &mut dyn ByteSource,
    ) -> impl Future<Output = Result<f64, Error>> + Send;

    /// One statement an agent sent, in a read-only transaction with time limits, keeping at most `limit` rows. When more
    /// rows exist, `total_rows` is one more than the rows kept. A `hidden` column's values come back as `[hidden]`, and
    /// a statement that filters on one or computes with it is refused.
    fn read(&self, statement: &str, limit: u32, hidden: &[ColumnRef]) -> impl Future<Output = Result<StatementResult, Error>> + Send;
    /// Statements an agent sent, each exactly one, in one transaction on a connection of their own that is closed
    /// afterwards, so nothing they set outlasts them. Any error rolls back all of them, and so does a statement that
    /// reads a `hidden` column, or breaks one of the rules in `checks`.
    fn execute(
        &self,
        statements: &[String],
        hidden: &[ColumnRef],
        checks: &Checks,
    ) -> impl Future<Output = Result<Vec<StatementResult>, Error>> + Send;
    /// Checks a rule against the data as it is, read-only, to tell the person what already breaks it and how long it
    /// takes before they add it. A custom rule must be one statement that reads; its plan names the tables it reads.
    fn try_rule(&self, rule: &Rule) -> impl Future<Output = Result<RuleTried, Error>> + Send;
    /// Every foreign key between the database's own tables.
    fn references(&self) -> impl Future<Output = Result<Vec<Reference>, Error>> + Send;

    /// How big a table is, in bytes, so a backup that would take long can be skipped. Refuses, saying why, when a
    /// backup couldn't be put back as `into` would: a partitioned table, or columns without a primary key to match by.
    fn backup_size(&self, table: &TableRef, into: &RestoreInto) -> impl Future<Output = Result<f64, Error>> + Send;
    /// Commits `changes` after backing `table` up into `sink` in the same transaction, so the rows backed up are the
    /// rows changed, and a backup that fails stops the commit.
    fn commit_with_backup(
        &self,
        changes: &Changes,
        table: &TableRef,
        sink: &mut dyn TableSink,
        checks: &Checks,
    ) -> impl Future<Output = Result<Committed, Error>> + Send;
    /// The statements a restore runs, for someone to read first.
    fn review_restore(&self, definition: &TableDefinition, into: &RestoreInto) -> impl Future<Output = Result<Vec<String>, Error>> + Send;
    /// Puts a backup back in one transaction, its rows streamed from `rows` as the backup wrote them.
    fn restore(&self, definition: &TableDefinition, into: &RestoreInto, rows: &mut dyn ByteSource)
    -> impl Future<Output = Result<f64, Error>> + Send;
}

/// Where an import's bytes come from, a piece at a time, so a file of any size streams without being held in memory.
pub trait ByteSource: Send {
    /// The next piece, or `None` at the end.
    fn next(&mut self) -> Result<Option<Vec<u8>>, Error>;
}

/// Where an export's rows go, one at a time, so a table of any size streams without being held in memory.
pub trait RowSink: Send {
    fn columns(&mut self, columns: &[PageColumn]) -> Result<(), Error>;
    fn row(&mut self, row: &[Cell]) -> Result<(), Error>;
}
