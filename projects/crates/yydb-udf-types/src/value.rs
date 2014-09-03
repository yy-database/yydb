//! UDF boundary values for protocol and runtime invocation.

use crate::signature::UdfType;

/// Phase-1 invocation boundary value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UdfValue {
    /// Explicit null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Signed 64-bit integer.
    I64(i64),
    /// UTF-8 text.
    Text(String),
}

impl UdfValue {
    /// Returns the Phase-1 UDF type of this value.
    pub fn ty(&self) -> UdfType {
        match self {
            Self::Null => UdfType::Null,
            Self::Bool(_) => UdfType::Bool,
            Self::I64(_) => UdfType::I64,
            Self::Text(_) => UdfType::Text,
        }
    }

    /// Returns the argument types for a slice of boundary values.
    pub fn argument_types(values: &[Self]) -> Vec<UdfType> {
        values.iter().map(Self::ty).collect()
    }
}
