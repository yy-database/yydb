//! Page 0 dual-slot `DatabaseHeader`.

use yydb_types::{Error, Result};

use crate::crc32c::crc32c;

pub const PAGE_MAGIC: &[u8; 5] = b"YDPG\x00";
pub const PAGE_SIZE: usize = 4096;
pub const SLOT_BYTES: usize = 2048;

/// Parsed database header slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseHeaderSlot {
    pub slot_kind: u8,
    pub generation: u64,
    pub checkpoint_lsn: u64,
    pub database_id: [u8; 16],
    pub storage_layout: u8,
    pub page_size_log2: u8,
    pub format_version: u32,
    pub catalog_root: u32,
    pub record_root: u32,
    pub primary_index_root: u32,
    pub secondary_index_root: u32,
    pub expiry_index_root: u32,
    pub quota_index_root: u32,
    pub manifest_root: u32,
}

/// Best header slot chosen from page 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseHeader {
    pub slot: DatabaseHeaderSlot,
}

fn parse_slot(bytes: &[u8], offset: usize) -> Result<DatabaseHeaderSlot> {
    let end = offset + SLOT_BYTES;
    if bytes.len() < end {
        return Err(Error::Corrupt("header slot truncated"));
    }
    let slot = &bytes[offset..end];
    if slot.get(0..5) != Some(PAGE_MAGIC.as_slice()) {
        return Err(Error::Corrupt("header magic mismatch"));
    }
    let stored = u32::from_le_bytes(slot[2044..2048].try_into().unwrap());
    let computed = crc32c(&slot[..2044]);
    if stored != computed {
        return Err(Error::Corrupt("header slot checksum mismatch"));
    }
    Ok(DatabaseHeaderSlot {
        slot_kind: slot[5],
        generation: u64::from_le_bytes(slot[6..14].try_into().unwrap()),
        checkpoint_lsn: u64::from_le_bytes(slot[14..22].try_into().unwrap()),
        database_id: slot[22..38].try_into().unwrap(),
        storage_layout: slot[38],
        page_size_log2: slot[39],
        format_version: u32::from_le_bytes(slot[40..44].try_into().unwrap()),
        catalog_root: u32::from_le_bytes(slot[44..48].try_into().unwrap()),
        record_root: u32::from_le_bytes(slot[48..52].try_into().unwrap()),
        primary_index_root: u32::from_le_bytes(slot[52..56].try_into().unwrap()),
        secondary_index_root: u32::from_le_bytes(slot[56..60].try_into().unwrap()),
        expiry_index_root: u32::from_le_bytes(slot[60..64].try_into().unwrap()),
        quota_index_root: u32::from_le_bytes(slot[64..68].try_into().unwrap()),
        manifest_root: u32::from_le_bytes(slot[68..72].try_into().unwrap()),
    })
}

/// Parse page 0 and return the winning header slot.
pub fn parse_page0(bytes: &[u8]) -> Result<DatabaseHeader> {
    if bytes.len() < PAGE_SIZE {
        return Err(Error::Corrupt("page0 too short"));
    }
    let slot_a = parse_slot(bytes, 0);
    let slot_b = parse_slot(bytes, SLOT_BYTES);
    match (slot_a, slot_b) {
        (Ok(a), Ok(b)) => {
            let pick = if a.generation > b.generation {
                a
            } else if b.generation > a.generation {
                b
            } else if a.checkpoint_lsn >= b.checkpoint_lsn {
                a
            } else {
                b
            };
            Ok(DatabaseHeader { slot: pick })
        }
        (Ok(slot), Err(_)) | (Err(_), Ok(slot)) => Ok(DatabaseHeader { slot }),
        (Err(_), Err(_)) => Err(Error::Corrupt("header slot checksum mismatch")),
    }
}

/// Build an empty dual-slot page 0 for tests and bootstrap.
pub fn encode_empty_page0(database_id: [u8; 16], storage_layout: u8) -> Vec<u8> {
    let mut page = vec![0_u8; PAGE_SIZE];
    for (idx, kind) in [(0, 0x41_u8), (SLOT_BYTES, 0x42_u8)] {
        let slot = &mut page[idx..idx + SLOT_BYTES];
        slot[0..5].copy_from_slice(PAGE_MAGIC);
        slot[5] = kind;
        slot[6..14].copy_from_slice(&1u64.to_le_bytes());
        slot[22..38].copy_from_slice(&database_id);
        slot[38] = storage_layout;
        slot[39] = 12;
        slot[40..44].copy_from_slice(&0u32.to_le_bytes());
        let checksum = crc32c(&slot[..2044]);
        slot[2044..2048].copy_from_slice(&checksum.to_le_bytes());
    }
    page
}
