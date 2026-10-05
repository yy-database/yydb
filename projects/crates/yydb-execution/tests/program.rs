use yydb_execution::{
    EvalError, FieldHandle, FileRef, LayoutField, Node, Program, RecordLayout, RecordValue, Type,
    Udf, UdfEffect, UdfPlacement, ValidatedProgram, ValidationError, Value, VectorMetric,
    VectorValue, VectorValueError,
};

#[test]
fn validates_and_evaluates_scalar_program() {
    let program = ValidatedProgram::validate(Program {
        parameters: vec![Type::I64],
        inputs: vec![Type::I64],
        nodes: vec![
            Node::Parameter {
                index: 0,
                ty: Type::I64,
            },
            Node::Input {
                index: 0,
                ty: Type::I64,
            },
            Node::AddI64 { left: 0, right: 1 },
        ],
        output: 2,
        output_type: Type::I64,
    })
    .expect("program is valid");

    assert_eq!(
        program.evaluate(&[Value::I64(2)], &[Value::I64(3)]),
        Ok(Value::I64(5))
    );
}

#[test]
fn rejects_out_of_order_reference() {
    let error = ValidatedProgram::validate(Program {
        parameters: vec![],
        inputs: vec![],
        nodes: vec![
            Node::AddI64 { left: 1, right: 1 },
            Node::Literal(Value::I64(1)),
        ],
        output: 0,
        output_type: Type::I64,
    })
    .expect_err("forward references are invalid");

    assert_eq!(
        error,
        ValidationError::ReferenceOutOfOrder {
            node: 0,
            reference: 1
        }
    );
}

#[test]
fn validates_nodes_that_are_not_on_the_output_path() {
    let error = ValidatedProgram::validate(Program {
        parameters: vec![],
        inputs: vec![],
        nodes: vec![
            Node::Literal(Value::I64(1)),
            Node::AddI64 { left: 2, right: 2 },
            Node::Literal(Value::I64(2)),
        ],
        output: 0,
        output_type: Type::I64,
    })
    .expect_err("every node must be valid, even when unreachable");

    assert_eq!(
        error,
        ValidationError::ReferenceOutOfOrder {
            node: 1,
            reference: 2
        }
    );
}

#[test]
fn rejects_integer_overflow_at_runtime() {
    let program = ValidatedProgram::validate(Program {
        parameters: vec![],
        inputs: vec![],
        nodes: vec![
            Node::Literal(Value::I64(i64::MAX)),
            Node::Literal(Value::I64(1)),
            Node::AddI64 { left: 0, right: 1 },
        ],
        output: 2,
        output_type: Type::I64,
    })
    .expect("program is valid");

    assert_eq!(program.evaluate(&[], &[]), Err(EvalError::IntegerOverflow));
}

#[test]
fn validates_and_evaluates_a_typed_udf() {
    let udf = Udf {
        id: "score.add".into(),
        version: 1,
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
    .validate()
    .expect("udf is valid");

    assert_eq!(udf.id(), "score.add");
    assert_eq!(
        udf.evaluate(&[], &[Value::I64(4), Value::I64(6)]),
        Ok(Value::I64(10))
    );
}

#[test]
fn preserves_file_and_vector_value_identity() {
    let file = Value::File(FileRef {
        object_id: "asset-1".into(),
        generation: 3,
        byte_len: 4096,
    });
    assert_eq!(file.ty(), Type::File);

    let vector = VectorValue::new(vec![0.25, 0.5, 0.75], VectorMetric::Cosine)
        .expect("finite vector is valid");
    assert_eq!(vector.dimension(), 3);
    assert_eq!(
        Value::Vector(vector).ty(),
        Type::Vector {
            dimension: 3,
            metric: VectorMetric::Cosine,
        }
    );
}

#[test]
fn validates_and_reads_a_typed_record_field() {
    let record = RecordValue::new(7, vec![Value::I64(42), Value::Text("ready".into())])
        .expect("record schema identity and field count are valid");
    let program = ValidatedProgram::validate(Program {
        parameters: vec![],
        inputs: vec![Type::Record {
            schema_id: 7,
            field_count: 2,
        }],
        nodes: vec![
            Node::Input {
                index: 0,
                ty: Type::Record {
                    schema_id: 7,
                    field_count: 2,
                },
            },
            Node::ReadField {
                record: 0,
                field: FieldHandle::new(7, 101, 0, Type::I64).expect("field handle is valid"),
            },
        ],
        output: 1,
        output_type: Type::I64,
    })
    .expect("record field access is valid");

    assert_eq!(
        program.evaluate(&[], &[Value::Record(record)]),
        Ok(Value::I64(42))
    );
}

#[test]
fn rejects_record_field_out_of_bounds() {
    let error = ValidatedProgram::validate(Program {
        parameters: vec![],
        inputs: vec![Type::Record {
            schema_id: 7,
            field_count: 1,
        }],
        nodes: vec![
            Node::Input {
                index: 0,
                ty: Type::Record {
                    schema_id: 7,
                    field_count: 1,
                },
            },
            Node::ReadField {
                record: 0,
                field: FieldHandle::new(7, 102, 1, Type::I64).expect("field handle is valid"),
            },
        ],
        output: 1,
        output_type: Type::I64,
    })
    .expect_err("record field index must fit the declared shape");

    assert_eq!(
        error,
        ValidationError::FieldOutOfBounds {
            node: 1,
            index: 1,
            field_count: 1,
        }
    );
}

#[test]
fn rejects_field_handles_from_another_schema() {
    let error = ValidatedProgram::validate(Program {
        parameters: vec![],
        inputs: vec![Type::Record {
            schema_id: 7,
            field_count: 1,
        }],
        nodes: vec![
            Node::Input {
                index: 0,
                ty: Type::Record {
                    schema_id: 7,
                    field_count: 1,
                },
            },
            Node::ReadField {
                record: 0,
                field: FieldHandle::new(8, 201, 0, Type::I64).expect("field handle is valid"),
            },
        ],
        output: 1,
        output_type: Type::I64,
    })
    .expect_err("field handle schema identity must match the record");

    assert_eq!(
        error,
        ValidationError::FieldSchemaMismatch {
            node: 1,
            expected: 8,
            found: 7,
        }
    );
}

#[test]
fn validates_field_handles_against_a_published_layout() {
    let field = FieldHandle::new(7, 101, 0, Type::I64).expect("field handle is valid");
    let layout = RecordLayout::new(
        7,
        vec![LayoutField {
            field_id: 101,
            index: 0,
            ty: Type::I64,
        }],
    )
    .expect("layout is valid");
    let record_type = Type::Record {
        schema_id: 7,
        field_count: 1,
    };
    let program = ValidatedProgram::validate_with_layouts(
        Program {
            parameters: vec![],
            inputs: vec![record_type],
            nodes: vec![
                Node::Input {
                    index: 0,
                    ty: record_type,
                },
                Node::ReadField { record: 0, field },
            ],
            output: 1,
            output_type: Type::I64,
        },
        &[layout],
    )
    .expect("layout-bound program validates");
    let row = RecordValue::new(7, vec![Value::I64(42)]).unwrap();
    assert_eq!(
        program.evaluate(&[], &[Value::Record(row)]),
        Ok(Value::I64(42))
    );
}

#[test]
fn rejects_a_field_handle_that_disagrees_with_layout_type() {
    let field = FieldHandle::new(7, 101, 0, Type::Text).expect("field handle is valid");
    let layout = RecordLayout::new(
        7,
        vec![LayoutField {
            field_id: 101,
            index: 0,
            ty: Type::I64,
        }],
    )
    .expect("layout is valid");
    let record_type = Type::Record {
        schema_id: 7,
        field_count: 1,
    };
    let error = ValidatedProgram::validate_with_layouts(
        Program {
            parameters: vec![],
            inputs: vec![record_type],
            nodes: vec![
                Node::Input {
                    index: 0,
                    ty: record_type,
                },
                Node::ReadField { record: 0, field },
            ],
            output: 1,
            output_type: Type::Text,
        },
        &[layout],
    )
    .expect_err("layout mismatch must be rejected");
    assert!(matches!(
        error,
        ValidationError::LayoutFieldMismatch { field_id: 101, .. }
    ));
}

#[test]
fn rejects_non_finite_vector_components() {
    assert_eq!(
        VectorValue::new(vec![1.0, f32::NAN], VectorMetric::Dot),
        Err(VectorValueError::NonFiniteComponent { index: 1 })
    );
}

#[test]
fn equality_handles_null_and_opaque_values_without_coercion() {
    let program = ValidatedProgram::validate(Program {
        parameters: vec![],
        inputs: vec![],
        nodes: vec![
            Node::Literal(Value::Null),
            Node::Literal(Value::Null),
            Node::Equal { left: 0, right: 1 },
        ],
        output: 2,
        output_type: Type::Bool,
    })
    .expect("null equality is valid");

    assert_eq!(program.evaluate(&[], &[]), Ok(Value::Bool(true)));
}
