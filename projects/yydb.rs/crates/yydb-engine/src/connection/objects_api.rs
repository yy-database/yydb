use std::sync::Arc;

use crate::{ChunkManifest, FileDescriptor, ObjectKind, ObjectRef, Result, Tier, Vector};

use super::Connection;

impl Connection {
    /// Publish process-local CAS objects into the single-file main image.
    ///
    /// A `.yydb` main file is the only durable home for object payloads, so an
    /// object published through the CAS surface has to reach the image before
    /// the call returns: hosts that open a fresh handle per operation (the
    /// Python binding does) would otherwise lose it with the handle. `.yydx`
    /// keeps its `<stem>-objects/` tree and needs no flush. An open transaction
    /// is skipped because its commit publishes the objects.
    fn persist_embedded_objects(&self) -> Result<()> {
        if !self.is_single_file_yydb() {
            return Ok(());
        }
        if self.txn.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).is_some() {
            return Ok(());
        }
        let _guard = self.operation_lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let state = self.read_state()?;
        self.write_state(&state)?;
        Ok(())
    }

    /// Store one CAS object at `<hash-prefix>/<chunk_hash>.blob`.
    pub fn put_chunk(&self, kind: ObjectKind, bytes: &[u8]) -> Result<ObjectRef> {
        let object = self.objects.put_chunk(kind, bytes)?;
        self.persist_embedded_objects()?;
        Ok(object)
    }

    /// Store a vector payload in CAS (`ObjectKind::VectorPayload`).
    pub fn put_vector(&self, vector: &Vector) -> Result<ObjectRef> {
        let object = self.objects.put_vector(vector)?;
        self.persist_embedded_objects()?;
        Ok(object)
    }

    /// Load a vector payload from CAS.
    pub fn get_vector(&self, object: &ObjectRef) -> Result<Vector> {
        self.objects.get_vector(object)
    }

    /// Read CAS bytes (fault-in to hot tier when needed).
    pub fn get_object(&self, object: &ObjectRef) -> Result<Arc<[u8]>> {
        self.objects.get_object(object)
    }

    /// Publish a logical `file` value: chunks first, then the manifest object.
    ///
    /// A `.yydx` layout keeps the object tree in `<stem>-objects/`; a
    /// single-file `.yydb` embeds the same objects as chunked records in the
    /// main image. Both are supported, but embedding large payloads inflates
    /// the single file and slows down every snapshot and backup.
    pub fn put_file(
        &self,
        reader: impl std::io::Read,
        chunk_size: usize,
        name: Option<String>,
        media_type: Option<String>,
    ) -> Result<FileDescriptor> {
        let descriptor = self.objects.put_file(reader, chunk_size, name, media_type)?;
        self.persist_embedded_objects()?;
        Ok(descriptor)
    }

    /// Chunk a logical file into CAS objects.
    pub fn put_file_chunked(&self, reader: impl std::io::Read, chunk_size: usize) -> Result<ChunkManifest> {
        let manifest = self.objects.put_file_chunked(reader, chunk_size)?;
        self.persist_embedded_objects()?;
        Ok(manifest)
    }

    /// Range-read a chunked file without assembling the whole payload.
    pub fn read_file_range(&self, manifest: &ChunkManifest, offset: u64, len: usize) -> Result<Vec<u8>> {
        self.objects.read_file_range(manifest, offset, len)
    }

    /// Read one VOS [`FileReadWindow`](vos::types::FileReadWindow) of a `file` value.
    ///
    /// The window follows VOS semantics: an offset at or past end of file yields
    /// an empty window, a request starting before end of file is truncated to the
    /// remaining octets, and a read past the end is a short read instead of an
    /// error. Every touched chunk is verified against the manifest, and a missing
    /// or tampered object is a structured error rather than empty content.
    pub fn read_file_window(&self, descriptor: &FileDescriptor, window: vos::types::FileReadWindow) -> Result<Vec<u8>> {
        self.objects.read_file_window(descriptor, window.offset, window.length)
    }

    /// Read the complete octets of a `file` value and verify its content digest.
    pub fn read_file(&self, descriptor: &FileDescriptor) -> Result<Vec<u8>> {
        self.objects.read_file(descriptor)
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
