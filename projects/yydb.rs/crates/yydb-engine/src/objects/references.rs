//! Metadata references to CAS objects stored in record values.
//!
//! A reference reaches the record map in two shapes: a raw `yydb:object:<hex>`
//! value attached by [`crate::Batch`], and a tagged JSON value inside a table
//! row. A row `file` value additionally pins the immutable manifest object and
//! every chunk that manifest lists, so reclaiming only the descriptor would
//! delete content the row can still read.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value as JsonValue;
use yydb_query::ROW_PREFIX;
use yydb_types::{ObjectRef, Result, Value, scalar};

use super::ObjectStore;

pub(crate) const OBJECT_REF_PREFIX: &[u8] = b"yydb:object:";

pub(crate) fn encode_object_ref(object: &ObjectRef) -> Vec<u8> {
    format!("yydb:object:{}", object.hash_hex()).into_bytes()
}

/// Hashes that committed metadata still references under `namespace_prefix`.
///
/// Table rows are stored under [`ROW_PREFIX`], so the reserved-key skip must
/// keep them: their JSON carries `object` and `file` references. A `file` value
/// contributes its manifest hash and the manifest's chunk hashes. A manifest
/// that cannot be read is an error, because reclaiming its chunks would destroy
/// content the committed descriptor still points at.
pub(crate) fn referenced_hashes(
    records: &BTreeMap<String, Vec<u8>>,
    namespace_prefix: &str,
    objects: &ObjectStore,
) -> Result<HashSet<[u8; 32]>> {
    let mut hashes = HashSet::new();
    for (key, value) in records {
        if !key.starts_with(namespace_prefix) {
            continue;
        }
        if let Some(hex) = value.strip_prefix(OBJECT_REF_PREFIX) {
            if let Some(hash) = decode_hash_hex(std::str::from_utf8(hex).unwrap_or("")) {
                hashes.insert(hash);
            }
            continue;
        }
        if key.starts_with("__yydb/") && !key.starts_with(ROW_PREFIX) {
            continue;
        }
        collect_json_references(value, objects, &mut hashes)?;
    }
    Ok(hashes)
}

fn collect_json_references(bytes: &[u8], objects: &ObjectStore, hashes: &mut HashSet<[u8; 32]>) -> Result<()> {
    let Ok(json) = serde_json::from_slice::<JsonValue>(bytes)
    else {
        return Ok(());
    };
    collect_value_references(&json, objects, hashes)
}

fn collect_value_references(json: &JsonValue, objects: &ObjectStore, hashes: &mut HashSet<[u8; 32]>) -> Result<()> {
    match json {
        JsonValue::Object(members) => {
            match scalar::tagged_value_from_json(json) {
                Some(Ok(Value::Object(object))) => {
                    hashes.insert(object.hash);
                    return Ok(());
                }
                Some(Ok(Value::File(descriptor))) => {
                    hashes.insert(descriptor.manifest.hash);
                    for chunk in objects.load_file_manifest(&descriptor)?.chunks {
                        hashes.insert(chunk.hash);
                    }
                    return Ok(());
                }
                Some(Err(error)) => return Err(error.into()),
                _ => {}
            }
            for member in members.values() {
                collect_value_references(member, objects, hashes)?;
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_value_references(item, objects, hashes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn decode_hash_hex(hash_hex: &str) -> Option<[u8; 32]> {
    if hash_hex.len() != 64 {
        return None;
    }
    let mut hash = [0_u8; 32];
    for (index, chunk) in hash_hex.as_bytes().chunks(2).enumerate() {
        let byte = u8::from_str_radix(&String::from_utf8_lossy(chunk), 16).ok()?;
        hash[index] = byte;
    }
    Some(hash)
}
