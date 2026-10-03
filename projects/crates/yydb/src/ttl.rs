//! TTL metadata, namespace quota, and eviction helpers.

use std::collections::BTreeMap;

use yydb_types::{
    Error, EvictBudget, EvictReport, EvictionPolicy, NamespaceQuota, NamespaceStats, RecordVersion,
    Result,
};

use crate::lease::{now_millis, read_u64_meta, write_u64_meta};

pub(crate) const TTL_PREFIX: &str = "__yydb/ttl/";
pub(crate) const QUOTA_PREFIX: &str = "__yydb/quota/";
pub(crate) const LRU_PREFIX: &str = "__yydb/lru/";
pub(crate) const VERSION_PREFIX: &str = "__yydb/version/";

fn ttl_meta_key(key: &str) -> String {
    format!("{TTL_PREFIX}{key}")
}

fn lru_meta_key(key: &str) -> String {
    format!("{LRU_PREFIX}{key}")
}

fn version_meta_key(key: &str) -> String {
    format!("{VERSION_PREFIX}{key}")
}

fn quota_meta_key(namespace: &str) -> String {
    format!("{QUOTA_PREFIX}{namespace}")
}

pub(crate) fn is_reserved_key(key: &str) -> bool {
    key.starts_with("__yydb/")
}

fn encode_quota(quota: &NamespaceQuota) -> Vec<u8> {
    let mut out = Vec::with_capacity(17);
    out.extend(quota.max_bytes.to_le_bytes());
    out.extend(quota.max_records.to_le_bytes());
    out.push(match quota.eviction_policy {
        EvictionPolicy::Lru => 0,
        EvictionPolicy::TtlFirst => 1,
    });
    out
}

fn decode_quota(bytes: &[u8]) -> Result<NamespaceQuota> {
    if bytes.len() != 17 {
        return Err(Error::Corrupt("invalid namespace quota record"));
    }
    let max_bytes = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let max_records = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let eviction_policy = match bytes[16] {
        0 => EvictionPolicy::Lru,
        1 => EvictionPolicy::TtlFirst,
        _ => return Err(Error::Corrupt("unknown eviction policy tag")),
    };
    Ok(NamespaceQuota {
        max_bytes,
        max_records,
        eviction_policy,
    })
}

fn matching_namespace(records: &BTreeMap<String, Vec<u8>>, key: &str) -> Option<String> {
    records
        .keys()
        .filter(|meta| meta.starts_with(QUOTA_PREFIX))
        .filter_map(|meta| meta.strip_prefix(QUOTA_PREFIX))
        .filter(|namespace| key.starts_with(namespace))
        .max_by_key(|namespace| namespace.len())
        .map(str::to_owned)
}

pub(crate) fn namespace_stats(
    records: &BTreeMap<String, Vec<u8>>,
    namespace: &str,
) -> NamespaceStats {
    let mut stats = NamespaceStats::default();
    for (key, value) in records {
        if is_reserved_key(key) || !key.starts_with(namespace) {
            continue;
        }
        stats.records_used += 1;
        stats.bytes_used += value.len() as u64;
    }
    stats
}

pub(crate) fn set_namespace_quota(
    records: &mut BTreeMap<String, Vec<u8>>,
    namespace: &str,
    quota: NamespaceQuota,
) {
    records.insert(quota_meta_key(namespace), encode_quota(&quota));
}

fn read_namespace_quota(
    records: &BTreeMap<String, Vec<u8>>,
    namespace: &str,
) -> Option<NamespaceQuota> {
    records
        .get(&quota_meta_key(namespace))
        .map(|bytes| decode_quota(bytes))
        .transpose()
        .ok()
        .flatten()
}

fn touch_lru(records: &mut BTreeMap<String, Vec<u8>>, key: &str) {
    let seq = read_u64_meta(records, "__yydb/meta/lru_clock") + 1;
    write_u64_meta(records, "__yydb/meta/lru_clock", seq);
    records.insert(lru_meta_key(key), seq.to_le_bytes().to_vec());
}

fn bump_record_version(records: &mut BTreeMap<String, Vec<u8>>, key: &str) -> RecordVersion {
    let next = read_u64_meta(records, &version_meta_key(key)) + 1;
    write_u64_meta(records, &version_meta_key(key), next);
    next
}

fn remove_record(records: &mut BTreeMap<String, Vec<u8>>, key: &str) -> u64 {
    let bytes = records
        .remove(key)
        .map(|value| value.len() as u64)
        .unwrap_or(0);
    records.remove(&ttl_meta_key(key));
    records.remove(&lru_meta_key(key));
    records.remove(&version_meta_key(key));
    bytes
}

fn candidate_keys(records: &BTreeMap<String, Vec<u8>>, namespace: &str) -> Vec<String> {
    records
        .keys()
        .filter(|key| !is_reserved_key(key) && key.starts_with(namespace))
        .cloned()
        .collect()
}

fn pick_victim(
    records: &BTreeMap<String, Vec<u8>>,
    namespace: &str,
    policy: EvictionPolicy,
) -> Option<String> {
    let keys = candidate_keys(records, namespace);
    match policy {
        EvictionPolicy::Lru => keys.into_iter().min_by_key(|key| {
            records
                .get(&lru_meta_key(key))
                .and_then(|bytes| {
                    if bytes.len() == 8 {
                        Some(u64::from_le_bytes(bytes.as_slice().try_into().unwrap()))
                    } else {
                        None
                    }
                })
                .unwrap_or(0)
        }),
        EvictionPolicy::TtlFirst => keys.into_iter().min_by_key(|key| {
            records
                .get(&ttl_meta_key(key))
                .and_then(|bytes| {
                    if bytes.len() == 8 {
                        Some(u64::from_le_bytes(bytes.as_slice().try_into().unwrap()))
                    } else {
                        None
                    }
                })
                .unwrap_or(u64::MAX)
        }),
    }
}

pub(crate) fn enforce_namespace_quota(
    records: &mut BTreeMap<String, Vec<u8>>,
    namespace: &str,
) -> Result<EvictReport> {
    let Some(quota) = read_namespace_quota(records, namespace) else {
        return Ok(EvictReport::default());
    };
    let mut report = EvictReport::default();
    while namespace_stats(records, namespace).records_used > quota.max_records
        || namespace_stats(records, namespace).bytes_used > quota.max_bytes
    {
        let victim = pick_victim(records, namespace, quota.eviction_policy).ok_or_else(|| {
            Error::QuotaExceeded {
                namespace: namespace.to_owned(),
            }
        })?;
        report.evicted_bytes += remove_record(records, &victim);
        report.evicted_records += 1;
    }
    Ok(report)
}

pub(crate) fn put_with_ttl(
    records: &mut BTreeMap<String, Vec<u8>>,
    key: &str,
    value: &[u8],
    ttl_ms: u64,
) -> Result<RecordVersion> {
    let expires_at = now_millis() + ttl_ms;
    records.insert(key.to_owned(), value.to_vec());
    records.insert(ttl_meta_key(key), expires_at.to_le_bytes().to_vec());
    touch_lru(records, key);
    let version = bump_record_version(records, key);
    if let Some(namespace) = matching_namespace(records, key) {
        enforce_namespace_quota(records, &namespace)?;
    }
    Ok(version)
}

pub(crate) fn put_record(
    records: &mut BTreeMap<String, Vec<u8>>,
    key: &str,
    value: &[u8],
) -> Result<()> {
    records.insert(key.to_owned(), value.to_vec());
    touch_lru(records, key);
    if let Some(namespace) = matching_namespace(records, key) {
        enforce_namespace_quota(records, &namespace)?;
    }
    Ok(())
}

pub(crate) fn evict_expired(
    records: &mut BTreeMap<String, Vec<u8>>,
    namespace_prefix: &str,
    budget: EvictBudget,
) -> Result<EvictReport> {
    let now = now_millis();
    let mut report = EvictReport::default();
    let keys = candidate_keys(records, namespace_prefix);
    for key in keys {
        if report.evicted_records as usize >= budget.max_records {
            break;
        }
        let expires_at = records.get(&ttl_meta_key(&key)).and_then(|bytes| {
            if bytes.len() == 8 {
                Some(u64::from_le_bytes(bytes.as_slice().try_into().unwrap()))
            } else {
                None
            }
        });
        if expires_at.is_some_and(|deadline| deadline <= now) {
            report.evicted_bytes += remove_record(records, &key);
            report.evicted_records += 1;
        }
    }
    Ok(report)
}

pub(crate) fn is_expired(records: &BTreeMap<String, Vec<u8>>, key: &str) -> bool {
    let now = now_millis();
    records
        .get(&ttl_meta_key(key))
        .and_then(|bytes| {
            if bytes.len() == 8 {
                Some(u64::from_le_bytes(bytes.as_slice().try_into().unwrap()))
            } else {
                None
            }
        })
        .is_some_and(|deadline| deadline <= now)
}
