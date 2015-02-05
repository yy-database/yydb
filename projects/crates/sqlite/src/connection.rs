//! Writable SQLite dialect surface backed by the YYDB engine.
//!
//! Statements are parsed by `oak-sql`, lowered by `yydb-query` into the shared
//! operator set and executed against YYDB storage. Indexing, locking,
//! transactions and durability therefore behave exactly like the VOS surface.

use std::path::Path;

use yydb::{Connection, OpenFlags, Value};
use yydb_query::QueryRow;

/// Result alias for this facade.
pub type Result<T> = yydb::Result<T>;

/// A SQLite-flavoured connection over a YYDB database.
///
/// ```rust,no_run
/// use sqlite::SqliteConnection;
///
/// let conn = SqliteConnection::open_in_memory()?;
/// conn.ensure_schema("table Account { @@account_id: i64, email: utf8 }")?;
/// conn.execute("INSERT INTO Account (account_id, email) VALUES (1, 'ada@example.test')")?;
/// assert_eq!(conn.query("SELECT email FROM Account")?.len(), 1);
/// # Ok::<(), yydb::Error>(())
/// ```
#[derive(Clone)]
pub struct SqliteConnection {
    inner: std::sync::Arc<Connection>,
}

impl std::fmt::Debug for SqliteConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteConnection").finish_non_exhaustive()
    }
}

impl SqliteConnection {
    /// Open an in-memory YYDB database.
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self { inner: std::sync::Arc::new(Connection::open_in_memory()?) })
    }

    /// Open or create a YYDB database file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_flags(path, OpenFlags::default())
    }

    /// Open or create a YYDB database file with explicit flags.
    pub fn open_with_flags(path: impl AsRef<Path>, flags: OpenFlags) -> Result<Self> {
        Ok(Self { inner: std::sync::Arc::new(Connection::open_with_flags(path, flags)?) })
    }

    /// Declare (or migrate) the database schema from a VOS document.
    ///
    /// DDL stays VOS-owned by design: SQL statements address rows, while table
    /// and column declarations come from one schema document so catalog,
    /// unique constraints and migrations keep a single source of truth.
    pub fn ensure_schema(&self, schema: &str) -> Result<()> {
        self.inner.ensure_schema(schema)
    }

    /// Execute one SQL write statement (`INSERT`, `UPDATE`, `DELETE`).
    ///
    /// Returns the number of affected rows.
    pub fn execute(&self, sql: &str) -> Result<usize> {
        self.inner.execute_sql_write(sql, &[])
    }

    /// Execute one SQL write statement with positional `?` bindings.
    pub fn execute_with_params(&self, sql: &str, values: &[Value]) -> Result<usize> {
        self.inner.execute_sql_write(sql, values)
    }

    /// Run one SQL read statement (`SELECT`).
    pub fn query(&self, sql: &str) -> Result<Vec<QueryRow>> {
        self.inner.query_sql(sql)
    }

    /// Run one SQL read statement with positional `?` bindings.
    pub fn query_with_params(&self, sql: &str, values: &[Value]) -> Result<Vec<QueryRow>> {
        self.inner.query_sql_with_params(sql, values)
    }

    /// Open a transaction; writes stay private until [`Self::commit`].
    pub fn begin(&self) -> Result<()> {
        self.inner.begin()
    }

    /// Commit the open transaction.
    pub fn commit(&self) -> Result<()> {
        self.inner.commit()
    }

    /// Roll back the open transaction.
    pub fn rollback(&self) -> Result<()> {
        self.inner.rollback()
    }

    /// Borrow the underlying YYDB connection for VOS access, catalog control
    /// or checkpointing.
    pub fn connection(&self) -> &Connection {
        &self.inner
    }
}
