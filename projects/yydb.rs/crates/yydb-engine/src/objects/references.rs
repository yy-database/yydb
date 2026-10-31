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
use yydb_types::{FileDescriptor, ObjectRef, Result, Value, scalar};

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
    scan_references(records, namespace_prefix, objects, false)
}

/// Same closure as [`referenced_hashes`], but an unreadable manifest contributes
/// only its own hash so read-only diagnostics can still report the gap instead
/// of failing outright. Never use this to authorize reclamation: the manifest's
/// chunks stay unknown, and deleting them would destroy content a committed
/// descriptor still points at.
pub(crate) fn referenced_hashes_tolerant(
    records: &BTreeMap<String, Vec<u8>>,
    namespace_prefix: &str,
    objects: &ObjectStore,
) -> Result<HashSet<[u8; 32]>> {
    scan_references(records, namespace_prefix, objects, true)
}

fn scan_references(
    records: &BTreeMap<String, Vec<u8>>,
    namespace_prefix: &str,
    objects: &ObjectStore,
    tolerant: bool,
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
        collect_json_references(value, objects, tolerant, &mut hashes)?;
    }
    Ok(hashes)
}

fn collect_json_references(bytes: &[u8], objects: &ObjectStore, tolerant: bool, hashes: &mut HashSet<[u8; 32]>) -> Result<()> {
    let Ok(json) = serde_json::from_slice::<JsonValue>(bytes)
    else {
        return Ok(());
    };
    collect_value_references(&json, objects, tolerant, hashes)
}

fn collect_value_references(
    json: &JsonValue,
    objects: &ObjectStore,
    tolerant: bool,
    hashes: &mut HashSet<[u8; 32]>,
) -> Result<()> {
    match json {
        JsonValue::Object(members) => {
            match scalar::tagged_value_from_json(json) {
                Some(Ok(Value::Object(object))) => {
                    hashes.insert(object.hash);
                    return Ok(());
                }
                Some(Ok(Value::File(descriptor))) => {
                    hashes.insert(descriptor.manifest.hash);
                    match objects.load_file_manifest(&descriptor) {
                        Ok(manifest) => {
                            for chunk in manifest.chunks {
                                hashes.insert(chunk.hash);
                            }
                        }
                        Err(error) if !tolerant => return Err(error.into()),
                        Err(_) => {}
                    }
                    return Ok(());
                }
                Some(Err(error)) => return Err(error.into()),
                _ => {}
            }
            for member in members.values() {
                collect_value_references(member, objects, tolerant, hashes)?;
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_value_references(item, objects, tolerant, hashes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Every committed `file` descriptor under `namespace_prefix`, deduplicated by
/// its manifest reference. Used by the restore path to verify each distinct
/// file value exactly once.
pub(crate) fn file_descriptors(records: &BTreeMap<String, Vec<u8>>, namespace_prefix: &str) -> Result<Vec<FileDescriptor>> {
    let mut seen = HashSet::new();
    let mut descriptors = Vec::new();
    for (key, value) in records {
        if !key.starts_with(namespace_prefix) {
            continue;
        }
        if value.starts_with(OBJECT_REF_PREFIX) {
            continue;
        }
        if key.starts_with("__yydb/") && !key.starts_with(ROW_PREFIX) {
            continue;
        }
        let Ok(json) = serde_json::from_slice::<JsonValue>(value)
        else {
            continue;
        };
        collect_file_descriptors(&json, &mut seen, &mut descriptors)?;
    }
    Ok(descriptors)
}

fn collect_file_descriptors(
    json: &JsonValue,
    seen: &mut HashSet<[u8; 32]>,
    descriptors: &mut Vec<FileDescriptor>,
) -> Result<()> {
    match json {
        JsonValue::Object(members) => {
            match scalar::tagged_value_from_json(json) {
                Some(Ok(Value::File(descriptor))) => {
                    if seen.insert(descriptor.manifest.hash) {
                        descriptors.push(descriptor.clone());
                    }
                    return Ok(());
                }
                Some(Err(error)) => return Err(error.into()),
                _ => {}
            }
            for member in members.values() {
                collect_file_descriptors(member, seen, descriptors)?;
            }
        }
        JsonValue::Array(items) => {
            for item in items {
                collect_file_descriptors(item, seen, descriptors)?;
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
