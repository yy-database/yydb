use std::sync::Arc;

use yydb_udf::{
    Budget, ImplementationKind, InvocationMode, NativeHandler, NativeUdfDefinition, Placement,
    RegisterOptions, Signature, UdfDefinition, UdfIdentity, UdfInvocation, UdfPolicy, UdfRegistry,
    UdfType, UdfValue,
};

fn double_native() -> NativeUdfDefinition {
    let identity = UdfIdentity::new("math", "double", 1).expect("identity");
    let signature = Signature::new(vec![UdfType::I64], UdfType::I64);
    let handler: NativeHandler = Arc::new(|_ctx, args| {
        let value = match &args[0] {
            UdfValue::I64(value) => *value,
            _ => return Err(yydb_udf::UdfError::UnsupportedType),
        };
        Ok(UdfValue::I64(value * 2))
    });
    NativeUdfDefinition {
        identity,
        signature,
        policy: UdfPolicy::pure_embedded(),
        handler,
        fingerprint: [7u8; 32],
    }
}

#[test]
fn native_udf_registers_and_invokes_through_host_registry() {
    let native = double_native();
    native.validate().expect("native definition");
    let definition = native.session_definition();
    let identity = definition.identity.clone();

    let mut registry = UdfRegistry::default();
    registry
        .catalog_mut()
        .register(definition)
        .expect("catalog");
    registry
        .bind_host(Arc::new(native.into_implementation()))
        .expect("bind host");

    let result = registry
        .invoke(
            &UdfInvocation::scalar(identity, vec![UdfValue::I64(21)]),
            Budget::new(8),
        )
        .expect("invoke");
    assert_eq!(result, UdfValue::I64(42));
}

#[test]
fn session_cannot_shadow_catalog_without_explicit_flag() {
    let identity = UdfIdentity::new("", "sum", 1).expect("identity");
    let catalog = UdfDefinition {
        identity: identity.clone(),
        signature: Signature::new(vec![UdfType::I64, UdfType::I64], UdfType::I64),
        policy: UdfPolicy::pure_embedded(),
        placement: Placement::Embedded,
        implementation_kind: ImplementationKind::VosProgram,
        fingerprint: [1u8; 32],
    };
    let mut registry = UdfRegistry::default();
    registry.catalog_mut().register(catalog).expect("catalog");

    let session = UdfDefinition {
        identity: identity.clone(),
        signature: Signature::new(vec![UdfType::I64], UdfType::I64),
        policy: UdfPolicy::pure_embedded(),
        placement: Placement::Embedded,
        implementation_kind: ImplementationKind::Native,
        fingerprint: [2u8; 32],
    };
    assert!(matches!(
        registry.register_session(session.clone(), RegisterOptions::new()),
        Err(yydb_udf::UdfError::NameConflict)
    ));
    registry
        .register_session(session, RegisterOptions::shadowing())
        .expect("shadow session");
    assert_eq!(
        registry
            .resolve_definition(&identity)
            .expect("definition")
            .implementation_kind,
        ImplementationKind::Native
    );
}

#[test]
fn remove_session_entry_unbinds_host() {
    let native = double_native();
    let definition = native.session_definition();
    let identity = definition.identity.clone();

    let mut registry = UdfRegistry::default();
    registry
        .register_session_micro(
            definition,
            Arc::new(native.into_implementation()),
            RegisterOptions::new(),
        )
        .expect("register");
    registry.remove_session_by_name("double").expect("remove");
    assert!(registry.resolve_definition(&identity).is_none());
    assert!(matches!(
        registry.invoke(
            &UdfInvocation::scalar(identity, vec![UdfValue::I64(1)]),
            Budget::new(1),
        ),
        Err(yydb_udf::UdfError::ImplementationUnavailable)
    ));
}

#[test]
fn missing_host_implementation_returns_unavailable() {
    let identity = UdfIdentity::new("text", "normalize", 1).expect("identity");
    let definition = UdfDefinition {
        identity: identity.clone(),
        signature: Signature::new(vec![UdfType::Text], UdfType::Text),
        policy: UdfPolicy::pure_embedded(),
        placement: Placement::Host,
        implementation_kind: ImplementationKind::HostMicro,
        fingerprint: [3u8; 32],
    };
    let mut registry = UdfRegistry::default();
    registry
        .register_session(definition, RegisterOptions::new())
        .expect("session");
    assert!(matches!(
        registry.invoke(
            &UdfInvocation {
                identity,
                args: vec![UdfValue::Text("Hi".into())],
                mode: InvocationMode::Scalar,
            },
            Budget::new(1),
        ),
        Err(yydb_udf::UdfError::ImplementationUnavailable)
    ));
}
