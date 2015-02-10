//! SQLite format 3 header and blank-database materialization (pure Rust).

use yydb_types::{Error, Result};

/// SQLite format 3 magic prefix.
pub const MAGIC: &[u8] = b"SQLite format 3\0";

/// Default page size for newly created YYDS SQLite files.
pub const DEFAULT_PAGE_SIZE: usize = 4096;

/// Library version stamped into YYDS blank database headers.
pub const ENGINE_LIBRARY_VERSION: &str = "3.45.0";

/// Packed SQLite library version integer written to the database header.
pub const ENGINE_LIBRARY_VERSION_NUMBER: u32 = 3_045_000;

/// Canonical blank database bytes (one 4096-byte page, validated by tests).
const BLANK_DATABASE: &[u8] = include_bytes!("assets/blank.sqlite");

/// Returns validated blank database bytes with the YYDS engine version stamped in.
pub fn blank_database() -> Result<Vec<u8>> {
    if BLANK_DATABASE.len() != DEFAULT_PAGE_SIZE {
        return Err(Error::Corrupt("blank sqlite asset has unexpected size"));
    }
    validate_header(BLANK_DATABASE)?;

    let mut pages = BLANK_DATABASE.to_vec();
    stamp_library_version(&mut pages, ENGINE_LIBRARY_VERSION_NUMBER);
    Ok(pages)
}

/// Validates the SQLite file header in `bytes`.
pub fn validate_header(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 100 {
        return Err(Error::Corrupt("sqlite header too short"));
    }
    if bytes.get(..MAGIC.len()) != Some(MAGIC) {
        return Err(Error::Corrupt("sqlite magic mismatch"));
    }

    let page_size = decode_page_size(bytes)?;
    if page_size < 512 || !page_size.is_power_of_two() {
        return Err(Error::Corrupt("sqlite page size invalid"));
    }

    if !matches!(bytes[18], 1 | 2) || !matches!(bytes[19], 1 | 2) {
        return Err(Error::Unsupported("sqlite file read/write format version"));
    }
    if page_size - usize::from(bytes[20]) < 480 {
        return Err(Error::Corrupt("sqlite usable page size too small"));
    }
    if bytes[21..24] != [64, 32, 32] {
        return Err(Error::Corrupt("sqlite payload fractions invalid"));
    }
    if read_u32(bytes, 44) > 4 {
        return Err(Error::Unsupported("sqlite schema format version"));
    }
    if read_u32(bytes, 56) > 3 {
        return Err(Error::Corrupt("sqlite text encoding invalid"));
    }
    if bytes[72..92].iter().any(|byte| *byte != 0) {
        return Err(Error::Corrupt("sqlite reserved header bytes are nonzero"));
    }

    Ok(())
}

/// Validates an on-disk SQLite payload and returns its page size.
pub fn validate_database(bytes: &[u8]) -> Result<usize> {
    if bytes.is_empty() {
        return Ok(DEFAULT_PAGE_SIZE);
    }
    validate_header(bytes)?;
    let page_size = decode_page_size(bytes)?;
    if !bytes.len().is_multiple_of(page_size) {
        return Err(Error::Corrupt("sqlite payload is not page-aligned"));
    }
    let declared_pages = read_u32(bytes, 28);
    if declared_pages != 0
        && read_u32(bytes, 24) == read_u32(bytes, 92)
        && u64::from(declared_pages) > (bytes.len() / page_size) as u64
    {
        return Err(Error::Corrupt("sqlite declared page count exceeds file size"));
    }
    Ok(page_size)
}

/// Decodes the SQLite library version string from a database header.
pub fn decode_library_version(bytes: &[u8]) -> Result<String> {
    validate_header(bytes)?;
    Ok(format_library_version(read_library_version_number(bytes)))
}

/// Writes the SQLite library version integer into `bytes` (header must exist).
pub fn stamp_library_version(bytes: &mut [u8], version_number: u32) {
    if bytes.len() >= 100 {
        bytes[96..100].copy_from_slice(&version_number.to_be_bytes());
    }
}

fn decode_page_size(bytes: &[u8]) -> Result<usize> {
    let raw = u16::from_be_bytes([bytes[16], bytes[17]]);
    let page_size = if raw == 1 { 65_536 } else { raw as usize };
    Ok(page_size)
}

fn read_library_version_number(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[96], bytes[97], bytes[98], bytes[99]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("validated header offset"))
}

fn format_library_version(raw: u32) -> String {
    let major = raw / 1_000_000;
    let minor = (raw % 1_000_000) / 1_000;
    let patch = raw % 1_000;
    format!("{major}.{minor}.{patch}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank() -> Vec<u8> {
        BLANK_DATABASE.to_vec()
    }

    #[test]
    fn blank_database_is_one_validated_page() {
        let pages = blank_database().expect("blank database");
        assert_eq!(pages.len(), DEFAULT_PAGE_SIZE);
        assert_eq!(validate_database(&pages).expect("page size"), DEFAULT_PAGE_SIZE);
        assert_eq!(decode_library_version(&pages).expect("library version"), ENGINE_LIBRARY_VERSION);
        assert_eq!(&pages[..MAGIC.len()], MAGIC);
        assert_eq!(u16::from_be_bytes([pages[16], pages[17]]), DEFAULT_PAGE_SIZE as u16);
        assert_eq!(u32::from_be_bytes(pages[28..32].try_into().unwrap()), 1);
        assert_eq!(pages[100], 13, "page 1 must be a table leaf");
        assert_eq!(u16::from_be_bytes([pages[105], pages[106]]), DEFAULT_PAGE_SIZE as u16);
    }

    #[test]
    fn truncated_headers_are_rejected() {
        let short = decode_library_version(&blank()[..99]).expect_err("short header");
        assert_eq!(short.code(), "yydb.corrupt");
        assert!(short.to_string().contains("header too short"), "{short}");
        assert_eq!(validate_header(&[]).expect_err("empty header").code(), "yydb.corrupt");
    }

    #[test]
    fn header_field_corpus_returns_structured_errors() {
        let mut wrong_magic = blank();
        wrong_magic[0] = b'X';
        let magic = validate_header(&wrong_magic).expect_err("magic");
        assert_eq!(magic.code(), "yydb.corrupt");
        assert!(magic.to_string().contains("magic mismatch"), "{magic}");

        let mut bad_page_size = blank();
        bad_page_size[16..18].copy_from_slice(&1000u16.to_be_bytes());
        let size = validate_header(&bad_page_size).expect_err("page size");
        assert_eq!(size.code(), "yydb.corrupt");
        assert!(size.to_string().contains("page size invalid"), "{size}");

        let mut bad_read_version = blank();
        bad_read_version[18] = 3;
        let version = validate_header(&bad_read_version).expect_err("read version");
        assert_eq!(version.code(), "yydb.unsupported");
        assert!(version.to_string().contains("format version"), "{version}");

        let mut bad_write_version = blank();
        bad_write_version[19] = 0;
        assert_eq!(validate_header(&bad_write_version).expect_err("write version").code(), "yydb.unsupported");

        let mut tiny_usable = blank();
        tiny_usable[16..18].copy_from_slice(&512u16.to_be_bytes());
        tiny_usable[20] = 33;
        let usable = validate_header(&tiny_usable).expect_err("usable size");
        assert_eq!(usable.code(), "yydb.corrupt");
        assert!(usable.to_string().contains("usable page size"), "{usable}");

        let mut bad_fractions = blank();
        bad_fractions[21] = 63;
        let fractions = validate_header(&bad_fractions).expect_err("payload fractions");
        assert_eq!(fractions.code(), "yydb.corrupt");
        assert!(fractions.to_string().contains("payload fractions"), "{fractions}");

        let mut bad_schema_version = blank();
        bad_schema_version[44..48].copy_from_slice(&5u32.to_be_bytes());
        let schema_version = validate_header(&bad_schema_version).expect_err("schema version");
        assert_eq!(schema_version.code(), "yydb.unsupported");

        let mut bad_encoding = blank();
        bad_encoding[56..60].copy_from_slice(&4u32.to_be_bytes());
        let encoding = validate_header(&bad_encoding).expect_err("encoding");
        assert_eq!(encoding.code(), "yydb.corrupt");
        assert!(encoding.to_string().contains("text encoding"), "{encoding}");

        let mut reserved = blank();
        reserved[72] = 1;
        let reserved_error = validate_header(&reserved).expect_err("reserved bytes");
        assert_eq!(reserved_error.code(), "yydb.corrupt");
        assert!(reserved_error.to_string().contains("reserved header"), "{reserved_error}");
    }

    #[test]
    fn database_shape_corpus_returns_structured_errors() {
        let misaligned = validate_database(&blank()[..DEFAULT_PAGE_SIZE - 1]).expect_err("misaligned");
        assert_eq!(misaligned.code(), "yydb.corrupt");
        assert!(misaligned.to_string().contains("page-aligned"), "{misaligned}");

        let mut declared = blank();
        declared[28..32].copy_from_slice(&2u32.to_be_bytes());
        let declared_error = validate_database(&declared).expect_err("declared pages");
        assert_eq!(declared_error.code(), "yydb.corrupt");
        assert!(declared_error.to_string().contains("declared page count"), "{declared_error}");

        assert_eq!(validate_database(&[]).expect("empty database is valid"), DEFAULT_PAGE_SIZE);
    }

    #[test]
    fn library_version_round_trips() {
        assert_eq!(format_library_version(ENGINE_LIBRARY_VERSION_NUMBER), ENGINE_LIBRARY_VERSION);
        let mut pages = blank();
        stamp_library_version(&mut pages, 3_042_001);
        assert_eq!(decode_library_version(&pages).expect("decode stamped version"), "3.42.1");

        let mut too_short = [0u8; 99];
        stamp_library_version(&mut too_short, ENGINE_LIBRARY_VERSION_NUMBER);
        assert_eq!(too_short[96..], [0u8; 3], "short buffers must not be written");
    }
}
