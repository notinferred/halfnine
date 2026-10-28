//! TableUltra's Postgres adapter: it implements [`tu_core::Database`] over a small sqlx pool, optionally through an SSH tunnel.

mod catalog;
mod editor;
mod errors;
mod sql;
mod tunnel;

use std::time::{Duration, Instant};

use futures_util::TryStreamExt;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgSslMode};
use sqlx::{AssertSqlSafe, PgConnection, Postgres, Row, Transaction};
use tu_core::{
    Activity, ConnectionSpec, CutCell, Database, Error, ErrorKind, ExportSource, Filter, Page, PageColumn, PageRequest, RowSink, RunOutcome,
    SchemaNode, ServerInfo, Structure, TableRef, TlsMode,
};

use crate::editor::{EditorConnection, Running};
use crate::errors::from_sqlx;
use crate::sql::{Built, ColumnMeta};
use crate::tunnel::Tunnel;

pub(crate) const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The secrets a connection needs, read from the keychain just before connecting and never kept.
#[derive(Default)]
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
            .application_name("TableUltra");
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

    /// The same, for work that may take as long as it needs and can be cancelled: an explain or an export.
    async fn reading_at_length(&self, read_only: bool) -> Result<(Transaction<'static, Postgres>, i32), Error> {
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
                })
            })
            .collect::<Result<_, sqlx::Error>>()
            .map_err(from_sqlx)
    }

    fn query(&self, built: Built) -> sqlx::query::Query<'static, sqlx::Postgres, sqlx::postgres::PgArguments> {
        // Every statement the app builds passes through here. Its identifiers came through `sql::ident` and its values are
        // bound parameters; the only text taken as SQL is a WHERE condition someone typed for their own table.
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
        let estimated_rows: Option<f64> = found.try_get(1).map_err(from_sqlx)?;
        let (columns, key, indexes, constraints, outgoing, incoming) = tokio::try_join!(
            sqlx::query(catalog::COLUMNS).bind(oid).fetch_all(&self.pool),
            sqlx::query(catalog::PRIMARY_KEY).bind(oid).fetch_optional(&self.pool),
            sqlx::query(catalog::INDEXES).bind(oid).fetch_all(&self.pool),
            sqlx::query(catalog::CONSTRAINTS).bind(oid).fetch_all(&self.pool),
            sqlx::query(catalog::FOREIGN_KEYS).bind(oid).fetch_all(&self.pool),
            sqlx::query(catalog::REFERENCED_BY).bind(oid).fetch_all(&self.pool),
        )
        .map_err(from_sqlx)?;
        catalog::structure(estimated_rows, &columns, key.as_ref(), &indexes, &constraints, &outgoing, &incoming).map_err(from_sqlx)
    }

    async fn page(&self, request: &PageRequest) -> Result<Page, Error> {
        let columns = self.columns(&request.table).await?;
        let built = sql::page(request, &columns)?;
        let mut tx = self.reading().await?;
        let started = Instant::now();
        let rows = self.query(built).fetch_all(&mut *tx).await.map_err(from_sqlx)?;
        let elapsed_ms = tu_core::millis(started.elapsed());
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
        let (mut tx, pid) = self.reading_at_length(self.read_only).await?;
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
        let (mut tx, pid) = self.reading_at_length(true).await?;
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

fn not_found(table: &TableRef) -> Error {
    Error::new(
        ErrorKind::NotFound,
        format!("There is no table or view named {}.{}.", table.schema, table.name),
    )
}
