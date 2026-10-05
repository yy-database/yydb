use crate::doctor;
use crate::refs;
use crate::{DoctorReport, ObjectRef, ReclaimReport, Result};

use super::Connection;

impl Connection {
    /// List CAS objects under `namespace_prefix` that have no committed metadata reference.
    pub fn scan_orphans(&self, namespace_prefix: &str) -> Result<Vec<ObjectRef>> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.read_state()?;
        let referenced = refs::referenced_hashes(&state.records, namespace_prefix);
        Ok(self
            .objects
            .list_objects()?
            .into_iter()
            .filter(|object| !referenced.contains(&object.hash))
            .collect())
    }

    /// Run read-only consistency probes against this database.
    pub fn doctor(&self) -> Result<DoctorReport> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.read_state()?;
        let mut report =
            doctor::diagnose(&state.records, &self.objects, self.path(), self.wal_path())?;
        if let Some(path) = self.path() {
            report.issues.extend(yydb_format::diagnose_file(path)?);
        }
        Ok(report)
    }

    /// Delete orphan objects that are still unreferenced after a fresh scan.
    pub fn reclaim_orphans(&self, objects: &[ObjectRef]) -> Result<ReclaimReport> {
        let _guard = self
            .operation_lock
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let state = self.read_state()?;
        let referenced = refs::referenced_hashes(&state.records, "");
        let mut report = ReclaimReport::default();
        for object in objects {
            if referenced.contains(&object.hash) {
                continue;
            }
            if self.objects.is_pinned(object) {
                continue;
            }
            if !self.objects.is_stored(object) {
                continue;
            }
            self.objects.remove_object(object)?;
            report.reclaimed_objects += 1;
        }
        Ok(report)
    }
}
