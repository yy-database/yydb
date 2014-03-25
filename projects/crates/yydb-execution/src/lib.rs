//! Stable local execution model owned by YYDB.
//!
//! VOS frontends lower into this model through the [`yydb`] facade binder. This
//! crate deliberately has no dependency on Oak, VOS, YYDS, or a remote execution
//! protocol. Applications should use `yydb::execution` instead of depending here
//! directly.

#![deny(missing_docs)]

/// A node identifier in an execution Program.
pub type NodeId = u32;

/// Metric used to interpret a vector value or vector index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorMetric {
    /// Cosine distance or similarity.
    Cosine,
    /// Euclidean distance.
    Euclidean,
    /// Inner product.
    Dot,
}

/// Runtime types supported by the execution model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    /// Explicit null value.
    Null,
    /// Boolean value.
    Bool,
    /// Signed 64-bit integer.
    I64,
    /// UTF-8 text value.
    Text,
    /// Opaque bytes.
    Bytes,
    /// Immutable file or object reference.
    File,
    /// Fixed-dimension vector with an explicit metric.
    Vector {
        /// Number of vector components.
        dimension: u32,
        /// Metric used by vector operations and indexes.
        metric: VectorMetric,
    },
    /// Schema-identified record with a fixed number of fields.
    Record {
        /// Stable schema identity assigned by the catalog.
        schema_id: u64,
        /// Number of positional fields in the record.
        field_count: u32,
    },
}

/// Immutable reference to a published file or object manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    /// Stable object identity supplied by the catalog.
    pub object_id: String,
    /// Published manifest generation.
    pub generation: u64,
    /// Logical byte length of the referenced object.
    pub byte_len: u64,
}

/// A vector value with an explicit metric and finite components.
#[derive(Debug, Clone)]
pub struct VectorValue {
    /// Vector components in logical order.
    values: Vec<f32>,
    /// Metric used to interpret this vector.
    metric: VectorMetric,
}

impl PartialEq for VectorValue {
    fn eq(&self, other: &Self) -> bool {
        self.metric == other.metric
            && self.values.len() == other.values.len()
            && self
                .values
                .iter()
                .zip(&other.values)
                .all(|(left, right)| left.to_bits() == right.to_bits())
    }
}

impl Eq for VectorValue {}

impl VectorValue {
    /// Creates a vector after rejecting non-finite components.
    pub fn new(values: Vec<f32>, metric: VectorMetric) -> Result<Self, VectorValueError> {
        if values.is_empty() || u32::try_from(values.len()).is_err() {
            return Err(VectorValueError::InvalidDimension);
        }
        if let Some(index) = values.iter().position(|value| !value.is_finite()) {
            return Err(VectorValueError::NonFiniteComponent { index });
        }
        Ok(Self { values, metric })
    }

    /// Returns the vector dimension.
    pub fn dimension(&self) -> u32 {
        self.values.len().try_into().expect("validated vector dimension")
    }

    /// Returns the finite vector components.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Returns the vector metric.
    pub fn metric(&self) -> VectorMetric {
        self.metric
    }
}

/// A schema-identified record value with positional fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordValue {
    schema_id: u64,
    fields: Vec<Value>,
}

/// A stable typed handle for one field in a record schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldHandle {
    schema_id: u64,
    field_id: u64,
    index: u32,
    ty: Type,
}

impl FieldHandle {
    /// Creates a field handle with non-zero schema and field identities.
    pub fn new(schema_id: u64, field_id: u64, index: u32, ty: Type) -> Result<Self, FieldHandleError> {
        if schema_id == 0 {
            return Err(FieldHandleError::InvalidSchemaId);
        }
        if field_id == 0 {
            return Err(FieldHandleError::InvalidFieldId);
        }
        Ok(Self { schema_id, field_id, index, ty })
    }

    /// Returns the schema identity owning this field.
    pub fn schema_id(&self) -> u64 {
        self.schema_id
    }

    /// Returns the durable field identity.
    pub fn field_id(&self) -> u64 {
        self.field_id
    }

    /// Returns the positional field index.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Returns the declared field type.
    pub fn ty(&self) -> Type {
        self.ty
    }
}

/// Failure while constructing a field handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldHandleError {
    /// Zero is reserved for an absent schema identity.
    InvalidSchemaId,
    /// Zero is reserved for an absent field identity.
    InvalidFieldId,
}

/// One field entry in a published record layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutField {
    /// Durable field identity.
    pub field_id: u64,
    /// Virtual row position.
    pub index: u32,
    /// Published execution type.
    pub ty: Type,
}

/// Published record layout used to verify field handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordLayout {
    schema_id: u64,
    fields: Vec<LayoutField>,
}

impl RecordLayout {
    /// Creates a layout with unique non-zero field identities and positions.
    pub fn new(schema_id: u64, fields: Vec<LayoutField>) -> Result<Self, RecordLayoutError> {
        if schema_id == 0 {
            return Err(RecordLayoutError::InvalidSchemaId);
        }
        for (position, field) in fields.iter().enumerate() {
            if field.field_id == 0 {
                return Err(RecordLayoutError::InvalidFieldId);
            }
            if fields[..position].iter().any(|prior| prior.field_id == field.field_id) {
                return Err(RecordLayoutError::DuplicateFieldId { field_id: field.field_id });
            }
            if fields[..position].iter().any(|prior| prior.index == field.index) {
                return Err(RecordLayoutError::DuplicateIndex { index: field.index });
            }
        }
        Ok(Self { schema_id, fields })
    }

    /// Returns the schema identity.
    pub fn schema_id(&self) -> u64 {
        self.schema_id
    }

    /// Returns the published field entries.
    pub fn fields(&self) -> &[LayoutField] {
        &self.fields
    }
}

/// Failure while constructing a record layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordLayoutError {
    /// Zero is reserved for an absent schema identity.
    InvalidSchemaId,
    /// Zero is reserved for an absent field identity.
    InvalidFieldId,
    /// A field identity occurs more than once.
    DuplicateFieldId {
        /// Repeated field identity.
        field_id: u64,
    },
    /// A virtual position occurs more than once.
    DuplicateIndex {
        /// Repeated virtual position.
        index: u32,
    },
}

impl RecordValue {
    /// Creates a record value with a non-zero schema identity.
    pub fn new(schema_id: u64, fields: Vec<Value>) -> Result<Self, RecordValueError> {
        if schema_id == 0 {
            return Err(RecordValueError::InvalidSchemaId);
        }
        if u32::try_from(fields.len()).is_err() {
            return Err(RecordValueError::TooManyFields);
        }
        Ok(Self { schema_id, fields })
    }

    /// Returns the stable schema identity.
    pub fn schema_id(&self) -> u64 {
        self.schema_id
    }

    /// Returns all positional fields.
    pub fn fields(&self) -> &[Value] {
        &self.fields
    }

    /// Returns one positional field.
    pub fn field(&self, index: u32) -> Option<&Value> {
        self.fields.get(index as usize)
    }
}

/// Failure while constructing a record value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordValueError {
    /// Zero is reserved for an absent schema identity.
    InvalidSchemaId,
    /// The field count does not fit the execution type range.
    TooManyFields,
}

/// Failure while constructing a vector value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VectorValueError {
    /// The vector is empty or exceeds the dimension range.
    InvalidDimension,
    /// A component was NaN or infinite.
    NonFiniteComponent {
        /// Position of the invalid component.
        index: usize,
    },
}

/// Runtime value supported by the execution model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Explicit null value.
    Null,
    /// Boolean value.
    Bool(bool),
    /// Signed 64-bit integer.
    I64(i64),
    /// UTF-8 text value.
    Text(String),
    /// Opaque bytes.
    Bytes(Vec<u8>),
    /// Immutable file or object reference.
    File(FileRef),
    /// Fixed-dimension vector value.
    Vector(VectorValue),
    /// Schema-identified record value.
    Record(RecordValue),
}

impl Value {
    /// Returns the execution type of this value.
    pub fn ty(&self) -> Type {
        match self {
            Self::Null => Type::Null,
            Self::Bool(_) => Type::Bool,
            Self::I64(_) => Type::I64,
            Self::Text(_) => Type::Text,
            Self::Bytes(_) => Type::Bytes,
            Self::File(_) => Type::File,
            Self::Vector(value) => Type::Vector {
                dimension: value.dimension(),
                metric: value.metric,
            },
            Self::Record(value) => Type::Record {
                schema_id: value.schema_id,
                field_count: value.fields.len().try_into().expect("validated record field count"),
            },
        }
    }
}

/// A scalar expression node in an execution program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A literal value embedded in the program.
    Literal(Value),
    /// Reads a positional parameter supplied by the caller.
    Parameter {
        /// Positional parameter index.
        index: u32,
        /// Declared parameter type.
        ty: Type,
    },
    /// Reads a positional input supplied by the execution host.
    Input {
        /// Positional input index.
        index: u32,
        /// Declared input type.
        ty: Type,
    },
    /// Reads one positional field from a schema-identified record.
    ReadField {
        /// Record expression node.
        record: NodeId,
        /// Stable schema and field contract.
        field: FieldHandle,
    },
    /// Checked signed integer addition.
    AddI64 {
        /// Left operand node.
        left: NodeId,
        /// Right operand node.
        right: NodeId,
    },
    /// Equality comparison for two values of the same type.
    Equal {
        /// Left operand node.
        left: NodeId,
        /// Right operand node.
        right: NodeId,
    },
}

/// A complete typed scalar program before backend-specific planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// Parameter types in positional order.
    pub parameters: Vec<Type>,
    /// Host input types in positional order.
    pub inputs: Vec<Type>,
    /// Nodes in topological order. References must point to earlier nodes.
    pub nodes: Vec<Node>,
    /// Node whose value is returned.
    pub output: NodeId,
    /// Type promised by the output node.
    pub output_type: Type,
}

/// Effect declared by a UDF contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdfEffect {
    /// Reads database state without mutating it.
    Read,
    /// Mutates state and must run inside a write boundary.
    Write,
    /// Calls a capability outside the database state.
    External,
}

/// Placement requested by a UDF contract before physical planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdfPlacement {
    /// Execute in the local embedded database process.
    Local,
    /// Execute beside a distributed shard.
    Shard,
    /// Execute at a distributed coordinator.
    Coordinator,
    /// Execute at an edge service or worker.
    Edge,
}

/// A typed UDF contract whose body is already lowered to this execution model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Udf {
    /// Stable frontend-assigned function identity.
    pub id: String,
    /// Monotonic implementation version for registration and cache invalidation.
    pub version: u32,
    /// Whether evaluation is safe to repeat.
    pub deterministic: bool,
    /// Declared side effect.
    pub effect: UdfEffect,
    /// Preferred physical placement.
    pub placement: UdfPlacement,
    /// Typed function body.
    pub program: Program,
}

impl Udf {
    /// Validates the UDF metadata and its lowered body.
    pub fn validate(self) -> Result<ValidatedUdf, UdfValidationError> {
        if self.id.is_empty() {
            return Err(UdfValidationError::EmptyId);
        }
        if self.version == 0 {
            return Err(UdfValidationError::InvalidVersion);
        }
        validate_program(&self.program)?;
        Ok(ValidatedUdf { metadata: self })
    }
}

/// A UDF accepted by the execution runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedUdf {
    metadata: Udf,
}

impl ValidatedUdf {
    /// Returns the stable UDF identity.
    pub fn id(&self) -> &str {
        &self.metadata.id
    }

    /// Returns the UDF implementation version.
    pub fn version(&self) -> u32 {
        self.metadata.version
    }

    /// Returns the parameter signature required by this body.
    pub fn parameter_types(&self) -> &[Type] {
        &self.metadata.program.parameters
    }

    /// Returns the host input signature required by this body.
    pub fn input_types(&self) -> &[Type] {
        &self.metadata.program.inputs
    }

    /// Returns the validated output type.
    pub fn output_type(&self) -> Type {
        self.metadata.program.output_type
    }

    /// Returns the declared effect for host capability admission.
    pub fn effect(&self) -> UdfEffect {
        self.metadata.effect
    }

    /// Returns the requested placement for host capability admission.
    pub fn placement(&self) -> UdfPlacement {
        self.metadata.placement
    }

    /// Evaluates the validated UDF body.
    pub fn evaluate(&self, parameters: &[Value], inputs: &[Value]) -> Result<Value, EvalError> {
        evaluate_program(&self.metadata.program, parameters, inputs)
    }
}

/// Contract error found before a UDF is registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UdfValidationError {
    /// The UDF identity is empty.
    EmptyId,
    /// Version zero is reserved for an absent implementation.
    InvalidVersion,
    /// The lowered body is invalid.
    Program(ValidationError),
}

impl From<ValidationError> for UdfValidationError {
    fn from(error: ValidationError) -> Self {
        Self::Program(error)
    }
}

/// A validated program ready for evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedProgram(Program);

impl ValidatedProgram {
    /// Validates and accepts a program.
    pub fn validate(program: Program) -> Result<Self, ValidationError> {
        validate_program(&program)?;
        Ok(Self(program))
    }

    /// Validates a program and its record field handles against published layouts.
    pub fn validate_with_layouts(
        program: Program,
        layouts: &[RecordLayout],
    ) -> Result<Self, ValidationError> {
        validate_program(&program)?;
        validate_layout_bindings(&program, layouts)?;
        Ok(Self(program))
    }

    /// Evaluates the program with positional parameters and host inputs.
    pub fn evaluate(&self, parameters: &[Value], inputs: &[Value]) -> Result<Value, EvalError> {
        evaluate_program(&self.0, parameters, inputs)
    }

    /// Returns the validated output type.
    pub fn output_type(&self) -> Type {
        self.0.output_type
    }
}

/// Structural or typing error found while validating a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// The program has no nodes.
    EmptyProgram,
    /// The program exceeds the node identity range.
    TooManyNodes,
    /// The output node is outside the node vector.
    OutputOutOfBounds {
        /// Requested output node.
        output: NodeId,
    },
    /// A node reference is not topologically earlier than its consumer.
    ReferenceOutOfOrder {
        /// Consumer node.
        node: NodeId,
        /// Referenced node.
        reference: NodeId,
    },
    /// A parameter index is outside the declared parameter list.
    ParameterOutOfBounds {
        /// Consumer node.
        node: NodeId,
        /// Invalid parameter index.
        index: u32,
    },
    /// An input index is outside the declared input list.
    InputOutOfBounds {
        /// Consumer node.
        node: NodeId,
        /// Invalid input index.
        index: u32,
    },
    /// A record field index is outside the declared record shape.
    FieldOutOfBounds {
        /// Consumer node.
        node: NodeId,
        /// Invalid field index.
        index: u32,
        /// Declared number of fields.
        field_count: u32,
    },
    /// A field handle belongs to a different record schema.
    FieldSchemaMismatch {
        /// Consumer node.
        node: NodeId,
        /// Expected schema identity.
        expected: u64,
        /// Found schema identity.
        found: u64,
    },
    /// No published layout exists for a record schema used by a field read.
    RecordLayoutNotFound {
        /// Consumer node.
        node: NodeId,
        /// Missing schema identity.
        schema_id: u64,
    },
    /// A field handle is absent from its published record layout.
    LayoutFieldNotFound {
        /// Consumer node.
        node: NodeId,
        /// Missing field identity.
        field_id: u64,
    },
    /// A field handle disagrees with its published layout entry.
    LayoutFieldMismatch {
        /// Consumer node.
        node: NodeId,
        /// Durable field identity.
        field_id: u64,
        /// Published virtual position.
        expected_index: u32,
        /// Handle virtual position.
        found_index: u32,
        /// Published type.
        expected_type: Type,
        /// Handle type.
        found_type: Type,
    },
    /// A node operand has a type different from the required type.
    TypeMismatch {
        /// Consumer node.
        node: NodeId,
        /// Required type.
        expected: Type,
        /// Found type.
        found: Type,
    },
    /// The program output type differs from the output node type.
    OutputTypeMismatch {
        /// Declared output type.
        expected: Type,
        /// Inferred output type.
        found: Type,
    },
}

/// Runtime failure while evaluating a validated program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// The caller supplied the wrong number of parameters.
    ParameterArity {
        /// Declared parameter count.
        expected: usize,
        /// Supplied parameter count.
        got: usize,
    },
    /// The caller supplied the wrong number of inputs.
    InputArity {
        /// Declared input count.
        expected: usize,
        /// Supplied input count.
        got: usize,
    },
    /// A caller value does not match the declared type.
    TypeMismatch {
        /// Declared type.
        expected: Type,
        /// Supplied type.
        found: Type,
    },
    /// A checked integer operation overflowed.
    IntegerOverflow,
}

fn reference_type(
    types: &[Type],
    node: NodeId,
    reference: NodeId,
) -> Result<Type, ValidationError> {
    types
        .get(reference as usize)
        .copied()
        .ok_or(ValidationError::ReferenceOutOfOrder { node, reference })
}

fn require_type(node: NodeId, expected: Type, found: Type) -> Result<(), ValidationError> {
    if expected != found {
        return Err(ValidationError::TypeMismatch {
            node,
            expected,
            found,
        });
    }
    Ok(())
}

fn validate_program(program: &Program) -> Result<(), ValidationError> {
    if program.nodes.is_empty() {
        return Err(ValidationError::EmptyProgram);
    }
    if program.nodes.len() > u32::MAX as usize {
        return Err(ValidationError::TooManyNodes);
    }
    if program.output as usize >= program.nodes.len() {
        return Err(ValidationError::OutputOutOfBounds {
            output: program.output,
        });
    }
    let mut types = Vec::with_capacity(program.nodes.len());
    for (position, expression) in program.nodes.iter().enumerate() {
        let node = position as NodeId;
        let inferred = match expression {
            Node::Literal(value) => value.ty(),
            Node::Parameter { index, ty } => {
                let expected = program.parameters.get(*index as usize).copied().ok_or(
                    ValidationError::ParameterOutOfBounds {
                        node,
                        index: *index,
                    },
                )?;
                require_type(node, expected, *ty)?;
                expected
            }
            Node::Input { index, ty } => {
                let expected = program.inputs.get(*index as usize).copied().ok_or(
                    ValidationError::InputOutOfBounds {
                        node,
                        index: *index,
                    },
                )?;
                require_type(node, expected, *ty)?;
                expected
            }
            Node::ReadField { record, field } => {
                let record_type = reference_type(&types, node, *record)?;
                let Type::Record { schema_id, field_count } = record_type else {
                    return Err(ValidationError::TypeMismatch {
                        node,
                        expected: Type::Record { schema_id: 0, field_count: 0 },
                        found: record_type,
                    });
                };
                if schema_id != field.schema_id {
                    return Err(ValidationError::FieldSchemaMismatch {
                        node,
                        expected: field.schema_id,
                        found: schema_id,
                    });
                }
                if field.index >= field_count {
                    return Err(ValidationError::FieldOutOfBounds {
                        node,
                        index: field.index,
                        field_count,
                    });
                }
                field.ty
            }
            Node::AddI64 { left, right } => {
                require_type(node, Type::I64, reference_type(&types, node, *left)?)?;
                require_type(node, Type::I64, reference_type(&types, node, *right)?)?;
                Type::I64
            }
            Node::Equal { left, right } => {
                require_type(
                    node,
                    reference_type(&types, node, *left)?,
                    reference_type(&types, node, *right)?,
                )?;
                Type::Bool
            }
        };
        types.push(inferred);
    }
    let found = types[program.output as usize];
    if found != program.output_type {
        return Err(ValidationError::OutputTypeMismatch {
            expected: program.output_type,
            found,
        });
    }
    Ok(())
}

fn validate_layout_bindings(
    program: &Program,
    layouts: &[RecordLayout],
) -> Result<(), ValidationError> {
    for (position, node) in program.nodes.iter().enumerate() {
        let node_id = position as NodeId;
        let Node::ReadField { field, .. } = node else { continue };
        let layout = layouts
            .iter()
            .find(|layout| layout.schema_id == field.schema_id)
            .ok_or(ValidationError::RecordLayoutNotFound {
                node: node_id,
                schema_id: field.schema_id,
            })?;
        let layout_field = layout
            .fields
            .iter()
            .find(|layout_field| layout_field.field_id == field.field_id)
            .ok_or(ValidationError::LayoutFieldNotFound {
                node: node_id,
                field_id: field.field_id,
            })?;
        if layout_field.index != field.index || layout_field.ty != field.ty {
            return Err(ValidationError::LayoutFieldMismatch {
                node: node_id,
                field_id: field.field_id,
                expected_index: layout_field.index,
                found_index: field.index,
                expected_type: layout_field.ty,
                found_type: field.ty,
            });
        }
    }
    Ok(())
}

fn evaluate_program(
    program: &Program,
    parameters: &[Value],
    inputs: &[Value],
) -> Result<Value, EvalError> {
    if parameters.len() != program.parameters.len() {
        return Err(EvalError::ParameterArity {
            expected: program.parameters.len(),
            got: parameters.len(),
        });
    }
    if inputs.len() != program.inputs.len() {
        return Err(EvalError::InputArity {
            expected: program.inputs.len(),
            got: inputs.len(),
        });
    }
    for (value, expected) in parameters.iter().zip(&program.parameters) {
        ensure_type(value, *expected)?;
    }
    for (value, expected) in inputs.iter().zip(&program.inputs) {
        ensure_type(value, *expected)?;
    }

    let mut values = Vec::with_capacity(program.nodes.len());
    for node in &program.nodes {
        let value = match node {
            Node::Literal(value) => value.clone(),
            Node::Parameter { index, .. } => {
                parameters[usize::try_from(*index).expect("validated parameter index")].clone()
            }
            Node::Input { index, .. } => {
                inputs[usize::try_from(*index).expect("validated input index")].clone()
            }
            Node::ReadField { record, field } => {
                let record = as_record(&values[usize::try_from(*record).expect("validated node index")]);
                let value = record.field(field.index).expect("validated record field index").clone();
                ensure_type(&value, field.ty)?;
                value
            }
            Node::AddI64 { left, right } => {
                let left = as_i64(&values[usize::try_from(*left).expect("validated node index")]);
                let right = as_i64(&values[usize::try_from(*right).expect("validated node index")]);
                Value::I64(left.checked_add(right).ok_or(EvalError::IntegerOverflow)?)
            }
            Node::Equal { left, right } => Value::Bool(
                values[usize::try_from(*left).expect("validated node index")]
                    == values[usize::try_from(*right).expect("validated node index")],
            ),
        };
        values.push(value);
    }
    Ok(values[usize::try_from(program.output).expect("validated output index")].clone())
}

fn ensure_type(value: &Value, expected: Type) -> Result<(), EvalError> {
    let found = value.ty();
    if found != expected {
        return Err(EvalError::TypeMismatch { expected, found });
    }
    Ok(())
}

fn as_i64(value: &Value) -> i64 {
    match value {
        Value::I64(value) => *value,
        Value::Null
        | Value::Bool(_)
        | Value::Text(_)
        | Value::Bytes(_)
        | Value::File(_)
        | Value::Vector(_)
        | Value::Record(_) => unreachable!("validated integer operand"),
    }
}

fn as_record(value: &Value) -> &RecordValue {
    match value {
        Value::Record(value) => value,
        Value::Null
        | Value::Bool(_)
        | Value::I64(_)
        | Value::Text(_)
        | Value::Bytes(_)
        | Value::File(_)
        | Value::Vector(_) => unreachable!("validated record operand"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(Value::Vector(vector).ty(), Type::Vector {
            dimension: 3,
            metric: VectorMetric::Cosine,
        });
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

        assert_eq!(program.evaluate(&[], &[Value::Record(record)]), Ok(Value::I64(42)));
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
                    Node::Input { index: 0, ty: record_type },
                    Node::ReadField { record: 0, field },
                ],
                output: 1,
                output_type: Type::I64,
            },
            &[layout],
        )
        .expect("layout-bound program validates");
        let row = RecordValue::new(7, vec![Value::I64(42)]).unwrap();
        assert_eq!(program.evaluate(&[], &[Value::Record(row)]), Ok(Value::I64(42)));
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
                    Node::Input { index: 0, ty: record_type },
                    Node::ReadField { record: 0, field },
                ],
                output: 1,
                output_type: Type::Text,
            },
            &[layout],
        )
        .expect_err("layout mismatch must be rejected");
        assert!(matches!(error, ValidationError::LayoutFieldMismatch { field_id: 101, .. }));
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
}
