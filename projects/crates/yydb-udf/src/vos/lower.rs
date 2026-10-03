//! Lower VOS `micro` scalar sources into validated local execution programs.

use std::collections::HashMap;

use vos::ast::{BuiltinType, Expr, FnDecl, FnKind, Literal, TypeExpr};
use yydb_execution::{Node, Program, Type, Udf, UdfEffect, UdfPlacement, Value};

use crate::capability::{Placement, UdfPolicy};
use crate::contract::{Signature, UdfType};
use crate::error::{Result, UdfError};
use crate::identity::UdfIdentity;
use crate::vos::LoweredUdf;

/// Lowers one VOS `macro` declaration into a [`LoweredUdf`].
///
/// Until the VOS schema parser accepts durable `macro` items, this entry point
/// rewrites the `macro` keyword to the supported `micro` program parser surface.
pub fn lower_vos_macro(source: &str, version: u32) -> Result<LoweredUdf> {
    let trimmed = source.trim();
    let micro_source = if let Some(rest) = trimmed.strip_prefix("macro") {
        format!("micro{rest}")
    } else {
        return Err(UdfError::InvalidDefinition);
    };
    lower_micro_scalar(&micro_source, version)
}

/// Lowers one VOS `micro` declaration into a [`LoweredUdf`].
pub fn lower_micro_scalar(source: &str, version: u32) -> Result<LoweredUdf> {
    let parsed =
        vos::parser::parse_program(source).map_err(|diagnostics| UdfError::ExecutionFailed {
            message: diagnostics
                .errors
                .into_iter()
                .map(|error| error.message)
                .collect::<Vec<_>>()
                .join(" | "),
        })?;
    if !parsed.statements.is_empty() || parsed.result.is_some() || parsed.micros.len() != 1 {
        return Err(UdfError::InvalidDefinition);
    }
    let declaration = &parsed.micros[0];
    if declaration.kind != FnKind::Micro {
        return Err(UdfError::InvalidDefinition);
    }
    lower_declaration(declaration, version, source)
}

fn lower_declaration(declaration: &FnDecl, version: u32, source: &str) -> Result<LoweredUdf> {
    let mut inputs = Vec::with_capacity(declaration.params.len());
    let mut names = HashMap::with_capacity(declaration.params.len());
    let mut arg_types = Vec::with_capacity(declaration.params.len());
    for (index, parameter) in declaration.params.iter().enumerate() {
        let ty = lower_type(&parameter.ty)?;
        let udf_ty = execution_type_to_udf_type(ty)?;
        if names
            .insert(parameter.name.clone(), (index as u32, ty))
            .is_some()
        {
            return Err(UdfError::InvalidDefinition);
        }
        inputs.push(ty);
        arg_types.push(udf_ty);
    }

    if !declaration.body.statements.is_empty() || !declaration.body.micros.is_empty() {
        return Err(UdfError::InvalidDefinition);
    }
    let expression = declaration
        .body
        .result
        .as_ref()
        .ok_or(UdfError::InvalidDefinition)?;
    let mut nodes = Vec::new();
    let output_type = lower_expr(expression, &names, &mut nodes)?;
    let return_ty = execution_type_to_udf_type(output_type)?;
    if let Some(declared) = &declaration.return_ty {
        let declared_ty = lower_type(declared)?;
        if declared_ty != output_type {
            return Err(UdfError::SignatureMismatch);
        }
    }

    let output = u32::try_from(nodes.len() - 1).expect("lowered VOS UDF has at least one node");
    let program = Udf {
        id: declaration.name.clone(),
        version,
        deterministic: true,
        effect: UdfEffect::Read,
        placement: UdfPlacement::Local,
        program: Program {
            parameters: Vec::new(),
            inputs,
            nodes,
            output,
            output_type,
        },
    }
    .validate()
    .map_err(|error| UdfError::ExecutionFailed {
        message: format!("lowered VOS UDF rejected by local execution model: {error:?}"),
    })?;

    let identity = UdfIdentity::new("", declaration.name.clone(), version)?;
    Ok(LoweredUdf {
        identity,
        signature: Signature::new(arg_types, return_ty),
        policy: UdfPolicy::pure_embedded(),
        placement: Placement::Embedded,
        program,
        source_fingerprint: LoweredUdf::fingerprint_source(source),
    })
}

fn lower_expr(
    expression: &Expr,
    parameters: &HashMap<String, (u32, Type)>,
    nodes: &mut Vec<Node>,
) -> Result<Type> {
    let ty = match expression {
        Expr::Name {
            name: parameter, ..
        } => {
            let (index, ty) = parameters
                .get(parameter)
                .ok_or(UdfError::InvalidDefinition)?;
            nodes.push(Node::Input {
                index: *index,
                ty: *ty,
            });
            *ty
        }
        Expr::Literal(literal) => {
            let (value, ty) = lower_literal(literal)?;
            nodes.push(Node::Literal(value));
            ty
        }
        Expr::Binary {
            op, left, right, ..
        } => {
            let left_ty = lower_expr(left, parameters, nodes)?;
            let left_node = u32::try_from(nodes.len() - 1).expect("left expression has a node");
            let right_ty = lower_expr(right, parameters, nodes)?;
            let right_node = u32::try_from(nodes.len() - 1).expect("right expression has a node");
            match op {
                vos::ast::BinaryOp::Add if left_ty == Type::I64 && right_ty == Type::I64 => {
                    nodes.push(Node::AddI64 {
                        left: left_node,
                        right: right_node,
                    });
                    Type::I64
                }
                vos::ast::BinaryOp::Eq if left_ty == right_ty => {
                    nodes.push(Node::Equal {
                        left: left_node,
                        right: right_node,
                    });
                    Type::Bool
                }
                _ => return Err(UdfError::UnsupportedEffect),
            }
        }
        _ => return Err(UdfError::UnsupportedEffect),
    };
    Ok(ty)
}

fn lower_literal(literal: &Literal) -> Result<(Value, Type)> {
    match literal {
        Literal::Bool(value) => Ok((Value::Bool(*value), Type::Bool)),
        Literal::Int(value) => value
            .parse::<i64>()
            .map(|value| (Value::I64(value), Type::I64))
            .map_err(|_| UdfError::InvalidDefinition),
        Literal::String(value) => Ok((Value::Text(value.clone()), Type::Text)),
        Literal::Null => Ok((Value::Null, Type::Null)),
        Literal::Float(_) | Literal::Ident(_) | _ => Err(UdfError::UnsupportedType),
    }
}

fn lower_type(ty: &TypeExpr) -> Result<Type> {
    match ty {
        TypeExpr::Builtin(BuiltinType::I64) => Ok(Type::I64),
        TypeExpr::Builtin(BuiltinType::Bool) => Ok(Type::Bool),
        TypeExpr::Builtin(BuiltinType::Utf8) => Ok(Type::Text),
        TypeExpr::Builtin(BuiltinType::Bytes) => Ok(Type::Bytes),
        _ => Err(UdfError::UnsupportedType),
    }
}

fn execution_type_to_udf_type(ty: Type) -> Result<UdfType> {
    UdfType::from_execution(ty)
}
