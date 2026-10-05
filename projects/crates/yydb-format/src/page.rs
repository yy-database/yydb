//! Data page header and checksum envelope.

use yydb_types::{Error, Result};

use crate::crc32c::crc32c;
use crate::header::PAGE_SIZE;

/// Bytes before the slotted payload.
pub const PAGE_HEADER_LEN: usize = 16;
/// Maximum slotted payload bytes.
pub const PAGE_PAYLOAD_LEN: usize = PAGE_SIZE - PAGE_HEADER_LEN - 4;

pub const PAGE_TYPE_LEAF: u8 = 0x10;
pub const PAGE_TYPE_INTERNAL: u8 = 0x11;

/// Parsed data-page header (pages >= 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageHeader {
    pub page_type: u8,
    pub page_id: u32,
    pub page_generation: u64,
    pub payload_len: u16,
    pub tree_id: u8,
}

impl PageHeader {
    /// Read header from a full page image.
    pub fn parse(page: &[u8]) -> Result<Self> {
        if page.len() != PAGE_SIZE {
            return Err(Error::Corrupt("page wrong size"));
        }
        let stored = u32::from_le_bytes(page[4092..4096].try_into().unwrap());
        if crc32c(&page[..4092]) != stored {
            return Err(Error::Corrupt("page checksum mismatch"));
        }
        Ok(Self {
            page_type: page[0],
            page_id: u32::from_le_bytes(page[1..5].try_into().unwrap()),
            page_generation: u64::from_le_bytes(page[5..13].try_into().unwrap()),
            payload_len: u16::from_le_bytes(page[13..15].try_into().unwrap()),
            tree_id: page[15],
        })
    }

    /// Payload slice after validation.
    pub fn payload<'a>(&self, page: &'a [u8]) -> Result<&'a [u8]> {
        let end = PAGE_HEADER_LEN + self.payload_len as usize;
        if end > PAGE_HEADER_LEN + PAGE_PAYLOAD_LEN {
            return Err(Error::Corrupt("page payload length invalid"));
        }
        Ok(&page[PAGE_HEADER_LEN..end])
    }
}

/// Build a full page image from header fields and payload bytes.
pub fn encode_page(
    page_type: u8,
    page_id: u32,
    page_generation: u64,
    tree_id: u8,
    payload: &[u8],
) -> Result<Vec<u8>> {
    if payload.len() > PAGE_PAYLOAD_LEN {
        return Err(Error::Corrupt("page payload too large"));
    }
    let mut page = vec![0_u8; PAGE_SIZE];
    page[0] = page_type;
    page[1..5].copy_from_slice(&page_id.to_le_bytes());
    page[5..13].copy_from_slice(&page_generation.to_le_bytes());
    page[13..15].copy_from_slice(&(payload.len() as u16).to_le_bytes());
    page[15] = tree_id;
    page[PAGE_HEADER_LEN..PAGE_HEADER_LEN + payload.len()].copy_from_slice(payload);
    let checksum = crc32c(&page[..4092]);
    page[4092..4096].copy_from_slice(&checksum.to_le_bytes());
    Ok(page)
}
