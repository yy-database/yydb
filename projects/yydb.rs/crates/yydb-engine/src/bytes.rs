//! Host side of the `bytes` contract: placement above the inline boundary and
//! row decoding of content-addressed references.
//!
//! VOS names the two placements but never picks a threshold, so the boundary
//! belongs here: [`YydbBytesPolicy`] declares it from `INLINE_BYTES_MAX` and
//! moves the octets into the existing [`ObjectStore`] instead of inventing a
//! second object layer. [`YydbVectorPolicy`] applies the same design to
//! `vector<N>` columns — small vectors stay inline in the row's tagged JSON,
//! larger ones become a `VectorPayload` reference in the same CAS.

use std::sync::Arc;

use vos::types::{BytesPlacement, BytesPlacementPolicy};
use yydb_query::{BytesPlacementHook, ObjectResolver, VectorPlacementHook};
use yydb_types::{Error, INLINE_BYTES_MAX, ObjectKind, ObjectRef, Result, Value, Vector};

use crate::ObjectStore;

/// YYDB's own inline-versus-object boundary for `bytes` values.
///
/// Octets at or below [`INLINE_BYTES_MAX`] stay in the row; larger payloads
/// become a content-addressed object the row references. The threshold is a
/// physical property of this store: row storage hex-encodes octets inside a
/// single leaf cell, so a larger inline payload would not fit a stored row and
/// is not part of the VOS contract.
pub struct YydbBytesPolicy {
    store: Arc<ObjectStore>,
    persistent_objects: bool,
}

impl YydbBytesPolicy {
    /// Builds the policy over `store`.
    ///
    /// `persistent_objects` is `false` for a single-file `.yydb` connection,
    /// whose object store is process-local: storing a reference there would
    /// lose the payload on reopen, so such a write is refused instead.
    pub fn new(store: Arc<ObjectStore>, persistent_objects: bool) -> Self {
        Self { store, persistent_objects }
    }
}

impl BytesPlacementPolicy for YydbBytesPolicy {
    fn placement_for(&self, octets: &[u8]) -> BytesPlacement {
        if octets.len() <= INLINE_BYTES_MAX { BytesPlacement::Inline } else { BytesPlacement::ObjectReference }
    }
}

impl BytesPlacementHook for YydbBytesPolicy {
    fn place(&self, octets: Vec<u8>) -> Result<Value> {
        match self.placement_for(&octets) {
            BytesPlacement::Inline => Ok(Value::Bytes(octets)),
            BytesPlacement::ObjectReference if !self.persistent_objects => Err(Error::Unsupported(
                "CAS payloads larger than INLINE_BYTES_MAX require a .yydx layout in single-file .yydb mode",
            )),
            BytesPlacement::ObjectReference => self.store.put_chunk(ObjectKind::Blob, &octets).map(Value::Object),
        }
    }
}

/// YYDB's own inline-versus-object boundary for `vector<N>` values.
///
/// Small vectors stay in the row's tagged JSON form (`{"$yydb":"vector",...}`);
/// larger ones become a `VectorPayload` object reference. The boundary is
/// derived from [`INLINE_BYTES_MAX`]: a vector's inline JSON occupies
/// roughly `53 + 8*N` bytes, so the safe threshold is around **120**
/// dimensions.
const INLINE_VECTOR_DIM_MAX: u32 = 120;

/// Host-side vector placement policy over an [`ObjectStore`].
///
/// Mirrors [`YydbBytesPolicy`]: dimensions at or below
/// [`INLINE_VECTOR_DIM_MAX`] stay inline; above that the payload is written
/// into CAS as a `VectorPayload` object and the row keeps a reference. A
/// single-file `.yydb` connection refuses vectors that need an object store
/// (same rule as `bytes`).
pub struct YydbVectorPolicy {
    store: Arc<ObjectStore>,
    persistent_objects: bool,
}

impl YydbVectorPolicy {
    /// Builds the policy over `store`.
    pub fn new(store: Arc<ObjectStore>, persistent_objects: bool) -> Self {
        Self { store, persistent_objects }
    }

    /// Whether `vector` fits the row's inline JSON budget.
    pub fn fits_inline(vector: &Vector) -> bool {
        vector.dim <= INLINE_VECTOR_DIM_MAX
    }
}

impl VectorPlacementHook for YydbVectorPolicy {
    fn place(&self, vector: Vector) -> Result<Value> {
        if Self::fits_inline(&vector) {
            return Ok(Value::Vector(vector));
        }
        if !self.persistent_objects {
            return Err(Error::Unsupported(
                "large vectors that require CAS storage need a .yydx layout; .yydb single-file mode rejects them",
            ));
        }
        self.store.put_vector(&vector).map(Value::Object)
    }
}

/// Resolves a row's `bytes` object reference through the connection object store.
pub struct ObjectStoreResolver {
    store: Arc<ObjectStore>,
}

impl ObjectStoreResolver {
    /// Builds a resolver over `store`.
    pub fn new(store: Arc<ObjectStore>) -> Self {
        Self { store }
    }
}

impl ObjectResolver for ObjectStoreResolver {
    fn resolve(&self, object: &ObjectRef) -> Result<Vec<u8>> {
        Ok(self.store.get_object(object)?.to_vec())
    }
}
