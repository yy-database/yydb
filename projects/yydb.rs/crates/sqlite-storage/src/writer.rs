//! SQLite format-3 writer.
//!
//! Produces byte-exact SQLite main files that upstream SQLite implementations
//! open directly: page allocation, rowid b-tree mutation with splits, index
//! b-trees, freeblocks, the freelist, overflow pages and in-memory
//! transactions. The writer keeps the whole file image in memory and emits it
//! with [`SqliteWriter::save`]; it never speaks SQL.
//!
//! Unsupported on purpose (reported, not faked): WITHOUT ROWID tables,
//! composite PRIMARY KEYs, AUTOINCREMENT, DROP, ALTER, views, triggers, WAL
//! output and UTF-16 database creation.

use std::{fs, path::Path};

use sqlite_formats::{
    TextEncoding, WriteValue, blank_with_page_size, bump_change_counter, decode_record, decode_varint, encode_record,
    encode_varint_into, read_freelist_count, read_freelist_trunk, read_schema_cookie, set_freelist_count, set_freelist_trunk,
    set_page_count, set_schema_cookie, validate_database,
};
use yydb_types::{Error, Result};

use crate::{SchemaObject, TableLimits, TableRow, read_schema, read_table};

const PAGE_INDEX_INTERIOR: u8 = 2;
const PAGE_TABLE_INTERIOR: u8 = 5;
const PAGE_INDEX_LEAF: u8 = 10;
const PAGE_TABLE_LEAF: u8 = 13;

/// One column of a table created by [`SqliteWriter::create_table`].
#[derive(Clone, Debug)]
pub struct ColumnSpec {
    /// Column name as stored in the schema.
    pub name: String,
    /// Declared type text (`INTEGER`, `TEXT`, `REAL`, `BLOB`, ...).
    pub declared_type: String,
    /// Reject explicit NULL inserts.
    pub not_null: bool,
    /// Applied when the insert omits the column.
    pub default_value: Option<WriteValue>,
    /// Single-column PRIMARY KEY.
    pub primary_key: bool,
    /// Single-column UNIQUE constraint.
    pub unique: bool,
}

/// Value slot of an [`SqliteWriter::update`] call.
#[derive(Clone, Debug)]
pub enum UpdateValue {
    /// Keep the stored value.
    Keep,
    /// Replace the stored value.
    Set(WriteValue),
}

/// Result of inserting into a child subtree: the child keeps every key up to
/// the first boundary key, and each boundary page holds the keys above its
/// predecessor up to (and including) its own boundary key.
struct Split {
    boundaries: Vec<(i64, u32)>,
}

/// Greedily packs sorted cells into groups that each fit one page, returning
/// at least two groups when the total exceeds `capacity`.
fn pack_groups<T>(cells: Vec<T>, capacity: usize, size_of: impl Fn(&T) -> usize) -> Vec<Vec<T>> {
    let mut groups: Vec<Vec<T>> = Vec::new();
    let mut space = 0usize;
    for cell in cells {
        let size = size_of(&cell);
        if groups.is_empty() || size > space {
            groups.push(Vec::new());
            space = capacity;
        }
        space = space.saturating_sub(size);
        groups.last_mut().expect("group pushed above").push(cell);
    }
    groups
}

struct MasterRow {
    rowid: i64,
    kind: String,
    name: String,
    table_name: String,
    root_page: u32,
    sql: Option<String>,
}

struct TableMeta {
    table_name: String,
    master_rowid: i64,
    root_page: u32,
    columns: Vec<ColumnSpec>,
    rowid_alias: Option<usize>,
    indexes: Vec<IndexMeta>,
}

#[derive(Clone)]
struct IndexMeta {
    master_rowid: i64,
    root_page: u32,
    columns: Vec<usize>,
    unique: bool,
}

struct Snapshot {
    pages: Vec<u8>,
    index_entries: std::collections::BTreeMap<i64, std::collections::BTreeSet<Vec<u8>>>,
}

/// In-memory SQLite format-3 file image with mutation operations.
///
/// Index trees are maintained as ordered in-memory key sets while the image
/// is being mutated and are materialized into b-tree pages by [`SqliteWriter::save`].
/// Deleting from a b-tree in place would demand full upstream-style
/// rebalancing (merge/redistribute) to keep SQLite's equal-leaf-depth
/// invariant; rebuilding from the sorted sets reuses the proven insert path
/// instead. `pages()` therefore reflects current table content at all times,
/// while index trees are only guaranteed current after `save`.
pub struct SqliteWriter {
    pages: Vec<u8>,
    page_size: usize,
    usable: usize,
    snapshot: Option<Snapshot>,
    /// Live index keys by `sqlite_master` rowid; rebuilt into pages on save.
    index_entries: std::collections::BTreeMap<i64, std::collections::BTreeSet<Vec<u8>>>,
}

impl SqliteWriter {
    /// Creates a fresh database image with the given page size.
    pub fn create(page_size: usize) -> Result<Self> {
        let pages = blank_with_page_size(page_size)?;
        let usable = page_size - usize::from(pages[20]);
        Ok(Self { pages, page_size, usable, snapshot: None, index_entries: std::collections::BTreeMap::new() })
    }

    /// Loads an existing SQLite main file for further mutation.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let pages = fs::read(path.as_ref()).map_err(Error::Io)?;
        let page_size = validate_database(&pages)?;
        let usable = page_size - usize::from(pages[20]);
        let mut writer = Self { pages, page_size, usable, snapshot: None, index_entries: std::collections::BTreeMap::new() };
        for row in writer.master_rows()? {
            if row.kind == "index" && row.root_page != 0 {
                let entries = writer.read_index_entries(row.root_page)?;
                writer.index_entries.insert(row.rowid, entries);
            }
        }
        Ok(writer)
    }

    /// Returns the raw file image.
    pub fn pages(&self) -> &[u8] {
        &self.pages
    }

    /// Returns the configured page size.
    pub fn page_size(&self) -> usize {
        self.page_size
    }

    /// Materializes every index tree from its live key set, bumps the change
    /// counter and writes the image to `path`.
    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        self.materialize_indexes()?;
        bump_change_counter(&mut self.pages);
        fs::write(path.as_ref(), &self.pages).map_err(Error::Io)
    }

    /// Starts a transaction; nested transactions are rejected.
    pub fn begin(&mut self) -> Result<()> {
        if self.snapshot.is_some() {
            return Err(Error::Unsupported("sqlite writer already has an open transaction"));
        }
        self.snapshot = Some(Snapshot { pages: self.pages.clone(), index_entries: self.index_entries.clone() });
        Ok(())
    }

    /// Commits the open transaction.
    pub fn commit(&mut self) -> Result<()> {
        if self.snapshot.take().is_none() {
            return Err(Error::Unsupported("sqlite writer has no open transaction"));
        }
        Ok(())
    }

    /// Rolls the image back to the state before [`SqliteWriter::begin`].
    pub fn rollback(&mut self) -> Result<()> {
        let snapshot = self.snapshot.take().ok_or(Error::Unsupported("sqlite writer has no open transaction"))?;
        self.pages = snapshot.pages;
        self.index_entries = snapshot.index_entries;
        Ok(())
    }

    /// Reads the schema objects through the crate's own reader.
    pub fn schema_objects(&self, limits: TableLimits) -> Result<Vec<SchemaObject>> {
        read_schema(&self.pages, limits)
    }

    /// Reads one table's rows through the crate's own reader.
    pub fn table_rows(&self, table: &str, limits: TableLimits) -> Result<Vec<TableRow>> {
        let meta = self.table_meta(table)?;
        read_table(&self.pages, meta.root_page, limits)
    }

    // ------------------------------------------------------------------
    // Schema objects
    // ------------------------------------------------------------------

    /// Creates a table with the given columns.
    pub fn create_table(&mut self, name: &str, columns: &[ColumnSpec]) -> Result<()> {
        if name.is_empty() {
            return Err(Error::Unsupported("sqlite table name must not be empty"));
        }
        if columns.is_empty() {
            return Err(Error::Unsupported("sqlite table needs at least one column"));
        }
        for object in self.master_rows()? {
            if eq_ignore_ascii_case(&object.name, name) {
                return Err(Error::Unsupported("sqlite schema object already exists"));
            }
        }
        let mut seen = Vec::new();
        for column in columns {
            if column.name.is_empty() {
                return Err(Error::Unsupported("sqlite column name must not be empty"));
            }
            if seen.iter().any(|existing: &String| eq_ignore_ascii_case(existing, &column.name)) {
                return Err(Error::Unsupported("sqlite duplicate column name"));
            }
            seen.push(column.name.clone());
        }
        let alias_count = columns.iter().filter(|column| is_rowid_alias(column)).count();
        let pk_count = columns.iter().filter(|column| column.primary_key).count();
        if pk_count > 1 {
            return Err(Error::Unsupported("sqlite composite PRIMARY KEY is not supported"));
        }
        if pk_count == 1 && alias_count == 0 {
            return Err(Error::Unsupported("sqlite non-INTEGER PRIMARY KEY is not supported"));
        }
        let rowid_alias = columns.iter().position(is_rowid_alias);

        let root_page = self.allocate_page()?;
        self.init_leaf_page(root_page, PAGE_TABLE_LEAF);
        let sql = render_create_table(name, columns);
        let master_rowid = self.insert_master_row("table", name, name, root_page, Some(&sql))?;

        let mut meta = TableMeta {
            table_name: name.to_string(),
            master_rowid,
            root_page,
            columns: columns.to_vec(),
            rowid_alias,
            indexes: Vec::new(),
        };
        let mut auto_number = 0usize;
        for (index, column) in columns.iter().enumerate() {
            // A rowid alias is the table rowid itself; SQLite keeps no
            // autoindex for it.
            if (column.primary_key || column.unique) && Some(index) != rowid_alias {
                auto_number += 1;
                let auto_name = format!("sqlite_autoindex_{name}_{auto_number}");
                self.create_index_inner(&auto_name, &mut meta, vec![index], true, true)?;
            }
        }
        self.bump_cookie();
        Ok(())
    }

    /// Creates an index over one or more columns of an existing table.
    pub fn create_index(&mut self, name: &str, table: &str, columns: &[String], unique: bool) -> Result<()> {
        if name.is_empty() || columns.is_empty() {
            return Err(Error::Unsupported("sqlite index needs a name and columns"));
        }
        for object in self.master_rows()? {
            if eq_ignore_ascii_case(&object.name, name) {
                return Err(Error::Unsupported("sqlite schema object already exists"));
            }
        }
        let mut meta = self.table_meta(table)?;
        let mut positions = Vec::with_capacity(columns.len());
        for column in columns {
            let position = meta
                .columns
                .iter()
                .position(|existing| eq_ignore_ascii_case(&existing.name, column))
                .ok_or(Error::Unsupported("sqlite index column does not exist"))?;
            positions.push(position);
        }
        self.create_index_inner(name, &mut meta, positions, unique, false)?;
        self.bump_cookie();
        Ok(())
    }

    fn create_index_inner(
        &mut self,
        name: &str,
        meta: &mut TableMeta,
        columns: Vec<usize>,
        unique: bool,
        automatic: bool,
    ) -> Result<()> {
        let root_page = self.allocate_page()?;
        self.init_leaf_page(root_page, PAGE_INDEX_LEAF);
        // Automatic constraint indexes carry no CREATE text in sqlite_master;
        // upstream SQLite re-derives them from the table definition and would
        // reject a duplicate if the text were present.
        let master_rowid = if automatic {
            self.insert_master_row("index", name, &table_name_of(meta), root_page, None)?
        }
        else {
            let column_names: Vec<String> = columns.iter().map(|index| meta.columns[*index].name.clone()).collect();
            let sql = render_create_index(name, &table_name_of(meta), &column_names, unique);
            self.insert_master_row("index", name, &table_name_of(meta), root_page, Some(&sql))?
        };
        let mut index = IndexMeta { master_rowid, root_page, columns, unique };

        // Backfill existing rows into the live key set, then materialize the
        // fresh root. Splits may regrow the root; the master row must follow.
        let rows = read_table(&self.pages, meta.root_page, TableLimits::default())?;
        let mut entries = std::collections::BTreeSet::new();
        for row in rows {
            let values = decode_record(&row.payload, TextEncoding::Utf8, Default::default())
                .map_err(|_| Error::Corrupt("sqlite record decode failed"))?;
            let owned = expand_alias(meta, row.rowid, &values);
            entries.insert(encode_index_key(meta, &index, row.rowid, &owned)?);
        }
        let initial_root = root_page;
        for key in &entries {
            index.root_page = self.index_insert(index.root_page, key)?;
        }
        if index.root_page != initial_root {
            self.set_master_root(master_rowid, index.root_page)?;
        }
        self.index_entries.insert(master_rowid, entries);
        meta.indexes.push(index);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Row mutation
    // ------------------------------------------------------------------

    /// Inserts one row; `None` slots apply the column default or NULL.
    /// Returns the rowid of the new row.
    pub fn insert(&mut self, table: &str, values: &[Option<WriteValue>]) -> Result<i64> {
        let mut meta = self.table_meta(table)?;
        if values.len() != meta.columns.len() {
            return Err(Error::Unsupported("sqlite insert arity mismatch"));
        }
        let mut resolved: Vec<WriteValue> = Vec::with_capacity(values.len());
        for (index, slot) in values.iter().enumerate() {
            let column = &meta.columns[index];
            let value = match slot {
                Some(value) => value.clone(),
                None => column.default_value.clone().unwrap_or(WriteValue::Null),
            };
            let value = apply_affinity(value, affinity_of(&column.declared_type));
            if matches!(value, WriteValue::Null) && column.not_null && Some(index) != meta.rowid_alias {
                return Err(Error::Unsupported("sqlite NOT NULL constraint failed"));
            }
            resolved.push(value);
        }

        let alias_index = meta.rowid_alias;
        let alias_value = alias_index.and_then(|index| match &resolved[index] {
            WriteValue::Integer(rowid) => Some(*rowid),
            _ => None,
        });
        let explicit_rowid = match alias_value {
            Some(rowid) => Some(rowid),
            None => {
                if let Some(index) = alias_index {
                    if !matches!(resolved[index], WriteValue::Null) {
                        return Err(Error::Unsupported("sqlite rowid alias must be an integer or NULL"));
                    }
                }
                None
            }
        };

        let rowid = match explicit_rowid {
            Some(rowid) => rowid,
            None => self.max_rowid(meta.root_page)?.saturating_add(1),
        };
        if explicit_rowid.is_none() && rowid == i64::MIN {
            return Err(Error::Unsupported("sqlite rowid space exhausted"));
        }

        if let Some(index) = alias_index {
            resolved[index] = WriteValue::Null;
        }
        for index_meta in &meta.indexes {
            if index_meta.unique {
                let key = encode_index_key(&meta, index_meta, rowid, &resolved)?;
                if self.set_prefix_match(index_meta.master_rowid, &key, index_meta.columns.len(), None)? {
                    return Err(Error::Unsupported("sqlite UNIQUE constraint failed"));
                }
            }
        }
        if explicit_rowid.is_some() && self.contains_rowid(meta.root_page, rowid)? {
            return Err(Error::Unsupported("sqlite UNIQUE constraint failed"));
        }

        let payload = encode_record(&resolved);
        let root = self.table_insert(meta.root_page, rowid, &payload)?;
        if root != meta.root_page {
            self.set_master_root(meta.master_rowid, root)?;
            meta.root_page = root;
        }
        for index_meta in &meta.indexes {
            let key = encode_index_key(&meta, index_meta, rowid, &resolved)?;
            self.index_entries.entry(index_meta.master_rowid).or_default().insert(key);
        }
        Ok(rowid)
    }

    /// Deletes one row by rowid; returns whether a row was removed.
    pub fn delete(&mut self, table: &str, rowid: i64) -> Result<bool> {
        let meta = self.table_meta(table)?;
        let Some(payload) = self.read_payload(meta.root_page, rowid)?
        else {
            return Ok(false);
        };
        let values = decode_record(&payload, TextEncoding::Utf8, Default::default())
            .map_err(|_| Error::Corrupt("sqlite record decode failed"))?;
        let owned = expand_alias(&meta, rowid, &values);
        for index_meta in &meta.indexes {
            let key = encode_index_key(&meta, index_meta, rowid, &owned)?;
            if let Some(entries) = self.index_entries.get_mut(&index_meta.master_rowid) {
                entries.remove(&key);
            }
        }
        let root = self.table_delete(meta.root_page, rowid)?;
        if root != meta.root_page {
            self.set_master_root(meta.master_rowid, root)?;
        }
        Ok(true)
    }

    /// Updates one row by rowid; `UpdateValue::Keep` preserves a column.
    /// Returns whether a row was changed.
    pub fn update(&mut self, table: &str, rowid: i64, values: &[UpdateValue]) -> Result<bool> {
        if values.is_empty() {
            return Ok(false);
        }
        let meta = self.table_meta(table)?;
        if values.len() != meta.columns.len() {
            return Err(Error::Unsupported("sqlite update arity mismatch"));
        }
        let Some(payload) = self.read_payload(meta.root_page, rowid)?
        else {
            return Ok(false);
        };
        let decoded = decode_record(&payload, TextEncoding::Utf8, Default::default())
            .map_err(|_| Error::Corrupt("sqlite record decode failed"))?;
        let mut owned = expand_alias(&meta, rowid, &decoded);
        let mut next_rowid = rowid;
        for (index, slot) in values.iter().enumerate() {
            let UpdateValue::Set(value) = slot
            else {
                continue;
            };
            let column = &meta.columns[index];
            let value = apply_affinity(value.clone(), affinity_of(&column.declared_type));
            if matches!(value, WriteValue::Null) && column.not_null && Some(index) != meta.rowid_alias {
                return Err(Error::Unsupported("sqlite NOT NULL constraint failed"));
            }
            if Some(index) == meta.rowid_alias {
                let WriteValue::Integer(rowid_value) = value
                else {
                    return Err(Error::Unsupported("sqlite rowid alias must be an integer"));
                };
                next_rowid = rowid_value;
            }
            owned[index] = value.clone();
        }
        if next_rowid != rowid && self.contains_rowid(meta.root_page, next_rowid)? {
            return Err(Error::Unsupported("sqlite UNIQUE constraint failed"));
        }
        for index_meta in &meta.indexes {
            if index_meta.unique {
                let key = encode_index_key(&meta, index_meta, next_rowid, &owned)?;
                if self.set_prefix_match(index_meta.master_rowid, &key, index_meta.columns.len(), Some(rowid))? {
                    return Err(Error::Unsupported("sqlite UNIQUE constraint failed"));
                }
            }
        }

        // The alias slot travels as `next_rowid` through the delete/insert
        // bookkeeping, but the stored record keeps NULL like every insert.
        self.delete(table, rowid)?;
        if let Some(index) = meta.rowid_alias {
            owned[index] = WriteValue::Null;
        }
        let payload = encode_record(&owned);
        let meta = self.table_meta(table)?;
        let root = self.table_insert(meta.root_page, next_rowid, &payload)?;
        if root != meta.root_page {
            self.set_master_root(meta.master_rowid, root)?;
        }
        let meta = self.table_meta(table)?;
        for index_meta in &meta.indexes {
            let key = encode_index_key(&meta, index_meta, next_rowid, &owned)?;
            self.index_entries.entry(index_meta.master_rowid).or_default().insert(key);
        }
        Ok(true)
    }

    // ------------------------------------------------------------------
    // sqlite_master bookkeeping
    // ------------------------------------------------------------------

    fn master_rows(&self) -> Result<Vec<MasterRow>> {
        let rows = read_table(&self.pages, 1, TableLimits::default())?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let values = decode_record(&row.payload, TextEncoding::Utf8, Default::default())
                .map_err(|_| Error::Corrupt("sqlite master record decode failed"))?;
            let text = |index: usize| match &values[index] {
                sqlite_formats::RecordValue::Text(text) => Ok(text.clone()),
                _ => Err(Error::Corrupt("sqlite master text column")),
            };
            let root_page = match &values[3] {
                sqlite_formats::RecordValue::Integer(page) => u32::try_from(*page).unwrap_or(0),
                sqlite_formats::RecordValue::Null => 0,
                _ => return Err(Error::Corrupt("sqlite master rootpage column")),
            };
            let sql = match &values[4] {
                sqlite_formats::RecordValue::Text(text) => Some(text.clone()),
                sqlite_formats::RecordValue::Null => None,
                _ => return Err(Error::Corrupt("sqlite master sql column")),
            };
            out.push(MasterRow { rowid: row.rowid, kind: text(0)?, name: text(1)?, table_name: text(2)?, root_page, sql });
        }
        Ok(out)
    }

    fn insert_master_row(
        &mut self,
        kind: &str,
        name: &str,
        table_name: &str,
        root_page: u32,
        sql: Option<&str>,
    ) -> Result<i64> {
        let rowid = self.max_rowid(1)?.saturating_add(1);
        let sql_value = match sql {
            Some(text) => WriteValue::Text(text.to_string()),
            None => WriteValue::Null,
        };
        let record = encode_record(&[
            WriteValue::Text(kind.to_string()),
            WriteValue::Text(name.to_string()),
            WriteValue::Text(table_name.to_string()),
            WriteValue::Integer(i64::from(root_page)),
            sql_value,
        ]);
        let root = self.table_insert(1, rowid, &record)?;
        if root != 1 {
            return Err(Error::Corrupt("sqlite master b-tree must not gain height"));
        }
        Ok(rowid)
    }

    fn set_master_root(&mut self, master_rowid: i64, root_page: u32) -> Result<()> {
        let Some(payload) = self.read_payload(1, master_rowid)?
        else {
            return Err(Error::Corrupt("sqlite master row vanished"));
        };
        let values = decode_record(&payload, TextEncoding::Utf8, Default::default())
            .map_err(|_| Error::Corrupt("sqlite master record decode failed"))?;
        let mut owned: Vec<WriteValue> = values
            .iter()
            .map(|value| match value {
                sqlite_formats::RecordValue::Null => WriteValue::Null,
                sqlite_formats::RecordValue::Integer(number) => WriteValue::Integer(*number),
                sqlite_formats::RecordValue::Real(real) => WriteValue::Real(*real),
                sqlite_formats::RecordValue::Blob(bytes) => WriteValue::Blob(bytes.to_vec()),
                sqlite_formats::RecordValue::Text(text) => WriteValue::Text(text.clone()),
            })
            .collect();
        owned[3] = WriteValue::Integer(i64::from(root_page));
        let record = encode_record(&owned);
        let root = self.table_replace(1, master_rowid, &record)?;
        if root != 1 {
            return Err(Error::Corrupt("sqlite master b-tree must not gain height"));
        }
        Ok(())
    }

    fn table_meta(&self, table: &str) -> Result<TableMeta> {
        let rows = self.master_rows()?;
        let table_row = rows
            .iter()
            .find(|row| row.kind == "table" && eq_ignore_ascii_case(&row.table_name, table))
            .ok_or_else(|| Error::Unsupported("sqlite table does not exist"))?;
        let sql = table_row.sql.as_deref().ok_or(Error::Unsupported("sqlite table has no CREATE text"))?;
        let columns = parse_create_table(sql)?;
        let rowid_alias = columns.iter().position(is_rowid_alias);
        let mut meta = TableMeta {
            table_name: table_row.table_name.clone(),
            master_rowid: table_row.rowid,
            root_page: table_row.root_page,
            columns,
            rowid_alias,
            indexes: Vec::new(),
        };
        for row in rows.iter().filter(|row| row.kind == "index" && eq_ignore_ascii_case(&row.table_name, table)) {
            let (positions, unique) = match row.sql.as_deref() {
                Some(sql) => {
                    let (column_names, unique) = parse_create_index(sql)?;
                    let mut positions = Vec::with_capacity(column_names.len());
                    for name in column_names {
                        let position = meta
                            .columns
                            .iter()
                            .position(|column| eq_ignore_ascii_case(&column.name, &name))
                            .ok_or(Error::Corrupt("sqlite index references unknown column"))?;
                        positions.push(position);
                    }
                    (positions, unique)
                }
                None => {
                    // Automatic constraint indexes store no CREATE text; the
                    // `sqlite_autoindex_<table>_<n>` suffix names the n-th
                    // PK/UNIQUE constraint column in declaration order.
                    let number = row
                        .name
                        .rsplit_once('_')
                        .and_then(|(_, suffix)| suffix.parse::<usize>().ok())
                        .ok_or(Error::Corrupt("sqlite autoindex name unrecognized"))?;
                    let constraint_columns: Vec<usize> = meta
                        .columns
                        .iter()
                        .enumerate()
                        .filter(|(index, column)| (column.primary_key || column.unique) && Some(*index) != rowid_alias)
                        .map(|(index, _)| index)
                        .collect();
                    let position =
                        *constraint_columns.get(number - 1).ok_or(Error::Corrupt("sqlite autoindex number out of range"))?;
                    (vec![position], true)
                }
            };
            meta.indexes.push(IndexMeta { master_rowid: row.rowid, root_page: row.root_page, columns: positions, unique });
        }
        Ok(meta)
    }

    fn bump_cookie(&mut self) {
        let cookie = read_schema_cookie(&self.pages).wrapping_add(1);
        set_schema_cookie(&mut self.pages, cookie);
    }

    // ------------------------------------------------------------------
    // Page mechanics
    // ------------------------------------------------------------------

    fn page_offset(&self, page: u32) -> usize {
        (page as usize - 1) * self.page_size
    }

    fn header_offset(page: u32) -> usize {
        if page == 1 { 100 } else { 0 }
    }

    fn init_leaf_page(&mut self, page: u32, kind: u8) {
        let offset = self.page_offset(page);
        self.pages[offset..offset + self.page_size].fill(0);
        let header = offset + Self::header_offset(page);
        self.pages[header] = kind;
        let content_start = if self.page_size == 65_536 { 1u16 } else { self.page_size as u16 };
        self.pages[header + 5..header + 7].copy_from_slice(&content_start.to_be_bytes());
    }

    fn allocate_page(&mut self) -> Result<u32> {
        let trunk = read_freelist_trunk(&self.pages);
        if trunk != 0 {
            let offset = self.page_offset(trunk);
            let leaf_count = u32::from_be_bytes(self.pages[offset + 4..offset + 8].try_into().unwrap());
            if leaf_count > 0 {
                let slot = offset + 8 + (leaf_count as usize - 1) * 4;
                let leaf = u32::from_be_bytes(self.pages[slot..slot + 4].try_into().unwrap());
                self.pages[offset + 4..offset + 8].copy_from_slice(&(leaf_count - 1).to_be_bytes());
                self.decrement_freelist();
                return Ok(leaf);
            }
            let next = u32::from_be_bytes(self.pages[offset..offset + 4].try_into().unwrap());
            set_freelist_trunk(&mut self.pages, next);
            self.decrement_freelist();
            return Ok(trunk);
        }
        let page = (self.pages.len() / self.page_size) as u32 + 1;
        self.pages.extend(std::iter::repeat_n(0u8, self.page_size));
        set_page_count(&mut self.pages, page);
        Ok(page)
    }

    fn decrement_freelist(&mut self) {
        let count = read_freelist_count(&self.pages);
        set_freelist_count(&mut self.pages, count.saturating_sub(1));
    }

    fn free_page(&mut self, page: u32) {
        let trunk = read_freelist_trunk(&self.pages);
        if trunk != 0 {
            let offset = self.page_offset(trunk);
            let leaf_count = u32::from_be_bytes(self.pages[offset + 4..offset + 8].try_into().unwrap());
            let capacity = (self.usable - 8) / 4;
            if (leaf_count as usize) < capacity {
                let slot = offset + 8 + leaf_count as usize * 4;
                self.pages[slot..slot + 4].copy_from_slice(&page.to_be_bytes());
                self.pages[offset + 4..offset + 8].copy_from_slice(&(leaf_count + 1).to_be_bytes());
                let count = read_freelist_count(&self.pages);
                set_freelist_count(&mut self.pages, count + 1);
                return;
            }
        }
        let offset = self.page_offset(page);
        self.pages[offset..offset + self.page_size].fill(0);
        self.pages[offset..offset + 4].copy_from_slice(&trunk.to_be_bytes());
        set_freelist_trunk(&mut self.pages, page);
        let count = read_freelist_count(&self.pages);
        set_freelist_count(&mut self.pages, count + 1);
    }

    /// Cell bytes plus the span `[start, end)` it occupies on the page.
    fn leaf_cells(&self, page: u32) -> Result<Vec<(i64, usize, Vec<u8>)>> {
        self.cells_common(page, |cell| {
            let (length, length_size) = decode_varint(cell)?;
            let length = usize::try_from(length).map_err(|_| Error::Unsupported("sqlite payload size exceeds host"))?;
            let (rowid, rowid_size) = decode_varint(&cell[length_size..])?;
            Ok(CellInfo { rowid: rowid as i64, prefix: length_size + rowid_size, length })
        })
    }

    fn cells_common<F>(&self, page: u32, mut parse: F) -> Result<Vec<(i64, usize, Vec<u8>)>>
    where
        F: FnMut(&[u8]) -> Result<CellInfo>,
    {
        let offset = self.page_offset(page);
        let page_bytes = &self.pages[offset..offset + self.page_size];
        let header_relative = Self::header_offset(page);
        let kind = page_bytes[header_relative];
        let header_size = match kind {
            PAGE_TABLE_INTERIOR | PAGE_INDEX_INTERIOR => 12,
            PAGE_TABLE_LEAF | PAGE_INDEX_LEAF => 8,
            _ => return Err(Error::Corrupt("sqlite page type invalid")),
        };
        let count = u16::from_be_bytes(page_bytes[header_relative + 3..header_relative + 5].try_into().unwrap()) as usize;
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            let pointer = header_relative + header_size + index * 2;
            let cell_offset = u16::from_be_bytes(page_bytes[pointer..pointer + 2].try_into().unwrap()) as usize;
            let info = parse(&page_bytes[cell_offset..])?;
            let local = self.local_payload(info.length);
            let end = cell_offset + info.prefix + local + if info.length > local { 4 } else { 0 };
            let bytes = page_bytes[cell_offset..end].to_vec();
            out.push((info.rowid, cell_offset, bytes));
        }
        Ok(out)
    }

    fn local_payload(&self, length: usize) -> usize {
        let maximum = self.usable - 35;
        if length <= maximum {
            return length;
        }
        let minimum = ((self.usable - 12) * 32 / 255) - 23;
        let candidate = minimum + (length - minimum) % (self.usable - 4);
        if candidate <= maximum { candidate } else { minimum }
    }

    fn write_overflow(&mut self, payload: &[u8], local: usize) -> Result<u32> {
        let mut remaining = &payload[local..];
        let per_page = self.usable - 4;
        let head = self.allocate_page()?;
        let mut current = head;
        loop {
            let chunk = remaining.len().min(per_page);
            let offset = self.page_offset(current);
            self.pages[offset..offset + self.page_size].fill(0);
            self.pages[offset + 4..offset + 4 + chunk].copy_from_slice(&remaining[..chunk]);
            remaining = &remaining[chunk..];
            if remaining.is_empty() {
                break;
            }
            let next = self.allocate_page()?;
            self.pages[offset..offset + 4].copy_from_slice(&next.to_be_bytes());
            current = next;
        }
        Ok(head)
    }

    fn free_overflow_chain(&mut self, head: u32) {
        let mut next = head;
        while next != 0 {
            let offset = self.page_offset(next);
            let following = u32::from_be_bytes(self.pages[offset..offset + 4].try_into().unwrap());
            self.free_page(next);
            next = following;
        }
    }

    fn build_cell(&mut self, kind: u8, rowid: Option<i64>, payload: &[u8]) -> Result<Vec<u8>> {
        let local = self.local_payload(payload.len());
        let mut cell = Vec::new();
        encode_varint_into(&mut cell, payload.len() as u64);
        if kind == PAGE_TABLE_LEAF {
            encode_varint_into(&mut cell, rowid.expect("table cell carries a rowid") as u64);
        }
        cell.extend_from_slice(&payload[..local]);
        if payload.len() > local {
            let head = self.write_overflow(payload, local)?;
            cell.extend_from_slice(&head.to_be_bytes());
        }
        Ok(cell)
    }

    /// Free space available for new cell bytes plus pointer entries.
    fn free_space(&self, page: u32) -> usize {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let page_bytes = &self.pages[offset..offset + self.page_size];
        let kind = page_bytes[header_relative];
        let header_size = if matches!(kind, PAGE_TABLE_INTERIOR | PAGE_INDEX_INTERIOR) { 12 } else { 8 };
        let count = u16::from_be_bytes(page_bytes[header_relative + 3..header_relative + 5].try_into().unwrap()) as usize;
        let pointers_end = header_relative + header_size + count * 2;
        let raw_start = u16::from_be_bytes(page_bytes[header_relative + 5..header_relative + 7].try_into().unwrap());
        let content_start = if raw_start == 0 { 65_536 } else { raw_start as usize };
        content_start.saturating_sub(pointers_end)
    }

    /// Writes sorted cells into a leaf page from scratch (no freeblocks).
    fn rebuild_leaf(&mut self, page: u32, kind: u8, cells: &[Vec<u8>]) {
        let offset = self.page_offset(page);
        self.pages[offset..offset + self.page_size].fill(0);
        let header_relative = Self::header_offset(page);
        self.pages[offset + header_relative] = kind;
        let count = cells.len();
        self.pages[offset + header_relative + 3..offset + header_relative + 5].copy_from_slice(&(count as u16).to_be_bytes());
        let mut content = if self.page_size == 65_536 { 65_536 } else { self.page_size };
        let pointer_base = header_relative + if matches!(kind, PAGE_TABLE_INTERIOR | PAGE_INDEX_INTERIOR) { 12 } else { 8 };
        for (index, cell) in cells.iter().enumerate() {
            content -= cell.len();
            self.pages[offset + content..offset + content + cell.len()].copy_from_slice(cell);
            let pointer = offset + pointer_base + index * 2;
            self.pages[pointer..pointer + 2].copy_from_slice(&(content as u16).to_be_bytes());
        }
        let content_field = if content == 65_536 { 1u16 } else { content as u16 };
        self.pages[offset + header_relative + 5..offset + header_relative + 7].copy_from_slice(&content_field.to_be_bytes());
    }

    /// Inserts one cell into a leaf without splitting; caller checked space.
    fn place_leaf_cell(&mut self, page: u32, position: usize, cell: &[u8]) {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let kind = self.pages[offset + header_relative];
        let header_size = if matches!(kind, PAGE_TABLE_INTERIOR | PAGE_INDEX_INTERIOR) { 12 } else { 8 };
        let count =
            u16::from_be_bytes(self.pages[offset + header_relative + 3..offset + header_relative + 5].try_into().unwrap())
                as usize;
        let raw_start =
            u16::from_be_bytes(self.pages[offset + header_relative + 5..offset + header_relative + 7].try_into().unwrap());
        let content_start = if raw_start == 0 { 65_536 } else { raw_start as usize };
        let target = content_start - cell.len();
        self.pages[offset + target..offset + target + cell.len()].copy_from_slice(cell);
        let pointer_base = offset + header_relative + header_size;
        // Shift pointers [position..count] right by one slot, then insert.
        let mut pointers = Vec::with_capacity(count + 1);
        for index in 0..count {
            let pointer = pointer_base + index * 2;
            pointers.push(u16::from_be_bytes(self.pages[pointer..pointer + 2].try_into().unwrap()));
        }
        pointers.insert(position, target as u16);
        for (index, pointer) in pointers.iter().enumerate() {
            let slot = pointer_base + index * 2;
            self.pages[slot..slot + 2].copy_from_slice(&pointer.to_be_bytes());
        }
        self.pages[offset + header_relative + 3..offset + header_relative + 5]
            .copy_from_slice(&((count + 1) as u16).to_be_bytes());
        let content_field = if target == 65_536 { 1u16 } else { target as u16 };
        self.pages[offset + header_relative + 5..offset + header_relative + 7].copy_from_slice(&content_field.to_be_bytes());
    }

    // ------------------------------------------------------------------
    // Rowid b-tree
    // ------------------------------------------------------------------

    fn max_rowid(&self, root: u32) -> Result<i64> {
        let rows = read_table(&self.pages, root, TableLimits::default())?;
        Ok(rows.last().map(|row| row.rowid).unwrap_or(0))
    }

    fn contains_rowid(&self, root: u32, rowid: i64) -> Result<bool> {
        Ok(self.read_payload(root, rowid)?.is_some())
    }

    fn read_payload(&self, root: u32, rowid: i64) -> Result<Option<Vec<u8>>> {
        let mut page = root;
        loop {
            let offset = self.page_offset(page);
            let header_relative = Self::header_offset(page);
            let page_bytes = &self.pages[offset..offset + self.page_size];
            let kind = page_bytes[header_relative];
            let count = u16::from_be_bytes(page_bytes[header_relative + 3..header_relative + 5].try_into().unwrap()) as usize;
            match kind {
                PAGE_TABLE_LEAF => {
                    for index in 0..count {
                        let pointer = header_relative + 8 + index * 2;
                        let cell_offset = u16::from_be_bytes(page_bytes[pointer..pointer + 2].try_into().unwrap()) as usize;
                        let (length, length_size) = decode_varint(&page_bytes[cell_offset..])?;
                        let (candidate, rowid_size) = decode_varint(&page_bytes[cell_offset + length_size..])?;
                        if candidate as i64 != rowid {
                            continue;
                        }
                        let length = usize::try_from(length).map_err(|_| Error::Unsupported("sqlite payload too large"))?;
                        let local = self.local_payload(length);
                        let body_start = cell_offset + length_size + rowid_size;
                        let mut payload = page_bytes[body_start..body_start + local].to_vec();
                        if length > local {
                            let mut next =
                                u32::from_be_bytes(page_bytes[body_start + local..body_start + local + 4].try_into().unwrap());
                            while payload.len() < length {
                                let overflow_offset = self.page_offset(next);
                                let chunk = (length - payload.len()).min(self.usable - 4);
                                payload.extend_from_slice(&self.pages[overflow_offset + 4..overflow_offset + 4 + chunk]);
                                next = u32::from_be_bytes(self.pages[overflow_offset..overflow_offset + 4].try_into().unwrap());
                            }
                        }
                        return Ok(Some(payload));
                    }
                    return Ok(None);
                }
                PAGE_TABLE_INTERIOR => {
                    let mut child =
                        u32::from_be_bytes(page_bytes[header_relative + 8..header_relative + 12].try_into().unwrap());
                    for index in 0..count {
                        let pointer = header_relative + 12 + index * 2;
                        let cell_offset = u16::from_be_bytes(page_bytes[pointer..pointer + 2].try_into().unwrap()) as usize;
                        let candidate = u32::from_be_bytes(page_bytes[cell_offset..cell_offset + 4].try_into().unwrap());
                        let (key, _) = decode_varint(&page_bytes[cell_offset + 4..])?;
                        if rowid <= key as i64 {
                            child = candidate;
                            break;
                        }
                    }
                    page = child;
                }
                _ => return Err(Error::Corrupt("sqlite table b-tree page type invalid")),
            }
        }
    }

    fn table_insert(&mut self, page: u32, rowid: i64, payload: &[u8]) -> Result<u32> {
        match self.table_insert_inner(page, rowid, payload)? {
            Some(split) => {
                let new_root = self.allocate_page()?;
                self.init_leaf_page(new_root, PAGE_TABLE_INTERIOR);
                let mut cells = Vec::with_capacity(split.boundaries.len());
                let mut left = page;
                for (key, right) in split.boundaries.iter() {
                    cells.push(interior_cell(left, *key));
                    left = *right;
                }
                self.rebuild_leaf(new_root, PAGE_TABLE_INTERIOR, &cells);
                let offset = self.page_offset(new_root);
                let header_relative = Self::header_offset(new_root);
                self.pages[offset + header_relative + 8..offset + header_relative + 12].copy_from_slice(&left.to_be_bytes());
                Ok(new_root)
            }
            None => Ok(page),
        }
    }

    fn table_insert_inner(&mut self, page: u32, rowid: i64, payload: &[u8]) -> Result<Option<Split>> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let kind = self.pages[offset + header_relative];
        match kind {
            PAGE_TABLE_LEAF => {
                let cells = self.leaf_cells(page)?;
                if cells.iter().any(|(candidate, _, _)| *candidate == rowid) {
                    return Err(Error::Unsupported("sqlite duplicate rowid"));
                }
                let cell = self.build_cell(PAGE_TABLE_LEAF, Some(rowid), payload)?;
                let position = cells.partition_point(|(candidate, _, _)| *candidate < rowid);
                if cell.len() + 2 <= self.free_space(page) {
                    self.place_leaf_cell(page, position, &cell);
                    return Ok(None);
                }
                // Split: pack the sorted cells into as many pages as the
                // payload sizes demand (one oversized overflow cell can make a
                // count-based half-page overflow its page).
                let mut all: Vec<(i64, Vec<u8>)> = cells.into_iter().map(|(key, _, bytes)| (key, bytes)).collect();
                all.insert(position, (rowid, cell));
                let capacity = self.usable - 8;
                let groups = pack_groups(all, capacity, |(_, bytes)| bytes.len() + 2);
                if groups.len() < 2 {
                    // Freeblocks fragmented the free space; one compacted page
                    // holds everything again.
                    let only = groups.into_iter().next().expect("one group");
                    self.rebuild_leaf(page, PAGE_TABLE_LEAF, &only.iter().map(|(_, bytes)| bytes.clone()).collect::<Vec<_>>());
                    return Ok(None);
                }
                let mut iter = groups.into_iter();
                let first = iter.next().expect("at least two groups");
                let mut previous_max = first.last().map(|(key, _)| *key).expect("group keeps its cells");
                self.rebuild_leaf(page, PAGE_TABLE_LEAF, &first.iter().map(|(_, bytes)| bytes.clone()).collect::<Vec<_>>());
                let mut boundaries = Vec::new();
                for group in iter {
                    let new_page = self.allocate_page()?;
                    self.rebuild_leaf(
                        new_page,
                        PAGE_TABLE_LEAF,
                        &group.iter().map(|(_, bytes)| bytes.clone()).collect::<Vec<_>>(),
                    );
                    boundaries.push((previous_max, new_page));
                    previous_max = group.last().map(|(key, _)| *key).expect("group keeps its cells");
                }
                Ok(Some(Split { boundaries }))
            }
            PAGE_TABLE_INTERIOR => {
                let count = u16::from_be_bytes(
                    self.pages[offset + header_relative + 3..offset + header_relative + 5].try_into().unwrap(),
                ) as usize;
                let mut child_index = None;
                let mut child = u32::from_be_bytes(
                    self.pages[offset + header_relative + 8..offset + header_relative + 12].try_into().unwrap(),
                );
                for index in 0..count {
                    let pointer = header_relative + 12 + index * 2;
                    let cell_offset =
                        u16::from_be_bytes(self.pages[offset + pointer..offset + pointer + 2].try_into().unwrap()) as usize;
                    let candidate =
                        u32::from_be_bytes(self.pages[offset + cell_offset..offset + cell_offset + 4].try_into().unwrap());
                    let (key, _) = decode_varint(&self.pages[offset + cell_offset + 4..])?;
                    if rowid <= key as i64 {
                        child = candidate;
                        child_index = Some(index);
                        break;
                    }
                }
                let Some(split) = self.table_insert_inner(child, rowid, payload)?
                else {
                    return Ok(None);
                };
                self.interior_accept(page, child_index, child, split.boundaries)
            }
            _ => Err(Error::Corrupt("sqlite table b-tree page type invalid")),
        }
    }

    /// Integrates a child's split boundaries into this interior page (may
    /// split this page in turn). The child keeps every key up to the first
    /// boundary; each boundary page covers the keys between its neighbours.
    fn interior_accept(
        &mut self,
        page: u32,
        child_index: Option<usize>,
        child: u32,
        boundaries: Vec<(i64, u32)>,
    ) -> Result<Option<Split>> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let kind = self.pages[offset + header_relative];
        let count =
            u16::from_be_bytes(self.pages[offset + header_relative + 3..offset + header_relative + 5].try_into().unwrap())
                as usize;

        // Collect existing cells as (child pointer, key, bytes).
        let mut cells: Vec<(u32, i64, Vec<u8>)> = Vec::with_capacity(count + boundaries.len() + 1);
        for index in 0..count {
            let pointer = header_relative + 12 + index * 2;
            let cell_offset =
                u16::from_be_bytes(self.pages[offset + pointer..offset + pointer + 2].try_into().unwrap()) as usize;
            let candidate = u32::from_be_bytes(self.pages[offset + cell_offset..offset + cell_offset + 4].try_into().unwrap());
            let (key, consumed) = decode_varint(&self.pages[offset + cell_offset + 4..])?;
            let end = cell_offset + 4 + consumed;
            cells.push((candidate, key as i64, self.pages[offset + cell_offset..offset + end].to_vec()));
        }
        let mut rightmost =
            u32::from_be_bytes(self.pages[offset + header_relative + 8..offset + header_relative + 12].try_into().unwrap());

        let last_page = boundaries.last().expect("split carries at least one boundary").1;
        match child_index {
            Some(index) => {
                let old_key = cells[index].1;
                let first_key = boundaries[0].0;
                cells[index].1 = first_key;
                cells[index].2 = interior_cell(child, first_key);
                let mut insert_at = index + 1;
                for window in boundaries.windows(2) {
                    let (upper_page, upper_key) = (window[0].1, window[1].0);
                    cells.insert(insert_at, (upper_page, upper_key, interior_cell(upper_page, upper_key)));
                    insert_at += 1;
                }
                cells.insert(insert_at, (last_page, old_key, interior_cell(last_page, old_key)));
            }
            None => {
                let first_key = boundaries[0].0;
                cells.push((child, first_key, interior_cell(child, first_key)));
                for window in boundaries.windows(2) {
                    let (upper_page, upper_key) = (window[0].1, window[1].0);
                    cells.push((upper_page, upper_key, interior_cell(upper_page, upper_key)));
                }
                rightmost = last_page;
            }
        }

        let total: usize = cells.iter().map(|(_, _, bytes)| bytes.len() + 2).sum();
        let header_size = 12usize;
        if total + header_size <= self.usable {
            // Fits: rewrite the page in place.
            let rebuilt: Vec<Vec<u8>> = cells.iter().map(|(_, _, bytes)| bytes.clone()).collect();
            self.rebuild_leaf(page, kind, &rebuilt);
            self.pages[offset + header_relative + 8..offset + header_relative + 12].copy_from_slice(&rightmost.to_be_bytes());
            return Ok(None);
        }

        // Split this interior page: the middle cell moves up.
        let mid = cells.len() / 2;
        let moved = cells[mid].clone();
        let right_cells: Vec<Vec<u8>> = cells[mid + 1..].iter().map(|(_, _, bytes)| bytes.clone()).collect();
        let right_page = self.allocate_page()?;
        self.rebuild_leaf(right_page, kind, &right_cells);
        let right_offset = self.page_offset(right_page);
        let right_header = Self::header_offset(right_page);
        self.pages[right_offset + right_header + 8..right_offset + right_header + 12].copy_from_slice(&rightmost.to_be_bytes());
        let left_cells: Vec<Vec<u8>> = cells[..mid].iter().map(|(_, _, bytes)| bytes.clone()).collect();
        self.rebuild_leaf(page, kind, &left_cells);
        self.pages[offset + header_relative + 8..offset + header_relative + 12].copy_from_slice(&moved.0.to_be_bytes());
        Ok(Some(Split { boundaries: vec![(moved.1, right_page)] }))
    }

    fn table_delete(&mut self, page: u32, rowid: i64) -> Result<u32> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let kind = self.pages[offset + header_relative];
        match kind {
            PAGE_TABLE_LEAF => {
                let cells = self.leaf_cells(page)?;
                let Some(position) = cells.iter().position(|(candidate, _, _)| *candidate == rowid)
                else {
                    return Err(Error::Unsupported("sqlite rowid not found"));
                };
                self.remove_cell(page, position)?;
                Ok(page)
            }
            PAGE_TABLE_INTERIOR => {
                let count = u16::from_be_bytes(
                    self.pages[offset + header_relative + 3..offset + header_relative + 5].try_into().unwrap(),
                ) as usize;
                let mut child_index = None;
                let mut child = u32::from_be_bytes(
                    self.pages[offset + header_relative + 8..offset + header_relative + 12].try_into().unwrap(),
                );
                for index in 0..count {
                    let pointer = header_relative + 12 + index * 2;
                    let cell_offset =
                        u16::from_be_bytes(self.pages[offset + pointer..offset + pointer + 2].try_into().unwrap()) as usize;
                    let candidate =
                        u32::from_be_bytes(self.pages[offset + cell_offset..offset + cell_offset + 4].try_into().unwrap());
                    let (key, _) = decode_varint(&self.pages[offset + cell_offset + 4..])?;
                    if rowid <= key as i64 {
                        child = candidate;
                        child_index = Some(index);
                        break;
                    }
                }
                let root = self.table_delete(child, rowid)?;
                if root != child {
                    // Child shrank to nothing and was freed; repoint.
                    self.repoint_child(page, child_index, child, root)?;
                }
                else {
                    self.reclaim_empty_leaf(page, child_index, child)?;
                }
                // Collapse an interior root that lost all cells.
                let offset = self.page_offset(page);
                let header_relative = Self::header_offset(page);
                let count = u16::from_be_bytes(
                    self.pages[offset + header_relative + 3..offset + header_relative + 5].try_into().unwrap(),
                );
                if count == 0 && page != 1 {
                    let rightmost = u32::from_be_bytes(
                        self.pages[offset + header_relative + 8..offset + header_relative + 12].try_into().unwrap(),
                    );
                    self.free_page(page);
                    return Ok(rightmost);
                }
                Ok(page)
            }
            _ => Err(Error::Corrupt("sqlite table b-tree page type invalid")),
        }
    }

    /// Frees a child leaf that lost its last cell and drops the routing entry
    /// in this interior page. A leaf that is the page's only routing stays
    /// attached so the subtree never dangles.
    fn reclaim_empty_leaf(&mut self, page: u32, child_index: Option<usize>, child: u32) -> Result<()> {
        if !self.is_empty_leaf(child)? {
            return Ok(());
        }
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let count =
            u16::from_be_bytes(self.pages[offset + header_relative + 3..offset + header_relative + 5].try_into().unwrap())
                as usize;
        if count == 0 {
            return Ok(());
        }
        self.free_page(child);

        // Rebuild the interior page without the routing entry for `child`.
        let mut cells: Vec<(u32, i64)> = Vec::with_capacity(count);
        for index in 0..count {
            let pointer = header_relative + 12 + index * 2;
            let cell_offset =
                u16::from_be_bytes(self.pages[offset + pointer..offset + pointer + 2].try_into().unwrap()) as usize;
            let candidate = u32::from_be_bytes(self.pages[offset + cell_offset..offset + cell_offset + 4].try_into().unwrap());
            let (key, _) = decode_varint(&self.pages[offset + cell_offset + 4..])?;
            cells.push((candidate, key as i64));
        }
        let mut rightmost =
            u32::from_be_bytes(self.pages[offset + header_relative + 8..offset + header_relative + 12].try_into().unwrap());
        match child_index {
            Some(index) => {
                cells.remove(index);
            }
            None => {
                // The freed leaf was the rightmost pointer; the last cell's
                // child takes over the rightmost slot.
                let (last_child, _) = cells.pop().expect("interior page has at least one cell");
                rightmost = last_child;
            }
        }
        let rebuilt: Vec<Vec<u8>> = cells.iter().map(|(child_page, key)| interior_cell(*child_page, *key)).collect();
        self.rebuild_leaf(page, PAGE_TABLE_INTERIOR, &rebuilt);
        self.pages[offset + header_relative + 8..offset + header_relative + 12].copy_from_slice(&rightmost.to_be_bytes());
        Ok(())
    }

    fn is_empty_leaf(&self, page: u32) -> Result<bool> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let kind = self.pages[offset + header_relative];
        if kind != PAGE_TABLE_LEAF {
            return Ok(false);
        }
        let count =
            u16::from_be_bytes(self.pages[offset + header_relative + 3..offset + header_relative + 5].try_into().unwrap());
        Ok(count == 0)
    }

    fn repoint_child(&mut self, page: u32, child_index: Option<usize>, old_child: u32, new_child: u32) -> Result<()> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        match child_index {
            Some(index) => {
                let pointer = header_relative + 12 + index * 2;
                let cell_offset =
                    u16::from_be_bytes(self.pages[offset + pointer..offset + pointer + 2].try_into().unwrap()) as usize;
                let stored = u32::from_be_bytes(self.pages[offset + cell_offset..offset + cell_offset + 4].try_into().unwrap());
                if stored != old_child {
                    return Err(Error::Corrupt("sqlite interior child pointer mismatch"));
                }
                self.pages[offset + cell_offset..offset + cell_offset + 4].copy_from_slice(&new_child.to_be_bytes());
            }
            None => {
                self.pages[offset + header_relative + 8..offset + header_relative + 12]
                    .copy_from_slice(&new_child.to_be_bytes());
            }
        }
        Ok(())
    }

    /// Removes cell `position` from a leaf page, maintaining the freeblock chain.
    fn remove_cell(&mut self, page: u32, position: usize) -> Result<()> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let kind = self.pages[offset + header_relative];
        let header_size = if matches!(kind, PAGE_TABLE_INTERIOR | PAGE_INDEX_INTERIOR) { 12 } else { 8 };
        let count =
            u16::from_be_bytes(self.pages[offset + header_relative + 3..offset + header_relative + 5].try_into().unwrap())
                as usize;
        if position >= count {
            return Err(Error::Corrupt("sqlite cell position out of range"));
        }

        // Locate the cell span including its overflow pages. Index interior
        // cells carry a 4-byte child pointer before the payload-length varint.
        let pointer = header_relative + header_size + position * 2;
        let cell_offset = u16::from_be_bytes(self.pages[offset + pointer..offset + pointer + 2].try_into().unwrap()) as usize;
        let child_bytes = if kind == PAGE_INDEX_INTERIOR { 4 } else { 0 };
        let (length, length_size) = decode_varint(&self.pages[offset + cell_offset + child_bytes..])?;
        let length = usize::try_from(length).map_err(|_| Error::Unsupported("sqlite payload too large"))?;
        let prefix = child_bytes
            + match kind {
                PAGE_TABLE_LEAF => {
                    let (_, rowid_size) = decode_varint(&self.pages[offset + cell_offset + length_size..])?;
                    length_size + rowid_size
                }
                _ => length_size,
            };
        let local = self.local_payload(length);
        let span = prefix + local + if length > local { 4 } else { 0 };
        if length > local {
            let head = u32::from_be_bytes(
                self.pages[offset + cell_offset + prefix + local..offset + cell_offset + prefix + local + 4]
                    .try_into()
                    .unwrap(),
            );
            self.free_overflow_chain(head);
        }

        // Drop the pointer entry.
        let pointer_base = offset + header_relative + header_size;
        let mut pointers = Vec::with_capacity(count - 1);
        for index in 0..count {
            if index == position {
                continue;
            }
            let slot = pointer_base + index * 2;
            pointers.push(u16::from_be_bytes(self.pages[slot..slot + 2].try_into().unwrap()));
        }
        for (index, value) in pointers.iter().enumerate() {
            let slot = pointer_base + index * 2;
            self.pages[slot..slot + 2].copy_from_slice(&value.to_be_bytes());
        }
        self.pages[offset + header_relative + 3..offset + header_relative + 5]
            .copy_from_slice(&((count - 1) as u16).to_be_bytes());

        self.add_freeblock(page, cell_offset, span);
        Ok(())
    }

    /// Inserts `[start, start+size)` into the page's ascending freeblock chain,
    /// coalescing with touching neighbours.
    fn add_freeblock(&mut self, page: u32, start: usize, size: usize) {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let mut first =
            u16::from_be_bytes(self.pages[offset + header_relative + 1..offset + header_relative + 3].try_into().unwrap())
                as usize;

        let mut blocks: Vec<(usize, usize)> = Vec::new();
        let mut cursor = first;
        while cursor != 0 {
            let next = u16::from_be_bytes(self.pages[offset + cursor..offset + cursor + 2].try_into().unwrap()) as usize;
            let block_size =
                u16::from_be_bytes(self.pages[offset + cursor + 2..offset + cursor + 4].try_into().unwrap()) as usize;
            blocks.push((cursor, block_size));
            cursor = next;
        }
        blocks.push((start, size));
        blocks.sort_unstable_by_key(|(block_start, _)| *block_start);

        let mut coalesced: Vec<(usize, usize)> = Vec::new();
        for block in blocks {
            if let Some(last) = coalesced.last_mut() {
                if last.0 + last.1 >= block.0 {
                    last.1 = (block.0 + block.1).saturating_sub(last.0).max(last.1);
                    continue;
                }
            }
            coalesced.push(block);
        }

        // Rewrite the chain from scratch (blocks are small and few).
        let mut previous = 0usize;
        for (index, (block_start, block_size)) in coalesced.iter().enumerate() {
            let next = if index + 1 < coalesced.len() { coalesced[index + 1].0 } else { 0 };
            self.pages[offset + block_start..offset + block_start + 2].copy_from_slice(&(next as u16).to_be_bytes());
            self.pages[offset + block_start + 2..offset + block_start + 4].copy_from_slice(&(*block_size as u16).to_be_bytes());
            if previous == 0 {
                first = *block_start;
            }
            previous = *block_start;
        }
        if coalesced.is_empty() {
            first = 0;
        }
        self.pages[offset + header_relative + 1..offset + header_relative + 3].copy_from_slice(&(first as u16).to_be_bytes());
    }

    /// Replaces the payload of an existing rowid cell in place when it fits,
    /// otherwise deletes and reinserts.
    fn table_replace(&mut self, root: u32, rowid: i64, payload: &[u8]) -> Result<u32> {
        self.table_delete(root, rowid)?;
        self.table_insert(root, rowid, payload)
    }

    // ------------------------------------------------------------------
    // Index b-tree
    // ------------------------------------------------------------------

    /// Index pages route strictly: keys below a cell key descend into that
    /// cell's child, keys at or above continue right. The divider of a split
    /// is a real entry that MOVES up to the parent and leaves both halves,
    /// exactly like upstream SQLite.
    fn index_insert(&mut self, root: u32, key: &[u8]) -> Result<u32> {
        match self.index_insert_inner(root, key)? {
            Some(split) => {
                let new_root = self.allocate_page()?;
                self.init_leaf_page(new_root, PAGE_INDEX_INTERIOR);
                let mut cells = Vec::with_capacity(split.dividers.len());
                let mut left = root;
                for (divider, page) in split.dividers.iter().zip(split.pages.iter()) {
                    cells.push(index_interior_cell(left, divider));
                    left = *page;
                }
                self.rebuild_leaf(new_root, PAGE_INDEX_INTERIOR, &cells);
                let offset = self.page_offset(new_root);
                let header_relative = Self::header_offset(new_root);
                self.pages[offset + header_relative + 8..offset + header_relative + 12].copy_from_slice(&left.to_be_bytes());
                Ok(new_root)
            }
            None => Ok(root),
        }
    }

    fn index_insert_inner(&mut self, page: u32, key: &[u8]) -> Result<Option<IndexSplit>> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let kind = self.pages[offset + header_relative];
        match kind {
            PAGE_INDEX_LEAF => {
                let entries = self.index_leaf_entries(page)?;
                let position = entries.partition_point(|(payload, _)| compare_index_payloads(payload, key).is_lt());
                if entries.get(position).is_some_and(|(payload, _)| payload.as_slice() == key) {
                    return Err(Error::Unsupported("sqlite duplicate index key"));
                }
                let cell = self.build_cell(PAGE_INDEX_LEAF, None, key)?;
                if cell.len() + 2 <= self.free_space(page) {
                    self.place_leaf_cell(page, position, &cell);
                    return Ok(None);
                }
                // Pack into as many pages as the payload sizes demand, then
                // move one divider entry per boundary up to the parent.
                let mut all: Vec<Vec<u8>> = entries.into_iter().map(|(_, bytes)| bytes).collect();
                all.insert(position, cell);
                let capacity = self.usable - 8;
                let mut groups = pack_groups(all, capacity, |bytes| bytes.len() + 2);
                if groups.len() < 2 {
                    // Freeblocks fragmented the free space; one compacted page
                    // holds everything again.
                    let only = groups.into_iter().next().expect("one group");
                    self.rebuild_leaf(page, PAGE_INDEX_LEAF, &only);
                    return Ok(None);
                }
                let mut dividers = Vec::with_capacity(groups.len() - 1);
                for boundary in 1..groups.len() {
                    let divider_cell = if groups[boundary].len() >= 2 {
                        groups[boundary].remove(0)
                    }
                    else if groups[boundary - 1].len() >= 2 {
                        groups[boundary - 1].pop().expect("checked length")
                    }
                    else {
                        return Err(Error::Unsupported("sqlite index cell too large to split"));
                    };
                    dividers.push(decode_cell_payload(&divider_cell)?);
                }
                let mut pages = Vec::with_capacity(groups.len() - 1);
                let mut iter = groups.into_iter();
                let first = iter.next().expect("at least two groups");
                for group in iter {
                    let new_page = self.allocate_page()?;
                    self.rebuild_leaf(new_page, PAGE_INDEX_LEAF, &group);
                    pages.push(new_page);
                }
                self.rebuild_leaf(page, PAGE_INDEX_LEAF, &first);
                Ok(Some(IndexSplit { dividers, pages }))
            }
            PAGE_INDEX_INTERIOR => {
                let entries = self.index_interior_entries(page)?;
                let mut child_index = None;
                let mut child = entries.rightmost;
                for (index, (payload, child_page)) in entries.cells.iter().enumerate() {
                    match compare_index_payloads(key, payload) {
                        std::cmp::Ordering::Less => {
                            child = *child_page;
                            child_index = Some(index);
                            break;
                        }
                        std::cmp::Ordering::Equal => {
                            return Err(Error::Unsupported("sqlite duplicate index key"));
                        }
                        std::cmp::Ordering::Greater => continue,
                    }
                }
                let Some(split) = self.index_insert_inner(child, key)?
                else {
                    return Ok(None);
                };
                self.index_interior_accept(page, child_index, child, split)
            }
            _ => Err(Error::Corrupt("sqlite index b-tree page type invalid")),
        }
    }

    /// Integrates a child split into this interior index page. Divider entries
    /// move up into this page; each group page covers the keys between its
    /// neighbouring dividers.
    fn index_interior_accept(
        &mut self,
        page: u32,
        child_index: Option<usize>,
        child: u32,
        split: IndexSplit,
    ) -> Result<Option<IndexSplit>> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let entries = self.index_interior_entries(page)?;
        let mut pairs: Vec<(Vec<u8>, u32)> = entries.cells;
        let mut rightmost = entries.rightmost;
        let last_page = *split.pages.last().expect("split carries at least one page");
        match child_index {
            Some(index) => {
                let old_payload = pairs[index].0.clone();
                pairs[index] = (split.dividers[0].clone(), child);
                let mut insert_at = index + 1;
                for (window_index, window) in split.dividers.windows(2).enumerate() {
                    pairs.insert(insert_at, (window[1].clone(), split.pages[window_index + 1]));
                    insert_at += 1;
                }
                pairs.insert(insert_at, (old_payload, last_page));
            }
            None => {
                pairs.push((split.dividers[0].clone(), child));
                for (window_index, window) in split.dividers.windows(2).enumerate() {
                    pairs.push((window[1].clone(), split.pages[window_index + 1]));
                }
                rightmost = last_page;
            }
        }
        let cells: Vec<Vec<u8>> = pairs.iter().map(|(payload, child_page)| index_interior_cell(*child_page, payload)).collect();
        let total: usize = cells.iter().map(|bytes| bytes.len() + 2).sum();
        if total + 12 <= self.usable {
            self.rebuild_leaf(page, PAGE_INDEX_INTERIOR, &cells);
            self.pages[offset + header_relative + 8..offset + header_relative + 12].copy_from_slice(&rightmost.to_be_bytes());
            return Ok(None);
        }
        // Split this interior page: the middle cell (an entry itself) moves up.
        let mid = cells.len() / 2;
        let moved_payload = pairs[mid].0.clone();
        let moved_child = pairs[mid].1;
        let right_pairs: Vec<(Vec<u8>, u32)> = pairs[mid + 1..].to_vec();
        let right_cells: Vec<Vec<u8>> =
            right_pairs.iter().map(|(payload, child_page)| index_interior_cell(*child_page, payload)).collect();
        let right_page = self.allocate_page()?;
        self.rebuild_leaf(right_page, PAGE_INDEX_INTERIOR, &right_cells);
        let right_offset = self.page_offset(right_page);
        let right_header = Self::header_offset(right_page);
        self.pages[right_offset + right_header + 8..right_offset + right_header + 12].copy_from_slice(&rightmost.to_be_bytes());
        let left_cells: Vec<Vec<u8>> = cells[..mid].to_vec();
        self.rebuild_leaf(page, PAGE_INDEX_INTERIOR, &left_cells);
        self.pages[offset + header_relative + 8..offset + header_relative + 12].copy_from_slice(&moved_child.to_be_bytes());
        Ok(Some(IndexSplit { dividers: vec![moved_payload], pages: vec![right_page] }))
    }

    fn index_leaf_entries(&self, page: u32) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let page_bytes = &self.pages[offset..offset + self.page_size];
        let count = u16::from_be_bytes(page_bytes[header_relative + 3..header_relative + 5].try_into().unwrap()) as usize;
        let mut out = Vec::with_capacity(count);
        for index in 0..count {
            let pointer = header_relative + 8 + index * 2;
            let cell_offset = u16::from_be_bytes(page_bytes[pointer..pointer + 2].try_into().unwrap()) as usize;
            let (length, length_size) = decode_varint(&page_bytes[cell_offset..])?;
            let length = usize::try_from(length).map_err(|_| Error::Unsupported("sqlite payload too large"))?;
            let local = self.local_payload(length);
            let body_start = cell_offset + length_size;
            let mut payload = page_bytes[body_start..body_start + local].to_vec();
            if length > local {
                let mut next = u32::from_be_bytes(page_bytes[body_start + local..body_start + local + 4].try_into().unwrap());
                while payload.len() < length {
                    let overflow_offset = self.page_offset(next);
                    let chunk = (length - payload.len()).min(self.usable - 4);
                    payload.extend_from_slice(&self.pages[overflow_offset + 4..overflow_offset + 4 + chunk]);
                    next = u32::from_be_bytes(self.pages[overflow_offset..overflow_offset + 4].try_into().unwrap());
                }
            }
            let end = body_start + local + if length > local { 4 } else { 0 };
            out.push((payload, page_bytes[cell_offset..end].to_vec()));
        }
        Ok(out)
    }

    fn index_interior_entries(&self, page: u32) -> Result<InteriorEntries> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let page_bytes = &self.pages[offset..offset + self.page_size];
        let count = u16::from_be_bytes(page_bytes[header_relative + 3..header_relative + 5].try_into().unwrap()) as usize;
        let mut cells = Vec::with_capacity(count);
        for index in 0..count {
            let pointer = header_relative + 12 + index * 2;
            let cell_offset = u16::from_be_bytes(page_bytes[pointer..pointer + 2].try_into().unwrap()) as usize;
            let child_page = u32::from_be_bytes(page_bytes[cell_offset..cell_offset + 4].try_into().unwrap());
            let (length, length_size) = decode_varint(&page_bytes[cell_offset + 4..])?;
            let length = usize::try_from(length).map_err(|_| Error::Unsupported("sqlite payload too large"))?;
            let local = self.local_payload(length);
            let body_start = cell_offset + 4 + length_size;
            let mut payload = page_bytes[body_start..body_start + local].to_vec();
            if length > local {
                let mut next = u32::from_be_bytes(page_bytes[body_start + local..body_start + local + 4].try_into().unwrap());
                while payload.len() < length {
                    let overflow_offset = self.page_offset(next);
                    let chunk = (length - payload.len()).min(self.usable - 4);
                    payload.extend_from_slice(&self.pages[overflow_offset + 4..overflow_offset + 4 + chunk]);
                    next = u32::from_be_bytes(self.pages[overflow_offset..overflow_offset + 4].try_into().unwrap());
                }
            }
            cells.push((payload, child_page));
        }
        let rightmost = u32::from_be_bytes(page_bytes[header_relative + 8..header_relative + 12].try_into().unwrap());
        Ok(InteriorEntries { cells, rightmost })
    }

    /// UNIQUE constraint probe against the live key set of one index.
    fn set_prefix_match(
        &self,
        master_rowid: i64,
        key: &[u8],
        prefix_columns: usize,
        excluded_rowid: Option<i64>,
    ) -> Result<bool> {
        let wanted = decode_record(key, TextEncoding::Utf8, Default::default())
            .map_err(|_| Error::Corrupt("sqlite index record decode failed"))?;
        let Some(entries) = self.index_entries.get(&master_rowid)
        else {
            return Ok(false);
        };
        for payload in entries {
            if self.prefix_matches(payload, &wanted, prefix_columns, excluded_rowid)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Collects every entry of an index b-tree into an ordered set. Used when
    /// opening an existing file: the tree becomes the live key set and is only
    /// materialized back into pages on save.
    fn read_index_entries(&self, root: u32) -> Result<std::collections::BTreeSet<Vec<u8>>> {
        let mut out = std::collections::BTreeSet::new();
        let mut pending = vec![root];
        while let Some(page) = pending.pop() {
            let offset = self.page_offset(page);
            let header_relative = Self::header_offset(page);
            let kind = self.pages[offset + header_relative];
            match kind {
                PAGE_INDEX_LEAF => {
                    for (payload, _) in self.index_leaf_entries(page)? {
                        out.insert(payload);
                    }
                }
                PAGE_INDEX_INTERIOR => {
                    let entries = self.index_interior_entries(page)?;
                    for (payload, child_page) in entries.cells {
                        out.insert(payload);
                        pending.push(child_page);
                    }
                    pending.push(entries.rightmost);
                }
                _ => return Err(Error::Corrupt("sqlite index b-tree page type invalid")),
            }
        }
        Ok(out)
    }

    /// Frees every page of an index subtree, including overflow chains.
    fn free_subtree(&mut self, page: u32) -> Result<()> {
        let offset = self.page_offset(page);
        let header_relative = Self::header_offset(page);
        let kind = self.pages[offset + header_relative];
        match kind {
            PAGE_INDEX_LEAF => {
                for (_, cell) in self.index_leaf_entries(page)? {
                    let (length, length_size) = decode_varint(&cell)?;
                    let length = usize::try_from(length).map_err(|_| Error::Unsupported("sqlite payload too large"))?;
                    let local = self.local_payload(length);
                    if length > local {
                        let head = u32::from_be_bytes(cell[length_size + local..length_size + local + 4].try_into().unwrap());
                        self.free_overflow_chain(head);
                    }
                }
                self.free_page(page);
                Ok(())
            }
            PAGE_INDEX_INTERIOR => {
                let entries = self.index_interior_entries(page)?;
                for (_, child_page) in &entries.cells {
                    self.free_subtree(*child_page)?;
                }
                self.free_subtree(entries.rightmost)?;
                self.free_page(page);
                Ok(())
            }
            _ => Err(Error::Corrupt("sqlite index b-tree page type invalid")),
        }
    }

    /// Rebuilds every index b-tree from its live key set. The old pages are
    /// returned to the freelist before the fresh tree is allocated so the
    /// file does not grow across repeated saves.
    fn materialize_indexes(&mut self) -> Result<()> {
        let rows = self.master_rows()?;
        for row in rows.iter().filter(|row| row.kind == "index") {
            let entries = self.index_entries.get(&row.rowid).cloned().unwrap_or_default();
            if row.root_page != 0 {
                self.free_subtree(row.root_page)?;
            }
            let mut root = self.allocate_page()?;
            self.init_leaf_page(root, PAGE_INDEX_LEAF);
            for key in &entries {
                root = self.index_insert(root, key)?;
            }
            if root != row.root_page {
                self.set_master_root(row.rowid, root)?;
            }
        }
        Ok(())
    }

    fn prefix_matches(
        &self,
        payload: &[u8],
        wanted: &[sqlite_formats::RecordValue<'_>],
        prefix_columns: usize,
        excluded_rowid: Option<i64>,
    ) -> Result<bool> {
        let candidate = decode_record(payload, TextEncoding::Utf8, Default::default())
            .map_err(|_| Error::Corrupt("sqlite index record decode failed"))?;
        if !prefix_equal(&candidate, wanted, prefix_columns) {
            return Ok(false);
        }
        if let Some(excluded) = excluded_rowid {
            if candidate
                .last()
                .is_some_and(|value| matches!(value, sqlite_formats::RecordValue::Integer(rowid) if *rowid == excluded))
            {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

struct CellInfo {
    rowid: i64,
    prefix: usize,
    length: usize,
}

struct IndexSplit {
    /// Divider entries moved up to the parent, between consecutive groups.
    dividers: Vec<Vec<u8>>,
    /// Pages holding the groups after the first, aligned with `dividers`.
    pages: Vec<u32>,
}

struct InteriorEntries {
    cells: Vec<(Vec<u8>, u32)>,
    rightmost: u32,
}

fn table_name_of(meta: &TableMeta) -> String {
    meta.table_name.clone()
}

fn interior_cell(child: u32, key: i64) -> Vec<u8> {
    let mut cell = Vec::with_capacity(16);
    cell.extend_from_slice(&child.to_be_bytes());
    encode_varint_into(&mut cell, key as u64);
    cell
}

fn index_interior_cell(child: u32, payload: &[u8]) -> Vec<u8> {
    let mut cell = Vec::with_capacity(payload.len() + 8);
    cell.extend_from_slice(&child.to_be_bytes());
    encode_varint_into(&mut cell, payload.len() as u64);
    cell.extend_from_slice(payload);
    cell
}

fn decode_cell_payload(cell: &[u8]) -> Result<Vec<u8>> {
    let (length, length_size) = decode_varint(cell)?;
    let length = usize::try_from(length).map_err(|_| Error::Unsupported("sqlite payload too large"))?;
    Ok(cell[length_size..length_size + length].to_vec())
}

fn compare_index_payloads(left: &[u8], right: &[u8]) -> std::cmp::Ordering {
    let left_values = decode_record(left, TextEncoding::Utf8, Default::default()).unwrap_or_default();
    let right_values = decode_record(right, TextEncoding::Utf8, Default::default()).unwrap_or_default();
    for (a, b) in left_values.iter().zip(right_values.iter()) {
        match compare_record_values(a, b) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    left_values.len().cmp(&right_values.len())
}

fn compare_record_values(a: &sqlite_formats::RecordValue<'_>, b: &sqlite_formats::RecordValue<'_>) -> std::cmp::Ordering {
    use sqlite_formats::RecordValue as V;
    let class = |value: &V| match value {
        V::Null => 0,
        V::Integer(_) | V::Real(_) => 1,
        V::Text(_) => 2,
        V::Blob(_) => 3,
    };
    match (class(a), class(b)) {
        (0, 0) => std::cmp::Ordering::Equal,
        (1, 1) => compare_numeric(a, b),
        (2, 2) => {
            let (V::Text(left), V::Text(right)) = (a, b)
            else {
                unreachable!()
            };
            left.as_bytes().cmp(right.as_bytes())
        }
        (3, 3) => {
            let (V::Blob(left), V::Blob(right)) = (a, b)
            else {
                unreachable!()
            };
            left.cmp(right)
        }
        (left, right) => left.cmp(&right),
    }
}

fn compare_numeric(a: &sqlite_formats::RecordValue<'_>, b: &sqlite_formats::RecordValue<'_>) -> std::cmp::Ordering {
    use sqlite_formats::RecordValue as V;
    let to_f64 = |value: &V| match value {
        V::Integer(number) => *number as f64,
        V::Real(real) => *real,
        _ => unreachable!(),
    };
    let (left, right) = (to_f64(a), to_f64(b));
    if left.is_nan() || right.is_nan() {
        return left.total_cmp(&right);
    }
    match (a, b) {
        (V::Integer(left_int), V::Real(right_real)) => exact_int_real(*left_int, *right_real),
        (V::Real(left_real), V::Integer(right_int)) => exact_int_real(*right_int, *left_real).reverse(),
        _ => left.total_cmp(&right),
    }
}

fn exact_int_real(integer: i64, real: f64) -> std::cmp::Ordering {
    if real.is_infinite() {
        return if real.is_sign_positive() { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
    }
    let truncated = real.trunc();
    if truncated == real && truncated.abs() < 9.223372036854776e18 {
        return integer.cmp(&(truncated as i64));
    }
    (integer as f64).total_cmp(&real)
}

fn prefix_equal(
    candidate: &[sqlite_formats::RecordValue<'_>],
    wanted: &[sqlite_formats::RecordValue<'_>],
    prefix_columns: usize,
) -> bool {
    if candidate.len() < prefix_columns || wanted.len() < prefix_columns {
        return false;
    }
    candidate[..prefix_columns]
        .iter()
        .zip(wanted[..prefix_columns].iter())
        .all(|(left, right)| compare_record_values(left, right).is_eq())
}

fn is_rowid_alias(column: &ColumnSpec) -> bool {
    column.primary_key && column.declared_type.eq_ignore_ascii_case("INTEGER")
}

/// SQLite column affinity derived from the declared type (fileformat2 §3.1).
#[derive(Clone, Copy, PartialEq)]
enum Affinity {
    Integer,
    Text,
    Blob,
    Real,
    Numeric,
}

fn affinity_of(declared: &str) -> Affinity {
    let upper = declared.to_ascii_uppercase();
    if upper.contains("INT") {
        Affinity::Integer
    }
    else if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        Affinity::Text
    }
    else if upper.contains("BLOB") || upper.trim().is_empty() {
        Affinity::Blob
    }
    else if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        Affinity::Real
    }
    else {
        Affinity::Numeric
    }
}

/// Applies SQLite affinity conversion to a value before storage.
fn apply_affinity(value: WriteValue, affinity: Affinity) -> WriteValue {
    match affinity {
        Affinity::Blob => value,
        Affinity::Real => match value {
            WriteValue::Integer(number) => WriteValue::Real(number as f64),
            other => other,
        },
        Affinity::Text => match value {
            WriteValue::Integer(number) => WriteValue::Text(number.to_string()),
            WriteValue::Real(real) => WriteValue::Text(render_real_as_text(real)),
            other => other,
        },
        Affinity::Numeric | Affinity::Integer => match value {
            WriteValue::Real(real) => match lossless_integer(real) {
                Some(integer) => WriteValue::Integer(integer),
                None => WriteValue::Real(real),
            },
            WriteValue::Text(text) => convert_text_numeric(&text, affinity),
            other => other,
        },
    }
}

fn lossless_integer(real: f64) -> Option<i64> {
    if !real.is_finite() || real.trunc() != real {
        return None;
    }
    if real.abs() >= 9.223372036854776e18 {
        return None;
    }
    let integer = real as i64;
    if (integer as f64) == real { Some(integer) } else { None }
}

fn convert_text_numeric(text: &str, affinity: Affinity) -> WriteValue {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return WriteValue::Text(text.to_string());
    }
    if let Ok(integer) = trimmed.parse::<i64>() {
        return WriteValue::Integer(integer);
    }
    if affinity == Affinity::Integer {
        return WriteValue::Text(text.to_string());
    }
    match trimmed.parse::<f64>() {
        Ok(real) if trimmed.bytes().any(|byte| matches!(byte, b'.' | b'e' | b'E')) => match lossless_integer(real) {
            Some(integer) => WriteValue::Integer(integer),
            None => WriteValue::Real(real),
        },
        _ => WriteValue::Text(text.to_string()),
    }
}

fn render_real_as_text(real: f64) -> String {
    if real.is_infinite() || real.is_nan() {
        return format!("{real}");
    }
    let mut text = format!("{real:.15e}");
    // %.15g trims trailing zeros of the mantissa; mirror that.
    if let Some(exponent_position) = text.find('e') {
        let (mantissa, exponent) = text.split_at(exponent_position);
        let mut mantissa = mantissa.trim_end_matches('0').trim_end_matches('.').to_string();
        if mantissa.is_empty() || mantissa == "-" {
            mantissa.push('0');
        }
        let exponent_value: i32 = exponent[1..].parse().unwrap_or(0);
        text = format!("{mantissa}e{exponent_value:+}");
    }
    text
}

fn eq_ignore_ascii_case(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

fn expand_alias(meta: &TableMeta, rowid: i64, values: &[sqlite_formats::RecordValue<'_>]) -> Vec<WriteValue> {
    let mut owned: Vec<WriteValue> = values
        .iter()
        .map(|value| match value {
            sqlite_formats::RecordValue::Null => WriteValue::Null,
            sqlite_formats::RecordValue::Integer(number) => WriteValue::Integer(*number),
            sqlite_formats::RecordValue::Real(real) => WriteValue::Real(*real),
            sqlite_formats::RecordValue::Blob(bytes) => WriteValue::Blob(bytes.to_vec()),
            sqlite_formats::RecordValue::Text(text) => WriteValue::Text(text.clone()),
        })
        .collect();
    if let Some(index) = meta.rowid_alias {
        if index < owned.len() {
            owned[index] = WriteValue::Integer(rowid);
        }
    }
    owned
}

fn encode_index_key(meta: &TableMeta, index: &IndexMeta, rowid: i64, values: &[WriteValue]) -> Result<Vec<u8>> {
    let mut key: Vec<WriteValue> = index.columns.iter().map(|column| values[*column].clone()).collect();
    let _ = meta;
    key.push(WriteValue::Integer(rowid));
    Ok(encode_record(&key))
}

fn render_create_table(name: &str, columns: &[ColumnSpec]) -> String {
    let mut parts = Vec::with_capacity(columns.len());
    for column in columns {
        let mut fragment = format!("{} {}", quote_identifier(&column.name), render_declared_type(&column.declared_type));
        if column.primary_key {
            fragment.push_str(" PRIMARY KEY");
        }
        if column.not_null {
            fragment.push_str(" NOT NULL");
        }
        if let Some(default) = &column.default_value {
            fragment.push_str(&format!(" DEFAULT {}", render_literal(default)));
        }
        if column.unique && !column.primary_key {
            fragment.push_str(" UNIQUE");
        }
        parts.push(fragment);
    }
    format!("CREATE TABLE {} ({})", quote_identifier(name), parts.join(", "))
}

fn render_create_index(name: &str, table: &str, columns: &[String], unique: bool) -> String {
    let keyword = if unique { "CREATE UNIQUE INDEX" } else { "CREATE INDEX" };
    let column_list = columns.iter().map(|column| quote_identifier(column)).collect::<Vec<_>>().join(", ");
    format!("{keyword} {} ON {} ({column_list})", quote_identifier(name), quote_identifier(table))
}

fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn render_declared_type(declared: &str) -> String {
    if declared.trim().is_empty() { "BLOB".to_string() } else { declared.trim().to_string() }
}

fn render_literal(value: &WriteValue) -> String {
    match value {
        WriteValue::Null => "NULL".to_string(),
        WriteValue::Integer(number) => number.to_string(),
        WriteValue::Real(real) => format!("{real:?}"),
        WriteValue::Blob(bytes) => {
            let hex: String = bytes.iter().map(|byte| format!("{byte:02X}")).collect();
            format!("X'{hex}'")
        }
        WriteValue::Text(text) => format!("'{}'", text.replace('\'', "''")),
    }
}

fn parse_create_table(sql: &str) -> Result<Vec<ColumnSpec>> {
    let trimmed = sql.trim();
    let upper = trimmed.to_ascii_uppercase();
    let Some(start) = upper.find("CREATE TABLE ")
    else {
        return Err(Error::Unsupported("sqlite CREATE TABLE text unrecognized"));
    };
    let rest = &trimmed[start + "CREATE TABLE ".len()..];
    let Some(open) = rest.find('(')
    else {
        return Err(Error::Unsupported("sqlite CREATE TABLE text lacks columns"));
    };
    let Some(close) = rest.rfind(')')
    else {
        return Err(Error::Unsupported("sqlite CREATE TABLE text lacks closing paren"));
    };
    if close <= open {
        return Err(Error::Unsupported("sqlite CREATE TABLE text malformed"));
    }
    let body = &rest[open + 1..close];
    let mut columns = Vec::new();
    for fragment in split_top_level(body) {
        columns.push(parse_column(fragment.trim())?);
    }
    Ok(columns)
}

fn parse_column(fragment: &str) -> Result<ColumnSpec> {
    let tokens = tokenize_fragment(fragment)?;
    if tokens.is_empty() {
        return Err(Error::Unsupported("sqlite column definition empty"));
    }
    let name = unquote_identifier(&tokens[0])?;
    let mut declared_type = String::new();
    let mut index = 1usize;
    while index < tokens.len() {
        let upper = tokens[index].to_ascii_uppercase();
        if matches!(upper.as_str(), "NOT" | "DEFAULT" | "PRIMARY" | "UNIQUE" | "NULL") {
            break;
        }
        if !declared_type.is_empty() {
            declared_type.push(' ');
        }
        declared_type.push_str(&tokens[index]);
        index += 1;
    }
    let mut not_null = false;
    let mut primary_key = false;
    let mut unique = false;
    let mut default_value = None;
    while index < tokens.len() {
        let upper = tokens[index].to_ascii_uppercase();
        match upper.as_str() {
            "NOT" => {
                require_keyword(&tokens, index + 1, "NULL")?;
                not_null = true;
                index += 2;
            }
            "PRIMARY" => {
                require_keyword(&tokens, index + 1, "KEY")?;
                primary_key = true;
                index += 2;
            }
            "UNIQUE" => {
                unique = true;
                index += 1;
            }
            "DEFAULT" => {
                let (value, consumed) = parse_literal(&tokens[index + 1..])?;
                default_value = Some(value);
                index += 1 + consumed;
            }
            "NULL" => {
                index += 1;
            }
            _ => return Err(Error::Unsupported("sqlite column constraint unrecognized")),
        }
    }
    Ok(ColumnSpec { name, declared_type, not_null, default_value, primary_key, unique })
}

fn require_keyword(tokens: &[String], index: usize, keyword: &str) -> Result<()> {
    match tokens.get(index) {
        Some(token) if token.eq_ignore_ascii_case(keyword) => Ok(()),
        _ => Err(Error::Unsupported("sqlite column constraint unrecognized")),
    }
}

fn parse_literal(tokens: &[String]) -> Result<(WriteValue, usize)> {
    let Some(first) = tokens.first()
    else {
        return Err(Error::Unsupported("sqlite DEFAULT literal missing"));
    };
    if first.eq_ignore_ascii_case("NULL") {
        return Ok((WriteValue::Null, 1));
    }
    if first.starts_with('\'') {
        let mut text = String::new();
        let mut consumed = 0usize;
        let mut closed = false;
        for token in tokens {
            consumed += 1;
            let trimmed = if consumed == 1 { &token[1..] } else { token.as_str() };
            text.push_str(trimmed);
            if text.ends_with('\'') && !text.ends_with("''") {
                text.pop();
                closed = true;
                break;
            }
            text.push(' ');
        }
        if !closed {
            return Err(Error::Unsupported("sqlite DEFAULT string literal unterminated"));
        }
        return Ok((WriteValue::Text(text.replace("''", "'")), consumed));
    }
    if first.starts_with("X'") || first.starts_with("x'") {
        let inner = &first[2..first.len() - 1];
        let mut bytes = Vec::with_capacity(inner.len() / 2);
        let mut chars = inner.chars();
        while let (Some(high), Some(low)) = (chars.next(), chars.next()) {
            let byte = u8::from_str_radix(&format!("{high}{low}"), 16)
                .map_err(|_| Error::Unsupported("sqlite DEFAULT blob literal invalid"))?;
            bytes.push(byte);
        }
        return Ok((WriteValue::Blob(bytes), 1));
    }
    if let Ok(integer) = first.parse::<i64>() {
        return Ok((WriteValue::Integer(integer), 1));
    }
    if let Ok(real) = first.parse::<f64>() {
        return Ok((WriteValue::Real(real), 1));
    }
    Err(Error::Unsupported("sqlite DEFAULT literal unrecognized"))
}

fn tokenize_fragment(fragment: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut chars = fragment.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            ' ' | '\t' | '\n' | '\r' => continue,
            '"' => {
                let mut token = String::from("\"");
                loop {
                    let Some(next) = chars.next()
                    else {
                        return Err(Error::Unsupported("sqlite identifier unterminated"));
                    };
                    token.push(next);
                    if next == '"' {
                        if chars.peek() == Some(&'"') {
                            token.push(chars.next().unwrap());
                            continue;
                        }
                        break;
                    }
                }
                tokens.push(token);
            }
            '\'' => {
                let mut token = String::from("'");
                loop {
                    let Some(next) = chars.next()
                    else {
                        return Err(Error::Unsupported("sqlite literal unterminated"));
                    };
                    token.push(next);
                    if next == '\'' {
                        if chars.peek() == Some(&'\'') {
                            token.push(chars.next().unwrap());
                            continue;
                        }
                        break;
                    }
                }
                tokens.push(token);
            }
            _ => {
                let mut token = String::new();
                token.push(character);
                while let Some(next) = chars.peek() {
                    if matches!(next, ' ' | '\t' | '\n' | '\r' | '"' | '\'') {
                        break;
                    }
                    token.push(*next);
                    chars.next();
                }
                tokens.push(token);
            }
        }
    }
    Ok(tokens)
}

fn unquote_identifier(token: &str) -> Result<String> {
    if token.starts_with('"') && token.ends_with('"') && token.len() >= 2 {
        return Ok(token[1..token.len() - 1].replace("\"\"", "\""));
    }
    Ok(token.to_string())
}

fn split_top_level(body: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    for character in body.chars() {
        if let Some(quote) = in_quote {
            current.push(character);
            if character == quote {
                in_quote = None;
            }
            continue;
        }
        match character {
            '"' | '\'' => {
                in_quote = Some(character);
                current.push(character);
            }
            '(' => {
                depth += 1;
                current.push(character);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(character);
            }
            ',' if depth == 0 => {
                parts.push(current.clone());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

fn parse_create_index(sql: &str) -> Result<(Vec<String>, bool)> {
    let upper = sql.to_ascii_uppercase();
    let unique = upper.starts_with("CREATE UNIQUE INDEX");
    let Some(on) = upper.find(" ON ")
    else {
        return Err(Error::Unsupported("sqlite CREATE INDEX text unrecognized"));
    };
    let rest = &sql[on + 4..];
    let Some(open) = rest.find('(')
    else {
        return Err(Error::Unsupported("sqlite CREATE INDEX text lacks columns"));
    };
    let Some(close) = rest.rfind(')')
    else {
        return Err(Error::Unsupported("sqlite CREATE INDEX text lacks closing paren"));
    };
    let mut columns = Vec::new();
    for fragment in rest[open + 1..close].split(',') {
        columns.push(unquote_identifier(fragment.trim())?);
    }
    Ok((columns, unique))
}
