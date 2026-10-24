//! Catalog-directed scalar coercion shared by every write frontend.
//!
//! A table column declares a canonical type; a frontend only supplies a raw
//! literal (VOS literal or SQL literal). This module converts the frontend
//! value into the exact scalar the column stores, so VOS and SQL inserts and
//! updates type their values through one implementation.

use vos::types::ResolvedCanonicalType;
use yydb_types::{Error, Result, Value};

fn coerce_error(message: impl Into<String>) -> Error {
    Error::Schema { message: message.into() }
}

/// Exact scalar target declared by a VOS canonical type.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ScalarTarget {
    Date,
    Time,
    Timestamp,
    Decimal { precision: u8, scale: u8 },
}

impl std::fmt::Display for ScalarTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Date => f.write_str("date"),
            Self::Time => f.write_str("time"),
            Self::Timestamp => f.write_str("datetime"),
            Self::Decimal { precision, scale } => write!(f, "decimal({precision}, {scale})"),
        }
    }
}

pub(crate) fn scalar_target(ty: &ResolvedCanonicalType) -> Option<ScalarTarget> {
    use vos::types::ResolvedCanonicalTypeArgument;
    match ty {
        ResolvedCanonicalType::Builtin(path) if path == &["date"] => Some(ScalarTarget::Date),
        ResolvedCanonicalType::Builtin(path) if path == &["time"] => Some(ScalarTarget::Time),
        ResolvedCanonicalType::Builtin(path) if path == &["datetime"] => Some(ScalarTarget::Timestamp),
        ResolvedCanonicalType::Builtin(path) if path == &["decimal"] => {
            Some(ScalarTarget::Decimal { precision: 38, scale: 0 })
        }
        ResolvedCanonicalType::Generic { path, arguments } if path == &["Decimal"] => {
            let [
                ResolvedCanonicalTypeArgument::Literal(precision),
                ResolvedCanonicalTypeArgument::Literal(scale),
            ] = arguments.as_slice()
            else {
                return None;
            };
            let precision = precision.parse::<u8>().ok()?;
            let scale = scale.parse::<u8>().ok()?;
            yydb_types::Decimal::new(0, precision, scale)
                .ok()
                .map(|decimal| ScalarTarget::Decimal { precision: decimal.precision(), scale: decimal.scale() })
        }
        _ => None,
    }
}

/// Coerce an already-evaluated frontend value into the column's scalar type.
///
/// Columns without an exact scalar target accept the value unchanged so
/// frontends do not invent storage rules the catalog never declared.
pub(crate) fn value_to_scalar(value: Value, ty: &ResolvedCanonicalType) -> Result<Value> {
    let Some(target) = scalar_target(ty)
    else {
        return Ok(value);
    };
    match (value, target) {
        (Value::Null, _) => Ok(Value::Null),
        (Value::Text(text), ScalarTarget::Date) => Ok(Value::Date(yydb_types::Date::parse_iso(&text).map_err(Error::from)?)),
        (Value::Text(text), ScalarTarget::Time) => Ok(Value::Time(yydb_types::Time::parse_iso(&text).map_err(Error::from)?)),
        (Value::Text(text), ScalarTarget::Timestamp) => {
            Ok(Value::Timestamp(yydb_types::Timestamp::parse_iso(&text).map_err(Error::from)?))
        }
        (Value::Text(text), ScalarTarget::Decimal { precision, scale }) => {
            Ok(Value::Decimal(yydb_types::Decimal::parse(&text, precision, scale).map_err(Error::from)?))
        }
        (Value::I64(number), ScalarTarget::Decimal { precision, scale }) => {
            Ok(Value::Decimal(yydb_types::Decimal::parse(&number.to_string(), precision, scale).map_err(Error::from)?))
        }
        (Value::Bool(_) | Value::F64(_), target) => Err(coerce_error(format!(
            "column type `{target}` requires an exact string or integer literal"
        ))),
        (_, target) => Err(coerce_error(format!("unsupported literal for column type `{target}`"))),
    }
}
