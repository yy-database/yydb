//! Engine state codec over opaque storage snapshots.
//!
//! [`yydb-storage`] reads and writes [`Snapshot`] values without understanding
//! VOS. This module owns the engine-side mapping between that snapshot and
//! [`State`], plus the `.yydx` blob manifest deltas published with a commit.

use std::collections::{BTreeMap, BTreeSet};

use yydb_storage::{BLOB_REF_OP_DELETE, BLOB_REF_OP_PUT, BlobManifestDelta, RecordDelta, Snapshot};
use yydb_types::{Error, Result};

use crate::{
    connection::State,
    objects::{EMBEDDED_OBJECT_PREFIX, ObjectStore, references::OBJECT_REF_PREFIX},
};

const META_SCHEMA: &[u8] = b"__yydb/meta/schema";
const META_IDENTITY: &[u8] = b"__yydb/meta/identity";
const META_CONTRACT: &[u8] = b"__yydb/meta/contract";
const META_UNIQUE: &[u8] = b"__yydb/meta/unique";
const META_PREFIX: &str = "__yydb/meta/";

/// `__yydb/meta/` rows owned by typed [`State`] fields rather than the records
/// map. Every other metadata row (commit/checkpoint sequence, fencing counter,
/// future reserved rows) round-trips through `State.records` so counters stay
/// durable across reopen.
const TYPED_META_KEYS: [&[u8]; 4] = [META_SCHEMA, META_IDENTITY, META_CONTRACT, META_UNIQUE];

/// Decode engine [`State`] from a durable snapshot.
pub(crate) fn state_from_snapshot(snapshot: &Snapshot) -> Result<State> {
    let schema = decode_schema(snapshot.metadata.get(META_SCHEMA).cloned())?;
    let identity_history = decode_identity(snapshot.metadata.get(META_IDENTITY).cloned())?;
    let resolved_contract = decode_contract(snapshot.metadata.get(META_CONTRACT).cloned())?;
    let unique_constraints = decode_unique_constraints(snapshot.metadata.get(META_UNIQUE).cloned())?;
    let mut records: BTreeMap<String, Vec<u8>> = snapshot
        .records
        .iter()
        .filter(|(key, _)| !key.starts_with(META_PREFIX))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in &snapshot.metadata {
        if TYPED_META_KEYS.contains(&key.as_slice()) {
            continue;
        }
        let key = String::from_utf8(key.clone()).map_err(|_| Error::Corrupt("metadata key is not UTF-8"))?;
        records.insert(key, value.clone());
    }
    Ok(State { schema, identity_history, resolved_contract, records, unique_constraints })
}

/// Merge the rows of a window snapshot onto already-decoded metadata.
///
/// A narrowed read fetches metadata once to derive its window and then fetches
/// only that window. Re-decoding the schema, identity and contract from the
/// second snapshot would repeat work the caller already paid for, so the
/// window rows are inserted into the decoded metadata state instead. Both
/// inputs are consumed: a read owns them and needs neither afterwards.
pub(crate) fn state_with_window(mut state: State, snapshot: Snapshot) -> Result<State> {
    for (key, value) in snapshot.records {
        if key.starts_with(META_PREFIX) {
            continue;
        }
        state.records.insert(key, value);
    }
    Ok(state)
}

/// Encode only the reserved metadata rows of [`State`].
///
/// Publication that knows its exact record delta still rewrites every metadata
/// row as a group, because the codec owns those rows and a caller may not model
/// all of them. This skips encoding the record map, which is the expensive half
/// of [`snapshot_from_state`].
pub(crate) fn metadata_from_state(state: &State) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
    let mut metadata = typed_metadata(state)?;
    for (key, value) in &state.records {
        if !key.starts_with(META_PREFIX) || TYPED_META_KEYS.contains(&key.as_bytes()) {
            continue;
        }
        metadata.insert(key.as_bytes().to_vec(), value.clone());
    }
    Ok(metadata)
}

/// Typed `State` fields as their reserved metadata rows.
fn typed_metadata(state: &State) -> Result<BTreeMap<Vec<u8>, Vec<u8>>> {
    let mut metadata = BTreeMap::new();
    metadata.insert(META_SCHEMA.to_vec(), encode_schema(&state.schema)?);
    metadata.insert(META_IDENTITY.to_vec(), encode_identity(&state.identity_history)?);
    metadata.insert(META_CONTRACT.to_vec(), encode_contract(&state.resolved_contract)?);
    if !state.unique_constraints.is_empty() {
        metadata.insert(META_UNIQUE.to_vec(), encode_unique_constraints(&state.unique_constraints)?);
    }
    Ok(metadata)
}

/// Encode engine [`State`] into a durable snapshot.
pub(crate) fn snapshot_from_state(state: &State) -> Result<Snapshot> {
    let mut metadata = typed_metadata(state)?;
    let mut records = BTreeMap::new();
    for (key, value) in &state.records {
        if key.starts_with(META_PREFIX) {
            // Typed fields own their rows; the records copy must not override
            // the encoding produced from the typed field above.
            if TYPED_META_KEYS.contains(&key.as_bytes()) {
                continue;
            }
            metadata.insert(key.as_bytes().to_vec(), value.clone());
        }
        else {
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
            deltas.push(BlobManifestDelta { manifest_key: key.clone(), op: BLOB_REF_OP_DELETE, chunks: Vec::new() });
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

/// Records that already existed before a narrowed write ran.
///
/// A narrowed write reads only reserved metadata and the record keys its
/// statement addresses, so it never holds a full snapshot to diff. This keeps
/// exactly what the image held for those keys: ordinary records by value, plus
/// the keys of embedded object records, which are content addressed and
/// therefore identical whenever the key is identical.
pub(crate) struct PriorRecords {
    /// Non-object records inside the read window, with their stored bytes.
    pub(crate) rows: BTreeMap<String, Vec<u8>>,
    /// Embedded object record keys the image already carries.
    pub(crate) objects: BTreeSet<String>,
}

/// Split a record map into comparable user rows and already-durable object keys.
///
/// Reserved `__yydb/meta/` rows are dropped because every publication rewrites
/// them as a group, so they are never part of a record delta.
pub(crate) fn prior_records(records: &BTreeMap<String, Vec<u8>>) -> PriorRecords {
    let mut prior = PriorRecords { rows: BTreeMap::new(), objects: BTreeSet::new() };
    for (key, value) in records {
        if key.starts_with(META_PREFIX) {
            continue;
        }
        if key.starts_with(EMBEDDED_OBJECT_PREFIX) {
            prior.objects.insert(key.clone());
        }
        else {
            prior.rows.insert(key.clone(), value.clone());
        }
    }
    prior
}

/// Diff the records about to be published against [`PriorRecords`].
pub(crate) fn record_delta(next: &BTreeMap<String, Vec<u8>>, prior: &PriorRecords) -> RecordDelta {
    let mut delta = RecordDelta::default();
    for (key, value) in next {
        if key.starts_with(EMBEDDED_OBJECT_PREFIX) && prior.objects.contains(key) {
            continue;
        }
        if prior.rows.get(key) == Some(value) {
            continue;
        }
        delta.puts.push((key.clone(), value.clone()));
    }
    for key in prior.rows.keys() {
        if !next.contains_key(key) {
            delta.deletes.push(key.clone());
        }
    }
    delta
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
            let encoded = history.to_json().map_err(|_| Error::Corrupt("catalog serialization failed"))?;
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
                    std::str::from_utf8(&bytes[5..end]).map_err(|_| Error::Corrupt("catalog meta is not UTF-8"))?,
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
            let encoded = serde_json::to_vec(contract).map_err(|_| Error::Corrupt("resolved contract serialization failed"))?;
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
                    &String::from_utf8(bytes[5..end].to_vec()).map_err(|_| Error::Corrupt("resolved contract is not UTF-8"))?,
                )
                .map_err(|_| Error::Corrupt("resolved contract meta is invalid"))?,
            ))
        }
        _ => Err(Error::Corrupt("unknown resolved contract meta marker")),
    }
}

fn encode_unique_constraints(constraints: &[yydb_types::TableUniqueConstraint]) -> Result<Vec<u8>> {
    let encoded: Vec<serde_json::Value> = constraints
        .iter()
        .map(|declared| {
            serde_json::json!({
                "table": declared.table(),
                "name": declared.constraint().name(),
                "columns": declared.constraint().columns(),
                "nullsDistinct": declared.constraint().nulls_distinct(),
            })
        })
        .collect();
    let encoded = serde_json::to_vec(&encoded).map_err(|_| Error::Corrupt("unique constraint serialization failed"))?;
    let mut bytes = Vec::with_capacity(1 + 4 + encoded.len());
    bytes.push(1);
    bytes.extend((encoded.len() as u32).to_le_bytes());
    bytes.extend(encoded);
    Ok(bytes)
}

fn decode_unique_constraints(bytes: Option<Vec<u8>>) -> Result<Vec<yydb_types::TableUniqueConstraint>> {
    let Some(bytes) = bytes
    else {
        return Ok(Vec::new());
    };
    match bytes.first() {
        None | Some(0) => Ok(Vec::new()),
        Some(1) => {
            if bytes.len() < 5 {
                return Err(Error::Corrupt("unique constraint meta truncated"));
            }
            let len = u32::from_le_bytes(bytes[1..5].try_into().unwrap()) as usize;
            let end = 5 + len;
            if bytes.len() != end {
                return Err(Error::Corrupt("unique constraint meta length mismatch"));
            }
            let entries: Vec<serde_json::Value> =
                serde_json::from_slice(&bytes[5..end]).map_err(|_| Error::Corrupt("unique constraint meta is invalid JSON"))?;
            entries
                .into_iter()
                .map(|entry| {
                    let table = entry
                        .get("table")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| Error::Corrupt("unique constraint meta is missing `table`"))?;
                    let name = entry
                        .get("name")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| Error::Corrupt("unique constraint meta is missing `name`"))?;
                    let columns = entry
                        .get("columns")
                        .and_then(serde_json::Value::as_array)
                        .ok_or_else(|| Error::Corrupt("unique constraint meta is missing `columns`"))?
                        .iter()
                        .map(|column| {
                            column
                                .as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| Error::Corrupt("unique constraint column is not a string"))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    let nulls_distinct = entry
                        .get("nullsDistinct")
                        .and_then(serde_json::Value::as_bool)
                        .ok_or_else(|| Error::Corrupt("unique constraint meta is missing `nullsDistinct`"))?;
                    let constraint = yydb_types::UniqueConstraint::new(name, columns, nulls_distinct)
                        .map_err(|_| Error::Corrupt("unique constraint meta is invalid"))?;
                    yydb_types::TableUniqueConstraint::new(table, constraint)
                        .map_err(|_| Error::Corrupt("unique constraint table name is invalid"))
                })
                .collect()
        }
        _ => Err(Error::Corrupt("unknown unique constraint meta marker")),
    }
}
