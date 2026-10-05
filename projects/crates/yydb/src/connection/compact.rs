use crate::{CompactReport, Error, Result};

use super::is_yydx_main_path;
use super::Connection;

impl Connection {
    /// Fold WAL into the main file and reclaim safe orphan blobs on `.yydx` layouts.
    ///
    /// v0 compaction is checkpoint plus orphan GC. Catalog-referenced and pinned chunks
    /// are preserved (Living `07` §8).
    pub fn compact_yydx(&self) -> Result<CompactReport> {
        let path = self
            .path()
            .ok_or(Error::Unsupported("compact_yydx requires a file-backed connection"))?;
        if !is_yydx_main_path(path) {
            return Err(Error::Unsupported("compact_yydx requires a .yydx main file path"));
        }
        self.checkpoint()?;
        let orphans = self.scan_orphans("")?;
        let reclaim = self.reclaim_orphans(&orphans)?;
        Ok(CompactReport {
            checkpointed: true,
            orphans_reclaimed: reclaim.reclaimed_objects,
        })
    }
}
