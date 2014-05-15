//! `YDPG` format v1 persistence bridge for [`Connection`].

use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

use yydb_format::{FilePager, KEY_KIND_USER, PAGE_MAGIC, TREE_RECORD};
use yydb_types::{Error, Result};

use crate::State;

const META_SCHEMA: &[u8] = b"__yydb/meta/schema";
const META_CATALOG: &[u8] = b"__yydb/meta/catalog";
const META_CONTRACT: &[u8] = b"__yydb/meta/contract";

/// True when `path` exists and begins with `YDPG` page magic.
pub fn file_is_ydpg(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut file = File::open(path)?;
    let mut magic = [0_u8; 5];
    let read = file.read(&mut magic)?;
    Ok(read == PAGE_MAGIC.len() && magic == *PAGE_MAGIC)
}

/// Load logical [`State`] from a format v1 pager.
pub fn read_state(pager: &mut FilePager) -> Result<State> {
    let schema = decode_schema(pager.get_kv(META_SCHEMA)?)?;
    let catalog = decode_catalog(pager.get_kv(META_CATALOG)?)?;
    let resolved_contract = decode_contract(pager.get_kv(META_CONTRACT)?)?;
    let mut records = BTreeMap::new();
    for (key, value) in pager.scan_kv()? {
        if key.tree_id != TREE_RECORD || key.key_kind != KEY_KIND_USER {
            continue;
        }
        if key.encoded.starts_with(b"__yydb/meta/") {
            continue;
        }
        let name = String::from_utf8(key.encoded)
            .map_err(|_| Error::Corrupt("record key is not UTF-8"))?;
        records.insert(name, value);
    }
    Ok(State {
        schema,
        catalog,
        resolved_contract,
        records,
    })
}

/// Persist logical [`State`] through incremental btree mutations.
pub fn write_state(pager: &mut FilePager, state: &State) -> Result<()> {
    pager.put_kv(META_SCHEMA, &encode_schema(&state.schema)?)?;
    pager.put_kv(META_CATALOG, &encode_catalog(&state.catalog)?)?;
    pager.put_kv(META_CONTRACT, &encode_contract(&state.resolved_contract)?)?;

    let mut existing = BTreeMap::new();
    for (key, value) in pager.scan_kv()? {
        if key.tree_id != TREE_RECORD || key.key_kind != KEY_KIND_USER {
            continue;
        }
        if key.encoded.starts_with(b"__yydb/meta/") {
            continue;
        }
        let name = String::from_utf8(key.encoded)
            .map_err(|_| Error::Corrupt("record key is not UTF-8"))?;
        existing.insert(name, value);
    }

    for (key, value) in &state.records {
        match existing.get(key) {
            Some(old) if old == value => {}
            _ => pager.put_kv(key.as_bytes(), value)?,
        }
    }
    for key in existing.keys() {
        if !state.records.contains_key(key) {
            pager.delete_kv(key.as_bytes())?;
        }
    }
    Ok(())
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

fn encode_catalog(catalog: &Option<vos::ast::CatalogSnapshot>) -> Result<Vec<u8>> {
    match catalog {
        None => Ok(vec![0]),
        Some(catalog) => {
            let encoded = serde_json::to_vec(catalog)
                .map_err(|_| Error::Corrupt("catalog serialization failed"))?;
            let mut bytes = Vec::with_capacity(1 + 4 + encoded.len());
            bytes.push(1);
            bytes.extend((encoded.len() as u32).to_le_bytes());
            bytes.extend(encoded);
            Ok(bytes)
        }
    }
}

fn decode_catalog(bytes: Option<Vec<u8>>) -> Result<Option<vos::ast::CatalogSnapshot>> {
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
            Ok(Some(serde_json::from_slice(&bytes[5..end]).map_err(
                |_| Error::Corrupt("catalog meta is invalid JSON"),
            )?))
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
