//! Metadata references to CAS objects stored in record values.

use std::collections::{BTreeMap, HashSet};

use yydb_types::ObjectRef;

pub(crate) const OBJECT_REF_PREFIX: &[u8] = b"yydb:object:";

pub(crate) fn encode_object_ref(object: &ObjectRef) -> Vec<u8> {
    format!("yydb:object:{}", object.hash_hex()).into_bytes()
}

pub(crate) fn referenced_hashes(
    records: &BTreeMap<String, Vec<u8>>,
    namespace_prefix: &str,
) -> HashSet<[u8; 32]> {
    let mut hashes = HashSet::new();
    for (key, value) in records {
        if key.starts_with("__yydb/") || !key.starts_with(namespace_prefix) {
            continue;
        }
        if let Some(hex) = value.strip_prefix(OBJECT_REF_PREFIX) {
            if let Ok(hash) = decode_hash_hex(std::str::from_utf8(hex).unwrap_or("")) {
                hashes.insert(hash);
            }
        }
    }
    hashes
}

fn decode_hash_hex(hash_hex: &str) -> Result<[u8; 32], ()> {
    if hash_hex.len() != 64 {
        return Err(());
    }
    let mut hash = [0_u8; 32];
    for (index, chunk) in hash_hex.as_bytes().chunks(2).enumerate() {
        let byte = u8::from_str_radix(&String::from_utf8_lossy(chunk), 16).map_err(|_| ())?;
        hash[index] = byte;
    }
    Ok(hash)
}
