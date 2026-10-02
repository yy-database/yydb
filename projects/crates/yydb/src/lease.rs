//! Lease records and fencing tokens stored in reserved record keys.

use std::time::{SystemTime, UNIX_EPOCH};

use yydb_types::{Error, LeaseExpectation, LeaseToken, ReleaseOutcome, Result};

pub(crate) const LEASE_PREFIX: &str = "__yydb/lease/";
pub(crate) const META_COMMIT_SEQ: &str = "__yydb/meta/commit_sequence";
pub(crate) const META_FENCING: &str = "__yydb/meta/next_fencing";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LeaseRecord {
    Queued,
    Claimed {
        worker_id: String,
        fencing_token: u64,
        lease_until_ms: u64,
    },
    RetryAt { when_ms: u64 },
    Failed { reason: String },
}

pub(crate) fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn lease_record_key(key: &str) -> String {
    format!("{LEASE_PREFIX}{key}")
}

pub(crate) fn decode_record(bytes: &[u8]) -> Result<LeaseRecord> {
    if bytes.is_empty() {
        return Err(Error::Corrupt("empty lease record"));
    }
    match bytes[0] {
        0 => Ok(LeaseRecord::Queued),
        1 => {
            if bytes.len() < 1 + 2 + 8 + 8 {
                return Err(Error::Corrupt("truncated claimed lease"));
            }
            let worker_len = u16::from_le_bytes([bytes[1], bytes[2]]) as usize;
            let start = 3;
            let end = start + worker_len;
            if bytes.len() < end + 16 {
                return Err(Error::Corrupt("truncated claimed lease worker"));
            }
            let worker_id = String::from_utf8(bytes[start..end].to_vec())
                .map_err(|_| Error::Corrupt("lease worker is not UTF-8"))?;
            let fencing_token = u64::from_le_bytes(bytes[end..end + 8].try_into().unwrap());
            let lease_until_ms =
                u64::from_le_bytes(bytes[end + 8..end + 16].try_into().unwrap());
            Ok(LeaseRecord::Claimed {
                worker_id,
                fencing_token,
                lease_until_ms,
            })
        }
        2 => {
            if bytes.len() < 9 {
                return Err(Error::Corrupt("truncated retry lease"));
            }
            let when_ms = u64::from_le_bytes(bytes[1..9].try_into().unwrap());
            Ok(LeaseRecord::RetryAt { when_ms })
        }
        3 => {
            if bytes.len() < 5 {
                return Err(Error::Corrupt("truncated failed lease"));
            }
            let reason_len = u32::from_le_bytes(bytes[1..5].try_into().unwrap()) as usize;
            let end = 5 + reason_len;
            if bytes.len() < end {
                return Err(Error::Corrupt("truncated failed lease reason"));
            }
            let reason = String::from_utf8(bytes[5..end].to_vec())
                .map_err(|_| Error::Corrupt("lease failure reason is not UTF-8"))?;
            Ok(LeaseRecord::Failed { reason })
        }
        _ => Err(Error::Corrupt("unknown lease tag")),
    }
}

pub(crate) fn encode_record(record: &LeaseRecord) -> Vec<u8> {
    match record {
        LeaseRecord::Queued => vec![0],
        LeaseRecord::Claimed {
            worker_id,
            fencing_token,
            lease_until_ms,
        } => {
            let worker = worker_id.as_bytes();
            let mut out = Vec::with_capacity(3 + worker.len() + 16);
            out.push(1);
            out.extend((worker.len() as u16).to_le_bytes());
            out.extend(worker);
            out.extend(fencing_token.to_le_bytes());
            out.extend(lease_until_ms.to_le_bytes());
            out
        }
        LeaseRecord::RetryAt { when_ms } => {
            let mut out = vec![2];
            out.extend(when_ms.to_le_bytes());
            out
        }
        LeaseRecord::Failed { reason } => {
            let reason_bytes = reason.as_bytes();
            let mut out = Vec::with_capacity(5 + reason_bytes.len());
            out.push(3);
            out.extend((reason_bytes.len() as u32).to_le_bytes());
            out.extend(reason_bytes);
            out
        }
    }
}

impl LeaseRecord {
    fn is_active_claim(&self, now_ms: u64) -> bool {
        matches!(
            self,
            LeaseRecord::Claimed {
                lease_until_ms,
                ..
            } if *lease_until_ms > now_ms
        )
    }
}

pub(crate) fn read_u64_meta(records: &std::collections::BTreeMap<String, Vec<u8>>, key: &str) -> u64 {
    records
        .get(key)
        .and_then(|bytes| {
            if bytes.len() == 8 {
                Some(u64::from_le_bytes(bytes.as_slice().try_into().unwrap()))
            } else {
                None
            }
        })
        .unwrap_or(0)
}

pub(crate) fn write_u64_meta(records: &mut std::collections::BTreeMap<String, Vec<u8>>, key: &str, value: u64) {
    records.insert(key.to_owned(), value.to_le_bytes().to_vec());
}

pub(crate) fn next_fencing_token(records: &mut std::collections::BTreeMap<String, Vec<u8>>) -> u64 {
    let next = read_u64_meta(records, META_FENCING) + 1;
    write_u64_meta(records, META_FENCING, next);
    next
}

pub(crate) fn bump_commit_sequence(records: &mut std::collections::BTreeMap<String, Vec<u8>>) -> u64 {
    let next = read_u64_meta(records, META_COMMIT_SEQ) + 1;
    write_u64_meta(records, META_COMMIT_SEQ, next);
    next
}

pub(crate) fn reconcile_expired_leases(
    records: &mut std::collections::BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let now = now_millis();
    let keys = records
        .keys()
        .filter(|key| key.starts_with(LEASE_PREFIX))
        .cloned()
        .collect::<Vec<_>>();
    for key in keys {
        let bytes = records.get(&key).cloned().unwrap_or_default();
        let record = decode_record(&bytes)?;
        if record.is_active_claim(now) {
            continue;
        }
        if matches!(record, LeaseRecord::Claimed { .. }) {
            records.insert(key, encode_record(&LeaseRecord::Queued));
        }
    }
    Ok(())
}

pub(crate) fn claim_lease(
    records: &mut std::collections::BTreeMap<String, Vec<u8>>,
    key: &str,
    worker_id: &str,
    lease_duration_ms: u64,
    expectation: LeaseExpectation,
) -> Result<LeaseToken> {
    let lease_key = lease_record_key(key);
    let now = now_millis();
    let current = records
        .get(&lease_key)
        .map(|bytes| decode_record(bytes))
        .transpose()?;

    let claimable = match (&current, expectation) {
        (None, _) => true,
        (Some(LeaseRecord::Queued), LeaseExpectation::Queued | LeaseExpectation::AbsentOrExpired) => true,
        (Some(LeaseRecord::Claimed { lease_until_ms, .. }), LeaseExpectation::AbsentOrExpired) => {
            *lease_until_ms <= now
        }
        (Some(LeaseRecord::RetryAt { .. } | LeaseRecord::Failed { .. }), LeaseExpectation::AbsentOrExpired) => true,
        _ => false,
    };
    if !claimable {
        return Err(Error::LeaseUnavailable {
            key: key.to_owned(),
        });
    }

    let fencing_token = next_fencing_token(records);
    let lease_until = now + lease_duration_ms;
    records.insert(
        lease_key,
        encode_record(&LeaseRecord::Claimed {
            worker_id: worker_id.to_owned(),
            fencing_token,
            lease_until_ms: lease_until,
        }),
    );
    Ok(LeaseToken {
        key: key.to_owned(),
        worker_id: worker_id.to_owned(),
        fencing_token,
        lease_until,
    })
}

pub(crate) fn verify_token(
    records: &std::collections::BTreeMap<String, Vec<u8>>,
    token: &LeaseToken,
) -> Result<()> {
    let lease_key = lease_record_key(&token.key);
    let now = now_millis();
    let current = records
        .get(&lease_key)
        .map(|bytes| decode_record(bytes))
        .transpose()?
        .ok_or_else(|| Error::LeaseUnavailable {
            key: token.key.clone(),
        })?;
    match current {
        LeaseRecord::Claimed {
            worker_id,
            fencing_token,
            lease_until_ms,
        } => {
            if worker_id != token.worker_id || fencing_token != token.fencing_token {
                return Err(Error::FencingMismatch {
                    key: token.key.clone(),
                });
            }
            if lease_until_ms <= now {
                return Err(Error::LeaseExpired {
                    key: token.key.clone(),
                });
            }
            Ok(())
        }
        _ => Err(Error::FencingMismatch {
            key: token.key.clone(),
        }),
    }
}

pub(crate) fn renew_lease(
    records: &mut std::collections::BTreeMap<String, Vec<u8>>,
    token: &LeaseToken,
    new_lease_until: u64,
) -> Result<()> {
    verify_token(records, token)?;
    let lease_key = lease_record_key(&token.key);
    records.insert(
        lease_key,
        encode_record(&LeaseRecord::Claimed {
            worker_id: token.worker_id.clone(),
            fencing_token: token.fencing_token,
            lease_until_ms: new_lease_until,
        }),
    );
    Ok(())
}

pub(crate) fn release_lease(
    records: &mut std::collections::BTreeMap<String, Vec<u8>>,
    token: &LeaseToken,
    outcome: ReleaseOutcome,
) -> Result<()> {
    verify_token(records, token)?;
    let lease_key = lease_record_key(&token.key);
    let next = match outcome {
        ReleaseOutcome::Requeue => LeaseRecord::Queued,
        ReleaseOutcome::RetryAt { when } => LeaseRecord::RetryAt { when_ms: when },
        ReleaseOutcome::Failed { reason } => LeaseRecord::Failed { reason },
    };
    records.insert(lease_key, encode_record(&next));
    Ok(())
}

pub(crate) fn lease_until(
    records: &std::collections::BTreeMap<String, Vec<u8>>,
    key: &str,
) -> Result<Option<u64>> {
    let lease_key = lease_record_key(key);
    let current = records
        .get(&lease_key)
        .map(|bytes| decode_record(bytes))
        .transpose()?;
    Ok(match current {
        Some(LeaseRecord::Claimed { lease_until_ms, .. }) => Some(lease_until_ms),
        _ => None,
    })
}
