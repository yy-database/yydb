//! Read-only SQLite main-file snapshots and blank database materialization.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use yydb_types::{Error, Result};

use crate::{
    SchemaObject, TableLimits, TableRow,
    format::{ENGINE_LIBRARY_VERSION_NUMBER, blank_database, decode_library_version, stamp_library_version, validate_database},
    read_named_table, read_schema,
};

/// Storage mode for [`SqliteDatabase`].
#[derive(Debug, Clone, PartialEq, Eq)]
enum Storage {
    Memory,
    File(PathBuf),
}

/// A SQLite main-file snapshot, not a transactional database engine.
pub struct SqliteDatabase {
    storage: Storage,
    page_size: usize,
    pages: Vec<u8>,
}

impl SqliteDatabase {
    /// Opens an in-memory SQLite database with a blank format-3 page.
    pub fn open_in_memory() -> Result<Self> {
        let pages = blank_database()?;
        let page_size = validate_database(&pages)?;
        Ok(Self { storage: Storage::Memory, page_size, pages })
    }

    /// Reads an existing snapshot or exclusively creates a blank file.
    /// This does not recover journals or produce a consistent live WAL snapshot.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            let bytes = fs::read(&path).map_err(Error::Io)?;
            let page_size = validate_database(&bytes)?;
            return Ok(Self { storage: Storage::File(path), page_size, pages: bytes });
        }

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(Error::Io)?;
            }
        }

        let pages = blank_database()?;
        let page_size = validate_database(&pages)?;
        let mut file = OpenOptions::new().write(true).create_new(true).open(&path).map_err(Error::Io)?;
        file.write_all(&pages).map_err(Error::Io)?;
        file.sync_all().map_err(Error::Io)?;
        Ok(Self { storage: Storage::File(path), page_size, pages })
    }

    /// Returns the configured database path for file-backed databases.
    pub fn path(&self) -> Option<&Path> {
        match &self.storage {
            Storage::Memory => None,
            Storage::File(path) => Some(path),
        }
    }

    /// Returns the page size decoded from the database header.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Returns the raw database bytes owned by this handle.
    pub fn pages(&self) -> &[u8] {
        &self.pages
    }

    /// Reads the main-file schema without parsing or executing SQL definitions.
    pub fn schema(&self, limits: TableLimits) -> Result<Vec<SchemaObject>> {
        read_schema(&self.pages, limits)
    }

    /// Reads raw records from an ordinary rowid table by its stored name.
    pub fn table(&self, name: &str, limits: TableLimits) -> Result<Vec<TableRow>> {
        read_named_table(&self.pages, name, limits)
    }

    /// Health probe for disguise tooling and connector smoke tests.
    pub fn ping(&self) -> Result<i32> {
        validate_database(&self.pages)?;
        Ok(1)
    }

    /// Returns the SQLite library version stamped in this snapshot header.
    pub fn sqlite_version(&self) -> Result<String> {
        decode_library_version(&self.pages)
    }

    /// Rejects file writes until a transactional pager is available.
    pub fn flush(&self) -> Result<()> {
        match &self.storage {
            Storage::Memory => Ok(()),
            Storage::File(_) => Err(Error::Unsupported("sqlite snapshot writes require a transactional pager")),
        }
    }

    /// Re-stamps the format-3 library version field with the YYDS engine version.
    pub fn restamp_engine_version(&mut self) -> Result<()> {
        validate_database(&self.pages)?;
        stamp_library_version(&mut self.pages, ENGINE_LIBRARY_VERSION_NUMBER);
        Ok(())
    }
}

/// Reads an existing SQLite file without keeping it open.
pub fn read_existing(path: impl AsRef<Path>) -> Result<Vec<u8>> {
    let path = path.as_ref();
    let mut file = File::open(path).map_err(Error::Io)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(Error::Io)?;
    validate_database(&bytes)?;
    Ok(bytes)
}
