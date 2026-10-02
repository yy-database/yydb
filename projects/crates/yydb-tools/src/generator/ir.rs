//! Language-agnostic IR between VOS lowering and TypeScript emission.

/// Scalar types supported by the MVP emitter (expand with `vos-parser`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarType {
    Uuid,
    Utf8,
    Bool,
    I64,
    F64,
}

impl ScalarType {
    #[allow(dead_code)]
    pub fn as_vos(self) -> &'static str {
        match self {
            Self::Uuid => "uuid",
            Self::Utf8 => "utf8",
            Self::Bool => "bool",
            Self::I64 => "i64",
            Self::F64 => "f64",
        }
    }

    /// TypeScript type for row fields.
    #[allow(dead_code)]
    pub fn as_ts(self) -> &'static str {
        match self {
            Self::Uuid | Self::Utf8 => "string",
            Self::Bool => "boolean",
            Self::I64 | Self::F64 => "number",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: ScalarType,
    /// True when this field is declared via `@@id`.
    pub is_id: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub fields: Vec<Field>,
}

/// One schema document ready for TypeScript emission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsSchemaIr {
    /// Original VOS source (database truth embedded into generated module).
    pub vos_source: String,
    pub schema_version: u32,
    pub tables: Vec<Table>,
}

#[allow(dead_code)]
impl TsSchemaIr {
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }
}
