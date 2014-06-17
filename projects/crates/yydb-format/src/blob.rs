//! `YBLO` v0 immutable chunk header.

use yydb_types::{Error, Result};

pub const BLOB_MAGIC: &[u8; 5] = b"YBLO\x00";
pub const BLOB_HEADER_BYTES: usize = 82;
pub const BLOB_CHUNK_KIND_DATA: u8 = 0x01;
pub const BLOB_CHUNK_DOMAIN: &[u8] = b"yydb.blob.chunk.v0";

/// Parsed blob chunk header (payload follows in file).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobChunkHeader {
    pub chunk_kind: u8,
    pub chunk_hash: [u8; 32],
    pub logical_offset: u64,
    pub chunk_len: u32,
    pub payload_hash: [u8; 32],
}

pub fn parse_blob_header(bytes: &[u8]) -> Result<BlobChunkHeader> {
    if bytes.len() < BLOB_HEADER_BYTES {
        return Err(Error::Corrupt("blob header truncated"));
    }
    if bytes.get(0..5) != Some(BLOB_MAGIC.as_slice()) {
        return Err(Error::Corrupt("blob magic mismatch"));
    }
    Ok(BlobChunkHeader {
        chunk_kind: bytes[5],
        chunk_hash: bytes[6..38].try_into().unwrap(),
        logical_offset: u64::from_le_bytes(bytes[38..46].try_into().unwrap()),
        chunk_len: u32::from_le_bytes(bytes[46..50].try_into().unwrap()),
        payload_hash: bytes[50..82].try_into().unwrap(),
    })
}

/// Domain-separated digest used as the on-disk chunk identity and file name.
pub fn blob_chunk_hash(payload: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(BLOB_CHUNK_DOMAIN);
    hasher.update(payload);
    *hasher.finalize().as_bytes()
}

/// Plain payload digest recorded in the chunk header.
pub fn blob_payload_hash(payload: &[u8]) -> [u8; 32] {
    *blake3::hash(payload).as_bytes()
}

/// Serialize one immutable `.blob` segment (`header + payload`).
pub fn encode_blob_chunk(payload: &[u8], logical_offset: u64) -> Vec<u8> {
    let chunk_hash = blob_chunk_hash(payload);
    let payload_hash = blob_payload_hash(payload);
    let mut out = Vec::with_capacity(BLOB_HEADER_BYTES + payload.len());
    out.extend_from_slice(BLOB_MAGIC);
    out.push(BLOB_CHUNK_KIND_DATA);
    out.extend_from_slice(&chunk_hash);
    out.extend_from_slice(&logical_offset.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&payload_hash);
    out.extend_from_slice(payload);
    out
}

fn verify_blob_chunk(bytes: &[u8], header: &BlobChunkHeader) -> Result<()> {
    let payload_start = BLOB_HEADER_BYTES;
    let payload_end = payload_start + header.chunk_len as usize;
    if bytes.len() != payload_end {
        return Err(Error::Corrupt("blob file length mismatch"));
    }
    let payload = &bytes[payload_start..payload_end];
    if blob_payload_hash(payload) != header.payload_hash {
        return Err(Error::Corrupt("blob payload_hash mismatch"));
    }
    if blob_chunk_hash(payload) != header.chunk_hash {
        return Err(Error::Corrupt("blob chunk_hash mismatch"));
    }
    Ok(())
}

/// Read and validate payload bytes from a `.blob` file.
pub fn read_blob_payload(bytes: &[u8]) -> Result<Vec<u8>> {
    let header = parse_blob_header(bytes)?;
    verify_blob_chunk(bytes, &header)?;
    let payload_start = BLOB_HEADER_BYTES;
    let payload_end = payload_start + header.chunk_len as usize;
    Ok(bytes[payload_start..payload_end].to_vec())
}

/// Parsed header plus validated payload.
pub fn parse_blob_chunk(bytes: &[u8]) -> Result<(BlobChunkHeader, Vec<u8>)> {
    let header = parse_blob_header(bytes)?;
    verify_blob_chunk(bytes, &header)?;
    let payload_start = BLOB_HEADER_BYTES;
    let payload_end = payload_start + header.chunk_len as usize;
    Ok((header, bytes[payload_start..payload_end].to_vec()))
}
