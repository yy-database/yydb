//! Engine state codec over opaque storage snapshots.
//!
//! [`yydb-storage`] reads and writes [`Snapshot`] values without understanding
//! VOS. This module owns the engine-side mapping between that snapshot and
//! [`State`], plus the `.yydx` blob manifest deltas published with a commit.

use std::collections::{BTreeMap, BTreeSet};

use yydb_storage::{BlobManifestDelta, BLOB_REF_OP_DELETE, BLOB_REF_OP_PUT};
use yydb_storage::Snapshot;
use yydb_types::{Error, Result};

use crate::connection::State;
use crate::objects::references::OBJECT_REF_PREFIX;
use crate::objects::ObjectStore;

const META_SCHEMA: &[u8] = b"__yydb/meta/schema";
const META_IDENTITY: &[u8] = b"__yydb/meta/identity";
const META_CONTRACT: &[u8] = b"__yydb/meta/contract";
const META_PREFIX: &str = "__yydb/meta/";

/// Decode engine [`State`] from a durable snapshot.
pub(crate) fn state_from_snapshot(snapshot: &Snapshot) -> Result<State> {
    let schema = decode_schema(snapshot.metadata.get(META_SCHEMA).cloned())?;
    let identity_history = decode_identity(snapshot.metadata.get(META_IDENTITY).cloned())?;
    let resolved_contract = decode_contract(snapshot.metadata.get(META_CONTRACT).cloned())?;
    let records = snapshot
        .records
        .iter()
        .filter(|(key, _)| !key.starts_with(META_PREFIX))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Ok(State {
        schema,
        identity_history,
        resolved_contract,
        records,
    })
}

/// Encode engine [`State`] into a durable snapshot.
pub(crate) fn snapshot_from_state(state: &State) -> Result<Snapshot> {
    let mut metadata = BTreeMap::new();
    metadata.insert(META_SCHEMA.to_vec(), encode_schema(&state.schema)?);
    metadata.insert(
        META_IDENTITY.to_vec(),
        encode_identity(&state.identity_history)?,
    );
    metadata.insert(
        META_CONTRACT.to_vec(),
        encode_contract(&state.resolved_contract)?,
    );
    let mut records = BTreeMap::new();
    for (key, value) in &state.records {
        if key.starts_with(META_PREFIX) {
            metadata.insert(key.as_bytes().to_vec(), value.clone());
        } else {
            records.insert(key.clone(), value.clone());
        }
    }
    Ok(Snapshot { metadata, records })
}

/// Collect `.yydx` blob manifest deltas between two record maps.
pub(crate) fn blob_deltas(
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
    objects: &ObjectStore,
) -> Result<Vec<BlobManifestDelta>> {
    let mut deltas = Vec::new();
    let keys = before.keys().chain(after.keys()).collect::<BTreeSet<_>>();
    for key in keys {
        let before_ref = object_hash_from_record_value(before.get(key));
        let after_ref = object_hash_from_record_value(after.get(key));
        if before_ref == after_ref {
            continue;
        }
        if before_ref.is_some() && after_ref.is_none() {
            deltas.push(BlobManifestDelta {
                manifest_key: key.clone(),
                op: BLOB_REF_OP_DELETE,
                chunks: Vec::new(),
            });
        }
        if let Some(hash) = after_ref {
            deltas.push(BlobManifestDelta {
                manifest_key: key.clone(),
                op: BLOB_REF_OP_PUT,
                chunks: vec![objects.chunk_ref(&hash)?],
            });
        }
    }
    Ok(deltas)
}

fn object_hash_from_record_value(value: Option<&Vec<u8>>) -> Option<[u8; 32]> {
    let value = value?;
    if !value.starts_with(OBJECT_REF_PREFIX) {
        return None;
    }
    decode_hash_hex(std::str::from_utf8(&value[OBJECT_REF_PREFIX.len()..]).unwrap_or(""))
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

fn encode_schema(schema: &Option<crate::SchemaVersion>) -> Result<Vec<u8>> {
    match schema {
        None => Ok(vec![0]),
        Some(schema) => {
            let doc = schema.document.as_bytes();
            let mut bytes = Vec::with_capacity(1 + 4 + 4 + doc.len());
            bytes.push(1);
            bytes.extend(schema.version.to_le_bytes());
            bytes.extend((doc.len() as u32).to_le_bytes());
            bytes.extend(doc);
            Ok(bytes)
        }
    }
}

fn decode_schema(bytes: Option<Vec<u8>>) -> Result<Option<crate::SchemaVersion>> {
    let bytes = bytes.unwrap_or(vec![0]);
    match bytes.first() {
        None | Some(0) => Ok(None),
        Some(1) => {
            if bytes.len() < 9 {
                return Err(Error::Corrupt("schema meta truncated"));
            }
            let version = u32::from_le_bytes(bytes[1..5].try_into().unwrap());
            let doc_len = u32::from_le_bytes(bytes[5..9].try_into().unwrap()) as usize;
            let end = 9 + doc_len;
            if bytes.len() != end {
                return Err(Error::Corrupt("schema meta length mismatch"));
            }
            Ok(Some(crate::SchemaVersion {
                version,
                document: String::from_utf8(bytes[9..end].to_vec())
                    .map_err(|_| Error::Corrupt("schema document is not UTF-8"))?,
            }))
        }
        _ => Err(Error::Corrupt("unknown schema meta marker")),
    }
}

fn encode_identity(history: &Option<vos::types::IdentityHistory>) -> Result<Vec<u8>> {
    match history {
        None => Ok(vec![0]),
        Some(history) => {
            let encoded = history
                .to_json()
                .map_err(|_| Error::Corrupt("catalog serialization failed"))?;
            let mut bytes = Vec::with_capacity(1 + 4 + encoded.len());
            bytes.push(1);
            bytes.extend((encoded.len() as u32).to_le_bytes());
            bytes.extend(encoded.into_bytes());
            Ok(bytes)
        }
    }
}

fn decode_identity(bytes: Option<Vec<u8>>) -> Result<Option<vos::types::IdentityHistory>> {
    let bytes = bytes.unwrap_or(vec![0]);
    match bytes.first() {
        None | Some(0) => Ok(None),
        Some(1) => {
            if bytes.len() < 5 {
                return Err(Error::Corrupt("catalog meta truncated"));
            }
            let len = u32::from_le_bytes(bytes[1..5].try_into().unwrap()) as usize;
            let end = 5 + len;
            if bytes.len() != end {
                return Err(Error::Corrupt("catalog meta length mismatch"));
            }
            Ok(Some(
                vos::types::IdentityHistory::from_json(
                    std::str::from_utf8(&bytes[5..end])
                        .map_err(|_| Error::Corrupt("catalog meta is not UTF-8"))?,
                )
                .map_err(|_| Error::Corrupt("catalog meta is invalid"))?,
            ))
        }
        _ => Err(Error::Corrupt("unknown catalog meta marker")),
    }
}

fn encode_contract(contract: &Option<vos::ResolvedContract>) -> Result<Vec<u8>> {
    match contract {
        None => Ok(vec![0]),
        Some(contract) => {
            let encoded = serde_json::to_vec(contract)
                .map_err(|_| Error::Corrupt("resolved contract serialization failed"))?;
            let mut bytes = Vec::with_capacity(1 + 4 + encoded.len());
            bytes.push(1);
            bytes.extend((encoded.len() as u32).to_le_bytes());
            bytes.extend(encoded);
            Ok(bytes)
        }
    }
}

fn decode_contract(bytes: Option<Vec<u8>>) -> Result<Option<vos::ResolvedContract>> {
    let bytes = bytes.unwrap_or(vec![0]);
    match bytes.first() {
        None | Some(0) => Ok(None),
        Some(1) => {
            if bytes.len() < 5 {
                return Err(Error::Corrupt("resolved contract meta truncated"));
            }
            let len = u32::from_le_bytes(bytes[1..5].try_into().unwrap()) as usize;
            let end = 5 + len;
            if bytes.len() != end {
                return Err(Error::Corrupt("resolved contract meta length mismatch"));
            }
            Ok(Some(
                vos::ResolvedContract::from_json(
                    &String::from_utf8(bytes[5..end].to_vec())
                        .map_err(|_| Error::Corrupt("resolved contract is not UTF-8"))?,
                )
                .map_err(|_| Error::Corrupt("resolved contract meta is invalid"))?,
            ))
        }
        _ => Err(Error::Corrupt("unknown resolved contract meta marker")),
    }
}
