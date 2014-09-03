//! Adapters between UDF boundary values and `yydb-execution`.

use yydb_execution::{Type, Value};

use crate::{Result, UdfError, UdfType, UdfValue};

/// Converts a runtime value into the UDF boundary subset.
pub fn from_execution(value: &Value) -> Result<UdfValue> {
    match value {
        Value::Null => Ok(UdfValue::Null),
        Value::Bool(value) => Ok(UdfValue::Bool(*value)),
        Value::I64(value) => Ok(UdfValue::I64(*value)),
        Value::Text(value) => Ok(UdfValue::Text(value.clone())),
        _ => Err(UdfError::UnsupportedType),
    }
}

/// Converts a boundary value into the execution runtime model.
pub fn into_execution(value: UdfValue) -> Result<Value> {
    match value {
        UdfValue::Null => Ok(Value::Null),
        UdfValue::Bool(value) => Ok(Value::Bool(value)),
        UdfValue::I64(value) => Ok(Value::I64(value)),
        UdfValue::Text(value) => Ok(Value::Text(value)),
    }
}

/// Maps an execution runtime type into the Phase-1 UDF subset.
pub fn type_from_execution(ty: Type) -> Result<UdfType> {
    match ty {
        Type::Null => Ok(UdfType::Null),
        Type::Bool => Ok(UdfType::Bool),
        Type::I64 => Ok(UdfType::I64),
        Type::Text => Ok(UdfType::Text),
        _ => Err(UdfError::UnsupportedType),
    }
}

/// Maps a UDF type to the execution runtime type.
pub fn type_to_execution(ty: UdfType) -> Type {
    match ty {
        UdfType::Null => Type::Null,
        UdfType::Bool => Type::Bool,
        UdfType::I64 => Type::I64,
        UdfType::Text => Type::Text,
    }
}
