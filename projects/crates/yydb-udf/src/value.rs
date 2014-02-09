//! UDF boundary values and adapters to `yydb-execution::Value`.

use yydb_execution::{Type, Value};

use crate::contract::UdfType;
use crate::error::{Result, UdfError};

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

    /// Converts a runtime value into the UDF boundary subset.
    pub fn from_execution(value: &Value) -> Result<Self> {
        match value {
            Value::Null => Ok(Self::Null),
            Value::Bool(value) => Ok(Self::Bool(*value)),
            Value::I64(value) => Ok(Self::I64(*value)),
            Value::Text(value) => Ok(Self::Text(value.clone())),
            _ => Err(UdfError::UnsupportedType),
        }
    }

    /// Converts a boundary value into the execution runtime model.
    pub fn into_execution(self) -> Result<Value> {
        match self {
            Self::Null => Ok(Value::Null),
            Self::Bool(value) => Ok(Value::Bool(value)),
            Self::I64(value) => Ok(Value::I64(value)),
            Self::Text(value) => Ok(Value::Text(value)),
        }
    }

    /// Returns the argument types for a slice of boundary values.
    pub fn argument_types(values: &[Self]) -> Vec<UdfType> {
        values.iter().map(Self::ty).collect()
    }
}

impl UdfType {
    /// Maps a UDF type to the execution runtime type.
    pub fn to_execution(self) -> Type {
        match self {
            Self::Null => Type::Null,
            Self::Bool => Type::Bool,
            Self::I64 => Type::I64,
            Self::Text => Type::Text,
        }
    }

    /// Maps an execution runtime type into the Phase-1 UDF subset.
    pub fn from_execution(ty: Type) -> Result<Self> {
        match ty {
            Type::Null => Ok(Self::Null),
            Type::Bool => Ok(Self::Bool),
            Type::I64 => Ok(Self::I64),
            Type::Text => Ok(Self::Text),
            _ => Err(UdfError::UnsupportedType),
        }
    }
}
