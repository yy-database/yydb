//! Language-neutral scalar execution model for YYDB and YYDS.
//!
//! VOS, SQL, and other frontends lower into this model. This crate deliberately
//! has no dependency on a frontend, storage engine, parser, or execution host.

#![deny(missing_docs)]

/// A node identifier in an execution Program.
pub type NodeId = u32;

/// Runtime types supported by the first scalar execution slice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Type {
    /// Boolean value.
    Bool,
    /// Signed 64-bit integer.
    I64,
    /// UTF-8 text value.
    Text,
}

/// Runtime value supported by the first scalar execution slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// Boolean value.
    Bool(bool),
    /// Signed 64-bit integer.
    I64(i64),
    /// UTF-8 text value.
    Text(String),
}

impl Value {
    /// Returns the execution type of this value.
    pub fn ty(&self) -> Type {
        match self {
            Self::Bool(_) => Type::Bool,
            Self::I64(_) => Type::I64,
            Self::Text(_) => Type::Text,
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

/// A validated program ready for evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedProgram(Program);

impl ValidatedProgram {
    /// Validates and accepts a program.
    pub fn validate(program: Program) -> Result<Self, ValidationError> {
        validate_program(&program)?;
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

fn node_type(program: &Program, node: NodeId) -> Result<Type, ValidationError> {
    let current =
        usize::try_from(node).map_err(|_| ValidationError::OutputOutOfBounds { output: node })?;
    let Some(value) = program.nodes.get(current) else {
        return Err(ValidationError::OutputOutOfBounds { output: node });
    };
    match value {
        Node::Literal(value) => Ok(value.ty()),
        Node::Parameter { index, ty } => {
            if usize::try_from(*index)
                .ok()
                .is_none_or(|index| index >= program.parameters.len())
            {
                return Err(ValidationError::ParameterOutOfBounds {
                    node,
                    index: *index,
                });
            }
            Ok(*ty)
        }
        Node::Input { index, ty } => {
            if usize::try_from(*index)
                .ok()
                .is_none_or(|index| index >= program.inputs.len())
            {
                return Err(ValidationError::InputOutOfBounds {
                    node,
                    index: *index,
                });
            }
            Ok(*ty)
        }
        Node::AddI64 { left, right } => {
            operand_type(program, node, *left, Type::I64)?;
            operand_type(program, node, *right, Type::I64)?;
            Ok(Type::I64)
        }
        Node::Equal { left, right } => {
            let left_ty = checked_reference_type(program, node, *left)?;
            let right_ty = checked_reference_type(program, node, *right)?;
            if left_ty != right_ty {
                return Err(ValidationError::TypeMismatch {
                    node,
                    expected: left_ty,
                    found: right_ty,
                });
            }
            Ok(Type::Bool)
        }
    }
}

fn checked_reference_type(
    program: &Program,
    node: NodeId,
    reference: NodeId,
) -> Result<Type, ValidationError> {
    if reference >= node {
        return Err(ValidationError::ReferenceOutOfOrder { node, reference });
    }
    node_type(program, reference)
}

fn operand_type(
    program: &Program,
    node: NodeId,
    reference: NodeId,
    expected: Type,
) -> Result<Type, ValidationError> {
    let found = checked_reference_type(program, node, reference)?;
    if found != expected {
        return Err(ValidationError::TypeMismatch {
            node,
            expected,
            found,
        });
    }
    Ok(found)
}

fn validate_program(program: &Program) -> Result<(), ValidationError> {
    if program.nodes.is_empty() {
        return Err(ValidationError::EmptyProgram);
    }
    if usize::try_from(program.output)
        .ok()
        .is_none_or(|output| output >= program.nodes.len())
    {
        return Err(ValidationError::OutputOutOfBounds {
            output: program.output,
        });
    }
    let found = node_type(program, program.output)?;
    if found != program.output_type {
        return Err(ValidationError::OutputTypeMismatch {
            expected: program.output_type,
            found,
        });
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
        Value::Bool(_) | Value::Text(_) => unreachable!("validated integer operand"),
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
}
