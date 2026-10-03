use yydb_udf::{
    lower_micro_scalar, lower_vos_macro, Budget, UdfContext, UdfImplementation, UdfType, UdfValue,
    VosProgramImplementation,
};

#[test]
fn lowers_vos_micro_scalar_into_validated_program() {
    let lowered =
        lower_micro_scalar("micro double(value: i64) -> i64 { value + value }", 1).expect("lower");
    assert_eq!(lowered.identity.name(), "double");
    assert_eq!(lowered.signature.args, vec![UdfType::I64]);
    assert_eq!(lowered.signature.returns, UdfType::I64);

    let mut context = UdfContext::new(Budget::new(4));
    let result = VosProgramImplementation::new(lowered)
        .invoke(&mut context, &[UdfValue::I64(21)])
        .expect("invoke");
    assert_eq!(result, UdfValue::I64(42));
}

#[test]
fn lowers_vos_macro_via_micro_rewrite() {
    let lowered = lower_vos_macro(
        "macro catalog_double(value: i64) -> i64 { value + value }",
        2,
    )
    .expect("lower macro");
    assert_eq!(lowered.identity.name(), "catalog_double");
    assert_eq!(lowered.identity.version, 2);
    assert_eq!(lowered.signature.args, vec![UdfType::I64]);
    assert_eq!(lowered.signature.returns, UdfType::I64);
}

#[test]
fn rejects_invalid_vos_micro_source() {
    assert!(lower_micro_scalar("micro broken(value: i64) -> i64 { value + }", 1).is_err());
    assert!(lower_micro_scalar("micro text(value: i64) -> i64 { value == value }", 1).is_err());
}
