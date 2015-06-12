//! Render query rows into a text or binary result format.
//!
//! A [`QueryRow`] still holds typed [`Value`]s, so the raw octets of a BLOB are
//! available at this layer. Encoding decides what a host receives:
//!
//! | Format | `bytes` cell | Other kinds |
//! |--------|--------------|-------------|
//! | [`ResultFormat::Text`] | refused: text cannot carry the octets | text spelling |
//! | [`ResultFormat::Binary`] | the raw octets, untouched | text spelling |
//!
//! The text format refuses a `bytes` cell instead of inventing a spelling for
//! it. Hex, base64 or replacement characters would each be a conversion the
//! database never performs, so the caller gets [`Error::ResultFormat`] naming
//! the column and the kind that could not be carried.

use std::collections::BTreeSet;

use yydb_types::{Error, Result, Value};

use super::{exec, ops::QueryRow};

/// How [`encode_result`] renders each cell of a row stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultFormat {
    /// Every cell is text, so a `bytes` cell has no representation and is
    /// refused.
    Text,
    /// A `bytes` cell carries its raw octets. Every other kind keeps the same
    /// text spelling the text format uses.
    Binary,
}

/// One encoded cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResultCell {
    /// SQL `NULL`: neither text nor octets, so it has no length.
    Null,
    /// A text cell.
    Text(String),
    /// A binary cell holding raw octets.
    Bytes(Vec<u8>),
}

impl ResultCell {
    /// Stable kind label of the encoded cell: `null`, `text` or `bytes`.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Text(_) => "text",
            Self::Bytes(_) => "bytes",
        }
    }

    /// Payload byte length of the cell, or `None` for `NULL`.
    ///
    /// For a text cell this is the UTF-8 byte length, not the character count,
    /// so it matches what a wire length prefix would need.
    pub fn byte_length(&self) -> Option<usize> {
        match self {
            Self::Null => None,
            Self::Text(text) => Some(text.len()),
            Self::Bytes(bytes) => Some(bytes.len()),
        }
    }
}

/// Metadata of one encoded column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMeta {
    /// Column name as the query produced it.
    pub name: String,
    /// Kind of the first non-`NULL` value in the column, or `null` when every
    /// row is `NULL`. A column that mixes kinds reports the kind it saw first.
    pub kind: &'static str,
}

/// A row stream rendered in one [`ResultFormat`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedResult {
    /// Format the cells were rendered in.
    pub format: ResultFormat,
    /// Column metadata in the same order as each row's cells.
    pub columns: Vec<ColumnMeta>,
    /// Rows of cells, each aligned with [`Self::columns`].
    pub rows: Vec<Vec<ResultCell>>,
}

/// Encode query rows into one [`ResultFormat`].
///
/// Columns are the union of every row's keys. [`QueryRow`] is a `BTreeMap`, so
/// that order is sorted and stable. A row that omits a key contributes `NULL`.
///
/// Returns [`Error::ResultFormat`] when the requested format cannot carry a
/// cell: the text format refuses every `bytes` cell, and both formats refuse a
/// kind that has no text spelling of its own (for example `vector`).
pub fn encode_result(rows: &[QueryRow], format: ResultFormat) -> Result<EncodedResult> {
    let names: Vec<String> = rows.iter().flat_map(|row| row.keys().cloned()).collect::<BTreeSet<_>>().into_iter().collect();
    let columns = names
        .iter()
        .map(|name| ColumnMeta {
            name: name.clone(),
            kind: rows
                .iter()
                .filter_map(|row| row.get(name))
                .find(|value| !matches!(value, Value::Null))
                .map_or("null", |value| value.kind()),
        })
        .collect();
    let mut encoded_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let mut cells = Vec::with_capacity(names.len());
        for name in &names {
            let value = row.get(name).unwrap_or(&Value::Null);
            match format {
                ResultFormat::Binary => cells.push(binary_cell(name, value)?),
                ResultFormat::Text => cells.push(text_cell(name, value)?),
            }
        }
        encoded_rows.push(cells);
    }
    Ok(EncodedResult { format, columns, rows: encoded_rows })
}

/// A binary cell keeps `bytes` raw and leaves every other kind as text.
fn binary_cell(column: &str, value: &Value) -> Result<ResultCell> {
    match value {
        Value::Bytes(bytes) => Ok(ResultCell::Bytes(bytes.clone())),
        other => text_cell(column, other),
    }
}

/// A text cell refuses anything the text format cannot spell, including `bytes`.
fn text_cell(column: &str, value: &Value) -> Result<ResultCell> {
    match value {
        Value::Null => Ok(ResultCell::Null),
        Value::Text(text) => Ok(ResultCell::Text(text.clone())),
        // The text spelling of a scalar is the same one `CAST(… AS TEXT)`
        // performs, so a host never sees a second, parallel formatting rule.
        _ => match exec::eval_cast(value, "TEXT") {
            Ok(Value::Text(text)) => Ok(ResultCell::Text(text)),
            _ => Err(Error::ResultFormat { column: column.to_owned(), kind: value.kind() }),
        },
    }
}
