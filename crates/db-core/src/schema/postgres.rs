//! PostgreSQL introspection over pg_catalog and information_schema, written from the public PostgreSQL documentation.

use async_trait::async_trait;

use super::{
    ColumnInfo, ConstraintInfo, IndexInfo, Introspector, ObjectInfo, ObjectKind, SchemaInfo,
};
use crate::{Connection, Error, Result};

pub struct PostgresIntrospector<'a> {
    #[allow(dead_code)]
    conn: &'a dyn Connection,
}

impl<'a> PostgresIntrospector<'a> {
    pub fn new(conn: &'a dyn Connection) -> Self {
        Self { conn }
    }
}

fn todo<T>(what: &str) -> Result<T> {
    Err(Error::unsupported(format!(
        "postgres {what}: not implemented yet"
    )))
}

#[async_trait]
impl Introspector for PostgresIntrospector<'_> {
    async fn current_schema(&self) -> Result<String> {
        todo("current_schema")
    }

    async fn schemas(&self) -> Result<Vec<SchemaInfo>> {
        todo("schemas")
    }

    async fn objects(&self, _schema: &str, _kinds: &[ObjectKind]) -> Result<Vec<ObjectInfo>> {
        todo("objects")
    }

    async fn columns(&self, _schema: &str, _table: &str) -> Result<Vec<ColumnInfo>> {
        todo("columns")
    }

    async fn indexes(&self, _schema: &str, _table: &str) -> Result<Vec<IndexInfo>> {
        todo("indexes")
    }

    async fn constraints(&self, _schema: &str, _table: &str) -> Result<Vec<ConstraintInfo>> {
        todo("constraints")
    }

    async fn source(
        &self,
        _schema: &str,
        _name: &str,
        _kind: ObjectKind,
    ) -> Result<Option<String>> {
        todo("source")
    }
}
