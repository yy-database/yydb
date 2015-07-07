//! Index and resolve tables from a durable [`ResolvedContract`].

use vos::{
    bind_identity, resolve_contract_from_bound,
    types::{
        FieldIdentity, IDENTITY_MANIFEST_VERSION, IdentityManifest, ResolvedCanonicalType, ResolvedContract,
        ResolvedFieldContract, ResolvedTypeContract, SchemaProjection, TypeContractKind, TypeIdentity,
    },
};
use yydb_types::{Error, MAX_VECTOR_DIM, Result};

fn catalog_error(message: impl Into<String>) -> Error {
    Error::Schema { message: message.into() }
}

/// Recursively validates one resolved canonical type.
fn validate_canonical_type(ty: &ResolvedCanonicalType) -> Result<()> {
    match ty {
        ResolvedCanonicalType::Vector { dimension, .. } => {
            if *dimension > MAX_VECTOR_DIM {
                return Err(Error::VectorDimensionExceeded { dimension: *dimension, max: MAX_VECTOR_DIM });
            }
        }
        ResolvedCanonicalType::Optional(inner) | ResolvedCanonicalType::Reference(inner) => {
            validate_canonical_type(inner)?;
        }
        ResolvedCanonicalType::List(inner) => validate_canonical_type(inner)?,
        _ => {}
    }
    Ok(())
}

/// Walks every field of every declared type and enforces YYDB product-level
/// constraints on resolved canonical types. Currently validates `vector<N>`
/// dimensions against [`MAX_VECTOR_DIM`].
fn validate_contract_types(contract: &ResolvedContract) -> Result<()> {
    for ty in &contract.types {
        for field in &ty.fields {
            validate_canonical_type(&field.canonical_type)?;
        }
    }
    Ok(())
}

/// Assigns durable identities to an unreleased projection in document order.
///
/// Mirrors the first-persistence path in `yydb-engine` so tests can bind a fresh
/// schema without depending on the engine crate.
fn allocate_initial_manifest(projection: &SchemaProjection) -> IdentityManifest {
    let mut next_field_id = 1_u64;
    let mut types = Vec::with_capacity(projection.types.len());
    for (index, projected) in projection.types.iter().enumerate() {
        let mut fields = Vec::with_capacity(projected.fields.len());
        for (slot, field) in projected.fields.iter().enumerate() {
            fields.push(FieldIdentity {
                canonical_name: field.canonical_name.clone(),
                field_id: next_field_id,
                virtual_field_index: Some(slot as u32),
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
    IdentityManifest { format_version: IDENTITY_MANIFEST_VERSION.to_owned(), types }
}

/// Bind and resolve a schema document into the strict contract consumed by query.
pub fn resolved_contract_from_document(document: &str) -> Result<ResolvedContract> {
    let projection = vos::validate_schema(document).map_err(|diagnostics| Error::Schema { message: diagnostics })?;
    let manifest = allocate_initial_manifest(&projection);
    let bound = bind_identity(&projection, &manifest)
        .map_err(|diagnostics| Error::Schema { message: format!("VOS identity binding failed: {diagnostics:?}") })?;
    let contract = resolve_contract_from_bound(&bound)
        .map_err(|diagnostics| Error::Schema { message: format!("VOS name resolution failed: {diagnostics:?}") })?;
    validate_contract_types(&contract)?;
    Ok(contract)
}

/// Short storage key for a resolved table type (last canonical path segment).
pub fn table_storage_name(entry: &ResolvedTypeContract) -> Result<&str> {
    entry
        .canonical_path
        .last()
        .map(String::as_str)
        .ok_or_else(|| catalog_error("resolved table type has an empty canonical path"))
}

fn table_display_name(entry: &ResolvedTypeContract) -> String {
    entry.canonical_path.join("::")
}

/// Look up a table type by the unqualified source name used in VOS programs.
pub fn table_type_by_name<'a>(contract: &'a ResolvedContract, name: &str) -> Result<&'a ResolvedTypeContract> {
    let mut matches = contract
        .types
        .iter()
        .filter(|entry| entry.kind == TypeContractKind::Table && entry.canonical_path.last().is_some_and(|last| last == name));
    let first = matches.next().ok_or_else(|| catalog_error(format!("unknown table `{name}`")))?;
    if matches.next().is_some() {
        return Err(catalog_error(format!("ambiguous table name `{name}`: multiple resolved types share this short name")));
    }
    Ok(first)
}

/// Look up a resolved type by durable `type_id`.
pub fn type_by_id<'a>(contract: &'a ResolvedContract, type_id: u64) -> Result<&'a ResolvedTypeContract> {
    contract
        .types
        .iter()
        .find(|entry| entry.type_id == type_id)
        .ok_or_else(|| catalog_error(format!("unknown type id `{type_id}`")))
}

fn field_by_name<'a>(entry: &'a ResolvedTypeContract, name: &str) -> Option<&'a ResolvedFieldContract> {
    entry.fields.iter().find(|field| field.canonical_name == name)
}

/// Primary-key field name for a resolved table type.
pub fn primary_field_name(entry: &ResolvedTypeContract) -> Result<&str> {
    entry
        .fields
        .iter()
        .find(|field| field.attributes.iter().any(|attribute| attribute.name == "primary"))
        .map(|field| field.canonical_name.as_str())
        .ok_or_else(|| catalog_error(format!("table `{}` has no primary key field", table_display_name(entry))))
}

fn reference_target_type_id(ty: &ResolvedCanonicalType) -> Result<u64> {
    match ty {
        ResolvedCanonicalType::Reference(inner) => user_type_id(inner.as_ref()),
        _ => Err(catalog_error("field is not a reference")),
    }
}

fn user_type_id(ty: &ResolvedCanonicalType) -> Result<u64> {
    match ty {
        ResolvedCanonicalType::User { type_id, .. } => Ok(*type_id),
        ResolvedCanonicalType::Reference(inner) => user_type_id(inner.as_ref()),
        ResolvedCanonicalType::Optional(inner) => user_type_id(inner.as_ref()),
        _ => Err(catalog_error("reference field must name a table type")),
    }
}

/// Storage table name reached by dereferencing a reference-typed field.
pub fn reference_target_storage_name(contract: &ResolvedContract, ty: &ResolvedCanonicalType) -> Result<String> {
    let type_id = reference_target_type_id(ty)?;
    let target = type_by_id(contract, type_id)?;
    Ok(table_storage_name(target)?.to_string())
}

/// Field lookup on a resolved table type.
pub fn field_on_table<'a>(entry: &'a ResolvedTypeContract, name: &str) -> Result<&'a ResolvedFieldContract> {
    field_by_name(entry, name)
        .ok_or_else(|| catalog_error(format!("unknown field `{name}` on table `{}`", table_display_name(entry))))
}

/// Storage key segment for rows of the VOS table name used in scans.
pub fn table_row_key_name(contract: &ResolvedContract, name: &str) -> Result<String> {
    let entry = table_type_by_name(contract, name)?;
    Ok(table_storage_name(entry)?.to_string())
}
