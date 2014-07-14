//! VOS schema documents shared with [`vos`](https://github.com/voml/vos-language)
//! (a pinned Oak-backed revision).
//!
//! YYDB uses **VOS for DDL and query**. It does **not** invent a private schema
//! dialect. Database-truth documents stored by
//! [`crate::Connection::ensure_schema`] are VOS source text; semantic checking
//! goes through the `vos` facade so YYDB / YYDS / tooling stay aligned with
//! Oak's parser and VOS semantic contract.

use std::collections::BTreeMap;

use vos::types::{
    resolved_schema_fingerprint, FieldIdentity, IdentityBoundProjection, IdentityHistory,
    IdentityManifest, ResolvedContract, SchemaProjection, TypeIdentity,
    IDENTITY_HISTORY_FORMAT_VERSION, IDENTITY_MANIFEST_VERSION, RESOLVED_CONTRACT_FORMAT_VERSION,
};
use vos::{bind_identity, evolve_identity, resolve_identity_types};
use yydb_execution::{FieldHandle, LayoutField, RecordLayout, RecordLayoutError, Type};
use yydb_types::{Error, Result};

/// Canonical remote used by this workspace for shared VOS semantics.
pub const VOS_GIT_DEV: &str = "https://github.com/voml/vos-language.git#branch=dev";

/// Parses a VOS document through Oak and returns its schema projection.
///
/// Rejects NUL bytes and enforces the VOS table-primary contract before the
/// projection becomes database truth.
pub(crate) fn project_schema(document: &str) -> Result<SchemaProjection> {
    if document.contains('\0') {
        return Err(Error::Schema {
            message: "VOS schema document must not contain NUL bytes".into(),
        });
    }
    vos::validate_schema(document).map_err(|message| Error::Schema { message })
}

/// Assigns durable identities to an unreleased projection in document order.
///
/// Type IDs start at `1`; field IDs are global and start at `1`; virtual slots
/// start at `0` per type. Used only for the first persistence of a schema.
pub(crate) fn allocate_initial_manifest(projection: &SchemaProjection) -> IdentityManifest {
    let mut next_field_id = 1_u64;
    let mut types = Vec::with_capacity(projection.types.len());
    for (index, projected) in projection.types.iter().enumerate() {
        let mut fields = Vec::with_capacity(projected.fields.len());
        for (slot, field) in projected.fields.iter().enumerate() {
            fields.push(FieldIdentity {
                canonical_name: field.canonical_name.clone(),
                field_id: next_field_id,
                virtual_field_index: slot as u32,
            });
            next_field_id += 1;
        }
        types.push(TypeIdentity {
            canonical_path: projected.canonical_path.clone(),
            type_id: index as u64 + 1,
            kind: projected.kind,
            fields,
        });
    }
    IdentityManifest {
        format_version: IDENTITY_MANIFEST_VERSION.to_owned(),
        types,
    }
}

/// Rebuilds an identity manifest from a durable identity snapshot.
pub(crate) fn manifest_from_snapshot(snapshot: &IdentityBoundProjection) -> IdentityManifest {
    IdentityManifest {
        format_version: snapshot.manifest_version.clone(),
        types: snapshot
            .types
            .iter()
            .map(|item| TypeIdentity {
                canonical_path: item.canonical_path.clone(),
                type_id: item.type_id,
                kind: item.kind,
                fields: item
                    .fields
                    .iter()
                    .map(|field| FieldIdentity {
                        canonical_name: field.canonical_name.clone(),
                        field_id: field.field_id,
                        virtual_field_index: field.virtual_field_index,
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// Builds an empty identity history used as the base for a first publication.
pub(crate) fn empty_history() -> IdentityHistory {
    IdentityHistory {
        format_version: IDENTITY_HISTORY_FORMAT_VERSION.to_owned(),
        manifest_version: IDENTITY_MANIFEST_VERSION.to_owned(),
        revision: 0,
        layout_epoch: 0,
        snapshot: IdentityBoundProjection {
            manifest_version: IDENTITY_MANIFEST_VERSION.to_owned(),
            types: Vec::new(),
        },
        retired_types: Vec::new(),
        retired_fields: Vec::new(),
    }
}

/// Binds an explicit identity manifest to a projection.
pub(crate) fn bind(
    projection: &SchemaProjection,
    manifest: &IdentityManifest,
) -> Result<IdentityBoundProjection> {
    bind_identity(projection, manifest).map_err(|diagnostics| Error::Schema {
        message: format!("VOS identity binding failed: {diagnostics:?}"),
    })
}

/// Applies an explicitly bound snapshot to durable identity history.
pub(crate) fn evolve(
    previous: &IdentityHistory,
    current: IdentityBoundProjection,
) -> Result<IdentityHistory> {
    evolve_identity(previous, current).map_err(|diagnostics| Error::Schema {
        message: format!("VOS identity evolution failed: {diagnostics:?}"),
    })
}

/// Resolves an already-bound projection into the strict YYDB-consumed contract.
pub(crate) fn resolve_contract_from_bound(
    bound: &IdentityBoundProjection,
) -> Result<ResolvedContract> {
    let resolved = resolve_identity_types(bound).map_err(|diagnostics| Error::Schema {
        message: format!("VOS name resolution failed: {diagnostics:?}"),
    })?;
    Ok(ResolvedContract {
        format_version: RESOLVED_CONTRACT_FORMAT_VERSION.to_owned(),
        identity_manifest_version: resolved.manifest_version.clone(),
        schema_fingerprint: resolved_schema_fingerprint(&resolved),
        types: resolved.types,
    })
}

/// A type-qualified field path used by [`RenameMap`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct FieldPath {
    /// Name of the owning type declaration.
    pub type_name: String,
    /// Current field name.
    pub field_name: String,
}

/// Explicit renames applied when a schema document evolves.
///
/// Type renames map an old type name to its new name; field renames map an old
/// [`FieldPath`] to the new field name. Unlisted types and fields keep their
/// durable identity while their spelling is unchanged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenameMap {
    /// Old type name to new type name.
    pub types: BTreeMap<String, String>,
    /// Old field path to new field name.
    pub fields: BTreeMap<FieldPath, String>,
}

impl RenameMap {
    fn type_target(&self, name: &str) -> Option<&str> {
        self.types.get(name).map(String::as_str)
    }

    fn field_target(&self, type_name: &str, field_name: &str) -> Option<&str> {
        self.fields
            .get(&FieldPath {
                type_name: type_name.to_owned(),
                field_name: field_name.to_owned(),
            })
            .map(String::as_str)
    }
}

fn validate_renames(snapshot: &IdentityBoundProjection, renames: &RenameMap) -> Result<()> {
    for old_type in renames.types.keys() {
        if !snapshot
            .types
            .iter()
            .any(|item| item.canonical_path.last() == Some(old_type))
        {
            return Err(Error::Schema {
                message: format!("rename map refers to unknown type `{old_type}`"),
            });
        }
    }
    for path in renames.fields.keys() {
        let known = snapshot.types.iter().any(|item| {
            item.canonical_path.last() == Some(&path.type_name)
                && item
                    .fields
                    .iter()
                    .any(|field| field.canonical_name == path.field_name)
        });
        if !known {
            return Err(Error::Schema {
                message: format!(
                    "rename map refers to unknown field `{}.{}`",
                    path.type_name, path.field_name
                ),
            });
        }
    }
    Ok(())
}

fn next_durable_id(ids: impl Iterator<Item = u64>) -> u64 {
    ids.max().map_or(1, |max| max + 1)
}

fn next_slot(slots: impl Iterator<Item = u32>) -> u32 {
    slots.max().map_or(0, |max| max + 1)
}

/// Builds an identity manifest for a migration, preserving durable identities.
///
/// Types and fields keep their `type_id` / `field_id` when their name matches a
/// previous identity, either directly or through `renames`. Everything else
/// receives a fresh identity above every active or retired one, and new fields
/// take the next unused virtual slot for their type.
pub(crate) fn migration_manifest(
    history: &IdentityHistory,
    projection: &SchemaProjection,
    renames: &RenameMap,
) -> Result<IdentityManifest> {
    let snapshot = &history.snapshot;
    validate_renames(snapshot, renames)?;

    let mut next_type_id = next_durable_id(
        snapshot
            .types
            .iter()
            .map(|item| item.type_id)
            .chain(history.retired_types.iter().map(|item| item.type_id)),
    );
    let mut next_field_id = next_durable_id(
        snapshot
            .types
            .iter()
            .flat_map(|item| item.fields.iter())
            .map(|field| field.field_id)
            .chain(history.retired_fields.iter().map(|field| field.field_id)),
    );

    let mut types = Vec::with_capacity(projection.types.len());
    for projected in &projection.types {
        let projected_name =
            projected
                .canonical_path
                .last()
                .cloned()
                .ok_or_else(|| Error::Schema {
                    message: "VOS type has an empty canonical path".into(),
                })?;
        let previous = snapshot.types.iter().find(|item| {
            item.canonical_path
                .last()
                .map(|name| renames.type_target(name).unwrap_or(name.as_str()))
                .is_some_and(|target| target == projected_name)
        });
        let (type_id, previous_type) = match previous {
            Some(previous) => (previous.type_id, Some(previous)),
            None => {
                let assigned = next_type_id;
                next_type_id += 1;
                (assigned, None)
            }
        };
        let mut slot = next_slot(
            previous_type
                .iter()
                .flat_map(|previous| previous.fields.iter())
                .map(|field| field.virtual_field_index)
                .chain(
                    history
                        .retired_fields
                        .iter()
                        .filter(|field| field.type_id == type_id)
                        .map(|field| field.virtual_field_index),
                ),
        );
        let mut fields = Vec::with_capacity(projected.fields.len());
        let mut used_field_ids = std::collections::BTreeSet::new();
        let mut used_slots = std::collections::BTreeSet::new();
        for field in &projected.fields {
            let previous_field = previous_type.and_then(|previous| {
                previous.fields.iter().find(|candidate| {
                    let previous_name = candidate.canonical_name.as_str();
                    let type_name = previous
                        .canonical_path
                        .last()
                        .map(String::as_str)
                        .unwrap_or_default();
                    let target = renames
                        .field_target(type_name, previous_name)
                        .unwrap_or(previous_name);
                    target == field.canonical_name
                        && !used_field_ids.contains(&candidate.field_id)
                        && !used_slots.contains(&candidate.virtual_field_index)
                })
            });
            let identity = match previous_field {
                Some(previous_field) => {
                    used_field_ids.insert(previous_field.field_id);
                    used_slots.insert(previous_field.virtual_field_index);
                    FieldIdentity {
                        canonical_name: field.canonical_name.clone(),
                        field_id: previous_field.field_id,
                        virtual_field_index: previous_field.virtual_field_index,
                    }
                }
                None => {
                    let identity = FieldIdentity {
                        canonical_name: field.canonical_name.clone(),
                        field_id: next_field_id,
                        virtual_field_index: slot,
                    };
                    next_field_id += 1;
                    slot += 1;
                    used_field_ids.insert(identity.field_id);
                    used_slots.insert(identity.virtual_field_index);
                    identity
                }
            };
            fields.push(identity);
        }
        types.push(TypeIdentity {
            canonical_path: projected.canonical_path.clone(),
            type_id,
            kind: projected.kind,
            fields,
        });
    }
    Ok(IdentityManifest {
        format_version: IDENTITY_MANIFEST_VERSION.to_owned(),
        types,
    })
}

/// Initial execution-facing catalog built from one validated VOS document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionCatalog {
    /// Cataloged table and class types in VOS document order.
    pub types: Vec<ExecutionType>,
}

/// One VOS table or class projected into the execution model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionType {
    /// Stable VOS TypeId used as the execution schema identity.
    pub schema_id: u64,
    /// Current VOS type name.
    pub name: String,
    /// Catalog kind.
    pub kind: ExecutionTypeKind,
    /// Fields in stable virtual-slot order.
    pub fields: Vec<ExecutionField>,
}

/// Type declaration kind used by the YYDB execution catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionTypeKind {
    /// Persistent table declaration.
    Table,
    /// Inline value/class declaration.
    Class,
}

/// One VOS field projected into a typed execution handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionField {
    /// Stable VOS FieldId.
    pub field_id: u64,
    /// Durable virtual slot used by the physical row layout.
    pub virtual_field: u32,
    /// Current VOS field name.
    pub name: String,
    /// Schema-bound execution handle.
    pub handle: FieldHandle,
}

impl ExecutionType {
    /// Converts catalog fields into the published execution layout contract.
    pub fn layout(&self) -> std::result::Result<RecordLayout, RecordLayoutError> {
        RecordLayout::new(
            self.schema_id,
            self.fields
                .iter()
                .map(|field| LayoutField {
                    field_id: field.field_id,
                    index: field.virtual_field,
                    ty: field.handle.ty(),
                })
                .collect(),
        )
    }
}

/// Parse, validate, and lower a fresh VOS document into execution handles.
///
/// This assigns a new identity set from document order. It is a fresh-document
/// projection, not a persisted identity ledger or migration. Rebuilding after
/// source reordering must not be used to replace deployed identities.
pub fn execution_catalog(document: &str) -> Result<ExecutionCatalog> {
    let projection = project_schema(document)?;
    let manifest = allocate_initial_manifest(&projection);
    let bound = bind(&projection, &manifest)?;
    let contract = resolve_contract_from_bound(&bound)?;
    execution_catalog_from_resolved_contract(&contract)
}

/// Lower a strict Oak-backed VOS contract into the YYDB execution catalog.
///
/// This is the downstream path for new integrations. It does not parse source,
/// inspect a VOS syntax AST, or infer identities from declaration order.
pub fn execution_catalog_from_resolved_contract(
    contract: &vos::ResolvedContract,
) -> Result<ExecutionCatalog> {
    contract.validate().map_err(|error| Error::Schema {
        message: format!(
            "invalid resolved VOS contract {}: {}",
            error.code, error.message
        ),
    })?;
    let mut types = Vec::with_capacity(contract.types.len());
    for item in &contract.types {
        let kind = match item.kind {
            vos::types::TypeContractKind::Table => ExecutionTypeKind::Table,
            vos::types::TypeContractKind::Class => ExecutionTypeKind::Class,
        };
        let name = item
            .canonical_path
            .last()
            .cloned()
            .ok_or_else(|| Error::Schema {
                message: "resolved VOS type has an empty canonical path".into(),
            })?;
        let mut fields = Vec::with_capacity(item.fields.len());
        for field in &item.fields {
            let ty = resolved_execution_type(&field.canonical_type)?;
            let handle =
                FieldHandle::new(item.type_id, field.field_id, field.virtual_field_index, ty)
                    .map_err(|_| Error::Schema {
                        message: "resolved VOS contract emitted an invalid field identity".into(),
                    })?;
            fields.push(ExecutionField {
                field_id: field.field_id,
                virtual_field: field.virtual_field_index,
                name: field.canonical_name.clone(),
                handle,
            });
        }
        types.push(ExecutionType {
            schema_id: item.type_id,
            name,
            kind,
            fields,
        });
    }
    Ok(ExecutionCatalog { types })
}

fn resolved_execution_type(ty: &vos::types::ResolvedCanonicalType) -> Result<Type> {
    use vos::types::ResolvedCanonicalType;
    match ty {
        ResolvedCanonicalType::Builtin(path) if path == &["i64"] => Ok(Type::I64),
        ResolvedCanonicalType::Builtin(path) if path == &["bool"] => Ok(Type::Bool),
        ResolvedCanonicalType::Builtin(path) if path == &["utf8"] => Ok(Type::Text),
        ResolvedCanonicalType::Builtin(path) if path == &["bytes"] => Ok(Type::Bytes),
        ResolvedCanonicalType::Builtin(path) if path == &["file"] => Ok(Type::File),
        ResolvedCanonicalType::Builtin(_) => Err(Error::Unsupported(
            "resolved VOS scalar type is not in the execution slice",
        )),
        ResolvedCanonicalType::Optional(_)
        | ResolvedCanonicalType::Reference(_)
        | ResolvedCanonicalType::List(_)
        | ResolvedCanonicalType::User { .. }
        | ResolvedCanonicalType::Generic { .. } => Err(Error::Unsupported(
            "resolved VOS composite type is not in the execution slice",
        )),
    }
}

const VERSION_PRAGMA: &str = "// @yydb-schema-version:";

/// Optional schema version declared in leading `//` comments.
///
/// Format: `// @yydb-schema-version: <u32>` before the first non-comment line.
pub fn document_version(document: &str) -> Option<u32> {
    for line in document.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(VERSION_PRAGMA) {
            return rest.trim().parse().ok();
        }
        if !line.is_empty() && !line.starts_with("//") {
            break;
        }
    }
    None
}

/// Version used when a schema is first persisted (`1` unless [`document_version`] is set).
pub fn initial_version(document: &str) -> u32 {
    document_version(document).unwrap_or(1)
}

/// Validate that an optional document pragma matches the expected migration target.
pub fn validate_migration_version(document: &str, expected: u32) -> Result<()> {
    if let Some(declared) = document_version(document) {
        if declared != expected {
            return Err(Error::Schema {
                message: format!(
                    "document declares schema version {declared}, migration expects {expected}"
                ),
            });
        }
    }
    Ok(())
}

/// Validate a schema document before it becomes database truth.
///
/// Uses Oak parsing and VOS semantic projection before persistence.
pub fn validate_document(document: &str) -> Result<()> {
    if document.contains('\0') {
        return Err(Error::Schema {
            message: "VOS schema document must not contain NUL bytes".into(),
        });
    }
    vos::validate_schema(document)
        .map(|_| ())
        .map_err(|message| Error::Schema { message })
}
