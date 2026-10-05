use crate::connection::Connection;
use crate::refs;
use crate::{ObjectRef, Result};

/// Pending record mutations applied atomically by [`Batch::commit`].
#[derive(Debug, Default)]
pub struct Batch {
    pub(crate) changes: Vec<(String, Option<Vec<u8>>)>,
}

impl Batch {
    /// Create an empty batch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue an upsert for commit.
    pub fn put(&mut self, key: impl Into<String>, value: impl AsRef<[u8]>) {
        self.changes
            .push((key.into(), Some(value.as_ref().to_vec())));
    }

    /// Queue a delete for commit.
    pub fn delete(&mut self, key: impl Into<String>) {
        self.changes.push((key.into(), None));
    }

    /// Drop pending mutations without writing.
    pub fn abort(self) {}

    /// Queue a metadata record that references a CAS object.
    pub fn attach_object(&mut self, object: &ObjectRef, metadata_key: impl Into<String>) {
        self.put(metadata_key, refs::encode_object_ref(object));
    }

    /// Atomically apply queued mutations through `conn`.
    pub fn commit(self, conn: &Connection) -> Result<()> {
        conn.write_batch(&self.changes)
    }
}
