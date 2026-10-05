//! B+ tree internal pages.

use yydb_types::{Error, Result};

use crate::key::TreeKey;
use crate::page::{encode_page, PageHeader, PAGE_PAYLOAD_LEN, PAGE_TYPE_INTERNAL};

/// One child pointer in an internal page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalEntry {
    pub separator: TreeKey,
    pub child_page_id: u32,
}

/// Mutable internal node.
#[derive(Debug, Clone, Default)]
pub struct InternalPage {
    pub page_id: u32,
    pub page_generation: u64,
    pub tree_id: u8,
    pub entries: Vec<InternalEntry>,
}

impl InternalPage {
    /// Parse an internal page image.
    pub fn decode(page_id: u32, page: &[u8]) -> Result<Self> {
        let header = PageHeader::parse(page)?;
        if header.page_type != PAGE_TYPE_INTERNAL {
            return Err(Error::Corrupt("expected internal page"));
        }
        if header.page_id != page_id {
            return Err(Error::Corrupt("page id mismatch"));
        }
        let payload = header.payload(page)?;
        if payload.len() < 2 {
            return Err(Error::Corrupt("internal payload too short"));
        }
        let child_count = u16::from_le_bytes(payload[0..2].try_into().unwrap()) as usize;
        if child_count < 2 {
            return Err(Error::Corrupt("internal child count invalid"));
        }
        let mut cursor = 2usize;
        let mut entries = Vec::with_capacity(child_count);
        for _ in 0..child_count {
            if cursor + 6 > payload.len() {
                return Err(Error::Corrupt("internal entry truncated"));
            }
            let separator_len =
                u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
            let child_page_id = u32::from_le_bytes(payload[cursor + 2..cursor + 6].try_into().unwrap());
            cursor += 6;
            let end = cursor + separator_len;
            if end > payload.len() {
                return Err(Error::Corrupt("internal separator truncated"));
            }
            let separator = TreeKey::from_bytes(&payload[cursor..end])
                .ok_or(Error::Corrupt("internal separator invalid"))?;
            cursor = end;
            entries.push(InternalEntry {
                separator,
                child_page_id,
            });
        }
        Ok(Self {
            page_id,
            page_generation: header.page_generation,
            tree_id: header.tree_id,
            entries,
        })
    }

    /// Encode to a full page image.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.entries.len() < 2 {
            return Err(Error::Corrupt("internal page needs two children"));
        }
        let mut payload = Vec::new();
        payload.extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        for entry in &self.entries {
            let separator = entry.separator.to_bytes();
            payload.extend_from_slice(&(separator.len() as u16).to_le_bytes());
            payload.extend_from_slice(&entry.child_page_id.to_le_bytes());
            payload.extend_from_slice(&separator);
        }
        if payload.len() > PAGE_PAYLOAD_LEN {
            return Err(Error::Corrupt("internal page too large"));
        }
        encode_page(
            PAGE_TYPE_INTERNAL,
            self.page_id,
            self.page_generation,
            self.tree_id,
            &payload,
        )
    }

    /// Pick child index for `key` (last separator <= key).
    pub fn child_index_for(&self, key: &TreeKey) -> usize {
        let mut idx = 0usize;
        for (i, entry) in self.entries.iter().enumerate().skip(1) {
            if key < &entry.separator {
                break;
            }
            idx = i;
        }
        idx
    }

    pub fn child_page_id(&self, index: usize) -> u32 {
        self.entries[index].child_page_id
    }
}
