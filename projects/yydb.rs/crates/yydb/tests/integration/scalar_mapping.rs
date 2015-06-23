//! Canonical VOS scalar alignment with the YYDB execution scalar set.

use yydb::{
    Error,
    catalog::{canonical_scalar_for_execution_type, execution_type_for_scalar},
    execution::Type,
    types::{Decimal, ScalarError, Timestamp},
    vos::types::CanonicalScalar,
};

const SUPPORTED: [(CanonicalScalar, Type); 8] = [
    (CanonicalScalar::I64, Type::I64),
    (CanonicalScalar::Bool, Type::Bool),
    (CanonicalScalar::Utf8, Type::Text),
    (CanonicalScalar::Bytes, Type::Bytes),
    (CanonicalScalar::Date, Type::Date),
    (CanonicalScalar::Time, Type::Time),
    (CanonicalScalar::DateTime, Type::Timestamp),
    (CanonicalScalar::Decimal, Type::Decimal { precision: 38, scale: 0 }),
];

#[test]
fn every_supported_scalar_roundtrips_through_execution_type() {
    for (scalar, ty) in SUPPORTED {
        assert_eq!(execution_type_for_scalar(scalar).expect("supported scalar lowers"), ty);
        assert_eq!(canonical_scalar_for_execution_type(&ty).expect("execution type maps back"), scalar);
    }
}

#[test]
fn every_unmapped_canonical_scalar_reports_a_structured_error() {
    for scalar in CanonicalScalar::ALL {
        if SUPPORTED.iter().any(|(supported, _)| *supported == scalar) {
            continue;
        }
        let error = execution_type_for_scalar(scalar).expect_err("scalar has no execution type");
        assert!(matches!(error, Error::Schema { .. }), "unexpected error for {scalar}: {error}");
        assert_eq!(error.code(), "yydb.schema.invalid");
    }
}

#[test]
fn utf8_and_utf16_are_never_merged() {
    assert_eq!(execution_type_for_scalar(CanonicalScalar::Utf8).expect("utf8 lowers"), Type::Text);
    assert_eq!(canonical_scalar_for_execution_type(&Type::Text).expect("text maps back"), CanonicalScalar::Utf8);
    let error = execution_type_for_scalar(CanonicalScalar::Utf16).expect_err("utf16 has no execution type");
    assert!(matches!(error, Error::Schema { .. }), "utf16 must not degrade to text: {error}");
}

#[test]
fn datetime_maps_to_a_utc_instant_only() {
    assert_eq!(execution_type_for_scalar(CanonicalScalar::DateTime).expect("datetime lowers"), Type::Timestamp);
    assert_eq!(canonical_scalar_for_execution_type(&Type::Timestamp).expect("timestamp maps back"), CanonicalScalar::DateTime);
    let instant = Timestamp::parse_iso("2024-02-29T12:34:56.123456+00:00").expect("UTC offset is accepted");
    assert_eq!(instant.to_iso(), "2024-02-29T12:34:56.123456Z");
    assert!(matches!(Timestamp::parse_iso("2024-02-29T12:34:56+08:00"), Err(ScalarError::UnsupportedUtcOffset { .. })));
}

#[test]
fn decimal_maps_to_the_i128_unscaled_form_without_floating_point() {
    assert_eq!(
        execution_type_for_scalar(CanonicalScalar::Decimal).expect("decimal lowers"),
        Type::Decimal { precision: 38, scale: 0 }
    );
    let value = Decimal::parse("-1234.50", 6, 2).expect("exact decimal");
    assert_eq!(value.unscaled(), -123_450_i128);
    let ty = Type::Decimal { precision: value.precision(), scale: value.scale() };
    assert_eq!(canonical_scalar_for_execution_type(&ty).expect("decimal maps back"), CanonicalScalar::Decimal);
    assert!(matches!(execution_type_for_scalar(CanonicalScalar::F32), Err(Error::Schema { .. })));
    assert!(matches!(execution_type_for_scalar(CanonicalScalar::F64), Err(Error::Schema { .. })));
}

#[test]
fn composite_execution_types_have_no_canonical_scalar() {
    let composites = [Type::Null, Type::File, Type::Vector { dimension: 3 }, Type::Record { schema_id: 1, field_count: 2 }];
    for ty in composites {
        let error = canonical_scalar_for_execution_type(&ty).expect_err("composite has no canonical scalar");
        assert!(matches!(error, Error::Schema { .. }), "unexpected error for {ty:?}: {error}");
        assert_eq!(error.code(), "yydb.schema.invalid");
    }
}

#[test]
fn invalid_decimal_shapes_are_rejected_in_both_directions() {
    for ty in [Type::Decimal { precision: 0, scale: 0 }, Type::Decimal { precision: 2, scale: 3 }] {
        let error = canonical_scalar_for_execution_type(&ty).expect_err("invalid decimal shape");
        assert!(matches!(error, Error::Schema { .. }), "unexpected error for {ty:?}: {error}");
    }
    assert!(matches!(
        yydb::catalog::execution_type_for_scalar(CanonicalScalar::Decimal),
        Ok(Type::Decimal { precision: 38, scale: 0 })
    ));
}
