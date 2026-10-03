use std::sync::Arc;

use yydb::{
    Connection, TypeScriptFunctionHandle, TypeScriptHostAdapter, TypeScriptMicroDefinition,
    UdfType, Value,
};
use yydb_udf::UdfValue;

struct TrimAdapter;

impl TypeScriptHostAdapter for TrimAdapter {
    fn invoke_scalar(
        &self,
        handle: &TypeScriptFunctionHandle,
        args: &[UdfValue],
    ) -> yydb_udf::Result<UdfValue> {
        assert_eq!(handle.function_id, "normalize");
        let text = match &args[0] {
            UdfValue::Text(value) => value,
            _ => return Err(yydb_udf::UdfError::UnsupportedType),
        };
        Ok(UdfValue::Text(text.trim().to_ascii_lowercase()))
    }

    fn invoke_batch(
        &self,
        _handle: &TypeScriptFunctionHandle,
        _batches: &[Vec<UdfValue>],
    ) -> yydb_udf::Result<Vec<UdfValue>> {
        Err(yydb_udf::UdfError::UnsupportedType)
    }
}

#[test]
fn registers_and_calls_a_ts_micro_through_connection() {
    let conn = Connection::open_in_memory().expect("open");
    conn.set_ts_host_adapter(Arc::new(TrimAdapter));
    let definition = TypeScriptMicroDefinition::from_scalar(
        "text.normalize",
        1,
        vec![UdfType::Text],
        UdfType::Text,
        1,
        "normalize",
        1,
        [4u8; 32],
    )
    .expect("definition");
    conn.register_ts_micro(definition).expect("register");

    let result = conn
        .call_scalar("text.normalize", &[Value::Text("  Hi  ".into())])
        .expect("call");
    assert_eq!(result, Value::Text("hi".into()));
    assert!(conn.list_scalars().contains(&"text.normalize".to_owned()));
}
