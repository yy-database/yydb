//! Slotted B+ tree leaf pages.

use yydb_types::{Error, Result};

use crate::key::TreeKey;
use crate::page::{encode_page, PageHeader, PAGE_PAYLOAD_LEN, PAGE_TYPE_LEAF};

const DIRECTORY_BYTES: usize = 4;

/// One key/value cell in a leaf page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeafCell {
    pub key: TreeKey,
    pub value: Vec<u8>,
}

/// Mutable slotted leaf representation.
#[derive(Debug, Clone, Default)]
pub struct LeafPage {
    pub page_id: u32,
    pub page_generation: u64,
    pub tree_id: u8,
    cells: Vec<LeafCell>,
}

impl LeafPage {
    /// Create an empty leaf page.
    pub fn empty(page_id: u32, page_generation: u64, tree_id: u8) -> Self {
        Self {
            page_id,
            page_generation,
            tree_id,
            cells: Vec::new(),
        }
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
            if cell.len() < 8 {
                return Err(Error::Corrupt("leaf cell truncated"));
            }
            let key_kind = cell[0];
            let tree_id = cell[1];
            if tree_id != header.tree_id {
                return Err(Error::Corrupt("leaf cell tree id mismatch"));
            }
            let key_len = u16::from_le_bytes(cell[2..4].try_into().unwrap()) as usize;
            let value_len = u32::from_le_bytes(cell[4..8].try_into().unwrap()) as usize;
            let key_end = 8 + key_len;
            let value_end = key_end + value_len;
            if value_end != cell.len() {
                return Err(Error::Corrupt("leaf cell length mismatch"));
            }
            let key = TreeKey::from_bytes(&cell[8..key_end])
                .ok_or(Error::Corrupt("leaf key invalid"))?;
            if key.key_kind != key_kind {
                return Err(Error::Corrupt("leaf key kind mismatch"));
            }
            cells.push(LeafCell {
                key,
                value: cell[key_end..value_end].to_vec(),
            });
        }
        for window in cells.windows(2) {
            if window[0].key >= window[1].key {
                return Err(Error::Corrupt("leaf keys unsorted"));
            }
        }
        Ok(Self {
            page_id,
            page_generation: header.page_generation,
            tree_id: header.tree_id,
            cells,
        })
    }

    /// Encode to a full page image.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let payload = self.encode_payload()?;
        encode_page(
            PAGE_TYPE_LEAF,
            self.page_id,
            self.page_generation,
            self.tree_id,
            &payload,
        )
    }

    fn encode_payload(&self) -> Result<Vec<u8>> {
        let slot_count = self.cells.len();
        let directory_bytes = DIRECTORY_BYTES + slot_count * 4;
        let mut cell_bytes: Vec<Vec<u8>> = Vec::with_capacity(slot_count);
        let mut total_cells = 0usize;
        for cell in &self.cells {
            let encoded = encode_cell(&cell.key, &cell.value)?;
            total_cells += encoded.len();
            cell_bytes.push(encoded);
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
        self.cells
            .iter()
            .find(|cell| cell.key == *key)
            .map(|cell| cell.value.as_slice())
    }

    /// Insert or replace a key. Returns `false` when the page has no space.
    pub fn upsert(&mut self, key: TreeKey, value: Vec<u8>) -> Result<bool> {
        if let Some(cell) = self.cells.iter_mut().find(|cell| cell.key == key) {
            cell.value = value;
            return Ok(self.encode_payload().is_ok());
        }
        let insert_at = self
            .cells
            .iter()
            .position(|cell| cell.key > key)
            .unwrap_or(self.cells.len());
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
        } else {
            false
        }
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Iterate cells in key order.
    pub fn cells(&self) -> &[LeafCell] {
        &self.cells
    }

    /// Split into left (retained) and right leaf. Separator is the first key in `right`.
    pub fn split(&mut self) -> Result<(TreeKey, LeafPage)> {
        if self.cells.len() < 2 {
            return Err(Error::Corrupt("leaf too small to split"));
        }
        let split_at = self.cells.len() / 2;
        let right_cells = self.cells.split_off(split_at);
        let separator = right_cells[0].key.clone();
        let right = LeafPage {
            page_id: 0,
            page_generation: self.page_generation,
            tree_id: self.tree_id,
            cells: right_cells,
        };
        Ok((separator, right))
    }
}

fn encode_cell(key: &TreeKey, value: &[u8]) -> Result<Vec<u8>> {
    let key_bytes = key.to_bytes();
    let mut cell = Vec::with_capacity(8 + key_bytes.len() + value.len());
    cell.push(key.key_kind);
    cell.push(key.tree_id);
    cell.extend_from_slice(&(key_bytes.len() as u16).to_le_bytes());
    cell.extend_from_slice(&(value.len() as u32).to_le_bytes());
    cell.extend_from_slice(&key_bytes);
    cell.extend_from_slice(value);
    Ok(cell)
}
