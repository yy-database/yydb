use std::collections::HashSet;

use yydb_types::{Error, Result};

use crate::{decode_varint, validate_database};

/// Resource bounds for a read-only table b-tree scan.
#[derive(Debug, Clone, Copy)]
pub struct TableLimits {
    /// Maximum number of b-tree and overflow pages visited together.
    pub max_pages: usize,
    /// Maximum number of returned rows.
    pub max_rows: usize,
    /// Maximum record payload size per row.
    pub max_payload_bytes: usize,
    /// Maximum sum of returned record payload sizes.
    pub max_total_payload_bytes: usize,
}

impl Default for TableLimits {
    fn default() -> Self {
        Self {
            max_pages: 100_000,
            max_rows: 100_000,
            max_payload_bytes: 16 * 1024 * 1024,
            max_total_payload_bytes: 64 * 1024 * 1024,
        }
    }
}

/// An assembled SQLite table leaf record, before record decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRow {
    /// Signed SQLite rowid, including INTEGER PRIMARY KEY aliases.
    pub rowid: i64,
    /// Complete record bytes, including any overflow payload.
    pub payload: Vec<u8>,
}

struct Scan<'snapshot> {
    bytes: &'snapshot [u8],
    page_size: usize,
    usable: usize,
    page_count: usize,
    visited: HashSet<u32>,
    limits: TableLimits,
}

impl<'snapshot> Scan<'snapshot> {
    fn page(&mut self, number: u32) -> Result<&'snapshot [u8]> {
        let index = usize::try_from(number).map_err(|_| Error::Corrupt("sqlite page number overflow"))?;
        if index == 0 || index > self.page_count {
            return Err(Error::Corrupt("sqlite page reference out of bounds"));
        }
        if self.visited.len() >= self.limits.max_pages {
            return Err(Error::Unsupported("sqlite table page limit exceeded"));
        }
        if !self.visited.insert(number) {
            return Err(Error::Corrupt("sqlite repeated b-tree or overflow page"));
        }
        let start = (index - 1) * self.page_size;
        Ok(&self.bytes[start..start + self.usable])
    }

    fn payload(&mut self, page: &[u8], offset: usize, length: usize) -> Result<(Vec<u8>, usize)> {
        let maximum = self.usable - 35;
        let minimum = ((self.usable - 12) * 32 / 255) - 23;
        let local = if length <= maximum {
            length
        }
        else {
            let candidate = minimum + (length - minimum) % (self.usable - 4);
            if candidate <= maximum { candidate } else { minimum }
        };
        let end = offset.checked_add(local).ok_or(Error::Corrupt("sqlite cell length overflow"))?;
        let initial = page.get(offset..end).ok_or(Error::Corrupt("sqlite cell payload truncated"))?;
        let overflow_pages = (length - local).div_ceil(self.usable - 4);
        if overflow_pages > self.limits.max_pages.saturating_sub(self.visited.len()) {
            return Err(Error::Unsupported("sqlite table page limit exceeded"));
        }
        let mut payload = Vec::new();
        payload.try_reserve_exact(length).map_err(|_| Error::Unsupported("sqlite payload allocation failed"))?;
        payload.extend_from_slice(initial);
        if local == length {
            return Ok((payload, end));
        }
        let mut next = read_u32(page, end)?;
        while payload.len() < length {
            let overflow = self.page(next)?;
            next = read_u32(overflow, 0)?;
            let count = (length - payload.len()).min(self.usable - 4);
            payload.extend_from_slice(&overflow[4..4 + count]);
        }
        if next != 0 {
            return Err(Error::Corrupt("sqlite overflow chain exceeds payload"));
        }
        Ok((payload, end + 4))
    }
}

/// Reads a rowid table from a validated, quiescent main-file snapshot.
/// Index b-trees (including WITHOUT ROWID tables) are explicitly unsupported.
/// This does not recover WAL/journals or perform a full integrity check.
pub fn read_table(bytes: &[u8], root_page: u32, limits: TableLimits) -> Result<Vec<TableRow>> {
    let page_size = validate_database(bytes)?;
    if bytes.is_empty() {
        return Err(Error::Corrupt("sqlite table root in empty database"));
    }
    let declared = read_u32(bytes, 28)? as usize;
    let page_count =
        if declared != 0 && read_u32(bytes, 24)? == read_u32(bytes, 92)? { declared } else { bytes.len() / page_size };
    let mut scan =
        Scan { bytes, page_size, usable: page_size - usize::from(bytes[20]), page_count, visited: HashSet::new(), limits };
    let mut pending = vec![(root_page, None::<i64>, None::<i64>)];
    let mut rows: Vec<TableRow> = Vec::new();
    let mut total_payload = 0usize;
    while let Some((number, lower, upper)) = pending.pop() {
        let page = scan.page(number)?;
        let header = if number == 1 { 100 } else { 0 };
        let kind = *page.get(header).ok_or(Error::Corrupt("sqlite b-tree header truncated"))?;
        let header_size = match kind {
            5 => 12,
            13 => 8,
            2 | 10 => return Err(Error::Unsupported("sqlite index b-tree scan")),
            _ => return Err(Error::Corrupt("sqlite table b-tree page type invalid")),
        };
        let count = read_u16(page, header + 3)?;
        let pointers_end = header + header_size + count * 2;
        if pointers_end > page.len() || page[header + 7] > 60 {
            return Err(Error::Corrupt("sqlite b-tree header invalid"));
        }
        let raw_start = read_u16(page, header + 5)?;
        let content_start = if raw_start == 0 { 65536 } else { raw_start };
        if content_start < pointers_end || content_start > page.len() {
            return Err(Error::Corrupt("sqlite b-tree content boundary invalid"));
        }
        let mut previous = lower;
        let mut spans = Vec::with_capacity(count);
        let mut children = Vec::new();
        for index in 0..count {
            let cell = read_u16(page, header + header_size + index * 2)?;
            if cell < content_start || cell >= page.len() {
                return Err(Error::Corrupt("sqlite cell pointer out of bounds"));
            }
            if kind == 5 {
                let child = read_u32(page, cell)?;
                let (key, consumed) =
                    decode_varint(page.get(cell + 4..).ok_or(Error::Corrupt("sqlite interior cell truncated"))?)?;
                let key = key as i64;
                check_key(key, previous, upper)?;
                children.push((child, previous, Some(key)));
                previous = Some(key);
                spans.push((cell, cell + 4 + consumed));
            }
            else {
                if rows.len() >= limits.max_rows {
                    return Err(Error::Unsupported("sqlite table row limit exceeded"));
                }
                let (length, length_size) = decode_varint(&page[cell..])?;
                let length = usize::try_from(length).map_err(|_| Error::Unsupported("sqlite payload size exceeds host"))?;
                if length > limits.max_payload_bytes {
                    return Err(Error::Unsupported("sqlite table payload limit exceeded"));
                }
                total_payload =
                    total_payload.checked_add(length).ok_or(Error::Unsupported("sqlite table total payload overflow"))?;
                if total_payload > limits.max_total_payload_bytes {
                    return Err(Error::Unsupported("sqlite table total payload limit exceeded"));
                }
                let (rowid, rowid_size) = decode_varint(&page[cell + length_size..])?;
                let rowid = rowid as i64;
                check_key(rowid, previous, upper)?;
                if rows.last().is_some_and(|last| last.rowid >= rowid) {
                    return Err(Error::Corrupt("sqlite table rows out of order"));
                }
                previous = Some(rowid);
                let (payload, end) = scan.payload(page, cell + length_size + rowid_size, length)?;
                rows.push(TableRow { rowid, payload });
                spans.push((cell, end));
            }
        }
        spans.sort_unstable();
        if spans.windows(2).any(|pair| pair[0].1 > pair[1].0) {
            return Err(Error::Corrupt("sqlite b-tree cells overlap"));
        }
        if kind == 5 {
            children.push((read_u32(page, header + 8)?, previous, upper));
            if pending.len().saturating_add(children.len()).saturating_add(scan.visited.len()) > limits.max_pages {
                return Err(Error::Unsupported("sqlite table page limit exceeded"));
            }
            pending.extend(children.into_iter().rev());
        }
    }
    Ok(rows)
}

fn check_key(key: i64, lower: Option<i64>, upper: Option<i64>) -> Result<()> {
    if lower.is_some_and(|bound| key <= bound) || upper.is_some_and(|bound| key > bound) {
        return Err(Error::Corrupt("sqlite b-tree key outside ordered range"));
    }
    Ok(())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<usize> {
    let value = bytes.get(offset..offset + 2).ok_or(Error::Corrupt("sqlite b-tree field truncated"))?;
    Ok(usize::from(u16::from_be_bytes(value.try_into().unwrap())))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let value = bytes.get(offset..offset + 4).ok_or(Error::Corrupt("sqlite page field truncated"))?;
    Ok(u32::from_be_bytes(value.try_into().unwrap()))
}
