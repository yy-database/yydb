//! Thin Node-API binding — atomic database operations over the [`yydb`] facade.
//!
//! The binding exposes one handle per open file plus single-purpose methods.
//! User-facing commands (`init`, `info`, `serve`) are composed in TypeScript so
//! this crate never grows CLI aggregate helpers.

#![deny(missing_docs)]

use napi::bindgen_prelude::{Buffer, Result};
use napi_derive::napi;

fn reason(error: yydb::Error) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}

/// Embedded `yydb` engine version string.
#[napi]
pub fn version() -> String {
    yydb::version().to_string()
}

/// Run the YY wire serve loop on `bind` for the database at `db_path`.
///
/// This call blocks until the listener stops. Only a dedicated process such as
/// the `yydb serve` CLI should invoke it.
#[napi]
pub fn serve(db_path: String, bind: String, insecure_bind: Option<bool>) -> Result<()> {
    yydb::server::run_serve(
        std::path::Path::new(&db_path),
        &bind,
        insecure_bind.unwrap_or(false),
    )
    .map_err(|error| napi::Error::from_reason(error.to_string()))
}

/// Handle to one open `.yydb` file.
#[napi]
pub struct Database {
    connection: yydb::Connection,
}

#[napi]
impl Database {
    /// Open (create when missing) the database at `db_path`.
    #[napi(factory)]
    pub fn open(db_path: String) -> Result<Self> {
        Ok(Self {
            connection: yydb::Connection::open(&db_path).map_err(reason)?,
        })
    }

    /// Absolute path of the database file.
    #[napi]
    pub fn path(&self) -> Option<String> {
        self.connection
            .path()
            .map(|path| path.display().to_string())
    }

    /// Path of the `-wal` sidecar for file-backed databases.
    #[napi]
    pub fn wal_path(&self) -> Option<String> {
        self.connection
            .wal_path()
            .map(|path| path.display().to_string())
    }

    /// Path of the `-shm` sidecar for file-backed databases.
    #[napi]
    pub fn shm_path(&self) -> Option<String> {
        self.connection
            .shm_path()
            .map(|path| path.display().to_string())
    }

    /// Published schema version, or `null` before the first schema.
    #[napi]
    pub fn schema_version(&self) -> Result<Option<u32>> {
        Ok(self
            .connection
            .schema()
            .map_err(reason)?
            .map(|schema| schema.version))
    }

    /// Published VOS schema document, or `null` before the first schema.
    #[napi]
    pub fn schema_document(&self) -> Result<Option<String>> {
        Ok(self
            .connection
            .schema()
            .map_err(reason)?
            .map(|schema| schema.document))
    }

    /// Validate and publish a VOS schema document.
    #[napi]
    pub fn ensure_schema(&self, document: String) -> Result<()> {
        self.connection.ensure_schema(&document).map_err(reason)
    }

    /// Number of user records in the database.
    #[napi]
    pub fn record_count(&self) -> Result<u32> {
        let count = self.connection.record_count().map_err(reason)?;
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// Current journal mode (`delete` or `wal`).
    #[napi]
    pub fn journal_mode(&self) -> String {
        self.connection.journal_mode().as_str().to_owned()
    }

    /// Switch the journal mode.
    #[napi]
    pub fn set_journal_mode(&self, mode: String) -> Result<()> {
        let mode = yydb::JournalMode::parse(&mode).map_err(reason)?;
        self.connection.set_journal_mode(mode).map_err(reason)
    }

    /// Fold the WAL into the main file and truncate the sidecar.
    #[napi]
    pub fn checkpoint(&self) -> Result<()> {
        self.connection.checkpoint().map_err(reason)
    }

    /// Number of frames currently in the WAL.
    #[napi]
    pub fn wal_frame_count(&self) -> Result<u32> {
        self.connection.wal_frame_count().map_err(reason)
    }

    /// Read one record, or `null` when the key is absent.
    #[napi]
    pub fn get(&self, key: String) -> Result<Option<Buffer>> {
        Ok(self.connection.get(&key).map_err(reason)?.map(Buffer::from))
    }

    /// Write one record.
    #[napi]
    pub fn put(&self, key: String, value: Buffer) -> Result<()> {
        self.connection.put(key, value.as_ref()).map_err(reason)
    }
}
