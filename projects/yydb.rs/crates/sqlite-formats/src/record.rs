//! SQLite record payload decoding, independent of YYDS and YYKV execution.

use yydb_types::{Error, Result};

/// Database text encoding specified by the format-3 header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextEncoding {
    /// UTF-8.
    Utf8,
    /// UTF-16 little endian.
    Utf16Le,
    /// UTF-16 big endian.
    Utf16Be,
}

/// A decoded SQLite record field. Blob bytes borrow the supplied payload.
#[derive(Debug, PartialEq)]
pub enum RecordValue<'input> {
    /// SQL NULL.
    Null,
    /// Signed SQLite integer.
    Integer(i64),
    /// IEEE-754 binary64 value.
    Real(f64),
    /// Binary bytes without UTF-8 conversion.
    Blob(&'input [u8]),
    /// Text decoded using the database encoding.
    Text(String),
}

/// Resource limits for a single materialized record payload.
#[derive(Clone, Copy, Debug)]
pub struct RecordLimits {
    /// Maximum payload size, including header.
    pub max_payload_bytes: usize,
    /// Maximum fields in a record.
    pub max_columns: usize,
}

impl Default for RecordLimits {
    fn default() -> Self {
        Self { max_payload_bytes: 1_048_576, max_columns: 1024 }
    }
}

/// Decodes a one-to-nine-byte SQLite unsigned varint and its consumed length.
pub fn decode_varint(bytes: &[u8]) -> Result<(u64, usize)> {
    let mut value = 0u64;
    for index in 0..9 {
        let byte = *bytes.get(index).ok_or(Error::Corrupt("truncated sqlite varint"))?;
        if index == 8 {
            return Ok(((value << 8) | u64::from(byte), 9));
        }
        value = (value << 7) | u64::from(byte & 0x7f);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    unreachable!()
}

/// Decodes one complete record payload after page/overflow assembly.
/// It does not traverse b-trees, recover journals, or execute SQL.
pub fn decode_record(payload: &[u8], encoding: TextEncoding, limits: RecordLimits) -> Result<Vec<RecordValue<'_>>> {
    if payload.len() > limits.max_payload_bytes {
        return Err(Error::Unsupported("sqlite record payload limit exceeded"));
    }
    let (header_size, mut header_offset) = decode_varint(payload)?;
    let header_size = usize::try_from(header_size).map_err(|_| Error::Corrupt("sqlite record header size overflow"))?;
    if header_size < header_offset || header_size > payload.len() {
        return Err(Error::Corrupt("sqlite record header size invalid"));
    }
    let mut body_offset = header_size;
    let mut values = Vec::new();
    while header_offset < header_size {
        if values.len() >= limits.max_columns {
            return Err(Error::Unsupported("sqlite record column limit exceeded"));
        }
        let (serial, consumed) = decode_varint(&payload[header_offset..header_size])?;
        header_offset += consumed;
        let length = match serial {
            0 | 8 | 9 => 0,
            1 => 1,
            2 => 2,
            3 => 3,
            4 => 4,
            5 => 6,
            6 | 7 => 8,
            10 | 11 => return Err(Error::Corrupt("sqlite reserved serial type")),
            serial => usize::try_from((serial - 12) / 2).map_err(|_| Error::Corrupt("sqlite serial length overflow"))?,
        };
        let end = body_offset
            .checked_add(length)
            .filter(|end| *end <= payload.len())
            .ok_or(Error::Corrupt("truncated sqlite record body"))?;
        let bytes = &payload[body_offset..end];
        let value = match serial {
            0 => RecordValue::Null,
            1..=6 => RecordValue::Integer(signed_integer(bytes)),
            7 => RecordValue::Real(f64::from_bits(u64::from_be_bytes(bytes.try_into().expect("eight-byte real")))),
            8 => RecordValue::Integer(0),
            9 => RecordValue::Integer(1),
            serial if serial % 2 == 0 => RecordValue::Blob(bytes),
            _ => RecordValue::Text(decode_text(bytes, encoding)?),
        };
        values.push(value);
        body_offset = end;
    }
    if body_offset != payload.len() {
        return Err(Error::Corrupt("sqlite record has trailing body bytes"));
    }
    Ok(values)
}

fn signed_integer(bytes: &[u8]) -> i64 {
    let mut value = if bytes[0] & 0x80 != 0 { u64::MAX } else { 0 };
    for byte in bytes {
        value = (value << 8) | u64::from(*byte)
    }
    value as i64
}

fn decode_text(bytes: &[u8], encoding: TextEncoding) -> Result<String> {
    if encoding == TextEncoding::Utf8 {
        return std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| Error::Unsupported("sqlite non-unicode UTF-8 text"));
    }
    if bytes.len() % 2 != 0 {
        return Err(Error::Corrupt("sqlite odd UTF-16 byte length"));
    }
    let words = bytes.chunks_exact(2).map(|chunk| match encoding {
        TextEncoding::Utf16Le => u16::from_le_bytes([chunk[0], chunk[1]]),
        TextEncoding::Utf16Be => u16::from_be_bytes([chunk[0], chunk[1]]),
        TextEncoding::Utf8 => unreachable!(),
    });
    char::decode_utf16(words)
        .collect::<std::result::Result<String, _>>()
        .map_err(|_| Error::Unsupported("sqlite non-unicode UTF-16 text"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_varint(out: &mut Vec<u8>, value: u64) {
        if value > 0x00ff_ffff_ffff_ffff {
            let mut bytes = [0u8; 9];
            for (index, byte) in bytes.iter_mut().take(8).enumerate() {
                *byte = (((value >> (57 - 7 * index)) & 0x7f) as u8) | 0x80;
            }
            bytes[8] = (value & 0xff) as u8;
            out.extend_from_slice(&bytes);
            return;
        }
        let mut bytes = [0u8; 8];
        let mut index = 7;
        bytes[index] = (value & 0x7f) as u8;
        let mut rest = value >> 7;
        while rest > 0 {
            index -= 1;
            bytes[index] = ((rest & 0x7f) as u8) | 0x80;
            rest >>= 7;
        }
        out.extend_from_slice(&bytes[index..]);
    }

    fn record(serials: &[u64], body: &[u8]) -> Vec<u8> {
        let mut header = Vec::new();
        for serial in serials {
            push_varint(&mut header, *serial);
        }
        let mut payload = Vec::new();
        push_varint(&mut payload, (header.len() + 1) as u64);
        payload.extend_from_slice(&header);
        payload.extend_from_slice(body);
        payload
    }

    fn decode(payload: &[u8], encoding: TextEncoding) -> Result<Vec<RecordValue<'_>>> {
        decode_record(payload, encoding, RecordLimits::default())
    }

    fn error(payload: &[u8], encoding: TextEncoding) -> Error {
        decode(payload, encoding).expect_err("malformed record must not decode")
    }

    #[test]
    fn varint_covers_every_byte_length() {
        assert_eq!(decode_varint(&[0x00]).expect("one byte"), (0, 1));
        assert_eq!(decode_varint(&[0x7f]).expect("one byte max"), (127, 1));
        assert_eq!(decode_varint(&[0x81, 0x00]).expect("two bytes"), (128, 2));
        assert_eq!(decode_varint(&[0x82, 0x01]).expect("two bytes offset"), (257, 2));
        assert_eq!(decode_varint(&[0xff, 0x7f]).expect("two bytes max"), (16_383, 2));
        assert_eq!(
            decode_varint(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]).expect("eight bytes"),
            (0x00ff_ffff_ffff_ffff, 8)
        );
        assert_eq!(decode_varint(&[0xff; 9]).expect("nine bytes"), (u64::MAX, 9));

        let truncated = decode_varint(&[0x80]).expect_err("continuation without payload");
        assert_eq!(truncated.code(), "yydb.corrupt");
        assert_eq!(decode_varint(&[]).expect_err("empty varint").code(), "yydb.corrupt");
    }

    #[test]
    fn integer_serial_types_cover_the_signed_range() {
        let payload = record(
            &[1, 2, 3, 4, 5, 6, 6],
            &[
                0x80, // -128
                0x80, 0x00, // -32768
                0x80, 0x00, 0x00, // -8388608
                0x80, 0x00, 0x00, 0x00, // -2147483648
                0x80, 0x00, 0x00, 0x00, 0x00, 0x00, // -2^47
                0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // i64::MIN
                0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, // i64::MAX
            ],
        );
        let values = decode(&payload, TextEncoding::Utf8).expect("decode integers");
        assert_eq!(
            values,
            [
                RecordValue::Integer(-128),
                RecordValue::Integer(-32_768),
                RecordValue::Integer(-8_388_608),
                RecordValue::Integer(-2_147_483_648),
                RecordValue::Integer(-140_737_488_355_328),
                RecordValue::Integer(i64::MIN),
                RecordValue::Integer(i64::MAX),
            ]
        );
    }

    #[test]
    fn constants_null_and_zero_length_classes_decode() {
        let payload = record(&[0, 8, 9, 12, 13], &[]);
        assert_eq!(
            decode(&payload, TextEncoding::Utf8).expect("decode constants"),
            [
                RecordValue::Null,
                RecordValue::Integer(0),
                RecordValue::Integer(1),
                RecordValue::Blob(&[]),
                RecordValue::Text(String::new()),
            ]
        );
    }

    #[test]
    fn real_and_blob_classes_preserve_exact_bits() {
        let negative_zero = (-0.0f64).to_bits().to_be_bytes();
        let payload = record(
            &[7, 14],
            &[
                negative_zero[0],
                negative_zero[1],
                negative_zero[2],
                negative_zero[3],
                negative_zero[4],
                negative_zero[5],
                negative_zero[6],
                negative_zero[7],
                0x00,
            ],
        );
        let values = decode(&payload, TextEncoding::Utf8).expect("decode real and blob");
        match &values[0] {
            RecordValue::Real(value) => assert_eq!(value.to_bits(), (-0.0f64).to_bits()),
            other => panic!("expected REAL, got {other:?}"),
        }
        assert_eq!(values[1], RecordValue::Blob(&[0x00]));
    }

    #[test]
    fn utf16_text_round_trips_through_both_byte_orders() {
        // "中文" plus U+1F600, which needs a surrogate pair.
        let text: [u8; 8] = [0x2d, 0x4e, 0x87, 0x65, 0x3d, 0xd8, 0x00, 0xde];
        let payload = record(&[13 + 2 * 8], &text);
        assert_eq!(decode(&payload, TextEncoding::Utf16Le).expect("le text"), [RecordValue::Text("中文😀".into())]);

        let big_endian: Vec<u8> = [[0x4e, 0x2d], [0x65, 0x87], [0xd8, 0x3d], [0xde, 0x00]].concat();
        let payload = record(&[13 + 2 * 8], &big_endian);
        assert_eq!(decode(&payload, TextEncoding::Utf16Be).expect("be text"), [RecordValue::Text("中文😀".into())]);
    }

    #[test]
    fn malformed_text_is_a_structured_error() {
        let invalid_utf8 = record(&[15], &[0xff]);
        assert_eq!(error(&invalid_utf8, TextEncoding::Utf8).code(), "yydb.unsupported");

        let odd_utf16 = record(&[15], &[0x41]);
        let odd = error(&odd_utf16, TextEncoding::Utf16Le);
        assert_eq!(odd.code(), "yydb.corrupt");
        assert!(odd.to_string().contains("odd UTF-16"), "{odd}");

        let lone_surrogate = record(&[17], &[0x00, 0xd8]);
        assert_eq!(error(&lone_surrogate, TextEncoding::Utf16Le).code(), "yydb.unsupported");
    }

    #[test]
    fn malformed_record_layout_is_a_structured_error() {
        let reserved = record(&[10], &[]);
        let reserved_error = error(&reserved, TextEncoding::Utf8);
        assert_eq!(reserved_error.code(), "yydb.corrupt");
        assert!(reserved_error.to_string().contains("reserved serial type"), "{reserved_error}");

        let zero_header = vec![0x00];
        assert!(error(&zero_header, TextEncoding::Utf8).to_string().contains("header size invalid"));

        let oversized_header = vec![0x05, 0x00];
        assert!(error(&oversized_header, TextEncoding::Utf8).to_string().contains("header size invalid"));

        let truncated_body = record(&[6], &[0x00, 0x01]);
        assert!(error(&truncated_body, TextEncoding::Utf8).to_string().contains("truncated"));

        let trailing_body = vec![0x02, 0x00, 0x00];
        assert!(error(&trailing_body, TextEncoding::Utf8).to_string().contains("trailing"));

        let reserved_error = error(&record(&[11], &[]), TextEncoding::Utf8);
        assert_eq!(reserved_error.code(), "yydb.corrupt");
    }

    #[test]
    fn limits_are_enforced_before_materializing() {
        let payload = record(&[0, 0], &[]);
        let payload_limit = decode_record(&payload, TextEncoding::Utf8, RecordLimits { max_payload_bytes: 1, max_columns: 8 })
            .expect_err("payload limit");
        assert_eq!(payload_limit.code(), "yydb.unsupported");

        let column_limit = decode_record(&payload, TextEncoding::Utf8, RecordLimits { max_payload_bytes: 64, max_columns: 1 })
            .expect_err("column limit");
        assert_eq!(column_limit.code(), "yydb.unsupported");
    }
}
