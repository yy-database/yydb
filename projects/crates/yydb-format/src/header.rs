//! Page 0 dual-slot `DatabaseHeader`.

use yydb_types::{Error, Result};

use crate::crc32c::crc32c;

/// Five-byte magic prefix for v0 page files.
pub const PAGE_MAGIC: &[u8; 5] = b"YDPG\x00";
/// Fixed page size for v0 on-disk layout.
pub const PAGE_SIZE: usize = 4096;
/// Bytes per header slot on page 0.
pub const SLOT_BYTES: usize = 2048;
/// Slot kind marker for header slot A.
pub const SLOT_KIND_A: u8 = 0x41;
/// Slot kind marker for header slot B.
pub const SLOT_KIND_B: u8 = 0x42;

/// Parsed database header slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseHeaderSlot {
    /// Active slot marker (`0x01` primary, `0x02` alternate).
    pub slot_kind: u8,
    /// Monotonic header generation.
    pub generation: u64,
    /// Last checkpoint LSN reflected in this slot.
    pub checkpoint_lsn: u64,
    /// Stable database identifier.
    pub database_id: [u8; 16],
    /// Storage layout discriminator.
    pub storage_layout: u8,
    /// `log2(page_size)` (12 for 4096-byte pages).
    pub page_size_log2: u8,
    /// Format version (`0` for v0).
    pub format_version: u32,
    /// Root page id for the catalog tree.
    pub catalog_root: u32,
    /// Root page id for the record tree.
    pub record_root: u32,
    /// Root page id for primary indexes.
    pub primary_index_root: u32,
    /// Root page id for secondary indexes.
    pub secondary_index_root: u32,
    /// Root page id for the expiry index.
    pub expiry_index_root: u32,
    /// Root page id for quota metadata.
    pub quota_index_root: u32,
    /// Root page id for the object manifest tree.
    pub manifest_root: u32,
}

/// Best header slot chosen from page 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseHeader {
    /// Winning header slot from page 0.
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

/// Byte offset of a header slot on page 0.
pub fn slot_offset_for_kind(slot_kind: u8) -> usize {
    if slot_kind == SLOT_KIND_B {
        SLOT_BYTES
    } else {
        0
    }
}

/// Offset of the inactive header slot paired with `active_offset`.
pub fn inactive_slot_offset(active_offset: usize) -> usize {
    if active_offset == 0 {
        SLOT_BYTES
    } else {
        0
    }
}

/// Invalidate the checksum of one header slot (crash-injection helper for tests).
#[doc(hidden)]
pub fn corrupt_header_slot_checksum(page0: &mut [u8], offset: usize) -> Result<()> {
    let end = offset + SLOT_BYTES;
    if page0.len() < end {
        return Err(Error::Corrupt("header slot truncated"));
    }
    page0[offset + 2044..offset + 2048].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    Ok(())
}

/// Recompute and store the CRC32C checksum for one header slot.
pub fn seal_header_slot(page0: &mut [u8], offset: usize) -> Result<()> {
    let end = offset + SLOT_BYTES;
    if page0.len() < end {
        return Err(Error::Corrupt("header slot truncated"));
    }
    let checksum = crc32c(&page0[offset..offset + 2044]);
    page0[offset + 2044..offset + 2048].copy_from_slice(&checksum.to_le_bytes());
    Ok(())
}

/// Build an empty dual-slot page 0 for tests and bootstrap.
pub fn encode_empty_page0(database_id: [u8; 16], storage_layout: u8) -> Vec<u8> {
    let mut page = vec![0_u8; PAGE_SIZE];
    for (idx, kind) in [(0, SLOT_KIND_A), (SLOT_BYTES, SLOT_KIND_B)] {
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
