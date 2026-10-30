//! Halfnine's Postgres adapter: it implements [`hn_core::Database`] over a small sqlx pool, optionally through an SSH tunnel.

pub mod backup;
mod catalog;
mod changes;
mod editor;
mod errors;
mod hidden;
mod sql;
mod tunnel;

use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use hn_core::{
    Activity, ByteSource, Changes, Choice, ColumnRef, ColumnValue, Committed, ConnectionSpec, CutCell, Database, Error, ErrorKind, ExportSource,
    Filter, Page, PageColumn, PageRequest, Reference, RelationKind, RestoreInto, RowSink, RunOutcome, SchemaNode, ServerInfo, StatementResult,
    Structure, StructureChange, TableDefinition, TableRef, TableSink, TlsMode,
};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgSslMode};
use sqlx::{AssertSqlSafe, PgConnection, Postgres, Row, Transaction};

use crate::changes::Step;
use crate::editor::{EditorConnection, Running};
use crate::errors::from_sqlx;
use crate::sql::{Built, ColumnMeta};
use crate::tunnel::Tunnel;

pub use crate::editor::first_word;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How an agent's query may begin. Anything else is refused before it reaches Postgres, which runs these read-only.
const AGENT_READS: [&str; 6] = ["SELECT", "WITH", "VALUES", "TABLE", "SHOW", "EXPLAIN"];
/// Statements an agent's change may not hold: it runs as one transaction that Halfnine begins and ends, and COPY needs a
/// protocol of its own.
pub const NOT_IN_A_CHANGE: [&str; 10] = [
    "BEGIN",
    "START",
    "COMMIT",
    "END",
    "ROLLBACK",
    "ABORT",
    "SAVEPOINT",
    "RELEASE",
    "PREPARE",
    "COPY",
];

/// The secrets a connection needs: read from the keychain, or typed, just before connecting.
#[derive(Clone, Default)]
pub struct Secrets {
    pub password: Option<String>,
    pub ssh: Option<String>,
}

pub struct PgDatabase {
    pool: PgPool,
    /// Kept to open the editor's own connection, which isn't part of the pool.
    options: PgConnectOptions,
    read_only: bool,
    editor: tokio::sync::Mutex<Option<EditorConnection>>,
    running: Running,
    // Held so the tunnel lives as long as the connections that use it.
    _tunnel: Option<Tunnel>,
}

impl PgDatabase {
    /// Connects to `database`, or to the connection's own database when `None`, and checks the credentials straight away.
    pub async fn connect(spec: &ConnectionSpec, database: Option<&str>, secrets: &Secrets) -> Result<Self, Error> {
        let tunnel = match &spec.ssh {
            Some(ssh) => Some(tunnel::open(ssh, &spec.host, spec.port, secrets.ssh.as_deref()).await?),
            None => None,
        };
        let (host, port) = match &tunnel {
            Some(t) => ("127.0.0.1", t.port),
            None => (spec.host.as_str(), spec.port),
        };
        let ssl_mode = match spec.tls {
            TlsMode::Disable => PgSslMode::Disable,
            TlsMode::Prefer => PgSslMode::Prefer,
            TlsMode::Require => PgSslMode::Require,
            // Through a tunnel the address is 127.0.0.1, which no certificate names, so the chain is still verified but not the host name.
            TlsMode::VerifyFull if tunnel.is_some() => PgSslMode::VerifyCa,
            TlsMode::VerifyFull => PgSslMode::VerifyFull,
        };
        let mut options = PgConnectOptions::new_without_pgpass()
            .host(host)
            .port(port)
            .username(&spec.user)
            .database(database.unwrap_or(&spec.database))
            .ssl_mode(ssl_mode)
            .application_name("Halfnine");
        if let Some(password) = &secrets.password {
            options = options.password(password);
        }

        let read_only = spec.read_only;
        let connecting = PgPoolOptions::new()
            .max_connections(3)
            .acquire_timeout(CONNECT_TIMEOUT)
            .idle_timeout(Duration::from_secs(300))
            .after_connect(move |conn, _| {
                Box::pin(async move {
                    sqlx::query("SET statement_timeout = '30s'").execute(&mut *conn).await?;
                    if read_only {
                        sqlx::query("SET default_transaction_read_only = on").execute(&mut *conn).await?;
                    }
                    Ok(())
                })
            })
            .connect_with(options.clone());
        let pool = tokio::time::timeout(CONNECT_TIMEOUT, connecting)
            .await
            .map_err(|_| Error::new(ErrorKind::Timeout, format!("{} didn't answer within 10 seconds.", spec.host)))?
            .map_err(from_sqlx)?;
        Ok(Self {
            pool,
            options,
            read_only,
            editor: tokio::sync::Mutex::new(None),
            running: Running::default(),
            _tunnel: tunnel,
        })
    }

    pub async fn close(&self) {
        if let Some(editor) = self.editor.lock().await.take() {
            let _ = sqlx::Connection::close(editor.conn).await;
        }
        self.pool.close().await;
    }

    /// A read-only transaction from the pool, for anything that only reads. Postgres, not the app, refuses any write in it.
    async fn reading(&self) -> Result<Transaction<'static, Postgres>, Error> {
        self.pool.begin_with("BEGIN TRANSACTION READ ONLY").await.map_err(from_sqlx)
    }

    /// A transaction for work that may take as long as it needs and can be cancelled: an explain, an export, a commit or
    /// an import. With `read_only`, or on a read-only connection, Postgres refuses any write in it.
    async fn long_transaction(&self, read_only: bool) -> Result<(Transaction<'static, Postgres>, i32), Error> {
        let begin = if read_only { "BEGIN TRANSACTION READ ONLY" } else { "BEGIN" };
        let mut tx = self.pool.begin_with(begin).await.map_err(from_sqlx)?;
        sqlx::raw_sql("SET LOCAL statement_timeout = 0")
            .execute(&mut *tx)
            .await
            .map_err(from_sqlx)?;
        // The first snapshot, after which Postgres won't make a read-only transaction read-write.
        let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .map_err(from_sqlx)?;
        Ok((tx, pid))
    }

    async fn columns(&self, table: &TableRef) -> Result<Vec<ColumnMeta>, Error> {
        let rows = sqlx::query(catalog::PAGE_COLUMNS)
            .bind(&table.schema)
            .bind(&table.name)
            .fetch_all(&self.pool)
            .await
            .map_err(from_sqlx)?;
        if rows.is_empty() {
            return Err(not_found(table));
        }
        rows.iter()
            .map(|r| {
                Ok(ColumnMeta {
                    name: r.try_get(0)?,
                    type_name: r.try_get(1)?,
                    key_position: r.try_get(2)?,
                    generated: r.try_get(3)?,
                    identity: r.try_get(4)?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(from_sqlx)
    }

    async fn relation_kind(&self, table: &TableRef) -> Result<Option<RelationKind>, Error> {
        let found = sqlx::query(catalog::RELATION)
            .bind(&table.schema)
            .bind(&table.name)
            .fetch_optional(&self.pool)
            .await
            .map_err(from_sqlx)?;
        Ok(match found {
            Some(row) => catalog::relation_kind(&row.try_get::<String, _>(2).map_err(from_sqlx)?),
            None => None,
        })
    }

    async fn steps(&self, changes: &Changes) -> Result<Vec<Step>, Error> {
        match changes {
            Changes::Rows {
                table,
                deletes,
                updates,
                inserts,
                restores,
            } => changes::rows(table, deletes, updates, inserts, restores, &self.columns(table).await?),
            Changes::Structure { table, changes } => {
                let kind = if changes.iter().any(|c| matches!(c, StructureChange::Drop)) {
                    self.relation_kind(table).await?
                } else {
                    None
                };
                changes::structure(table, kind, changes)
            }
        }
    }

    /// Runs a commit's statements in order, returning the keys of the rows it added. The first that fails, or that
    /// changes other than the one row it must, stops the rest.
    async fn run_steps(&self, tx: &mut PgConnection, steps: Vec<Step>) -> Result<Vec<Vec<ColumnValue>>, Error> {
        let mut inserted = Vec::new();
        for step in steps {
            // Each statement is written once, so it isn't kept as a prepared statement.
            let done = if step.returns_key {
                match self.query(step.run).persistent(false).fetch_one(&mut *tx).await {
                    Ok(row) => {
                        inserted.push(returned_key(&row)?);
                        Ok(1)
                    }
                    Err(error) => Err(error),
                }
            } else {
                self.query(step.run)
                    .persistent(false)
                    .execute(&mut *tx)
                    .await
                    .map(|done| done.rows_affected())
            };
            match done {
                Ok(affected) => match step.conflict {
                    Some(conflict) if affected != 1 => return Err(Error::new(ErrorKind::Conflict, conflict)),
                    _ => {}
                },
                Err(error) => return Err(failed(&step.what, from_sqlx(error))),
            }
        }
        Ok(inserted)
    }

    /// Writes the table and its rows to `sink` under a lock that lets reads go on and makes writes wait, so the backup
    /// holds exactly the rows the change that follows drops or empties.
    async fn back_up(&self, tx: &mut PgConnection, table: &TableRef, sink: &mut dyn TableSink) -> Result<(), Error> {
        sqlx::raw_sql("SET LOCAL lock_timeout = '10s'")
            .execute(&mut *tx)
            .await
            .map_err(from_sqlx)?;
        let lock = Built {
            sql: format!("LOCK TABLE {} IN SHARE MODE", sql::table(table)),
            params: Vec::new(),
        };
        self.query(lock).persistent(false).execute(&mut *tx).await.map_err(from_sqlx)?;
        let definition = backup::definition(tx, table).await?;
        sink.definition(&definition)?;
        let source = ExportSource::Table {
            table: table.clone(),
            sort: None,
            filter: None,
        };
        export_rows(tx, &source, Some(backup::columns(&definition)), sink, self).await?;
        sink.finish()
    }

    /// The statements that put a backup back, after checking the table can take it: a dropped table isn't there again,
    /// an emptied one is still empty, and a dropped column hasn't come back.
    async fn restore_plan(&self, conn: &mut PgConnection, definition: &TableDefinition, into: &RestoreInto) -> Result<backup::Plan, Error> {
        let table = &definition.table;
        let found = sqlx::query(catalog::RELATION)
            .bind(&table.schema)
            .bind(&table.name)
            .fetch_optional(&mut *conn)
            .await
            .map_err(from_sqlx)?;
        let present = match (into, found) {
            (RestoreInto::Recreate, None) => backup::Present::default(),
            (RestoreInto::Recreate, Some(_)) => {
                return Err(Error::new(
                    ErrorKind::Invalid,
                    format!("There's a {} again. Rename or drop it, then restore.", table.name),
                ));
            }
            (_, None) => return Err(not_found(table)),
            (_, Some(row)) => backup::present(conn, row.try_get(0).map_err(from_sqlx)?, definition, into, self).await?,
        };
        backup::plan(definition, into, &present)
    }

    async fn put_back(
        &self,
        tx: &mut PgConnection,
        definition: &TableDefinition,
        into: &RestoreInto,
        rows: &mut dyn ByteSource,
    ) -> Result<u64, Error> {
        let plan = self.restore_plan(tx, definition, into).await?;
        self.run_steps(tx, plan.before).await?;
        let restored = match plan.rows {
            Some(step) => {
                let import = sql::Import {
                    create: plan.stage,
                    copy: plan.copy,
                    insert: step.run.sql,
                };
                import_rows(tx, &import, rows, self).await.map_err(|error| failed(&step.what, error))?
            }
            None => 0,
        };
        self.run_steps(tx, plan.after).await?;
        Ok(restored)
    }

    fn query(&self, built: Built) -> sqlx::query::Query<'static, sqlx::Postgres, sqlx::postgres::PgArguments> {
        // Every statement the app builds passes through here. Its identifiers came through `sql::ident` and its values are
        // bound parameters. The only text taken as SQL is what someone typed for their own table: a WHERE condition, or a
        // column's type or default. Postgres runs each as part of one prepared statement, which refuses a second statement.
        let mut query = sqlx::query(AssertSqlSafe(built.sql));
        for param in built.params {
            query = query.bind(param);
        }
        query
    }
}

impl Database for PgDatabase {
    async fn server(&self) -> Result<ServerInfo, Error> {
        let row = sqlx::query(catalog::SERVER).fetch_one(&self.pool).await.map_err(from_sqlx)?;
        (|| {
            Ok(ServerInfo {
                version: row.try_get(0)?,
                database: row.try_get(1)?,
                user: row.try_get(2)?,
                databases: row.try_get(3)?,
            })
        })()
        .map_err(from_sqlx)
    }

    async fn schema(&self) -> Result<Vec<SchemaNode>, Error> {
        let (schemas, relations, functions, enums) = tokio::try_join!(
            sqlx::query(catalog::SCHEMAS).fetch_all(&self.pool),
            sqlx::query(catalog::RELATIONS).fetch_all(&self.pool),
            sqlx::query(catalog::FUNCTIONS).fetch_all(&self.pool),
            sqlx::query(catalog::ENUMS).fetch_all(&self.pool),
        )
        .map_err(from_sqlx)?;
        catalog::schema_tree(&schemas, &relations, &functions, &enums).map_err(from_sqlx)
    }

    async fn structure(&self, table: &TableRef) -> Result<Structure, Error> {
        let found = sqlx::query(catalog::RELATION)
            .bind(&table.schema)
            .bind(&table.name)
            .fetch_optional(&self.pool)
            .await
            .map_err(from_sqlx)?
            .ok_or_else(|| not_found(table))?;
        let oid: i64 = found.try_get(0).map_err(from_sqlx)?;
        let (columns, key, indexes, constraints, outgoing, incoming) = tokio::try_join!(
            sqlx::query(catalog::COLUMNS).bind(oid).fetch_all(&self.pool),
            sqlx::query(catalog::PRIMARY_KEY).bind(oid).fetch_optional(&self.pool),
            sqlx::query(catalog::INDEXES).bind(oid).fetch_all(&self.pool),
            sqlx::query(catalog::CONSTRAINTS).bind(oid).fetch_all(&self.pool),
            sqlx::query(catalog::FOREIGN_KEYS).bind(oid).fetch_all(&self.pool),
            sqlx::query(catalog::REFERENCED_BY).bind(oid).fetch_all(&self.pool),
        )
        .map_err(from_sqlx)?;
        catalog::structure(&found, &columns, key.as_ref(), &indexes, &constraints, &outgoing, &incoming).map_err(from_sqlx)
    }

    async fn page(&self, request: &PageRequest) -> Result<Page, Error> {
        let columns = self.columns(&request.table).await?;
        let built = sql::page(request, &columns)?;
        let mut tx = self.reading().await?;
        let started = Instant::now();
        let rows = self.query(built).fetch_all(&mut *tx).await.map_err(from_sqlx)?;
        let elapsed_ms = hn_core::millis(started.elapsed());
        let _ = tx.rollback().await;

        let limit = request.limit.clamp(1, sql::PAGE_LIMIT) as usize;
        let has_more = rows.len() > limit;
        let mut cut = Vec::new();
        let mut cells = Vec::with_capacity(rows.len().min(limit));
        for (r, row) in rows.iter().take(limit).enumerate() {
            let mut values = Vec::with_capacity(columns.len());
            for c in 0..columns.len() {
                let mut value: Option<String> = row.try_get(c).map_err(from_sqlx)?;
                if let Some(text) = &mut value
                    && sql::cut(text)
                {
                    cut.push(CutCell {
                        row: r as u32,
                        column: c as u32,
                    });
                }
                values.push(value);
            }
            cells.push(values);
        }
        Ok(Page {
            columns: columns
                .into_iter()
                .map(|c| PageColumn {
                    name: c.name,
                    type_name: c.type_name,
                })
                .collect(),
            rows: cells,
            cut,
            has_more,
            elapsed_ms,
        })
    }

    async fn count(&self, table: &TableRef, filter: Option<&Filter>) -> Result<f64, Error> {
        let columns = self.columns(table).await?;
        let built = sql::count(table, filter, &columns)?;
        let mut tx = self.reading().await?;
        let row = self.query(built).fetch_one(&mut *tx).await.map_err(from_sqlx)?;
        let _ = tx.rollback().await;
        row.try_get(0).map_err(from_sqlx)
    }

    async fn run(&self, statements: &[String]) -> Result<RunOutcome, Error> {
        let mut slot = self.editor.try_lock().map_err(|_| {
            Error::new(
                ErrorKind::Invalid,
                "A query is still running on this connection. Cancel it or wait for it to finish.",
            )
        })?;
        if slot.is_none() {
            *slot = Some(editor::open(&self.options, self.read_only).await?);
        }
        let Some(connection) = slot.as_mut() else {
            return Err(Error::new(ErrorKind::Network, "The editor's connection couldn't be opened."));
        };
        let outcome = {
            let _running = self.running.enter(connection.pid);
            editor::run(&mut connection.conn, statements, self.read_only).await
        };
        // A connection that broke is let go, and the next run opens a fresh one.
        if outcome.failed.as_ref().is_some_and(|f| f.error.kind == ErrorKind::Network) {
            *slot = None;
        }
        Ok(outcome)
    }

    async fn cancel(&self) -> Result<bool, Error> {
        let mut cancelled = false;
        for pid in self.running.pids() {
            let done: bool = sqlx::query_scalar("SELECT pg_cancel_backend($1)")
                .bind(pid)
                .fetch_one(&self.pool)
                .await
                .map_err(from_sqlx)?;
            cancelled |= done;
        }
        Ok(cancelled)
    }

    async fn explain(&self, statement: &str, analyze: bool) -> Result<String, Error> {
        let (mut tx, pid) = self.long_transaction(self.read_only).await?;
        let options = if analyze { "FORMAT JSON, ANALYZE, BUFFERS" } else { "FORMAT JSON" };
        let sql = format!("EXPLAIN ({options}) {}", statement.trim().trim_end_matches(';'));
        editor::describe(&mut tx, &sql).await?;
        let row = {
            let _running = self.running.enter(pid);
            sqlx::raw_sql(AssertSqlSafe(sql)).fetch_one(&mut *tx).await.map_err(from_sqlx)
        };
        // Only the plan is wanted. With ANALYZE the statement really ran, and this undoes what it changed.
        let _ = tx.rollback().await;
        editor::text_values(&row?)?
            .into_iter()
            .next()
            .flatten()
            .ok_or_else(|| Error::new(ErrorKind::Query, "Postgres returned no plan."))
    }

    async fn export(&self, source: &ExportSource, sink: &mut dyn RowSink) -> Result<f64, Error> {
        let columns = match source {
            ExportSource::Table { table, .. } => Some(self.columns(table).await?),
            ExportSource::Statement { .. } => None,
        };
        // An export only reads, whatever the connection allows, so it runs read-only everywhere.
        let (mut tx, pid) = self.long_transaction(true).await?;
        let _running = self.running.enter(pid);
        let written = export_rows(&mut tx, source, columns, sink, self).await;
        let _ = tx.rollback().await;
        written
    }

    async fn activity(&self) -> Result<Vec<Activity>, Error> {
        let rows = sqlx::query(editor::ACTIVITY).fetch_all(&self.pool).await.map_err(from_sqlx)?;
        rows.iter().map(editor::activity).collect::<Result<_, _>>().map_err(from_sqlx)
    }

    async fn stop(&self, pid: i32, terminate: bool) -> Result<bool, Error> {
        if self.read_only {
            return Err(Error::new(
                ErrorKind::ReadOnly,
                "This connection is read-only, so it doesn't stop other sessions.",
            ));
        }
        let sql = if terminate {
            "SELECT pg_terminate_backend($1)"
        } else {
            "SELECT pg_cancel_backend($1)"
        };
        sqlx::query_scalar(sql).bind(pid).fetch_one(&self.pool).await.map_err(from_sqlx)
    }

    async fn review(&self, changes: &Changes) -> Result<Vec<String>, Error> {
        Ok(self.steps(changes).await?.into_iter().map(|step| step.shown).collect())
    }

    async fn commit(&self, changes: &Changes) -> Result<Committed, Error> {
        let steps = self.steps(changes).await?;
        let statements = steps.len() as u32;
        let (mut tx, pid) = self.long_transaction(self.read_only).await?;
        let _running = self.running.enter(pid);
        let started = Instant::now();
        let done = self.run_steps(&mut tx, steps).await;
        settle(tx, done, statements, started).await
    }

    async fn backup_size(&self, table: &TableRef, into: &RestoreInto) -> Result<f64, Error> {
        let mut tx = self.reading().await?;
        let size = backup::size(&mut tx, table, into).await;
        let _ = tx.rollback().await;
        size
    }

    async fn commit_with_backup(&self, changes: &Changes, table: &TableRef, sink: &mut dyn TableSink) -> Result<Committed, Error> {
        let steps = self.steps(changes).await?;
        let statements = steps.len() as u32;
        let (mut tx, pid) = self.long_transaction(self.read_only).await?;
        let _running = self.running.enter(pid);
        let started = Instant::now();
        let done = match self.back_up(&mut tx, table, sink).await {
            Ok(()) => self.run_steps(&mut tx, steps).await,
            Err(error) => Err(Error {
                kind: ErrorKind::NoBackup,
                message: format!("The backup failed, so nothing changed: {}", error.message),
                ..error
            }),
        };
        settle(tx, done, statements, started).await
    }

    async fn review_restore(&self, definition: &TableDefinition, into: &RestoreInto) -> Result<Vec<String>, Error> {
        let mut tx = self.reading().await?;
        let plan = self.restore_plan(&mut tx, definition, into).await;
        let _ = tx.rollback().await;
        Ok(plan?.shown())
    }

    async fn restore(&self, definition: &TableDefinition, into: &RestoreInto, rows: &mut dyn ByteSource) -> Result<f64, Error> {
        let (mut tx, pid) = self.long_transaction(self.read_only).await?;
        let _running = self.running.enter(pid);
        match self.put_back(&mut tx, definition, into, rows).await {
            Ok(restored) => {
                tx.commit().await.map_err(from_sqlx)?;
                Ok(restored as f64)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(error)
            }
        }
    }

    async fn choices(&self, table: &TableRef, column: &str, search: &str) -> Result<Vec<Choice>, Error> {
        let built = sql::choices(table, column, search, &self.columns(table).await?)?;
        let mut tx = self.reading().await?;
        let rows = self.query(built).fetch_all(&mut *tx).await.map_err(from_sqlx)?;
        let _ = tx.rollback().await;
        rows.iter()
            .map(|r| {
                Ok(Choice {
                    value: r.try_get(0)?,
                    label: r.try_get(1)?,
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(from_sqlx)
    }

    async fn import(&self, table: &TableRef, columns: &[Option<String>], delimiter: u8, source: &mut dyn ByteSource) -> Result<f64, Error> {
        let import = sql::import(table, columns, delimiter, &self.columns(table).await?)?;
        let (mut tx, pid) = self.long_transaction(self.read_only).await?;
        let _running = self.running.enter(pid);
        match import_rows(&mut tx, &import, source, self).await {
            Ok(rows) => {
                tx.commit().await.map_err(from_sqlx)?;
                Ok(rows as f64)
            }
            Err(error) => {
                let _ = tx.rollback().await;
                Err(Error {
                    message: format!("The import failed, so nothing was written: {}", error.message),
                    ..error
                })
            }
        }
    }

    async fn read(&self, statement: &str, limit: u32, hidden: &[ColumnRef]) -> Result<StatementResult, Error> {
        let word = editor::first_word(statement);
        if !AGENT_READS.contains(&word.as_str()) {
            return Err(Error::new(
                ErrorKind::Invalid,
                "A query only reads, so it starts with SELECT, WITH, VALUES, TABLE, SHOW or EXPLAIN.",
            ));
        }
        let masked = self.hidden_columns(hidden).await?;
        let mut tx = self.reading().await?;
        sqlx::raw_sql("SET LOCAL statement_timeout = '15s'; SET LOCAL lock_timeout = '5s'")
            .execute(&mut *tx)
            .await
            .map_err(from_sqlx)?;
        // The first snapshot, after which Postgres won't make the transaction read-write.
        let pid = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *tx)
            .await
            .map_err(from_sqlx)?;
        let read = {
            let _running = self.running.enter(pid);
            read_rows(&mut tx, statement, limit, hidden, &masked).await
        };
        // Whatever the statement set, such as `set_config`, ends with the transaction.
        let _ = tx.rollback().await;
        let mut result = read?;
        let kept = limit as usize;
        if result.rows.len() > kept {
            result.rows.truncate(kept);
            result.cut.retain(|c| (c.row as usize) < kept);
        }
        Ok(result)
    }

    async fn execute(&self, statements: &[String], hidden: &[ColumnRef]) -> Result<Vec<StatementResult>, Error> {
        if let Some(word) = statements
            .iter()
            .map(|s| editor::first_word(s))
            .find(|w| NOT_IN_A_CHANGE.contains(&w.as_str()))
        {
            return Err(Error::new(
                ErrorKind::Invalid,
                format!("{word} isn't allowed here: Halfnine runs the statements as one transaction of its own."),
            ));
        }
        let mut connection = editor::open(&self.options, self.read_only).await?;
        let executed = {
            let _running = self.running.enter(connection.pid);
            self.execute_all(&mut connection.conn, statements, hidden).await
        };
        // Closed rather than returned to a pool, so nothing the statements set reaches anything else.
        let _ = sqlx::Connection::close(connection.conn).await;
        executed
    }

    async fn references(&self) -> Result<Vec<Reference>, Error> {
        let rows = sqlx::query(catalog::REFERENCES).fetch_all(&self.pool).await.map_err(from_sqlx)?;
        rows.iter().map(catalog::reference).collect::<Result<_, _>>().map_err(from_sqlx)
    }
}

impl PgDatabase {
    /// Begins the transaction an agent's statements run in: read-only when the connection is.
    async fn agent_transaction(&self, conn: &mut PgConnection) -> Result<(), Error> {
        if self.read_only {
            editor::begin_read_only(conn).await
        } else {
            sqlx::raw_sql("BEGIN").execute(&mut *conn).await.map(|_| ()).map_err(from_sqlx)
        }
    }

    async fn execute_all(&self, conn: &mut PgConnection, statements: &[String], hidden: &[ColumnRef]) -> Result<Vec<StatementResult>, Error> {
        self.agent_transaction(conn).await?;
        let mut results = Vec::with_capacity(statements.len());
        for (index, statement) in statements.iter().enumerate() {
            if !hidden.is_empty() {
                refuse_hidden_reads(conn, statement, hidden).await?;
            }
            let result = editor::run_in_transaction(conn, statement).await.map_err(|error| Error {
                message: format!("Statement {} failed, so nothing was written: {}", index + 1, error.message),
                ..error
            })?;
            results.push(result);
        }
        sqlx::raw_sql("COMMIT").execute(&mut *conn).await.map_err(from_sqlx)?;
        Ok(results)
    }
}

/// Refuses a statement whose plan reads a hidden column, which a write could copy somewhere an agent can see. Only some
/// statements have a plan; the attempt is undone for the others, which read no values.
async fn refuse_hidden_reads(conn: &mut PgConnection, statement: &str, hidden: &[ColumnRef]) -> Result<(), Error> {
    sqlx::raw_sql("SAVEPOINT hn_hidden").execute(&mut *conn).await.map_err(from_sqlx)?;
    let planned = plan(conn, statement).await;
    let undo = if planned.is_ok() {
        "RELEASE SAVEPOINT hn_hidden"
    } else {
        "ROLLBACK TO SAVEPOINT hn_hidden"
    };
    sqlx::raw_sql(undo).execute(&mut *conn).await.map_err(from_sqlx)?;
    match planned.ok().and_then(|plan| hidden::misuse(&plan, hidden, hidden::Allow::Nothing)) {
        Some(column) => Err(hidden_refused(&column, "This statement reads it")),
        None => Ok(()),
    }
}

impl PgDatabase {
    /// Each hidden column's table oid and column number, as result columns report their origin.
    async fn hidden_columns(&self, hidden: &[ColumnRef]) -> Result<Vec<(u32, i16)>, Error> {
        if hidden.is_empty() {
            return Ok(Vec::new());
        }
        let field = |f: fn(&ColumnRef) -> &String| hidden.iter().map(|h| f(h).clone()).collect::<Vec<String>>();
        let rows = sqlx::query(catalog::HIDDEN_COLUMNS)
            .bind(field(|h| &h.schema))
            .bind(field(|h| &h.table))
            .bind(field(|h| &h.column))
            .fetch_all(&self.pool)
            .await
            .map_err(from_sqlx)?;
        rows.iter()
            .map(|r| Ok((u32::try_from(r.try_get::<i64, _>(0)?).unwrap_or_default(), r.try_get(1)?)))
            .collect::<Result<_, sqlx::Error>>()
            .map_err(from_sqlx)
    }
}

/// Streams the file into Postgres's own CSV reader, which tells NULL (an empty field) from empty text (`""`).
async fn import_rows(tx: &mut PgConnection, import: &sql::Import, source: &mut dyn ByteSource, db: &PgDatabase) -> Result<u64, Error> {
    let statement = |sql: &str| {
        db.query(Built {
            sql: sql.to_string(),
            params: Vec::new(),
        })
        .persistent(false)
    };
    statement(&import.create).execute(&mut *tx).await.map_err(from_sqlx)?;
    // COPY has its own protocol, so it can't go through `query`. Its statement is fixed text: no name or value in it
    // comes from outside.
    let mut copy = tx.copy_in_raw(&import.copy).await.map_err(from_sqlx)?;
    loop {
        match source.next() {
            Ok(Some(chunk)) => {
                let sent = copy.send(chunk).await.map(|_| ());
                sent.map_err(from_sqlx)?;
            }
            Ok(None) => break,
            Err(error) => {
                let _ = copy.abort("The file couldn't be read.").await;
                return Err(error);
            }
        }
    }
    copy.finish().await.map_err(from_sqlx)?;
    let inserted = statement(&import.insert).execute(&mut *tx).await.map_err(from_sqlx)?;
    Ok(inserted.rows_affected())
}

async fn export_rows(
    tx: &mut PgConnection,
    source: &ExportSource,
    columns: Option<Vec<ColumnMeta>>,
    sink: &mut dyn RowSink,
    db: &PgDatabase,
) -> Result<f64, Error> {
    let mut written = 0u64;
    match (source, columns) {
        (ExportSource::Table { table, sort, filter }, Some(columns)) => {
            sink.columns(
                &columns
                    .iter()
                    .map(|c| PageColumn {
                        name: c.name.clone(),
                        type_name: c.type_name.clone(),
                    })
                    .collect::<Vec<_>>(),
            )?;
            let built = sql::export(table, filter.as_ref(), sort.as_ref(), &columns)?;
            let mut rows = db.query(built).fetch(&mut *tx);
            while let Some(row) = rows.try_next().await.map_err(from_sqlx)? {
                let values = (0..columns.len())
                    .map(|c| row.try_get(c))
                    .collect::<Result<Vec<Option<String>>, _>>()
                    .map_err(from_sqlx)?;
                sink.row(&values)?;
                written += 1;
            }
        }
        (ExportSource::Statement { sql }, _) => {
            sink.columns(&editor::describe(tx, sql).await?)?;
            let mut rows = sqlx::raw_sql(AssertSqlSafe(sql.clone())).fetch(&mut *tx);
            while let Some(row) = rows.try_next().await.map_err(from_sqlx)? {
                sink.row(&editor::text_values(&row)?)?;
                written += 1;
            }
        }
        (ExportSource::Table { table, .. }, None) => return Err(not_found(table)),
    }
    Ok(written as f64)
}

/// Collects an agent's query: through a cursor when Postgres allows one, so it never produces more than one row past
/// `limit`, or else whole, as for SHOW and EXPLAIN, whose results are small and hold no table's values. A hidden
/// column is refused if the plan computes with it, and masked where the query returns it.
async fn read_rows(
    tx: &mut PgConnection,
    statement: &str,
    limit: u32,
    hidden: &[ColumnRef],
    masked: &[(u32, i16)],
) -> Result<StatementResult, Error> {
    let word = editor::first_word(statement);
    if word == "SHOW" || word == "EXPLAIN" {
        return editor::run_in_transaction(tx, statement).await;
    }
    if !hidden.is_empty() {
        let plan = plan(tx, statement).await?;
        if let Some(column) = hidden::misuse(&plan, hidden, hidden::Allow::Returned) {
            return Err(hidden_refused(&column, "This query filters on it or computes with it"));
        }
    }
    let described = editor::describe_origins(tx, statement).await?;
    let hide: Vec<usize> = described
        .iter()
        .enumerate()
        .filter(|(_, (_, origin))| origin.is_some_and(|o| masked.contains(&o)))
        .map(|(i, _)| i)
        .collect();
    let columns = described.into_iter().map(|(column, _)| column).collect();
    let started = Instant::now();
    // `describe` proved the statement is one statement, so it stays one after the prefix.
    let declare = format!("DECLARE hn_agent NO SCROLL CURSOR FOR {}", statement.trim().trim_end_matches(';'));
    sqlx::raw_sql(AssertSqlSafe(declare)).execute(&mut *tx).await.map_err(from_sqlx)?;
    let (mut rows, cut, total_rows, _) = editor::collect(tx, &format!("FETCH {} FROM hn_agent", limit + 1)).await?;
    for row in &mut rows {
        for &at in &hide {
            if let Some(value) = row.get_mut(at).filter(|v| v.is_some()) {
                *value = Some(hidden::MASK.to_string());
            }
        }
    }
    Ok(StatementResult {
        columns,
        rows,
        cut: cut.into_iter().filter(|c| !hide.contains(&(c.column as usize))).collect(),
        total_rows,
        rows_affected: 0,
        elapsed_ms: hn_core::millis(started.elapsed()),
    })
}

/// Postgres's plan for one statement, with every expression written out, without running it.
async fn plan(conn: &mut PgConnection, statement: &str) -> Result<serde_json::Value, Error> {
    let sql = format!("EXPLAIN (VERBOSE, FORMAT JSON) {}", statement.trim().trim_end_matches(';'));
    let row = sqlx::raw_sql(AssertSqlSafe(sql)).fetch_one(&mut *conn).await.map_err(from_sqlx)?;
    let text = editor::text_values(&row)?.into_iter().next().flatten().unwrap_or_default();
    serde_json::from_str(&text).map_err(|e| Error::new(ErrorKind::Query, format!("Postgres's plan couldn't be read: {e}.")))
}

fn hidden_refused(column: &ColumnRef, why: &str) -> Error {
    Error::new(
        ErrorKind::Invalid,
        format!(
            "{}.{}.{} is hidden from agents. {why}, so Halfnine refused it. Its values show as {} when a query returns the column itself.",
            column.schema,
            column.table,
            column.column,
            hidden::MASK
        ),
    )
}

/// The key an INSERT returned, named by the table's key columns in order.
fn returned_key(row: &sqlx::postgres::PgRow) -> Result<Vec<ColumnValue>, Error> {
    let names: Vec<String> = row.columns().iter().map(|c| sqlx::Column::name(c).to_string()).collect();
    names
        .into_iter()
        .enumerate()
        .map(|(i, column)| {
            Ok(ColumnValue {
                column,
                value: row.try_get(i).map_err(from_sqlx)?,
            })
        })
        .collect()
}

/// Ends a commit: commits it if every statement ran, and rolls it back otherwise.
async fn settle(
    tx: Transaction<'static, Postgres>,
    done: Result<Vec<Vec<ColumnValue>>, Error>,
    statements: u32,
    started: Instant,
) -> Result<Committed, Error> {
    match done {
        Ok(inserted) => {
            tx.commit().await.map_err(from_sqlx)?;
            Ok(Committed {
                statements,
                elapsed_ms: hn_core::millis(started.elapsed()),
                inserted,
            })
        }
        Err(error) => {
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

fn failed(what: &str, error: Error) -> Error {
    Error {
        message: format!("{what} failed, so nothing was written: {}", error.message),
        ..error
    }
}

fn not_found(table: &TableRef) -> Error {
    Error::new(
        ErrorKind::NotFound,
        format!("There is no table or view named {}.{}.", table.schema, table.name),
    )
}
