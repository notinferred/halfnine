//! What the SQL editor runs: statements exactly as written, each first described by Postgres to prove it is one statement,
//! then run in the simple protocol so every value comes back as Postgres's own text.

use std::sync::Mutex;
use std::time::Instant;

use futures_util::TryStreamExt;
use hn_core::{Activity, Cell, CutCell, Error, ErrorKind, PageColumn, RunFailure, RunOutcome, StatementResult};
use sqlx::postgres::{PgConnectOptions, PgRow};
use sqlx::{AssertSqlSafe, Column, Connection, Either, Executor, PgConnection, Row, SqlSafeStr, Statement, TypeInfo, ValueRef};

use crate::CONNECT_TIMEOUT;
use crate::errors::from_sqlx;
use crate::sql::cut;

/// The most rows of one result the editor keeps. The rest are counted, and an export reads them all.
pub(crate) const RESULT_LIMIT: usize = 10_000;

/// The editor's own connection, apart from the browsing pool, so `SET`, `BEGIN` and temporary tables last between runs.
pub(crate) struct EditorConnection {
    pub conn: PgConnection,
    /// The server process behind it, which Cancel signals.
    pub pid: i32,
}

pub(crate) async fn open(options: &PgConnectOptions, read_only: bool) -> Result<EditorConnection, Error> {
    let mut conn = tokio::time::timeout(CONNECT_TIMEOUT, PgConnection::connect_with(options))
        .await
        .map_err(|_| Error::new(ErrorKind::Timeout, "The server didn't answer within 10 seconds."))?
        .map_err(from_sqlx)?;
    // The editor waits as long as a statement takes; Cancel is how it stops one.
    sqlx::raw_sql("SET statement_timeout = 0").execute(&mut conn).await.map_err(from_sqlx)?;
    if read_only {
        sqlx::raw_sql("SET default_transaction_read_only = on")
            .execute(&mut conn)
            .await
            .map_err(from_sqlx)?;
    }
    let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut conn)
        .await
        .map_err(from_sqlx)?;
    Ok(EditorConnection { conn, pid })
}

/// Runs each statement in turn and stops at the first error, as `psql -v ON_ERROR_STOP=1` does.
pub(crate) async fn run(conn: &mut PgConnection, statements: &[String], read_only: bool) -> RunOutcome {
    let started = Instant::now();
    let mut results = Vec::new();
    let mut failed = None;
    for (index, sql) in statements.iter().enumerate() {
        match run_one(conn, sql, read_only).await {
            Ok(result) => results.push(result),
            Err(error) => {
                failed = Some(RunFailure {
                    statement: index as u32,
                    error,
                });
                break;
            }
        }
    }
    RunOutcome {
        results,
        failed,
        in_transaction: in_transaction(conn).await,
        elapsed_ms: hn_core::millis(started.elapsed()),
    }
}

/// Whether a transaction block is still open. sqlx counts only the transactions it opened itself, so Postgres is asked.
/// Inside a block, `now()` is when the block began, so it differs from this statement's start. Outside one, a
/// single simple-protocol message starts both at once. A failed block refuses any statement with 25P02.
async fn in_transaction(conn: &mut PgConnection) -> bool {
    match sqlx::raw_sql("SELECT now() <> statement_timestamp()").fetch_one(&mut *conn).await {
        Ok(row) => text_values(&row).ok().and_then(|v| v.into_iter().next().flatten()).as_deref() == Some("t"),
        Err(sqlx::Error::Database(db)) => db.code().as_deref() == Some("25P02"),
        Err(_) => false,
    }
}

async fn run_one(conn: &mut PgConnection, sql: &str, read_only: bool) -> Result<StatementResult, Error> {
    let columns = describe(conn, sql).await?;
    if read_only {
        begin_read_only(conn).await?;
    }
    let started = Instant::now();
    let collected = collect(conn, sql).await;
    if read_only {
        // After a statement that ended the transaction itself, this only draws a warning from Postgres.
        let end = if collected.is_ok() { "COMMIT" } else { "ROLLBACK" };
        sqlx::raw_sql(end).execute(&mut *conn).await.map_err(from_sqlx)?;
    }
    let (rows, cut, total_rows, rows_affected) = collected?;
    Ok(StatementResult {
        columns,
        rows,
        cut,
        total_rows,
        rows_affected,
        elapsed_ms: hn_core::millis(started.elapsed()),
    })
}

/// Describes `sql`, then runs it, on a connection already inside the transaction it belongs to.
pub(crate) async fn run_in_transaction(conn: &mut PgConnection, sql: &str) -> Result<StatementResult, Error> {
    let columns = describe(conn, sql).await?;
    let started = Instant::now();
    let (rows, cut, total_rows, rows_affected) = collect(conn, sql).await?;
    Ok(StatementResult {
        columns,
        rows,
        cut,
        total_rows,
        rows_affected,
        elapsed_ms: hn_core::millis(started.elapsed()),
    })
}

/// The statement's first keyword in capitals, after any spaces, comments and opening parentheses, such as `SELECT`.
pub fn first_word(sql: &str) -> String {
    let mut rest = sql;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '(');
        if let Some(line) = rest.strip_prefix("--") {
            rest = line.split_once('\n').map_or("", |(_, after)| after);
        } else if rest.starts_with("/*") {
            rest = after_block_comment(rest);
        } else {
            break;
        }
    }
    rest.chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_uppercase()
}

/// What follows the block comment `sql` starts with. Postgres nests them, so `/* a /* b */ c */` is one comment.
fn after_block_comment(sql: &str) -> &str {
    let mut depth = 0;
    let mut at = 0;
    while at < sql.len() {
        let rest = &sql[at..];
        if rest.starts_with("/*") {
            depth += 1;
            at += 2;
        } else if rest.starts_with("*/") {
            depth -= 1;
            at += 2;
            if depth == 0 {
                return &sql[at..];
            }
        } else {
            at += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    ""
}

/// A read-only transaction that has already taken its first snapshot. From then on Postgres refuses to make the
/// transaction read-write, so nothing a statement does inside it can lift the protection.
pub(crate) async fn begin_read_only(conn: &mut PgConnection) -> Result<(), Error> {
    sqlx::raw_sql("BEGIN TRANSACTION READ ONLY")
        .execute(&mut *conn)
        .await
        .map_err(from_sqlx)?;
    sqlx::raw_sql("SELECT 1").execute(&mut *conn).await.map_err(from_sqlx)?;
    Ok(())
}

/// Describes `sql` without running it. Postgres refuses to describe more than one statement at a time, which is how a
/// piece of the editor is known to be exactly one statement before it runs.
pub(crate) async fn describe(conn: &mut PgConnection, sql: &str) -> Result<Vec<PageColumn>, Error> {
    Ok(describe_origins(conn, sql).await?.into_iter().map(|(column, _)| column).collect())
}

/// Where a result column comes from, as Postgres reports it: the table's oid and the column's number, when the column
/// is a table column returned as it is.
pub(crate) type Origin = Option<(u32, i16)>;

/// As [`describe`], with each column's origin.
pub(crate) async fn describe_origins(conn: &mut PgConnection, sql: &str) -> Result<Vec<(PageColumn, Origin)>, Error> {
    let statement = (&mut *conn).prepare(AssertSqlSafe(sql.to_string()).into_sql_str()).await.map_err(|e| {
        let error = from_sqlx(e);
        if error.message.contains("multiple commands") {
            Error::new(
                ErrorKind::Invalid,
                "This holds more than one statement. Put each on its own, ending with a semicolon.",
            )
        } else {
            error
        }
    })?;
    let columns = statement
        .columns()
        .iter()
        .map(|c| {
            let column = PageColumn {
                name: c.name().to_string(),
                type_name: type_name(c.type_info().name()),
            };
            let origin = c.relation_id().zip(c.relation_attribute_no()).map(|(table, number)| (table.0, number));
            (column, origin)
        })
        .collect();
    // Forgotten at once, so a statement that runs again after an ALTER TABLE is described afresh.
    conn.clear_cached_statements().await.map_err(from_sqlx)?;
    Ok(columns)
}

/// Rows kept, cells cut, rows returned and rows written.
pub(crate) type Collected = (Vec<Vec<Cell>>, Vec<CutCell>, u32, u32);

pub(crate) async fn collect(conn: &mut PgConnection, sql: &str) -> Result<Collected, Error> {
    let mut stream = sqlx::raw_sql(AssertSqlSafe(sql.to_string())).fetch_many(&mut *conn);
    let mut rows = Vec::new();
    let mut cuts = Vec::new();
    let mut total = 0u64;
    let mut affected = 0u64;
    while let Some(item) = stream.try_next().await.map_err(from_sqlx)? {
        match item {
            Either::Left(done) => affected += done.rows_affected(),
            Either::Right(row) => {
                if rows.len() < RESULT_LIMIT {
                    let index = rows.len() as u32;
                    let mut values = text_values(&row)?;
                    for (column, value) in values.iter_mut().enumerate() {
                        if let Some(text) = value
                            && cut(text)
                        {
                            cuts.push(CutCell {
                                row: index,
                                column: column as u32,
                            });
                        }
                    }
                    rows.push(values);
                }
                total += 1;
            }
        }
    }
    let count = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
    Ok((rows, cuts, count(total), count(affected)))
}

/// A row from the simple protocol, whose values are all Postgres's own text.
pub(crate) fn text_values(row: &PgRow) -> Result<Vec<Cell>, Error> {
    (0..row.len())
        .map(|i| {
            let raw = row.try_get_raw(i).map_err(from_sqlx)?;
            if raw.is_null() {
                return Ok(None);
            }
            let text = raw
                .as_str()
                .map_err(|e| Error::new(ErrorKind::Query, format!("A value wasn't text: {e}.")))?;
            Ok(Some(text.to_string()))
        })
        .collect()
}

/// sqlx names built-in types in capitals (`INT8`); Postgres and the rest of the app write them in lower case.
fn type_name(name: &str) -> String {
    if name.chars().any(char::is_lowercase) {
        name.to_string()
    } else {
        name.to_lowercase()
    }
}

/// The server processes running something for this connection now, so Cancel knows which to signal.
#[derive(Default)]
pub(crate) struct Running(Mutex<Vec<i32>>);

impl Running {
    pub(crate) fn enter(&self, pid: i32) -> RunningGuard<'_> {
        self.lock().push(pid);
        RunningGuard { running: self, pid }
    }

    pub(crate) fn pids(&self) -> Vec<i32> {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<i32>> {
        self.0.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) struct RunningGuard<'a> {
    running: &'a Running,
    pid: i32,
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        self.running.lock().retain(|p| *p != self.pid);
    }
}

pub(crate) const ACTIVITY: &str = "SELECT pid, usename::text, datname::text, application_name, client_addr::text, state, \
     wait_event_type || ': ' || wait_event, (extract(epoch FROM clock_timestamp() - query_start) * 1000)::float8, left(query, 2000) \
     FROM pg_stat_activity WHERE backend_type = 'client backend' ORDER BY query_start DESC NULLS LAST";

pub(crate) fn activity(row: &PgRow) -> Result<Activity, sqlx::Error> {
    Ok(Activity {
        pid: row.try_get(0)?,
        user: row.try_get(1)?,
        database: row.try_get(2)?,
        application: row.try_get(3)?,
        client: row.try_get(4)?,
        state: row.try_get(5)?,
        waiting_on: row.try_get(6)?,
        duration_ms: row.try_get(7)?,
        query: row.try_get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::first_word;

    #[test]
    fn finds_the_first_keyword_past_comments_and_parentheses() {
        assert_eq!(first_word("select 1"), "SELECT");
        assert_eq!(
            first_word("  -- why\n /* a /* nested */ comment */ (WITH x AS (SELECT 1) TABLE x)"),
            "WITH"
        );
        assert_eq!(first_word("/* unclosed"), "");
        assert_eq!(first_word("drop_me()"), "DROP_ME");
        assert_eq!(first_word("TRUNCATE ünïcödé"), "TRUNCATE");
    }
}
