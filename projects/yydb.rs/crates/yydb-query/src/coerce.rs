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
    Date,
    Time,
    Timestamp,
    Decimal { precision: u8, scale: u8 },
}

impl std::fmt::Display for ScalarTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Real => f.write_str("f64"),
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
/// into text or a wider integer. `uuid` is the one opaque exception: it keeps
/// its canonical textual form in VOS query writes. Columns without a scalar
/// target (records and user types) accept the value unchanged.
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
    match ty {
        ResolvedCanonicalType::Scalar(
            CanonicalScalar::I64
            | CanonicalScalar::Bool
            | CanonicalScalar::Utf8
            | CanonicalScalar::Bytes
            | CanonicalScalar::Uuid,
        ) => Ok(value),
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
        _ => Ok(value),
    }
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
