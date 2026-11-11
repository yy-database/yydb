//! Acceptance tests over the `database` execution facade.
//!
//! Each test drives the actual engine (`Connection`) and asserts observable
//! state, so a declarative case can never stand in for a passing run.

pub mod batch;
pub mod cas;
pub mod doctor;
pub mod lease;
pub mod objects;
pub mod retention;
pub mod scan;
pub mod udf;

use std::path::PathBuf;

/// Isolated directory for one file-backed fixture.
///
/// Each test gets its own directory keyed by test label, so parallel test
/// threads never share a `.yydb` file or a `-wal` sidecar.
pub struct TempDb {
    root: PathBuf,
}

impl TempDb {
    pub fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!("yydb-acceptance-{}-{label}", std::process::id()));
        if root.exists() {
            std::fs::remove_dir_all(&root).expect("clear stale fixture directory");
        }
        std::fs::create_dir_all(&root).expect("create fixture directory");
        Self { root }
    }

    pub fn yydb(&self) -> PathBuf {
        self.root.join("app.yydb")
    }

    pub fn yydx(&self) -> PathBuf {
        self.root.join("app.yydx")
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        if self.root.exists() {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

/// `now_ms` without pulling a clock crate: the lease and TTL surfaces read the
/// same wall clock as `std::time::SystemTime`.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("system clock after epoch").as_millis() as u64
}
