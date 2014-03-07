use std::sync::Arc;

use yydb_udf::{
    Budget, HostFunctionHandle, HostMicroDefinition, HostMicroImplementation, HostRuntimeAdapter,
    UdfContext, UdfImplementation, UdfInvocation, UdfType, UdfValue,
};

struct TrimAdapter;

impl HostRuntimeAdapter for TrimAdapter {
    fn invoke_scalar(
        &self,
        handle: &HostFunctionHandle,
        args: &[UdfValue],
    ) -> yydb_udf::Result<UdfValue> {
        assert_eq!(handle.function_id, "1");
        let text = match &args[0] {
            UdfValue::Text(value) => value,
            _ => return Err(yydb_udf::UdfError::UnsupportedType),
        };
        Ok(UdfValue::Text(text.trim().to_ascii_lowercase()))
    }

    fn invoke_batch(
        &self,
        _handle: &HostFunctionHandle,
        _batches: &[Vec<UdfValue>],
    ) -> yydb_udf::Result<Vec<UdfValue>> {
        Err(yydb_udf::UdfError::UnsupportedType)
    }
}

#[test]
fn host_micro_registers_and_invokes_through_runtime_adapter() {
    let definition = HostMicroDefinition::from_scalar(
        "text.normalize",
        1,
        vec![UdfType::Text],
        UdfType::Text,
        7,
        "1",
        1,
        [9u8; 32],
    )
    .expect("definition");
    definition.validate().expect("validate");

    let session = definition.session_definition();
    let identity = session.identity.clone();
    let adapter: Arc<dyn HostRuntimeAdapter> = Arc::new(TrimAdapter);
    let implementation = Arc::new(HostMicroImplementation::new(definition, adapter));

    let mut context = UdfContext::new(Budget::new(4));
    let result = implementation
        .invoke(&mut context, &[UdfValue::Text("  Hi  ".into())])
        .expect("invoke");
    assert_eq!(result, UdfValue::Text("hi".into()));

    let mut registry = yydb_udf::UdfRegistry::default();
    registry
        .register_session_micro(session, implementation, yydb_udf::RegisterOptions::new())
        .expect("register");
    let routed = registry
        .invoke(
            &UdfInvocation::scalar(identity, vec![UdfValue::Text("  Yo  ".into())]),
            Budget::new(4),
        )
        .expect("routed invoke");
    assert_eq!(routed, UdfValue::Text("yo".into()));
}
