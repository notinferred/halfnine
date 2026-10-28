//! TableUltra's Postgres adapter: it implements [`tu_core::Database`] over a small sqlx pool, optionally through an SSH tunnel.

mod catalog;
mod errors;
mod sql;
mod tunnel;

use std::time::{Duration, Instant};

use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgSslMode};
use sqlx::{AssertSqlSafe, Row};
use tu_core::{
    ConnectionSpec, CutCell, Database, Error, ErrorKind, Filter, Page, PageColumn, PageRequest, SchemaNode, ServerInfo, Structure, TableRef, TlsMode,
};

use crate::errors::from_sqlx;
use crate::sql::{Built, CELL_LIMIT, ColumnMeta};
use crate::tunnel::Tunnel;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// The secrets a connection needs, read from the keychain just before connecting and never kept.
#[derive(Default)]
pub struct Secrets {
    pub password: Option<String>,
    pub ssh: Option<String>,
}

pub struct PgDatabase {
    pool: PgPool,
    // Held so the tunnel lives as long as the pool that uses it.
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
            .connect_with(options);
        let pool = tokio::time::timeout(CONNECT_TIMEOUT, connecting)
            .await
            .map_err(|_| Error::new(ErrorKind::Timeout, format!("{} didn't answer within 10 seconds.", spec.host)))?
            .map_err(from_sqlx)?;
        Ok(Self { pool, _tunnel: tunnel })
    }

    pub async fn close(&self) {
        self.pool.close().await;
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
        let started = Instant::now();
        let rows = self.query(built).fetch_all(&self.pool).await.map_err(from_sqlx)?;
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

        let limit = request.limit.clamp(1, sql::PAGE_LIMIT) as usize;
        let has_more = rows.len() > limit;
        let mut cut = Vec::new();
        let mut cells = Vec::with_capacity(rows.len().min(limit));
        for (r, row) in rows.iter().take(limit).enumerate() {
            let mut values = Vec::with_capacity(columns.len());
            for c in 0..columns.len() {
                let mut value: Option<String> = row.try_get(c).map_err(from_sqlx)?;
                if let Some(text) = &mut value
                    && let Some((at, _)) = text.char_indices().nth(CELL_LIMIT)
                {
                    text.truncate(at);
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
        let row = self.query(built).fetch_one(&self.pool).await.map_err(from_sqlx)?;
        row.try_get(0).map_err(from_sqlx)
    }
}

fn not_found(table: &TableRef) -> Error {
    Error::new(
        ErrorKind::NotFound,
        format!("There is no table or view named {}.{}.", table.schema, table.name),
    )
}
