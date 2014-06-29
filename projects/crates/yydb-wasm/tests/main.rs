//! Single integration-test binary for OPFS acceptance + session regression.
//!
//! Filter OPFS fixtures: `cargo test -p yydb-wasm --test main yydb_opfs_`

#[path = "session/memory.rs"]
mod memory_session;
#[path = "session/persistent.rs"]
mod persistent_session;
#[path = "session/schema_validate.rs"]
mod schema_validate;

#[path = "acceptance/opfs/capability_missing.rs"]
mod yydb_opfs_capability_missing;
#[path = "acceptance/opfs/crash_after_blob_publish.rs"]
mod yydb_opfs_crash_after_blob_publish;
#[path = "acceptance/opfs/crash_after_commit.rs"]
mod yydb_opfs_crash_after_commit;
#[path = "acceptance/opfs/doctor_missing_blob.rs"]
mod yydb_opfs_doctor_missing_blob;
#[path = "acceptance/opfs/persistent_open.rs"]
mod yydb_opfs_persistent_open;
#[path = "acceptance/opfs/quota_exhausted.rs"]
mod yydb_opfs_quota_exhausted;
#[path = "acceptance/opfs/single_writer_busy.rs"]
mod yydb_opfs_single_writer_busy;
