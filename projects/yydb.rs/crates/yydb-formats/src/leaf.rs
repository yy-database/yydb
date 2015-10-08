//! Slotted B+ tree leaf pages.

use yydb_types::{Error, Result};

use crate::{
    key::TreeKey,
    page::{PAGE_PAYLOAD_LEN, PAGE_TYPE_LEAF, PageHeader, encode_page},
};

const DIRECTORY_BYTES: usize = 4;
/// Cell header: shared key prefix length, key suffix length, value length.
const CELL_HEADER_BYTES: usize = 5;
/// Widest shared key prefix a cell header can express.
const MAX_COMMON_PREFIX: usize = u8::MAX as usize;

/// One key/value cell in a leaf page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafCell {
    /// Cell key.
    pub key: TreeKey,
    /// Cell value bytes.
    pub value: Vec<u8>,
}

/// Mutable slotted leaf representation.
#[derive(Debug, Clone, Default)]
pub struct LeafPage {
    /// Page id for encoding.
    pub page_id: u32,
    /// Page generation for encoding.
    pub page_generation: u64,
    /// Tree id stamped in the page header.
    pub tree_id: u8,
    cells: Vec<LeafCell>,
}

impl LeafPage {
    /// Create an empty leaf page.
    pub fn empty(page_id: u32, page_generation: u64, tree_id: u8) -> Self {
        Self { page_id, page_generation, tree_id, cells: Vec::new() }
    }

    /// Parse a leaf page image.
    pub fn decode(page_id: u32, page: &[u8]) -> Result<Self> {
        let header = PageHeader::parse(page)?;
        if header.page_type != PAGE_TYPE_LEAF {
            return Err(Error::Corrupt("expected leaf page"));
        }
        if header.page_id != page_id {
            return Err(Error::Corrupt("page id mismatch"));
        }
        let payload = header.payload(page)?;
        if payload.len() < DIRECTORY_BYTES {
            return Err(Error::Corrupt("leaf payload too short"));
        }
        let slot_count = u16::from_le_bytes(payload[0..2].try_into().unwrap()) as usize;
        let free_start = u16::from_le_bytes(payload[2..4].try_into().unwrap()) as usize;
        let directory_end = DIRECTORY_BYTES + slot_count * 4;
        if directory_end > free_start || free_start > payload.len() {
            return Err(Error::Corrupt("leaf directory invalid"));
        }
        let mut cells = Vec::with_capacity(slot_count);
        let mut key_bytes = Vec::with_capacity(64);
        for idx in 0..slot_count {
            let base = DIRECTORY_BYTES + idx * 4;
            let cell_offset = u16::from_le_bytes(payload[base..base + 2].try_into().unwrap()) as usize;
            let cell_len = u16::from_le_bytes(payload[base + 2..base + 4].try_into().unwrap()) as usize;
            let start = cell_offset;
            let end = start + cell_len;
            if end > free_start || start < directory_end {
                return Err(Error::Corrupt("leaf cell out of bounds"));
            }
            let cell = &payload[start..end];
            if cell.len() < CELL_HEADER_BYTES {
                return Err(Error::Corrupt("leaf cell truncated"));
            }
            let common = cell[0] as usize;
            let suffix_len = u16::from_le_bytes(cell[1..3].try_into().unwrap()) as usize;
            let value_len = u16::from_le_bytes(cell[3..5].try_into().unwrap()) as usize;
            let key_start = CELL_HEADER_BYTES;
            let key_end = key_start + suffix_len;
            let value_end = key_end + value_len;
            if value_end != cell.len() {
                return Err(Error::Corrupt("leaf cell length mismatch"));
            }
            if common > key_bytes.len() {
                return Err(Error::Corrupt("leaf cell prefix exceeds the previous key"));
            }
            key_bytes.truncate(common);
            key_bytes.extend_from_slice(&cell[key_start..key_end]);
            let key = TreeKey::from_bytes(&key_bytes).ok_or(Error::Corrupt("leaf key invalid"))?;
            if key.tree_id != header.tree_id {
                return Err(Error::Corrupt("leaf cell tree id mismatch"));
            }
            cells.push(LeafCell { key, value: cell[key_end..value_end].to_vec() });
        }
        for window in cells.windows(2) {
            if window[0].key >= window[1].key {
                return Err(Error::Corrupt("leaf keys unsorted"));
            }
        }
        Ok(Self { page_id, page_generation: header.page_generation, tree_id: header.tree_id, cells })
    }

    /// Encode to a full page image.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let payload = self.encode_payload()?;
        encode_page(PAGE_TYPE_LEAF, self.page_id, self.page_generation, self.tree_id, &payload)
    }

    fn encode_payload(&self) -> Result<Vec<u8>> {
        let slot_count = self.cells.len();
        let directory_bytes = DIRECTORY_BYTES + slot_count * 4;
        let mut cell_bytes: Vec<Vec<u8>> = Vec::with_capacity(slot_count);
        let mut total_cells = 0usize;
        let mut previous: Option<&TreeKey> = None;
        for cell in &self.cells {
            let encoded = encode_cell(previous, &cell.key, &cell.value)?;
            total_cells += encoded.len();
            cell_bytes.push(encoded);
            previous = Some(&cell.key);
        }
        let free_start = directory_bytes + total_cells;
        if free_start > PAGE_PAYLOAD_LEN {
            return Err(Error::Corrupt("leaf page full"));
        }
        let mut payload = vec![0_u8; free_start];
        payload[0..2].copy_from_slice(&(slot_count as u16).to_le_bytes());
        payload[2..4].copy_from_slice(&(free_start as u16).to_le_bytes());
        let mut offset = directory_bytes;
        for (idx, encoded) in cell_bytes.iter().enumerate() {
            let base = DIRECTORY_BYTES + idx * 4;
            payload[base..base + 2].copy_from_slice(&(offset as u16).to_le_bytes());
            payload[base + 2..base + 4].copy_from_slice(&(encoded.len() as u16).to_le_bytes());
            payload[offset..offset + encoded.len()].copy_from_slice(encoded);
            offset += encoded.len();
        }
        Ok(payload)
    }

    /// Lookup a key in this leaf.
    pub fn get(&self, key: &TreeKey) -> Option<&[u8]> {
        self.cells.iter().find(|cell| cell.key == *key).map(|cell| cell.value.as_slice())
    }

    /// Insert or replace a key. Returns `false` when the page has no space.
    pub fn upsert(&mut self, key: TreeKey, value: Vec<u8>) -> Result<bool> {
        if let Some(index) = self.cells.iter().position(|cell| cell.key == key) {
            let previous = std::mem::replace(&mut self.cells[index].value, value);
            if self.encode_payload().is_ok() {
                return Ok(true);
            }
            // Keep the in-memory leaf identical to the page on disk so the caller
            // can split and retry instead of silently dropping the stored value.
            self.cells[index].value = previous;
            return Ok(false);
        }
        let insert_at = self.cells.iter().position(|cell| cell.key > key).unwrap_or(self.cells.len());
        self.cells.insert(insert_at, LeafCell { key, value });
        match self.encode_payload() {
            Ok(_) => Ok(true),
            Err(Error::Corrupt("leaf page full")) => {
                self.cells.remove(insert_at);
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    /// Remove a key if present.
    pub fn delete(&mut self, key: &TreeKey) -> bool {
        if let Some(idx) = self.cells.iter().position(|cell| &cell.key == key) {
            self.cells.remove(idx);
            true
        }
        else {
            false
        }
    }

    /// Number of cells currently stored in this leaf.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether this leaf stores no cells.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Iterate cells in key order.
    pub fn cells(&self) -> &[LeafCell] {
        &self.cells
    }

    /// Take every cell out, leaving this leaf empty.
    pub fn take_cells(&mut self) -> Vec<LeafCell> {
        std::mem::take(&mut self.cells)
    }

    /// Replace every cell. The caller owns ascending key order.
    pub fn set_cells(&mut self, cells: Vec<LeafCell>) {
        self.cells = cells;
    }

    /// Payload bytes used by `cells`, including their directory slots.
    pub fn payload_usage(cells: &[LeafCell]) -> Result<usize> {
        payload_usage(cells)
    }

    /// On-disk cell length without building the encoded buffer.
    ///
    /// `previous` is the key stored before `cell` in the same page; the cell pays
    /// only the suffix that differs from it.
    pub fn cell_bytes_len(previous: Option<&TreeKey>, cell: &LeafCell) -> usize {
        let common = previous.map(|previous| common_key_prefix(previous, &cell.key)).unwrap_or(0);
        CELL_HEADER_BYTES + (2 + cell.key.encoded.len() - common) + cell.value.len()
    }

    /// Split into left (retained) and right leaf. Separator is the first key in `right`.
    pub fn split(&mut self) -> Result<(TreeKey, LeafPage)> {
        if self.cells.len() < 2 {
            return Err(Error::Corrupt("leaf too small to split"));
        }
        let split_at = self.cells.len() / 2;
        let right_cells = self.cells.split_off(split_at);
        let separator = right_cells[0].key.clone();
        let right = LeafPage { page_id: 0, page_generation: self.page_generation, tree_id: self.tree_id, cells: right_cells };
        Ok((separator, right))
    }

    /// Insert `incoming` and split into two leaves that both encode.
    ///
    /// Cell sizes vary by orders of magnitude: a row holding a large `bytes`
    /// payload is much larger than an integer cell. Splitting at the cell-count
    /// midpoint can therefore leave a half that cannot hold the incoming cell
    /// even when the same cell fits an otherwise empty page. This picks the most
    /// byte-balanced split point among those where both halves fit, and refuses
    /// the record when no leaf can hold it.
    ///
    /// An insert at either edge of the key order is the append (or prepend)
    /// shape that a sequential writer produces. There the balanced split is
    /// wrong: it would hand the new sibling half a page and leave the existing
    /// page half empty, so a bulk append ends up with twice the pages it needs.
    /// Edge inserts therefore keep every cell the page already held and start
    /// the sibling with only the incoming row.
    pub fn split_for_insert(&mut self, incoming: LeafCell) -> Result<(TreeKey, LeafPage)> {
        let insert_at = self.cells.iter().position(|cell| cell.key > incoming.key).unwrap_or(self.cells.len());
        let edge_split = if insert_at == 0 {
            Some(1)
        }
        else if insert_at == self.cells.len() {
            Some(self.cells.len())
        }
        else {
            None
        };
        self.cells.insert(insert_at, incoming);

        let split_at = match edge_split {
            Some(candidate) if self.split_fits(candidate)? => candidate,
            _ => self.balanced_split()?,
        };

        let right_cells = self.cells.split_off(split_at);
        let separator = right_cells[0].key.clone();
        let right = LeafPage { page_id: 0, page_generation: self.page_generation, tree_id: self.tree_id, cells: right_cells };
        Ok((separator, right))
    }

    /// Whether both halves around `split_at` fit one page.
    fn split_fits(&self, split_at: usize) -> Result<bool> {
        if split_at == 0 || split_at >= self.cells.len() {
            return Ok(false);
        }
        Ok(payload_usage(&self.cells[..split_at])? <= PAGE_PAYLOAD_LEN
            && payload_usage(&self.cells[split_at..])? <= PAGE_PAYLOAD_LEN)
    }

    /// Most byte-balanced split point among those where both halves fit.
    fn balanced_split(&self) -> Result<usize> {
        let mut best: Option<(usize, usize)> = None;
        for split_at in 1..self.cells.len() {
            let left = payload_usage(&self.cells[..split_at])?;
            let right = payload_usage(&self.cells[split_at..])?;
            if left > PAGE_PAYLOAD_LEN || right > PAGE_PAYLOAD_LEN {
                continue;
            }
            let imbalance = left.abs_diff(right);
            if best.is_none_or(|(_, current)| imbalance <= current) {
                best = Some((split_at, imbalance));
            }
        }
        best.map(|(split_at, _)| split_at).ok_or(Error::Unsupported("record is too large for one leaf page"))
    }
}

/// Payload bytes used by `cells`, including their directory slots.
///
/// The first cell pays its full key; every later cell pays only the suffix that
/// differs from the key before it, which is what [`encode_cell`] writes.
fn payload_usage(cells: &[LeafCell]) -> Result<usize> {
    let mut total = DIRECTORY_BYTES + cells.len() * 4;
    let mut previous: Option<&TreeKey> = None;
    for cell in cells {
        total += LeafPage::cell_bytes_len(previous, cell);
        previous = Some(&cell.key);
    }
    Ok(total)
}

/// Bytes `key` shares with the key stored before it in the same page.
///
/// Keys serialize as `tree_id`, `key_kind`, then the sortable payload, so the
/// comparison never allocates.
fn common_key_prefix(previous: &TreeKey, key: &TreeKey) -> usize {
    if previous.tree_id != key.tree_id {
        return 0;
    }
    if previous.key_kind != key.key_kind {
        return 1;
    }
    let shared = previous.encoded.iter().zip(key.encoded.iter()).take_while(|(left, right)| left == right).count();
    (2 + shared).min(MAX_COMMON_PREFIX)
}

fn encode_cell(previous: Option<&TreeKey>, key: &TreeKey, value: &[u8]) -> Result<Vec<u8>> {
    let key_bytes = key.to_bytes();
    let common = previous.map(|previous| common_key_prefix(previous, key)).unwrap_or(0);
    if common > key_bytes.len() {
        return Err(Error::Corrupt("leaf cell prefix exceeds the key length"));
    }
    let suffix = &key_bytes[common..];
    let suffix_len = u16::try_from(suffix.len()).map_err(|_| Error::Corrupt("leaf page full"))?;
    let value_len = u16::try_from(value.len()).map_err(|_| Error::Corrupt("leaf page full"))?;
    let mut cell = Vec::with_capacity(CELL_HEADER_BYTES + suffix.len() + value.len());
    cell.push(common as u8);
    cell.extend_from_slice(&suffix_len.to_le_bytes());
    cell.extend_from_slice(&value_len.to_le_bytes());
    cell.extend_from_slice(suffix);
    cell.extend_from_slice(value);
    Ok(cell)
}
