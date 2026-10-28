use std::future::Future;

use crate::data::{Filter, Page, PageRequest, SchemaNode, ServerInfo, Structure, TableRef};
use crate::error::Error;

/// What the app needs from a database. Each adapter (Postgres today) implements it; the app depends only on this.
pub trait Database: Send + Sync {
    fn server(&self) -> impl Future<Output = Result<ServerInfo, Error>> + Send;
    fn schema(&self) -> impl Future<Output = Result<Vec<SchemaNode>, Error>> + Send;
    fn structure(&self, table: &TableRef) -> impl Future<Output = Result<Structure, Error>> + Send;
    fn page(&self, request: &PageRequest) -> impl Future<Output = Result<Page, Error>> + Send;
    /// The exact number of rows, which can be slow on big tables, so only on request.
    fn count(&self, table: &TableRef, filter: Option<&Filter>) -> impl Future<Output = Result<f64, Error>> + Send;
}
