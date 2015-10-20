//! Table row persistence inside the YYDB record map.

use std::{cell::RefCell, collections::BTreeMap, sync::Arc};

use yydb_types::{
    Date, Decimal, Error, FileDescriptor, FileManifestRef, HashAlgo, ObjectKind, ObjectRef, Result, Time, Timestamp, Value,
    Vector,
};

use super::ops::QueryRow;

/// Record-map key prefix that owns table rows.
///
/// Row keys are `__yydb/v0/row/<table>/<pk>`, and their JSON values carry
/// tagged object and file references that garbage collection must count.
pub const ROW_PREFIX: &str = "__yydb/v0/row/";

/// Reads the octets behind an out-of-line `bytes` object reference.
///
/// The storage host owns placement, so the row codec asks it to resolve a
/// reference instead of knowing how objects are kept.
pub trait ObjectResolver: Send + Sync {
    /// Returns the stored octets, failing closed when the object is missing or corrupt.
    fn resolve(&self, object: &ObjectRef) -> Result<Vec<u8>>;
}

type SharedResolver = Arc<dyn ObjectResolver>;

/// Storage hook that decides where a written `bytes` value is kept.
///
/// The octet threshold is a physical property of the host, so the query layer
/// never picks one: the host installs a hook and receives either the original
/// inline octets or an object reference it created itself.
pub trait BytesPlacementHook: Send + Sync {
    /// Returns the value to store: inline octets or a reference into the host object store.
    fn place(&self, octets: Vec<u8>) -> Result<Value>;
}

/// Storage hook that decides where a written `vector<N>` value is kept.
///
/// Small vectors stay inline in the row's tagged JSON form; larger ones become
/// a content-addressed object reference (`ObjectKind::VectorPayload`). The
/// dimension threshold belongs to the host (same design as `bytes`), so the
/// query layer never picks one.
pub trait VectorPlacementHook: Send + Sync {
    /// Returns the value to store: inline `Value::Vector` or `Value::Object`
    /// referencing a `VectorPayload` in the host object store.
    fn place(&self, vector: Vector) -> Result<Value>;
}

type SharedPlacementHook = Arc<dyn BytesPlacementHook>;
type SharedVectorHook = Arc<dyn VectorPlacementHook>;

thread_local! {
    /// Resolver installed for the duration of one statement execution on this thread.
    static OBJECT_RESOLVER: RefCell<Option<SharedResolver>> = RefCell::new(None);
    /// Placement hook installed for the duration of one write execution on this thread.
    static BYTES_PLACEMENT: RefCell<Option<SharedPlacementHook>> = RefCell::new(None);
    /// Vector placement hook installed for the duration of one write execution on this thread.
    static VECTOR_PLACEMENT: RefCell<Option<SharedVectorHook>> = RefCell::new(None);
}

struct ResolverGuard(Option<SharedResolver>);

impl Drop for ResolverGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        OBJECT_RESOLVER.with(|slot| {
            slot.replace(previous);
        });
    }
}

/// Runs `body` with `resolver` installed for row decoding on this thread.
///
/// Rows whose `bytes` values were placed in the host object store cannot be
/// decoded without a resolver, so a host that stores references always installs
/// one. Without it, decoding a reference returns [`Error::Unsupported`] instead
/// of guessing or downgrading the value to text.
pub fn with_object_resolver<T>(resolver: Option<SharedResolver>, body: impl FnOnce() -> T) -> T {
    let previous = OBJECT_RESOLVER.with(|slot| slot.replace(resolver));
    let _guard = ResolverGuard(previous);
    body()
}

struct PlacementGuard(Option<SharedPlacementHook>);

impl Drop for PlacementGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        BYTES_PLACEMENT.with(|slot| {
            slot.replace(previous);
        });
    }
}

struct VectorPlacementGuard(Option<SharedVectorHook>);

impl Drop for VectorPlacementGuard {
    fn drop(&mut self) {
        let previous = self.0.take();
        VECTOR_PLACEMENT.with(|slot| {
            slot.replace(previous);
        });
    }
}

/// Runs `body` with `hook` installed for `bytes` placement on this thread.
///
/// Without a hook every `bytes` value stays inline, which is what a host with
/// no object store can actually keep.
pub fn with_bytes_placement<T>(hook: Option<SharedPlacementHook>, body: impl FnOnce() -> T) -> T {
    let previous = BYTES_PLACEMENT.with(|slot| slot.replace(hook));
    let _guard = PlacementGuard(previous);
    body()
}

/// Runs `body` with `hook` installed for `vector<N>` placement on this thread.
///
/// Without a hook every vector stays inline in the row's tagged JSON form,
/// which matches hosts with no object store.
pub fn with_vector_placement<T>(hook: Option<SharedVectorHook>, body: impl FnOnce() -> T) -> T {
    let previous = VECTOR_PLACEMENT.with(|slot| slot.replace(hook));
    let _guard = VectorPlacementGuard(previous);
    body()
}

/// Places one `bytes` value through the installed hook.
pub(crate) fn place_bytes(octets: Vec<u8>) -> Result<Value> {
    BYTES_PLACEMENT.with(|slot| match slot.borrow().as_ref() {
        Some(hook) => hook.place(octets),
        None => Ok(Value::Bytes(octets)),
    })
}

/// Places one `vector<N>` value through the installed hook.
///
/// Without a hook the vector stays inline as `Value::Vector`; a hook may
/// return a `Value::Object` referencing a CAS `VectorPayload`.
pub(crate) fn place_vector(vector: Vector) -> Result<Value> {
    VECTOR_PLACEMENT.with(|slot| match slot.borrow().as_ref() {
        Some(hook) => hook.place(vector),
        None => Ok(Value::Vector(vector)),
    })
}

/// One record-map row borrowed from a storage window.
///
/// The borrow is bounded by the [`RowSource`] that handed it out: a pipeline
/// decodes it into an owned [`QueryRow`] and never keeps the reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BorrowedRow<'a> {
    /// Logical record name.
    pub key: &'a str,
    /// Stored row bytes, borrowed from the page image that holds them.
    pub bytes: &'a [u8],
}

/// Storage view that hands out borrowed rows for one record-key prefix.
///
/// A narrowed read keeps row bytes in the storage window instead of copying
/// them into a record map. The engine owns that window and passes it here for
/// the duration of one statement execution; every row is decoded into an owned
/// [`QueryRow`] before it leaves the operator.
pub trait RowSource {
    /// Rows whose record name starts with `prefix`, in ascending key order.
    ///
    /// The iterator borrows `self`, so the source must outlive it. It owns its
    /// copy of `prefix`, so the caller may pass a temporary string.
    fn rows_with_prefix<'a>(&'a self, prefix: &str) -> Box<dyn Iterator<Item = Result<BorrowedRow<'a>>> + 'a>;

    /// Exact row lookup by full record key.
    ///
    /// Defaults to the first row of [`Self::rows_with_prefix`]: a source yields
    /// ascending keys, so a longer key sharing the requested prefix means the
    /// exact row is absent.
    fn row_at<'a>(&'a self, key: &str) -> Option<Result<BorrowedRow<'a>>> {
        let first = self.rows_with_prefix(key).next()?;
        match first {
            Ok(row) if row.key == key => Some(Ok(row)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        }
    }
}

/// Record rows one read pipeline can scan.
///
/// A full read owns every row in the map. A narrowed read owns only metadata
/// rows and borrows the statement's window from [`RowSource`].
pub(crate) struct ReadRows<'a> {
    records: &'a BTreeMap<String, Vec<u8>>,
    source: Option<&'a dyn RowSource>,
}

impl<'a> ReadRows<'a> {
    /// Rows come from the owned record map only.
    pub(crate) fn owned(records: &'a BTreeMap<String, Vec<u8>>) -> Self {
        Self { records, source: None }
    }

    /// Rows come from `source`; `records` still answers metadata lookups.
    pub(crate) fn borrowed(records: &'a BTreeMap<String, Vec<u8>>, source: &'a dyn RowSource) -> Self {
        Self { records, source: Some(source) }
    }

    /// Decode every row of `table`, whichever side holds the bytes.
    pub(crate) fn load_table(&self, table: &str) -> Result<Vec<QueryRow>> {
        match self.source {
            Some(source) => load_table_borrowed(source, table),
            None => load_table(self.records, table),
        }
    }

    /// Decode one row addressed by table and primary key.
    pub(crate) fn load_row(&self, table: &str, pk: &str) -> Result<QueryRow> {
        let key = row_key(table, pk);
        match self.source {
            Some(source) => match source.row_at(&key) {
                Some(record) => decode_row(record?.bytes),
                None => Err(missing_row(table, pk)),
            },
            None => load_row(self.records, table, pk),
        }
    }
}

fn missing_row(table: &str, pk: &str) -> Error {
    Error::Schema { message: format!("missing row `{table}` pk `{pk}`") }
}

fn load_table_borrowed(source: &dyn RowSource, table: &str) -> Result<Vec<QueryRow>> {
    let prefix = table_prefix(table);
    let mut rows = Vec::new();
    for record in source.rows_with_prefix(&prefix) {
        rows.push(decode_row(record?.bytes)?);
    }
    Ok(rows)
}

/// Build the storage key for one table row.
pub fn row_key(table: &str, pk: &str) -> String {
    format!("{ROW_PREFIX}{table}/{pk}")
}

fn table_prefix(table: &str) -> String {
    format!("{ROW_PREFIX}{table}/")
}

/// Load all rows for `table` from the record map.
pub fn load_table(records: &BTreeMap<String, Vec<u8>>, table: &str) -> Result<Vec<QueryRow>> {
    // A read never needs the record-map key, so it decodes straight out of the
    // prefix range instead of cloning one record name per row.
    let prefix = table_prefix(table);
    let mut rows = Vec::new();
    for (key, bytes) in records.range(prefix.clone()..) {
        if !key.starts_with(&prefix) {
            break;
        }
        rows.push(decode_row(bytes)?);
    }
    Ok(rows)
}

/// Load all rows for `table` together with their record-map keys.
///
/// Write operators need the key to delete or replace a row in place, so this
/// variant is the shared primitive and [`load_table`] is the read-only view.
pub fn load_table_entries(records: &BTreeMap<String, Vec<u8>>, table: &str) -> Result<Vec<(String, QueryRow)>> {
    let prefix = table_prefix(table);
    let mut out = Vec::new();
    for (key, bytes) in records.range(prefix.clone()..) {
        if !key.starts_with(&prefix) {
            break;
        }
        out.push((key.clone(), decode_row(bytes)?));
    }
    Ok(out)
}

/// Remove the row stored under `key`. Returns whether a row was present.
pub fn remove_row(records: &mut BTreeMap<String, Vec<u8>>, key: &str) -> bool {
    records.remove(key).is_some()
}

/// Upsert one logical row keyed by `pk`.
pub fn upsert_row(records: &mut BTreeMap<String, Vec<u8>>, table: &str, pk: &str, row: &QueryRow) -> Result<()> {
    records.insert(row_key(table, pk), encode_row(row)?);
    Ok(())
}

/// Version byte prefixed to every stored row record.
///
/// Row records are self-describing and versioned. A record whose first byte is
/// not [`ROW_CODEC_VERSION`] was written by the retired JSON row codec, and
/// decoding it fails closed with a migration hint instead of guessing a shape.
pub const ROW_CODEC_VERSION: u8 = 1;

const TAG_BOOL: u8 = 0x01;
const TAG_I64: u8 = 0x02;
const TAG_F64: u8 = 0x03;
const TAG_TEXT: u8 = 0x04;
const TAG_UUID: u8 = 0x05;
const TAG_DATE: u8 = 0x06;
const TAG_TIME: u8 = 0x07;
const TAG_TIMESTAMP: u8 = 0x08;
const TAG_DECIMAL: u8 = 0x09;
const TAG_BYTES: u8 = 0x0A;
const TAG_VECTOR: u8 = 0x0B;
const TAG_OBJECT: u8 = 0x0C;
const TAG_FILE: u8 = 0x0D;

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Encode one row as the binary row record format.
///
/// Layout: `version u8`, `column count u16`, `null bitmap ceil(n/8)`, then one
/// `name length u16 + UTF-8 name` per column in ascending name order, then one
/// tagged value for every column the bitmap does not mark null. Integers keep
/// their declared width, text and octets carry a length prefix, floats are raw
/// IEEE-754 bits, and keys never repeat the enclosing table name.
fn encode_row(row: &QueryRow) -> Result<Vec<u8>> {
    let column_count = u16::try_from(row.len()).map_err(|_| Error::Unsupported("query row has more than 65535 columns"))?;
    let mut out = Vec::with_capacity(4 + row.len() * 12);
    out.push(ROW_CODEC_VERSION);
    out.extend_from_slice(&column_count.to_le_bytes());
    let bitmap_len = row.len().div_ceil(8);
    let bitmap_start = out.len();
    out.resize(bitmap_start + bitmap_len, 0);
    for (index, (name, value)) in row.iter().enumerate() {
        if matches!(value, Value::Null) {
            out[bitmap_start + index / 8] |= 1 << (index % 8);
        }
        put_len(&mut out, name.len(), "column name")?;
        out.extend_from_slice(name.as_bytes());
    }
    for value in row.values() {
        if matches!(value, Value::Null) {
            continue;
        }
        encode_value(&mut out, value)?;
    }
    Ok(out)
}

fn put_len(out: &mut Vec<u8>, len: usize, _what: &str) -> Result<()> {
    let len = u16::try_from(len).map_err(|_| Error::Unsupported("row record field exceeds 65535 bytes"))?;
    out.extend_from_slice(&len.to_le_bytes());
    Ok(())
}

fn encode_value(out: &mut Vec<u8>, value: &Value) -> Result<()> {
    match value {
        Value::Null => return Err(Error::Corrupt("null columns are carried by the row null bitmap")),
        Value::Bool(flag) => {
            out.push(TAG_BOOL);
            out.push(u8::from(*flag));
        }
        Value::I64(integer) => {
            out.push(TAG_I64);
            out.extend_from_slice(&integer.to_le_bytes());
        }
        Value::F64(number) => {
            if !number.is_finite() {
                return Err(Error::Unsupported("query row encoding rejects non-finite REAL values"));
            }
            out.push(TAG_F64);
            out.extend_from_slice(&number.to_le_bytes());
        }
        Value::Text(text) => {
            out.push(TAG_TEXT);
            put_len(out, text.len(), "text value")?;
            out.extend_from_slice(text.as_bytes());
        }
        Value::Uuid(id) => {
            out.push(TAG_UUID);
            out.extend_from_slice(id.as_bytes());
        }
        Value::Date(date) => {
            out.push(TAG_DATE);
            out.extend_from_slice(&date.days().to_le_bytes());
        }
        Value::Time(time) => {
            out.push(TAG_TIME);
            out.extend_from_slice(&time.nanos().to_le_bytes());
        }
        Value::Timestamp(timestamp) => {
            out.push(TAG_TIMESTAMP);
            out.extend_from_slice(&timestamp.unix_micros().to_le_bytes());
        }
        Value::Decimal(decimal) => {
            out.push(TAG_DECIMAL);
            out.extend_from_slice(&decimal.unscaled().to_le_bytes());
            out.push(decimal.precision());
            out.push(decimal.scale());
        }
        Value::Bytes(octets) => {
            out.push(TAG_BYTES);
            put_len(out, octets.len(), "bytes value")?;
            out.extend_from_slice(octets);
        }
        Value::Vector(vector) => {
            out.push(TAG_VECTOR);
            let dimension =
                u16::try_from(vector.dim).map_err(|_| Error::Unsupported("inline row vector dimension exceeds 65535"))?;
            let payload_len = u32::try_from(dimension as usize * 4)
                .map_err(|_| Error::Unsupported("inline row vector payload exceeds 4 GiB"))?;
            out.extend_from_slice(&dimension.to_le_bytes());
            out.extend_from_slice(&payload_len.to_le_bytes());
            for component in vector.data.iter() {
                out.extend_from_slice(&component.to_le_bytes());
            }
        }
        Value::Object(object) => {
            out.push(TAG_OBJECT);
            out.push(algo_tag(object.algo)?);
            out.push(object_kind_tag(object.kind)?);
            out.extend_from_slice(&object.hash);
            out.extend_from_slice(&object.size.to_le_bytes());
        }
        Value::File(file) => {
            out.push(TAG_FILE);
            let digest =
                parse_digest_hex(&file.digest).ok_or(Error::Corrupt("file descriptor digest is not 64 hexadecimal digits"))?;
            out.extend_from_slice(&digest);
            out.extend_from_slice(&file.octets.to_le_bytes());
            let mut flags = 0_u8;
            if file.name.is_some() {
                flags |= 0b01;
            }
            if file.media_type.is_some() {
                flags |= 0b10;
            }
            out.push(flags);
            if let Some(name) = &file.name {
                put_len(out, name.len(), "file name")?;
                out.extend_from_slice(name.as_bytes());
            }
            if let Some(media_type) = &file.media_type {
                put_len(out, media_type.len(), "file media type")?;
                out.extend_from_slice(media_type.as_bytes());
            }
            out.extend_from_slice(&file.manifest.hash);
            out.extend_from_slice(&file.manifest.size.to_le_bytes());
        }
        _ => {
            return Err(Error::Unsupported(
                "query row encoding supports null, bool, i64, real, text, uuid, bytes, vector, date, time, timestamp, \
                 decimal, bytes object references, and file descriptors only",
            ));
        }
    }
    Ok(())
}

fn algo_tag(algo: HashAlgo) -> Result<u8> {
    match algo {
        HashAlgo::Blake3 => Ok(0),
        _ => Err(Error::Unsupported("query row encoding supports the blake3 digest algorithm only")),
    }
}

fn object_kind_tag(kind: ObjectKind) -> Result<u8> {
    Ok(match kind {
        ObjectKind::Blob => 0,
        ObjectKind::VectorPayload => 1,
        ObjectKind::FileManifest => 2,
        ObjectKind::AnnSegment => 3,
        _ => return Err(Error::Unsupported("query row encoding does not support this object kind")),
    })
}

fn algo_from_tag(tag: u8) -> Result<HashAlgo> {
    match tag {
        0 => Ok(HashAlgo::Blake3),
        _ => Err(Error::Corrupt("row record digest algorithm tag is unknown")),
    }
}

fn object_kind_from_tag(tag: u8) -> Result<ObjectKind> {
    match tag {
        0 => Ok(ObjectKind::Blob),
        1 => Ok(ObjectKind::VectorPayload),
        2 => Ok(ObjectKind::FileManifest),
        3 => Ok(ObjectKind::AnnSegment),
        _ => Err(Error::Corrupt("row record object kind tag is unknown")),
    }
}

fn digest_hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(HEX_DIGITS[(byte >> 4) as usize] as char);
        out.push(HEX_DIGITS[(byte & 0x0F) as usize] as char);
    }
    out
}

fn parse_digest_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut digest = [0_u8; 32];
    for (index, pair) in text.as_bytes().chunks(2).enumerate() {
        digest[index] = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Some(digest)
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// Load one row by primary key.
pub fn load_row(records: &BTreeMap<String, Vec<u8>>, table: &str, pk: &str) -> Result<QueryRow> {
    let key = row_key(table, pk);
    let bytes = records.get(&key).ok_or_else(|| missing_row(table, pk))?;
    decode_row(bytes)
}

fn decode_row(bytes: &[u8]) -> Result<QueryRow> {
    parse_row(bytes, &mut None, true)
}

/// One stored row record's out-of-line references.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RowReferences {
    /// Objects the row stores directly.
    pub objects: Vec<ObjectRef>,
    /// File descriptors the row stores.
    pub files: Vec<FileDescriptor>,
}

/// Collect the out-of-line objects one stored row record points at.
///
/// The reference closure must see exactly what the row committed, so this reads
/// the same record layout the reader uses and never resolves an object.
pub fn row_references(bytes: &[u8]) -> Result<RowReferences> {
    let mut references = Some(RowReferences::default());
    parse_row(bytes, &mut references, false)?;
    references.take().ok_or(Error::Corrupt("row record reference walk produced no result"))
}

fn parse_row(bytes: &[u8], references: &mut Option<RowReferences>, resolve: bool) -> Result<QueryRow> {
    match bytes.first() {
        Some(version) if *version == ROW_CODEC_VERSION => {}
        Some(_) => {
            return Err(Error::Unsupported(
                "row record uses the retired JSON row encoding: rebuild the database from source data to use the \
                 binary row format",
            ));
        }
        None => return Err(Error::Corrupt("row record is empty")),
    }
    let mut header = RowCursor::new(bytes);
    header.take(1)?;
    let column_count = header.u16()? as usize;
    let bitmap_len = column_count.div_ceil(8);
    // The bitmap stays a slice of the record; copying it would add one heap
    // allocation per decoded row for no benefit.
    let bitmap = header.take(bitmap_len)?;
    // Walk the name table once to find where the values start instead of
    // collecting the names into a `Vec`: the names are only needed one at a
    // time, and each decoded name is moved straight into the row as its key.
    let names_start = header;
    let mut walk = names_start;
    for _ in 0..column_count {
        walk.skip_text()?;
    }
    let mut names = names_start;
    let mut values = walk;
    let mut row = QueryRow::new();
    for index in 0..column_count {
        let name = names.text()?;
        if bitmap[index / 8] & (1 << (index % 8)) != 0 {
            row.insert(name, Value::Null);
            continue;
        }
        let value = decode_value(&mut values, references, resolve)?;
        row.insert(name, value);
    }
    if values.remaining() != 0 {
        return Err(Error::Corrupt("row record has trailing bytes"));
    }
    Ok(row)
}

/// Bounds-checked forward reader over one row record.
///
/// `Copy` lets one recorded row be walked twice from two saved positions: one
/// cursor reads the name table, the other reads the values that follow it.
#[derive(Clone, Copy)]
struct RowCursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> RowCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(len)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(Error::Corrupt("row record is truncated"))?;
        let slice = &self.bytes[self.position..end];
        self.position = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn i128(&mut self) -> Result<i128> {
        Ok(i128::from_le_bytes(self.take(16)?.try_into().unwrap()))
    }

    fn digest(&mut self) -> Result<[u8; 32]> {
        Ok(self.take(32)?.try_into().unwrap())
    }

    fn text(&mut self) -> Result<String> {
        let len = self.u16()? as usize;
        let bytes = self.take(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::Corrupt("row record text is not UTF-8"))
    }

    /// Advance past one length-prefixed UTF-8 string without materializing it.
    fn skip_text(&mut self) -> Result<()> {
        let len = self.u16()? as usize;
        self.take(len)?;
        Ok(())
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.position
    }
}

fn decode_value(cursor: &mut RowCursor<'_>, references: &mut Option<RowReferences>, resolve: bool) -> Result<Value> {
    Ok(match cursor.u8()? {
        TAG_BOOL => match cursor.u8()? {
            0 => Value::Bool(false),
            1 => Value::Bool(true),
            _ => return Err(Error::Corrupt("row record boolean is not 0 or 1")),
        },
        TAG_I64 => Value::I64(cursor.i64()?),
        TAG_F64 => {
            let real = f64::from_le_bytes(cursor.take(8)?.try_into().unwrap());
            if !real.is_finite() {
                return Err(Error::Corrupt("row record real is not finite"));
            }
            Value::F64(real)
        }
        TAG_TEXT => Value::Text(cursor.text()?),
        TAG_UUID => Value::Uuid(uuid::Uuid::from_bytes(cursor.take(16)?.try_into().unwrap())),
        TAG_DATE => Value::Date(Date::from_days(cursor.i32()?)?),
        TAG_TIME => Value::Time(Time::from_nanos(cursor.i64()?)?),
        TAG_TIMESTAMP => Value::Timestamp(Timestamp::from_unix_micros(cursor.i64()?)?),
        TAG_DECIMAL => {
            let unscaled = cursor.i128()?;
            let precision = cursor.u8()?;
            let scale = cursor.u8()?;
            Value::Decimal(Decimal::new(unscaled, precision, scale)?)
        }
        TAG_BYTES => {
            let len = cursor.u16()? as usize;
            Value::Bytes(cursor.take(len)?.to_vec())
        }
        TAG_VECTOR => {
            let dimension = cursor.u16()? as usize;
            let payload_len = cursor.u32()? as usize;
            if dimension.saturating_mul(4) != payload_len {
                return Err(Error::Scalar {
                    kind: "vector",
                    message: "vector payload length disagrees with its declared dimension".into(),
                });
            }
            let payload = cursor.take(payload_len)?;
            let mut components = Vec::with_capacity(dimension);
            for index in 0..dimension {
                let offset = index * 4;
                let component = f32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
                if !component.is_finite() {
                    return Err(Error::Scalar { kind: "vector", message: format!("component {index} is not finite") });
                }
                components.push(component);
            }
            Value::Vector(
                Vector::new(components).map_err(|error| Error::Scalar { kind: "vector", message: error.to_string() })?,
            )
        }
        TAG_OBJECT => {
            let algo = algo_from_tag(cursor.u8()?)?;
            let kind = object_kind_from_tag(cursor.u8()?)?;
            let hash = cursor.digest()?;
            let size = cursor.u64()?;
            let object = ObjectRef { algo, hash, size, kind };
            if let Some(references) = references.as_mut() {
                references.objects.push(object.clone());
            }
            if resolve { resolve_object_reference(Value::Object(object))? } else { Value::Object(object) }
        }
        TAG_FILE => {
            let digest = digest_hex(&cursor.digest()?);
            let octets = cursor.u64()?;
            let flags = cursor.u8()?;
            if flags & !0b11 != 0 {
                return Err(Error::Corrupt("row record file flags are invalid"));
            }
            let name = if flags & 0b01 != 0 { Some(cursor.text()?) } else { None };
            let media_type = if flags & 0b10 != 0 { Some(cursor.text()?) } else { None };
            let hash = cursor.digest()?;
            let size = cursor.u64()?;
            let descriptor = FileDescriptor { digest, octets, name, media_type, manifest: FileManifestRef { hash, size } };
            if let Some(references) = references.as_mut() {
                references.files.push(descriptor.clone());
            }
            Value::File(descriptor)
        }
        _ => return Err(Error::Corrupt("row record value tag is unknown")),
    })
}

fn resolve_object_reference(value: Value) -> Result<Value> {
    match value {
        Value::Object(object) => OBJECT_RESOLVER.with(|slot| match slot.borrow().as_ref() {
            Some(resolver) => {
                let bytes = resolver.resolve(&object)?;
                match object.kind {
                    ObjectKind::VectorPayload => Vector::from_le_bytes(&bytes).map(Value::Vector),
                    _ => Ok(Value::Bytes(bytes)),
                }
            }
            None => Err(Error::Unsupported("a stored object reference needs an object store to decode")),
        }),
        other => Ok(other),
    }
}
