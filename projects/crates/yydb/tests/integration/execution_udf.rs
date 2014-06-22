use yydb::{
    execution::{Node, Program, Type, Udf, UdfEffect, UdfPlacement},
    Connection, Error, Value,
};

fn sum_body() -> Udf {
    Udf {
        id: "sum".into(),
        version: 2,
        deterministic: true,
        effect: UdfEffect::Read,
        placement: UdfPlacement::Local,
        program: Program {
            parameters: vec![],
            inputs: vec![Type::I64, Type::I64],
            nodes: vec![
                Node::Input {
                    index: 0,
                    ty: Type::I64,
                },
                Node::Input {
                    index: 1,
                    ty: Type::I64,
                },
                Node::AddI64 { left: 0, right: 1 },
            ],
            output: 2,
            output_type: Type::I64,
        },
    }
}

#[test]
fn registers_and_evaluates_yydb_local_execution_in_a_real_connection() {
    let conn = Connection::open_in_memory().unwrap();
    conn.register_execution_udf(sum_body().validate().unwrap())
        .unwrap();
    assert_eq!(
        conn.call_scalar_version("sum", 2, &[Value::I64(4), Value::I64(6)])
            .unwrap(),
        Value::I64(10)
    );
    assert!(matches!(
        conn.call_scalar("sum", &[Value::I64(4), Value::I64(6)]),
        Err(Error::UdfVersionMismatch { .. })
    ));
    assert!(matches!(
        conn.call_scalar_version("sum", 2, &[Value::I64(4)]),
        Err(Error::UdfArity { .. })
    ));
}

#[test]
fn rejects_wrong_types_and_overflow_without_panicking() {
    let conn = Connection::open_in_memory().unwrap();
    conn.register_execution_udf(sum_body().validate().unwrap())
        .unwrap();
    assert!(matches!(
        conn.call_scalar_version("sum", 2, &[Value::Bool(true), Value::I64(1)]),
        Err(Error::Udf { .. })
    ));
    assert!(matches!(
        conn.call_scalar_version("sum", 2, &[Value::I64(i64::MAX), Value::I64(1)]),
        Err(Error::Udf { .. })
    ));
    assert!(matches!(
        conn.call_scalar_version("sum", 2, &[Value::Null, Value::I64(1)]),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn rejects_unsupported_effects_placements_and_captured_parameters_before_registration() {
    let conn = Connection::open_in_memory().unwrap();
    for effect in [UdfEffect::Write, UdfEffect::External] {
        let mut body = sum_body();
        body.effect = effect;
        assert!(matches!(
            conn.register_execution_udf(body.validate().unwrap()),
            Err(Error::Unsupported(_))
        ));
    }
    for placement in [
        UdfPlacement::Shard,
        UdfPlacement::Coordinator,
        UdfPlacement::Edge,
    ] {
        let mut body = sum_body();
        body.placement = placement;
        assert!(matches!(
            conn.register_execution_udf(body.validate().unwrap()),
            Err(Error::Unsupported(_))
        ));
    }
    let mut body = sum_body();
    body.program.parameters.push(Type::I64);
    assert!(matches!(
        conn.register_execution_udf(body.validate().unwrap()),
        Err(Error::Unsupported(_))
    ));
    assert!(conn.list_scalars().is_empty());
}

#[test]
fn lowers_and_evaluates_a_vos_catalog_macro() {
    let conn = Connection::open_in_memory().unwrap();
    conn.register_vos_macro("macro triple(value: i64) -> i64 { value + value + value }")
        .unwrap();
    assert_eq!(
        conn.call_scalar("triple", &[Value::I64(7)]).unwrap(),
        Value::I64(21)
    );
}

#[test]
fn lowers_and_evaluates_an_oak_validated_vos_scalar() {
    let conn = Connection::open_in_memory().unwrap();
    conn.register_vos_scalar("micro double(value: i64) -> i64 { value + value }")
        .unwrap();
    assert_eq!(
        conn.call_scalar("double", &[Value::I64(21)]).unwrap(),
        Value::I64(42)
    );
}

#[test]
fn rejects_vos_udf_parse_errors_and_unsupported_lowering() {
    let conn = Connection::open_in_memory().unwrap();
    assert!(matches!(
        conn.register_vos_scalar("micro broken(value: i64) -> i64 { value + }")
            .unwrap_err(),
        Error::Udf { .. }
    ));
    assert!(matches!(
        conn.register_vos_scalar("micro text(value: i64) -> i64 { value == value }")
            .unwrap_err(),
        Error::Udf { .. }
    ));
    assert!(conn.list_scalars().is_empty());
}
