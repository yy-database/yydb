//! Single integration-test binary for acceptance + regression suites.
//!
//! Filter acceptance fixtures: `cargo test -p yydb --test main yydb_`

#[path = "integration/catalog_ledger.rs"]
mod catalog_ledger;
#[path = "integration/connection_smoke.rs"]
mod connection_smoke;
#[path = "integration/execution_catalog.rs"]
mod execution_catalog;
#[path = "integration/execution_udf.rs"]
mod execution_udf;
#[path = "integration/host_micro_udf.rs"]
mod host_micro_udf;
#[path = "integration/main_snapshot.rs"]
mod main_snapshot;
#[path = "integration/query_dml.rs"]
mod query_dml;
#[path = "integration/schema_pragma.rs"]
mod schema_pragma;
#[path = "integration/vos_execute.rs"]
mod vos_execute;
#[path = "integration/vos_query.rs"]
mod vos_query;
#[path = "integration/vos_transaction.rs"]
mod vos_transaction;
#[path = "integration/wire_dispatch.rs"]
mod wire_dispatch;

#[path = "acceptance/yydb/batch_abort_clean.rs"]
mod yydb_batch_abort_clean;
#[path = "acceptance/yydb/batch_commit_visible.rs"]
mod yydb_batch_commit_visible;
#[path = "acceptance/yydb/batch_crash_mid_commit.rs"]
mod yydb_batch_crash_mid_commit;
#[path = "acceptance/yydb/cas_create_if_absent.rs"]
mod yydb_cas_create_if_absent;
#[path = "acceptance/yydb/cas_race_two_writers.rs"]
mod yydb_cas_race_two_writers;
#[path = "acceptance/yydb/lease_claim_reclaim.rs"]
mod yydb_lease_claim_reclaim;
#[path = "acceptance/yydb/lease_fencing_reject.rs"]
mod yydb_lease_fencing_reject;
#[path = "acceptance/yydb/lease_renew_extends.rs"]
mod yydb_lease_renew_extends;
#[path = "acceptance/yydb/object_batch_attach.rs"]
mod yydb_object_batch_attach;
#[path = "acceptance/yydb/object_dedup_fingerprint.rs"]
mod yydb_object_dedup_fingerprint;
#[path = "acceptance/yydb/orphan_reclaim_safe.rs"]
mod yydb_orphan_reclaim_safe;
#[path = "acceptance/yydb/quota_response_evict.rs"]
mod yydb_quota_response_evict;
#[path = "acceptance/yydb/scan_bounded_10k.rs"]
mod yydb_scan_bounded_10k;
#[path = "acceptance/yydb/scan_resume_cursor.rs"]
mod yydb_scan_resume_cursor;
#[path = "acceptance/yydb/ttl_robots_expiry.rs"]
mod yydb_ttl_robots_expiry;
#[path = "acceptance/yydb/udf_deterministic.rs"]
mod yydb_udf_deterministic;
#[path = "acceptance/yydb/udf_version_mismatch.rs"]
mod yydb_udf_version_mismatch;
#[path = "acceptance/yydb/wal_reopen_doctor.rs"]
mod yydb_wal_reopen_doctor;

#[path = "acceptance/format_v0/blob_publish_orphan.rs"]
mod yydb_format_v0_blob_publish_orphan;
#[path = "acceptance/format_v0/blob_publish_orphan_wal.rs"]
mod yydb_format_v0_blob_publish_orphan_wal;
#[path = "acceptance/format_v0/checkpoint_header.rs"]
mod yydb_format_v0_checkpoint_header;
#[path = "acceptance/format_v0/checkpoint_header_crash.rs"]
mod yydb_format_v0_checkpoint_header_crash;
#[path = "acceptance/format_v0/connection_roundtrip.rs"]
mod yydb_format_v0_connection_roundtrip;
#[path = "acceptance/format_v0/doctor_roundtrip.rs"]
mod yydb_format_v0_doctor_roundtrip;
#[path = "acceptance/format_v0/file_roundtrip.rs"]
mod yydb_format_v0_file_roundtrip;
#[path = "acceptance/format_v0/golden.rs"]
mod yydb_format_v0_golden;
#[path = "acceptance/format_v0/kv_leaf_split.rs"]
mod yydb_format_v0_kv_leaf_split;
#[path = "acceptance/format_v0/kv_roundtrip.rs"]
mod yydb_format_v0_kv_roundtrip;
#[path = "acceptance/format_v0/roundtrip.rs"]
mod yydb_format_v0_roundtrip;
#[path = "acceptance/format_v0/shm_sidecar.rs"]
mod yydb_format_v0_shm_sidecar;
#[path = "acceptance/format_v0/wal_blob_refs.rs"]
mod yydb_format_v0_wal_blob_refs;
#[path = "acceptance/format_v0/wal_blob_refs_crash.rs"]
mod yydb_format_v0_wal_blob_refs_crash;
#[path = "acceptance/format_v0/wal_crash.rs"]
mod yydb_format_v0_wal_crash;
#[path = "acceptance/format_v0/wal_truncate_reopen.rs"]
mod yydb_format_v0_wal_truncate_reopen;
#[path = "acceptance/format_v0/writer_lock.rs"]
mod yydb_format_v0_writer_lock;
#[path = "acceptance/format_v0/yydx_wal_pages_before_blob_refs_crash.rs"]
mod yydb_format_v0_yydx_wal_pages_before_blob_refs_crash;
