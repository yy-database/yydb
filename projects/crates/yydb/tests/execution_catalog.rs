use yydb::{
    execution::{Node, Program, RecordValue, Type, ValidatedProgram, Value},
    schema::execution_catalog,
    Connection, Error,
};

fn resolved_user_contract() -> yydb::vos::ResolvedContract {
    yydb::vos::ResolvedContract {
        format_version: "vos-resolved-contract-v1".into(),
        identity_manifest_version: "vos-identity-manifest-v0".into(),
        schema_fingerprint: "0".repeat(64),
        types: vec![yydb::vos::contract::ResolvedTypeContract {
            type_id: 7,
            canonical_path: vec!["demo".into(), "User".into()],
            kind: yydb::vos::contract::TypeContractKind::Table,
            fields: vec![yydb::vos::contract::ResolvedFieldContract {
                field_id: 11,
                virtual_field_index: 3,
                canonical_name: "id".into(),
                canonical_type: yydb::vos::contract::ResolvedCanonicalType::Builtin(vec![
                    "i64".into(),
                ]),
                attributes: vec![],
                default_value: None,
            }],
        }],
    }
}

#[test]
fn lowers_resolved_contract_without_reparsing_or_reassigning_identity() {
    let contract = resolved_user_contract();
    let catalog = yydb::schema::execution_catalog_from_resolved_contract(&contract)
        .expect("resolved contract lowers");
    let table = &catalog.types[0];
    assert_eq!(table.schema_id, 7);
    assert_eq!(table.name, "User");
    assert_eq!(table.fields[0].field_id, 11);
    assert_eq!(table.fields[0].virtual_field, 3);
    assert_eq!(table.fields[0].handle.index(), 3);
}

#[test]
fn lowers_vos_catalog_identities_into_a_field_read() {
    let source = "class Request { name: utf8 } table User { @@id: i64, active: bool }";
    let catalog = execution_catalog(source).expect("initial VOS catalog lowers");
    let table = &catalog.types[1];
    assert_eq!(table.schema_id, 2);
    assert_eq!(table.name, "User");
    let field = &table.fields[0];
    assert_eq!(field.field_id, 2);
    assert_eq!(field.handle.field_id(), field.field_id);
    assert_eq!(field.handle.schema_id(), table.schema_id);
    assert_eq!(field.handle.index(), field.virtual_field);
    let row_type = Type::Record {
        schema_id: table.schema_id,
        field_count: 2,
    };
    let layout = table.layout().expect("catalog layout is valid");
    let program = ValidatedProgram::validate_with_layouts(
        Program {
            parameters: vec![],
            inputs: vec![row_type],
            nodes: vec![
                Node::Input {
                    index: 0,
                    ty: row_type,
                },
                Node::ReadField {
                    record: 0,
                    field: field.handle,
                },
            ],
            output: 1,
            output_type: Type::I64,
        },
        &[layout],
    )
    .expect("catalog-bound program validates");
    let row = RecordValue::new(table.schema_id, vec![Value::I64(42), Value::Bool(true)]).unwrap();
    assert_eq!(
        program.evaluate(&[], &[Value::Record(row)]),
        Ok(Value::I64(42))
    );
}

#[test]
fn rejects_invalid_schema_and_unsupported_types_without_coercion() {
    assert!(matches!(
        execution_catalog("table User { id: i64 }"),
        Err(Error::Schema { .. })
    ));
    for ty in ["utf16", "uuid", "i64?", "[i64]"] {
        let source = format!("table User {{ @@id: i64, value: {ty} }}");
        assert!(
            execution_catalog(&source).is_err(),
            "unsupported type accepted: {ty}"
        );
    }
    assert!(matches!(
        execution_catalog("table User { @@id: i64 } ]"),
        Err(Error::Schema { .. })
    ));
}

#[test]
fn refuses_invalid_vos_before_publishing_schema() {
    let conn = Connection::open_in_memory().unwrap();
    assert!(matches!(
        conn.ensure_schema("table User { id: i64 }"),
        Err(Error::Schema { .. })
    ));
    assert!(conn.schema().unwrap().is_none());
    conn.ensure_schema("table User { @@id: i64 }").unwrap();
    assert_eq!(conn.schema().unwrap().unwrap().version, 1);
}
