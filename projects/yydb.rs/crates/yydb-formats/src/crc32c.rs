//! Castagnoli CRC32C used by page and WAL checksums.
//!
//! Slice-by-8 table implementation. The polynomial, initial value and final
//! xor are unchanged, so every stored checksum keeps its meaning and no page
//! image changes shape: this only removes the per-byte bit loop that used to
//! dominate every page load.

const POLY: u32 = 0x82F63B78;

/// Tables for slice-by-8: `TABLES[0]` is the classic byte table, each further
/// table is the previous one shifted by one byte.
static TABLES: [[u32; 256]; 8] = build_tables();

const fn build_tables() -> [[u32; 256]; 8] {
    let mut tables = [[0_u32; 256]; 8];
    let mut index = 0;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ POLY } else { crc >> 1 };
            bit += 1;
        }
        tables[0][index] = crc;
        index += 1;
    }
    let mut table = 1;
    while table < 8 {
        let mut entry = 0;
        while entry < 256 {
            let previous = tables[table - 1][entry];
            tables[table][entry] = (previous >> 8) ^ tables[0][(previous & 0xFF) as usize];
            entry += 1;
        }
        table += 1;
    }
    tables
}

/// Compute CRC32C over `data`.
pub fn crc32c(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF;
    let mut chunks = data.chunks_exact(8);
    for chunk in &mut chunks {
        let low = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) ^ crc;
        let high = u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
        crc = TABLES[7][(low & 0xFF) as usize]
            ^ TABLES[6][((low >> 8) & 0xFF) as usize]
            ^ TABLES[5][((low >> 16) & 0xFF) as usize]
            ^ TABLES[4][(low >> 24) as usize]
            ^ TABLES[3][(high & 0xFF) as usize]
            ^ TABLES[2][((high >> 8) & 0xFF) as usize]
            ^ TABLES[1][((high >> 16) & 0xFF) as usize]
            ^ TABLES[0][(high >> 24) as usize];
    }
    for byte in chunks.remainder() {
        crc = (crc >> 8) ^ TABLES[0][((crc ^ *byte as u32) & 0xFF) as usize];
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::{POLY, crc32c};

    fn bitwise(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFF;
        for byte in data {
            crc ^= *byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ POLY } else { crc >> 1 };
            }
        }
        crc ^ 0xFFFF_FFFF
    }

    #[test]
    fn matches_standard_vectors() {
        assert_eq!(crc32c(b""), 0);
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn matches_bitwise_reference_for_every_remainder() {
        let data: Vec<u8> = (0..=255).chain(0..=255).collect();
        for len in 0..=64 {
            let prefix = &data[..len];
            assert_eq!(crc32c(prefix), bitwise(prefix), "length {len}");
        }
        assert_eq!(crc32c(&data), bitwise(&data));
    }
}
