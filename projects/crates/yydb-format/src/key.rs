//! B+ tree key encoding.

/// Logical tree identifiers stored in page headers.
pub const TREE_CATALOG: u8 = 0x01;
/// Application / system KV records.
pub const TREE_RECORD: u8 = 0x02;

/// Key kind values from format v0.
pub const KEY_KIND_USER: u8 = 0x02;

/// Sortable database key: `tree_id` + `key_kind` + encoded bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TreeKey {
    pub tree_id: u8,
    pub key_kind: u8,
    pub encoded: Vec<u8>,
}

impl TreeKey {
    /// Build a user KV key in the record tree.
    pub fn user_record(name: impl AsRef<[u8]>) -> Self {
        Self {
            tree_id: TREE_RECORD,
            key_kind: KEY_KIND_USER,
            encoded: name.as_ref().to_vec(),
        }
    }

    /// Serialize to on-disk cell key bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(2 + self.encoded.len());
        out.push(self.tree_id);
        out.push(self.key_kind);
        out.extend_from_slice(&self.encoded);
        out
    }

    /// Lower-bound separator for the first child in an internal page.
    pub fn min_separator(tree_id: u8) -> Self {
        Self {
            tree_id,
            key_kind: 0,
            encoded: Vec::new(),
        }
    }

    /// Parse cell key bytes.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 2 {
            return None;
        }
        Some(Self {
            tree_id: bytes[0],
            key_kind: bytes[1],
            encoded: bytes[2..].to_vec(),
        })
    }
}
