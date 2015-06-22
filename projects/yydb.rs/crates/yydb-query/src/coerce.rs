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
    Real,
    Bytes,
    Date,
    Time,
    Timestamp,
    Decimal { precision: u8, scale: u8 },
}

impl std::fmt::Display for ScalarTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Real => f.write_str("f64"),
            Self::Bytes => f.write_str("bytes"),
            Self::Date => f.write_str("date"),
            Self::Time => f.write_str("time"),
            Self::Timestamp => f.write_str("datetime"),
            Self::Decimal { precision, scale } => write!(f, "decimal({precision}, {scale})"),
        }
    }
}

pub(crate) fn scalar_target(ty: &ResolvedCanonicalType) -> Option<ScalarTarget> {
    use vos::types::{CanonicalScalar, ResolvedCanonicalTypeArgument};
    match ty {
        ResolvedCanonicalType::Scalar(CanonicalScalar::F64) => Some(ScalarTarget::Real),
        ResolvedCanonicalType::Scalar(CanonicalScalar::Bytes) => Some(ScalarTarget::Bytes),
        ResolvedCanonicalType::Scalar(CanonicalScalar::Date) => Some(ScalarTarget::Date),
        ResolvedCanonicalType::Scalar(CanonicalScalar::Time) => Some(ScalarTarget::Time),
        ResolvedCanonicalType::Scalar(CanonicalScalar::DateTime) => Some(ScalarTarget::Timestamp),
        ResolvedCanonicalType::Scalar(CanonicalScalar::Decimal) => {
            Some(ScalarTarget::Decimal { precision: yydb_types::scalar::DECIMAL_MAX_PRECISION, scale: 0 })
        }
        ResolvedCanonicalType::Generic { path, arguments } if path == &["Decimal"] => {
            let [ResolvedCanonicalTypeArgument::Literal(precision), ResolvedCanonicalTypeArgument::Literal(scale)] =
                arguments.as_slice()
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
/// Canonical scalars outside the YYDB execution set are rejected instead of
/// being stored untyped, so a declared `utf16` or narrow integer never degrades
/// into text or a wider integer. A `uuid` column keeps the canonical lowercase
/// spelling, so a UUID value and a UUID text literal normalize to the same
/// value. A `vector<N>` column keeps exactly `N` finite components. Columns
/// without a scalar target (records and user types) accept the value unchanged.
pub(crate) fn value_to_scalar(value: Value, ty: &ResolvedCanonicalType) -> Result<Value> {
    use vos::types::CanonicalScalar;
    // A REAL value belongs in an `f64` column. Every other declared column
    // keeps an exact value, so a REAL is rejected before the write is staged
    // rather than being rounded into an integer or degraded into text.
    if let Value::F64(number) = &value {
        return match scalar_target(ty) {
            Some(ScalarTarget::Real) => coerce_to(value, ScalarTarget::Real),
            _ => Err(coerce_error(format!(
                "REAL value `{number}` cannot be stored: the target column is not `f64` and YYDB never converts \
                 REAL into an integer, bool, text, uuid, date, time, timestamp or decimal column"
            ))),
        };
    }
    // A BLOB value belongs in a `bytes` column. Every other target keeps an
    // exact value, so bytes are rejected before staging instead of being
    // reinterpreted as text or a number.
    if let Value::Bytes(_) = &value {
        return match scalar_target(ty) {
            Some(ScalarTarget::Bytes) => match value {
                Value::Bytes(octets) => super::store::place_bytes(octets),
                other => Ok(other),
            },
            _ => Err(coerce_error(
                "BLOB value cannot be stored: the target column is not `bytes` and YYDB never converts BLOB into an \
                 integer, bool, text, uuid, date, time, timestamp or decimal column",
            )),
        };
    }
    // A VECTOR value belongs in a `vector<N>` column of the same dimension.
    // Every other target keeps an exact value, so a vector is rejected before
    // staging instead of being reinterpreted as text, bytes or a number.
    if matches!(&value, Value::Vector(_)) {
        return match ty {
            ResolvedCanonicalType::Vector { dimension, .. } => coerce_vector(value, *dimension),
            _ => Err(coerce_error(
                "VECTOR value cannot be stored: the target column is not `vector<N>` and YYDB never converts a vector \
                 into an integer, bool, text, bytes, uuid, date, time, timestamp or decimal column",
            )),
        };
    }
    match ty {
        ResolvedCanonicalType::Scalar(CanonicalScalar::Uuid) => coerce_uuid(value),
        ResolvedCanonicalType::Scalar(CanonicalScalar::I64 | CanonicalScalar::Bool | CanonicalScalar::Utf8) => Ok(value),
        ResolvedCanonicalType::Scalar(scalar) => {
            let target = scalar_target(ty)
                .ok_or_else(|| coerce_error(format!("VOS scalar `{scalar}` is not in the YYDB execution scalar set")))?;
            coerce_to(value, target)
        }
        ResolvedCanonicalType::Generic { path, .. } if path == &["Decimal"] => {
            let target = scalar_target(ty)
                .ok_or_else(|| coerce_error("`Decimal` requires two literal arguments: precision and scale"))?;
            coerce_to(value, target)
        }
        ResolvedCanonicalType::Vector { dimension, .. } => coerce_vector(value, *dimension),
        _ => Ok(value),
    }
}

/// Check a vector value against the dimension the column declares.
///
/// The declared dimension is part of the column contract, so a vector with
/// another component count cannot be stored as if it were the declared one, and
/// a non-finite component is refused before the write is staged instead of
/// landing a value that no comparison or distance can interpret.
fn coerce_vector(value: Value, dimension: u32) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Vector(vector) => {
            if vector.dim != dimension {
                return Err(coerce_error(format!(
                    "column type `vector<{dimension}>` requires {dimension} components, got {}",
                    vector.dim
                )));
            }
            if let Some(index) = vector.data.iter().position(|component| !component.is_finite()) {
                return Err(coerce_error(format!(
                    "column type `vector<{dimension}>` requires finite components, component {index} is not finite"
                )));
            }
            Ok(Value::Vector(vector))
        }
        other => Err(coerce_error(format!(
            "column type `vector<{dimension}>` requires a vector value, got `{}`",
            other.kind()
        ))),
    }
}

/// Normalize a `uuid` column value to its canonical lowercase spelling.
///
/// A UUID value and a UUID text literal both end up as [`Value::Uuid`], so rows
/// written through the VOS and SQL frontends project, compare, and sort the
/// same way. Text that is not a UUID is rejected instead of being stored as if
/// it were one, and every other kind is rejected as well.
fn coerce_uuid(value: Value) -> Result<Value> {
    let parsed = match &value {
        Value::Null => return Ok(Value::Null),
        Value::Uuid(id) => return Ok(Value::Uuid(*id)),
        Value::Text(text) => uuid::Uuid::parse_str(text).map_err(|error| {
            coerce_error(format!("column type `uuid` requires a UUID literal: `{text}` is not one ({error})"))
        })?,
        other => {
            return Err(coerce_error(format!("column type `uuid` requires a UUID literal, got `{}`", other.kind())));
        }
    };
    Ok(Value::Uuid(parsed))
}

fn coerce_to(value: Value, target: ScalarTarget) -> Result<Value> {
    match (value, target) {
        (Value::Null, _) => Ok(Value::Null),
        (Value::I64(number), ScalarTarget::Real) => Ok(Value::F64(number as f64)),
        (Value::U64(number), ScalarTarget::Real) => Ok(Value::F64(number as f64)),
        (Value::F64(number), ScalarTarget::Real) if number.is_finite() => Ok(Value::F64(number)),
        (Value::F64(number), ScalarTarget::Real) => {
            Err(coerce_error(format!("REAL value `{number}` is not finite and cannot be stored in an `f64` column")))
        }
        (Value::Text(_) | Value::Bool(_), ScalarTarget::Real) => {
            Err(coerce_error("column type `f64` requires an integer or REAL numeric literal"))
        }
        (Value::Bytes(bytes), ScalarTarget::Bytes) => super::store::place_bytes(bytes),
        (Value::Object(object), ScalarTarget::Bytes) => Ok(Value::Object(object)),
        (Value::Text(_), ScalarTarget::Bytes) => Err(coerce_error(
            "column type `bytes` requires a BLOB value: YYDB never converts text into BLOB implicitly, use \
             `CAST(... AS BLOB)` for an explicit conversion",
        )),
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
        (Value::Bool(_), target) => {
            Err(coerce_error(format!("column type `{target}` requires an exact string or integer literal")))
        }
        (_, target) => Err(coerce_error(format!("unsupported literal for column type `{target}`"))),
    }
}
