use yydb_execution::Value;
use yydb_udf::{UdfType, UdfValue};

#[test]
fn udf_value_round_trips_phase1_execution_types() {
    for (udf, execution) in [
        (UdfValue::Null, Value::Null),
        (UdfValue::Bool(true), Value::Bool(true)),
        (UdfValue::I64(7), Value::I64(7)),
        (UdfValue::Text("yydb".into()), Value::Text("yydb".into())),
    ] {
        assert_eq!(UdfValue::from_execution(&execution).expect("from"), udf);
        assert_eq!(udf.into_execution().expect("into"), execution);
    }
}

#[test]
fn unsupported_execution_types_are_rejected_at_boundary() {
    assert!(matches!(
        UdfValue::from_execution(&Value::Bytes(vec![1, 2, 3])),
        Err(yydb_udf::UdfError::UnsupportedType)
    ));
    assert!(matches!(
        UdfType::from_execution(yydb_execution::Type::Bytes),
        Err(yydb_udf::UdfError::UnsupportedType)
    ));
}
