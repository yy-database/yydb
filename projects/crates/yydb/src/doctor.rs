//! Read-only consistency probes for `Connection::doctor`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use yydb_types::{
    DoctorIssue, DoctorNamespaceStats, DoctorReport, DoctorSeverity, EvictionPolicy,
    NamespaceQuota, ObjectRef, Result,
};

use crate::lease::{read_u64_meta, write_u64_meta, META_COMMIT_SEQ};
use crate::objects::ObjectStore;
use crate::refs;
use crate::ttl::{self, QUOTA_PREFIX, TTL_PREFIX};

pub(crate) const META_CHECKPOINT_SEQ: &str = "__yydb/meta/checkpoint_sequence";
pub(crate) const PENDING_BATCH_KEY: &str = "__yydb/batch/pending";

pub(crate) fn diagnose(
    records: &BTreeMap<String, Vec<u8>>,
    objects: &ObjectStore,
    file_path: Option<&std::path::Path>,
    wal_path: Option<std::path::PathBuf>,
) -> Result<DoctorReport> {
    let mut issues = Vec::new();

    if records.contains_key(PENDING_BATCH_KEY) {
        issues.push(DoctorIssue {
            severity: DoctorSeverity::Error,
            code: "yydb.doctor.half_batch".into(),
            message: "pending batch marker is still present".into(),
            key_hint: Some(PENDING_BATCH_KEY.into()),
        });
    }

    let last_commit_sequence = read_u64_meta(records, META_COMMIT_SEQ);
    let last_checkpoint_sequence = read_u64_meta(records, META_CHECKPOINT_SEQ);
    if last_checkpoint_sequence > last_commit_sequence {
        issues.push(DoctorIssue {
            severity: DoctorSeverity::Error,
            code: "yydb.doctor.sequence_regress".into(),
            message: "checkpoint sequence is ahead of commit sequence".into(),
            key_hint: None,
        });
    }

    let checkpoint_lag_records = last_commit_sequence.saturating_sub(last_checkpoint_sequence);
    if checkpoint_lag_records > 0 {
        issues.push(DoctorIssue {
            severity: DoctorSeverity::Warn,
            code: "yydb.doctor.checkpoint_stale".into(),
            message: format!("checkpoint lags commit by {checkpoint_lag_records} records"),
            key_hint: None,
        });
    }

    let referenced = refs::referenced_hashes(records, "");
    let orphans: Vec<ObjectRef> = objects
        .list_objects()?
        .into_iter()
        .filter(|object| !referenced.contains(&object.hash))
        .collect();
    let orphan_object_count = orphans.len() as u64;

    let namespaces = collect_namespace_stats(records);

    let wal_bytes_uncheckpointed = wal_path
        .and_then(|path| fs::metadata(path).ok())
        .map(|meta| meta.len())
        .unwrap_or(0);

    Ok(DoctorReport {
        file_path: file_path.map(|path| path.display().to_string()),
        last_commit_sequence,
        last_checkpoint_sequence,
        checkpoint_lag_records,
        wal_bytes_uncheckpointed,
        orphan_object_count,
        namespaces,
        issues,
    })
}

pub(crate) fn record_checkpoint(records: &mut BTreeMap<String, Vec<u8>>) {
    let sequence = read_u64_meta(records, META_COMMIT_SEQ);
    write_u64_meta(records, META_CHECKPOINT_SEQ, sequence);
}

fn collect_namespace_stats(records: &BTreeMap<String, Vec<u8>>) -> Vec<DoctorNamespaceStats> {
    let mut prefixes = BTreeSet::new();
    for key in records.keys() {
        if let Some(prefix) = key.strip_prefix(QUOTA_PREFIX) {
            prefixes.insert(prefix.to_owned());
        }
    }
    for key in records.keys() {
        if ttl::is_reserved_key(key) {
            continue;
        }
        if let Some((head, _)) = key.split_once('/') {
            prefixes.insert(format!("{head}/"));
        }
    }

    let now = crate::lease::now_millis();
    prefixes
        .into_iter()
        .map(|prefix| {
            let stats = ttl::namespace_stats(records, &prefix);
            let quota = records
                .get(&format!("{QUOTA_PREFIX}{prefix}"))
                .and_then(|bytes| decode_quota(bytes).ok());
            let ttl_expired_pending = records
                .keys()
                .filter(|key| !ttl::is_reserved_key(key) && key.starts_with(&prefix))
                .filter(|key| ttl_deadline(records, key).is_some_and(|deadline| deadline <= now))
                .count() as u64;
            DoctorNamespaceStats {
                prefix,
                record_count: stats.records_used,
                bytes_used: stats.bytes_used,
                object_bytes: 0,
                quota_max_bytes: quota.map(|quota| quota.max_bytes),
                ttl_expired_pending,
            }
        })
        .collect()
}

fn ttl_deadline(records: &BTreeMap<String, Vec<u8>>, key: &str) -> Option<u64> {
    records
        .get(&format!("{TTL_PREFIX}{key}"))
        .and_then(|bytes| {
            if bytes.len() == 8 {
                Some(u64::from_le_bytes(bytes.as_slice().try_into().unwrap()))
            } else {
                None
            }
        })
}

fn decode_quota(bytes: &[u8]) -> Result<NamespaceQuota> {
    if bytes.len() != 17 {
        return Err(yydb_types::Error::Corrupt("invalid namespace quota record"));
    }
    let max_bytes = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
    let max_records = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let eviction_policy = match bytes[16] {
        0 => EvictionPolicy::Lru,
        1 => EvictionPolicy::TtlFirst,
        _ => return Err(yydb_types::Error::Corrupt("unknown eviction policy tag")),
    };
    Ok(NamespaceQuota {
        max_bytes,
        max_records,
        eviction_policy,
    })
}
