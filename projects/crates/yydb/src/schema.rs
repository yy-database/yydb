//! VOS schema documents shared with [`vos`](https://github.com/voml/vos-language)
//! (a pinned Oak-backed revision).
//!
//! YYDB uses **VOS for DDL and query**. It does **not** invent a private schema
//! dialect. Database-truth documents stored by
//! [`crate::Connection::ensure_schema`] are VOS source text; semantic checking
//! goes through the `vos` facade so YYDB / YYDS / tooling stay aligned as
//! `vos-parser` grows.

use yy_execution::{FieldHandle, LayoutField, RecordLayout, RecordLayoutError, Type};
use yydb_types::{Error, Result};

/// Canonical remote used by this workspace for shared VOS semantics.
pub const VOS_GIT_DEV: &str = "https://github.com/voml/vos-language.git#branch=dev";

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
    pub kind: vos::ast::TypeKind,
    /// Fields in stable virtual-slot order.
    pub fields: Vec<ExecutionField>,
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

/// Parse, validate, and lower initial VOS catalog identities into execution handles.
///
/// This is a fresh-document projection, not a persisted identity ledger or migration.
/// Rebuilding after source reordering must not be used to replace deployed identities.
pub fn execution_catalog(document: &str) -> Result<ExecutionCatalog> {
    validate_document(document)?;
    let document = vos::parser::parse_document(document).map_err(|diagnostics| Error::Schema {
        message: diagnostics.to_string(),
    })?;
    let catalog =
        vos::catalog_from_document(&document).map_err(|message| Error::Schema { message })?;
    let mut types = Vec::with_capacity(catalog.types.len());

    for entry in catalog.types {
        let schema_id = entry.type_id.0;
        let mut fields = Vec::with_capacity(entry.fields.len());
        for field in entry.fields {
            let ty = execution_type(&field.ty)?;
            let handle = FieldHandle::new(schema_id, field.field_id.0, field.virtual_field, ty)
                .map_err(|_| Error::Schema {
                    message: "VOS catalog emitted an invalid field identity".into(),
                })?;
            fields.push(ExecutionField {
                field_id: field.field_id.0,
                virtual_field: field.virtual_field,
                name: field.current_name,
                handle,
            });
        }
        types.push(ExecutionType {
            schema_id,
            name: entry.name,
            kind: entry.kind,
            fields,
        });
    }
    Ok(ExecutionCatalog { types })
}

fn execution_type(ty: &vos::ast::TypeExpr) -> Result<Type> {
    use vos::ast::{BuiltinType, TypeExpr};
    match ty {
        TypeExpr::Builtin(BuiltinType::I64) => Ok(Type::I64),
        TypeExpr::Builtin(BuiltinType::Bool) => Ok(Type::Bool),
        TypeExpr::Builtin(BuiltinType::Utf8) => Ok(Type::Text),
        TypeExpr::Builtin(BuiltinType::Bytes) => Ok(Type::Bytes),
        TypeExpr::File => Ok(Type::File),
        TypeExpr::Builtin(_) => Err(Error::Unsupported(
            "VOS scalar type is not in the execution slice",
        )),
        TypeExpr::Vector { .. } => Err(Error::Unsupported(
            "VOS vector metric is not resolved in the execution slice",
        )),
        TypeExpr::Named(_) | TypeExpr::Reference(_) | TypeExpr::Optional(_) | TypeExpr::List(_) => {
            Err(Error::Unsupported(
                "VOS composite field type is not in the execution slice",
            ))
        }
        _ => Err(Error::Unsupported(
            "VOS field type is not in the execution slice",
        )),
    }
}

/// Validate a schema document before it becomes database truth.
///
/// Uses the Oak-backed VOS parser and semantic checker before persistence.
pub fn validate_document(document: &str) -> Result<()> {
    if document.contains('\0') {
        return Err(Error::Schema {
            message: "VOS schema document must not contain NUL bytes".into(),
        });
    }
    vos::parser::parse_document(document)
        .map(|_| ())
        .map_err(|diagnostics| Error::Schema {
            message: diagnostics.to_string(),
        })
}
