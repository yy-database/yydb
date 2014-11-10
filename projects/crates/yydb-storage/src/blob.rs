//! Durable `YYBB` content-addressed blob I/O under `<root>/<hash-prefix>/`.

use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use yydb_formats::{BlobChunkRef, blob_chunk_hash, encode_blob_chunk, parse_blob_header, read_blob_payload};
use yydb_types::{Error, Result};

/// On-disk content-addressed blob root.
#[derive(Debug, Clone)]
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// Open (or create) a durable blob root.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Absolute blob root directory.
    pub fn root(&self) -> &Path {
        self.root.as_path()
    }

    /// Path for a chunk digest: `<root>/<hash-prefix>/<chunk_hash>.blob`.
    pub fn path_for_hash(&self, hash_hex: &str) -> PathBuf {
        let prefix: String = hash_hex.chars().take(2).collect();
        self.root.join(prefix).join(format!("{hash_hex}.blob"))
    }

    /// Write one encoded chunk and verify an existing segment on hash collision.
    pub fn put(&self, hash_hex: &str, bytes: &[u8]) -> Result<()> {
        let path = self.path_for_hash(hash_hex);
        let encoded = encode_blob_chunk(bytes, 0);
        if path.exists() {
            let existing = fs::read(&path)?;
            if existing != encoded {
                return Err(object_corrupt(format!("blob chunk hash collision at {hash_hex}")));
            }
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&path)?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        Ok(())
    }

    /// Read and validate a chunk payload, checking the caller's expected size.
    pub fn read(&self, hash_hex: &str, expected_size: u64) -> Result<Vec<u8>> {
        let path = self.path_for_hash(hash_hex);
        if !path.exists() {
            return Err(Error::ObjectNotFound { hash_hex: hash_hex.to_owned() });
        }
        let payload = decode_payload(&fs::read(&path)?)?;
        if payload.len() as u64 != expected_size {
            return Err(object_corrupt(format!(
                "size mismatch for {hash_hex}: expected {expected_size}, got {}",
                payload.len()
            )));
        }
        Ok(payload)
    }

    /// Read the `YYBB` header fields recorded for a chunk.
    pub fn chunk_ref(&self, hash_hex: &str) -> Result<BlobChunkRef> {
        let path = self.path_for_hash(hash_hex);
        if !path.exists() {
            return Err(Error::ObjectNotFound { hash_hex: hash_hex.to_owned() });
        }
        let header = parse_blob_header(&fs::read(&path)?).map_err(|error| match error {
            Error::Corrupt(message) => object_corrupt(message),
            other => other,
        })?;
        Ok(BlobChunkRef { chunk_hash: header.chunk_hash, chunk_len: header.chunk_len, logical_offset: header.logical_offset })
    }

    /// Enumerate every `.blob` segment as `(hash_hex, payload size)`.
    pub fn list(&self) -> Result<Vec<(String, u64)>> {
        let mut objects = Vec::new();
        if !self.root.exists() {
            return Ok(objects);
        }
        for prefix_entry in fs::read_dir(&self.root)? {
            let prefix_path = prefix_entry?.path();
            if !prefix_path.is_dir() {
                continue;
            }
            for file_entry in fs::read_dir(prefix_path)? {
                let path = file_entry?.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("blob") {
                    continue;
                }
                let hash_hex = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .ok_or_else(|| object_corrupt("object file name is not UTF-8"))?;
                if hash_hex.len() != 64 {
                    continue;
                }
                let payload = decode_payload(&fs::read(&path)?)?;
                if hash_hex != hex_of(&blob_chunk_hash(&payload)) {
                    return Err(object_corrupt(format!("blob file name does not match chunk_hash {hash_hex}")));
                }
                objects.push((hash_hex.to_owned(), payload.len() as u64));
            }
        }
        Ok(objects)
    }

    /// Remove one blob segment when present.
    pub fn remove(&self, hash_hex: &str) -> Result<()> {
        let path = self.path_for_hash(hash_hex);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn object_corrupt(message: impl Into<String>) -> Error {
    Error::ObjectCorrupt { message: message.into() }
}

fn decode_payload(file_bytes: &[u8]) -> Result<Vec<u8>> {
    read_blob_payload(file_bytes).map_err(|error| match error {
        Error::Corrupt(message) => object_corrupt(message),
        other => other,
    })
}

fn hex_of(hash: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in hash {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}
