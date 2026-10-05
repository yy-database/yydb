//! WAL `BlobRefs` manifest delta encoding (v0 §8.3).

use yydb_types::{Error, Result};

/// WAL frame type for `BlobRefs`.
pub const FRAME_BLOB_REFS: u8 = 0x03;

/// Manifest delta op: attach or replace blob manifest.
pub const BLOB_REF_OP_PUT: u8 = 0x01;
/// Manifest delta op: remove manifest entry.
pub const BLOB_REF_OP_DELETE: u8 = 0x02;

/// One immutable chunk reference inside a manifest delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobChunkRef {
    /// Domain-separated chunk identity.
    pub chunk_hash: [u8; 32],
    /// Payload length in bytes.
    pub chunk_len: u32,
    /// Offset inside the logical whole object.
    pub logical_offset: u64,
}

/// One catalog manifest change recorded in a WAL `BlobRefs` frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobManifestDelta {
    /// Metadata key bound to the manifest.
    pub manifest_key: String,
    /// [`BLOB_REF_OP_PUT`] or [`BLOB_REF_OP_DELETE`].
    pub op: u8,
    /// Chunk list for put operations.
    pub chunks: Vec<BlobChunkRef>,
}

/// Encode the body of a `BlobRefs` WAL frame.
pub fn encode_blob_refs_body(txid: u64, deltas: &[BlobManifestDelta]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    body.extend_from_slice(&txid.to_le_bytes());
    body.extend_from_slice(&(deltas.len() as u32).to_le_bytes());
    for delta in deltas {
        encode_manifest_delta(&mut body, delta)?;
    }
    Ok(body)
}

/// Canonical bytes for commit digest (deltas sorted by manifest key).
pub fn canonical_blob_refs_bytes(deltas: &[BlobManifestDelta]) -> Vec<u8> {
    let mut sorted = deltas.to_vec();
    sorted.sort_by(|left, right| left.manifest_key.cmp(&right.manifest_key));
    let mut out = Vec::new();
    for delta in sorted {
        encode_manifest_delta(&mut out, &delta).expect("canonical blob refs encoding");
    }
    out
}

/// BLAKE3 digest over page images plus sorted blob-ref canonical bytes.
pub fn commit_digest(pages: &[(u32, Vec<u8>)], blob_refs: &[BlobManifestDelta]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    for (_, image) in pages {
        hasher.update(image);
    }
    hasher.update(&canonical_blob_refs_bytes(blob_refs));
    *hasher.finalize().as_bytes()
}

fn encode_manifest_delta(out: &mut Vec<u8>, delta: &BlobManifestDelta) -> Result<()> {
    let key = delta.manifest_key.as_bytes();
    if key.len() > u16::MAX as usize {
        return Err(Error::Corrupt("manifest key too long"));
    }
    out.extend_from_slice(&(key.len() as u16).to_le_bytes());
    out.push(delta.op);
    if delta.op == BLOB_REF_OP_PUT {
        out.extend_from_slice(&(delta.chunks.len() as u32).to_le_bytes());
        for chunk in &delta.chunks {
            out.extend_from_slice(&chunk.chunk_len.to_le_bytes());
            out.extend_from_slice(&chunk.chunk_hash);
            out.extend_from_slice(&chunk.logical_offset.to_le_bytes());
        }
    } else if delta.op != BLOB_REF_OP_DELETE {
        return Err(Error::Corrupt("unknown blob ref op"));
    }
    out.extend_from_slice(key);
    Ok(())
}
