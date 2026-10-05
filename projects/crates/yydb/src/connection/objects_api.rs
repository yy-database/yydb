use std::sync::Arc;

use crate::{ChunkManifest, Error, ObjectKind, ObjectRef, Result, Tier, Vector};

use super::Connection;

impl Connection {
    /// Store one CAS object at `<hash-prefix>/<chunk_hash>.blob`.
    pub fn put_chunk(&self, kind: ObjectKind, bytes: &[u8]) -> Result<ObjectRef> {
        self.guard_yydb_cas_payload(bytes.len())?;
        self.objects.put_chunk(kind, bytes)
    }

    /// Store a vector payload in CAS (`ObjectKind::VectorPayload`).
    pub fn put_vector(&self, vector: &Vector) -> Result<ObjectRef> {
        self.guard_yydb_cas_payload(vector.to_le_bytes().len())?;
        self.objects.put_vector(vector)
    }

    /// Load a vector payload from CAS.
    pub fn get_vector(&self, object: &ObjectRef) -> Result<Vector> {
        self.objects.get_vector(object)
    }

    /// Read CAS bytes (fault-in to hot tier when needed).
    pub fn get_object(&self, object: &ObjectRef) -> Result<Arc<[u8]>> {
        self.objects.get_object(object)
    }

    /// Chunk a logical file into CAS objects.
    pub fn put_file_chunked(
        &self,
        reader: impl std::io::Read,
        chunk_size: usize,
    ) -> Result<ChunkManifest> {
        if self.is_single_file_yydb() {
            return Err(Error::Unsupported(
                "chunked file objects require a .yydx layout; .yydb single-file mode rejects them",
            ));
        }
        self.objects.put_file_chunked(reader, chunk_size)
    }

    /// Range-read a chunked file without assembling the whole payload.
    pub fn read_file_range(
        &self,
        manifest: &ChunkManifest,
        offset: u64,
        len: usize,
    ) -> Result<Vec<u8>> {
        self.objects.read_file_range(manifest, offset, len)
    }

    /// Pin an object in the hot tier.
    pub fn pin_object(&self, object: &ObjectRef) -> Result<Tier> {
        self.objects.pin_object(object)
    }

    /// Evict an object from the hot tier (disk CAS remains).
    pub fn evict_object(&self, object: &ObjectRef) -> Result<Tier> {
        self.objects.evict_object(object)
    }

    /// Current hot/cold hint for an object.
    pub fn object_tier(&self, object: &ObjectRef) -> Tier {
        self.objects.tier_of(object)
    }
}
