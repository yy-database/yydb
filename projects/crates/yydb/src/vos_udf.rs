use std::collections::HashMap;

use vos::ast::{BuiltinType, Expr, FnDecl, FnKind, Literal, TypeExpr};
use yydb_execution::{Node, Program, Type, Udf, UdfEffect, UdfPlacement, Value};
use yydb_types::{Error, Result};

pub(crate) fn lower(source: &str, version: u32) -> Result<yydb_execution::ValidatedUdf> {
    let parsed = vos::parser::parse_program(source).map_err(|diagnostics| Error::Udf {
        name: "<vos>".into(),
        message: diagnostics
            .errors
            .into_iter()
            .map(|error| error.message)
            .collect::<Vec<_>>()
            .join(" | "),
    })?;
    if parsed.statements.len() != 0 || parsed.result.is_some() || parsed.micros.len() != 1 {
        return Err(Error::Udf {
            name: "<vos>".into(),
            message: "VOS UDF source must contain exactly one micro declaration".into(),
        });
    }
    let declaration = &parsed.micros[0];
    if declaration.kind != FnKind::Micro {
        return Err(Error::Udf {
            name: declaration.name.clone(),
            message: "only VOS micro declarations can bind as local scalar UDFs".into(),
        });
    }
    lower_declaration(declaration, version)
}

fn lower_declaration(declaration: &FnDecl, version: u32) -> Result<yydb_execution::ValidatedUdf> {
    let mut inputs = Vec::with_capacity(declaration.params.len());
    let mut names = HashMap::with_capacity(declaration.params.len());
    for (index, parameter) in declaration.params.iter().enumerate() {
        let ty = lower_type(&parameter.ty, &declaration.name)?;
        if names
            .insert(parameter.name.clone(), (index as u32, ty))
            .is_some()
        {
            return udf_error(&declaration.name, "duplicate VOS UDF parameter");
        }
        inputs.push(ty);
    }

    if !declaration.body.statements.is_empty() || declaration.body.micros.len() != 0 {
        return udf_error(
            &declaration.name,
            "local scalar VOS UDF bodies currently require one expression",
        );
    }
    let expression = declaration.body.result.as_ref().ok_or_else(|| Error::Udf {
        name: declaration.name.clone(),
        message: "VOS UDF body must return an expression".into(),
    })?;
    let mut nodes = Vec::new();
    let output_type = lower_expr(expression, &names, &mut nodes, &declaration.name)?;
    if let Some(return_ty) = &declaration.return_ty {
        let declared = lower_type(return_ty, &declaration.name)?;
        if declared != output_type {
            return udf_error(
                &declaration.name,
                "VOS UDF return type does not match its body",
            );
        }
    }

    let output = u32::try_from(nodes.len() - 1).expect("lowered VOS UDF has at least one node");
    Udf {
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
    .map_err(|error| Error::Udf {
        name: declaration.name.clone(),
        message: format!("lowered VOS UDF rejected by local execution model: {error:?}"),
    })
}

fn lower_expr(
    expression: &Expr,
    parameters: &HashMap<String, (u32, Type)>,
    nodes: &mut Vec<Node>,
    name: &str,
) -> Result<Type> {
    let ty = match expression {
        Expr::Name {
            name: parameter, ..
        } => {
            let (index, ty) = parameters.get(parameter).ok_or_else(|| Error::Udf {
                name: name.into(),
                message: format!("unknown VOS UDF parameter `{parameter}`"),
            })?;
            nodes.push(Node::Input {
                index: *index,
                ty: *ty,
            });
            *ty
        }
        Expr::Literal(literal) => {
            let (value, ty) = lower_literal(literal, name)?;
            nodes.push(Node::Literal(value));
            ty
        }
        Expr::Binary {
            op, left, right, ..
        } => {
            let left_ty = lower_expr(left, parameters, nodes, name)?;
            let left_node = u32::try_from(nodes.len() - 1).expect("left expression has a node");
            let right_ty = lower_expr(right, parameters, nodes, name)?;
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
                _ => {
                    return udf_error(
                        name,
                        "VOS UDF expression uses an operator outside the local scalar subset",
                    )
                }
            }
        }
        _ => {
            return udf_error(
                name,
                "VOS UDF expression is outside the local scalar subset",
            )
        }
    };
    Ok(ty)
}

fn lower_literal(literal: &Literal, name: &str) -> Result<(Value, Type)> {
    match literal {
        Literal::Bool(value) => Ok((Value::Bool(*value), Type::Bool)),
        Literal::Int(value) => value
            .parse::<i64>()
            .map(|value| (Value::I64(value), Type::I64))
            .map_err(|_| Error::Udf {
                name: name.into(),
                message: "VOS UDF integer literal does not fit i64".into(),
            }),
        Literal::String(value) => Ok((Value::Text(value.clone()), Type::Text)),
        Literal::Null => Ok((Value::Null, Type::Null)),
        Literal::Float(_) | Literal::Ident(_) | _ => {
            udf_error(name, "VOS UDF literal is outside the local scalar subset")
        }
    }
}

fn lower_type(ty: &TypeExpr, name: &str) -> Result<Type> {
    match ty {
        TypeExpr::Builtin(BuiltinType::I64) => Ok(Type::I64),
        TypeExpr::Builtin(BuiltinType::Bool) => Ok(Type::Bool),
        TypeExpr::Builtin(BuiltinType::Utf8) => Ok(Type::Text),
        TypeExpr::Builtin(BuiltinType::Bytes) => Ok(Type::Bytes),
        _ => udf_error(name, "VOS UDF type is outside the local scalar subset"),
    }
}

fn udf_error<T>(name: &str, message: &str) -> Result<T> {
    Err(Error::Udf {
        name: name.into(),
        message: message.into(),
    })
}
