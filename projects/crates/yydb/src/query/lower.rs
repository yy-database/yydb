//! Lower VOS expression pipelines into Phase 1 query ops.

use vos::ast::expr::{BinaryOp, Expr, ProjItem, Stmt};
use vos::ast::{Literal, Program};

use yydb_types::{Error, Result};

use super::ops::{CmpOp, LiteralKind, Pred, ProjectField, QueryOp, SortKey};

fn query_error(message: impl Into<String>) -> Error {
    Error::Schema {
        message: message.into(),
    }
}

/// Lower a VOS program into an ordered physical op list.
pub fn lower_program(program: &Program) -> Result<Vec<QueryOp>> {
    let mut bindings: Vec<(String, Expr)> = Vec::new();
    for stmt in &program.statements {
        match stmt {
            Stmt::Let(let_) => bindings.push((let_.name.clone(), let_.value.clone())),
            Stmt::Expr(expr) => return lower_exec_expr(expr, &bindings),
            _ => {
                return Err(query_error("unsupported statement in Phase 1 query program"));
            }
        }
    }
    let Some(result) = &program.result else {
        return Err(query_error("query program has no result expression"));
    };
    lower_exec_expr(result, &bindings)
}

fn lower_exec_expr(expr: &Expr, bindings: &[(String, Expr)]) -> Result<Vec<QueryOp>> {
    let expr = expand_bindings(expr, bindings)?;
    let pipeline = split_collect(&expr)?;
    let mut ops = lower_pipeline(&pipeline)?;
    ops.push(QueryOp::Collect);
    Ok(ops)
}

fn expand_bindings(expr: &Expr, bindings: &[(String, Expr)]) -> Result<Expr> {
    match expr {
        Expr::Name { name, .. } => {
            if let Some((_, value)) = bindings.iter().rev().find(|(n, _)| n == name) {
                expand_bindings(value, bindings)
            } else {
                Ok(expr.clone())
            }
        }
        Expr::Member {
            object,
            name,
            sep,
            span,
        } => Ok(Expr::Member {
            object: Box::new(expand_bindings(object, bindings)?),
            name: name.clone(),
            sep: *sep,
            span: *span,
        }),
        Expr::Call { callee, args, span } => Ok(Expr::Call {
            callee: Box::new(expand_bindings(callee, bindings)?),
            args: args
                .iter()
                .map(|arg| expand_bindings(arg, bindings))
                .collect::<Result<Vec<_>>>()?,
            span: *span,
        }),
        other => Ok(other.clone()),
    }
}

fn split_collect(expr: &Expr) -> Result<Expr> {
    match expr {
        Expr::Call { callee, args, span: _ } => match callee.as_ref() {
            Expr::Member { object, name, .. } if name == "collect" => {
                if !args.is_empty() {
                    return Err(query_error("`.collect()` takes no arguments"));
                }
                Ok(object.as_ref().clone())
            }
            _ => Err(query_error(
                "Phase 1 only executes pipelines ending in `.collect()`",
            )),
        },
        _ => Err(query_error(
            "Phase 1 only executes pipelines ending in `.collect()`",
        )),
    }
}

fn lower_pipeline(expr: &Expr) -> Result<Vec<QueryOp>> {
    let mut methods: Vec<(&str, &[Expr])> = Vec::new();
    let mut cur = expr;
    loop {
        match cur {
            Expr::Call { callee, args, .. } => match callee.as_ref() {
                Expr::Member { object, name, .. } => {
                    methods.push((name.as_str(), args.as_slice()));
                    cur = object.as_ref();
                }
                _ => return Err(query_error("unsupported call shape in query pipeline")),
            },
            Expr::Member { object, name, .. } if name == "all" => {
                methods.push(("all", &[]));
                cur = object.as_ref();
            }
            Expr::Name { name, .. } => {
                let mut ops = vec![QueryOp::Scan {
                    table: name.clone(),
                }];
                for (method, args) in methods.into_iter().rev() {
                    push_method(&mut ops, method, args)?;
                }
                return Ok(ops);
            }
            _ => return Err(query_error("query pipeline must start from a table name")),
        }
    }
}

fn push_method(ops: &mut Vec<QueryOp>, method: &str, args: &[Expr]) -> Result<()> {
    match method {
        "all" => Ok(()),
        "filter" | "where" => {
            let pred_expr = args
                .first()
                .ok_or_else(|| query_error(format!("`.{method}` requires a predicate")))?;
            ops.push(QueryOp::Filter {
                predicate: lower_predicate(pred_expr)?,
            });
            Ok(())
        }
        "map" => {
            let proj = args
                .first()
                .ok_or_else(|| query_error("`.map` requires a projection"))?;
            ops.push(QueryOp::Project {
                fields: lower_projection(proj)?,
            });
            Ok(())
        }
        "sort_by" => {
            let key = args
                .first()
                .ok_or_else(|| query_error("`.sort_by` requires a key lambda"))?;
            ops.push(QueryOp::Sort {
                keys: vec![SortKey {
                    field: lower_field_lambda(key)?,
                    ascending: true,
                }],
            });
            Ok(())
        }
        "sort_by_desc" => {
            let key = args
                .first()
                .ok_or_else(|| query_error("`.sort_by_desc` requires a key lambda"))?;
            ops.push(QueryOp::Sort {
                keys: vec![SortKey {
                    field: lower_field_lambda(key)?,
                    ascending: false,
                }],
            });
            Ok(())
        }
        "skip" => {
            let count = literal_u64(args.first(), method)?;
            ops.push(QueryOp::Skip { count });
            Ok(())
        }
        "take" => {
            let count = literal_u64(args.first(), method)?;
            ops.push(QueryOp::Take { count });
            Ok(())
        }
        "insert" | "update" | "delete" => Err(query_error(format!(
            "write method `.{method}` is not supported by the Phase 1 query executor"
        ))),
        other => Err(query_error(format!("unsupported pipeline method `.{other}`"))),
    }
}

fn lower_predicate(expr: &Expr) -> Result<Pred> {
    let body = match expr {
        Expr::Lambda(lambda) => lambda.body.as_ref(),
        other => other,
    };
    lower_pred_body(body)
}

fn lower_pred_body(expr: &Expr) -> Result<Pred> {
    match expr {
        Expr::Literal(Literal::Bool(true)) => Ok(Pred::True),
        Expr::Literal(Literal::Bool(false)) => Ok(Pred::False),
        Expr::Binary {
            op: BinaryOp::And,
            left,
            right,
            ..
        } => Ok(Pred::And(
            Box::new(lower_pred_body(left)?),
            Box::new(lower_pred_body(right)?),
        )),
        Expr::Binary {
            op: BinaryOp::Or,
            left,
            right,
            ..
        } => Ok(Pred::Or(
            Box::new(lower_pred_body(left)?),
            Box::new(lower_pred_body(right)?),
        )),
        Expr::Binary {
            op,
            left,
            right,
            ..
        } => {
            let field = match left.as_ref() {
                Expr::Member { name, .. } | Expr::Name { name, .. } => name.clone(),
                _ => {
                    return Err(query_error(
                        "Phase 1 filters must compare a field access on the left",
                    ));
                }
            };
            let (literal, kind) = match right.as_ref() {
                Expr::Literal(Literal::Bool(b)) => (b.to_string(), LiteralKind::Bool),
                Expr::Literal(Literal::Int(t)) => (t.clone(), LiteralKind::Int),
                Expr::Literal(Literal::String(s)) => (s.clone(), LiteralKind::Str),
                Expr::Literal(Literal::Null) => ("null".into(), LiteralKind::Null),
                Expr::Literal(Literal::Float(t)) => (t.clone(), LiteralKind::Str),
                Expr::Literal(Literal::Ident(t)) => (t.clone(), LiteralKind::Str),
                _ => {
                    return Err(query_error(
                        "Phase 1 filters require a literal right-hand side",
                    ));
                }
            };
            if matches!(op, BinaryOp::Eq) && kind == LiteralKind::Bool {
                let value = literal == "true";
                return Ok(Pred::FieldBool { field, value });
            }
            let cmp = match op {
                BinaryOp::Eq => CmpOp::Eq,
                BinaryOp::Ne => CmpOp::Ne,
                BinaryOp::Lt => CmpOp::Lt,
                BinaryOp::Le => CmpOp::Le,
                BinaryOp::Gt => CmpOp::Gt,
                BinaryOp::Ge => CmpOp::Ge,
                _ => return Err(query_error("unsupported comparison in filter")),
            };
            Ok(Pred::FieldCmp {
                field,
                op: cmp,
                literal,
                kind,
            })
        }
        Expr::Member { name, .. } | Expr::Name { name, .. } => Ok(Pred::FieldBool {
            field: name.clone(),
            value: true,
        }),
        _ => Err(query_error("unsupported predicate shape")),
    }
}

fn lower_field_lambda(expr: &Expr) -> Result<String> {
    let body = match expr {
        Expr::Lambda(lambda) => lambda.body.as_ref(),
        other => other,
    };
    match body {
        Expr::Member { name, .. } => Ok(name.clone()),
        _ => Err(query_error("Phase 1 sort key must be a field access lambda")),
    }
}

fn lower_projection(expr: &Expr) -> Result<Vec<ProjectField>> {
    let body = match expr {
        Expr::Lambda(lambda) => lambda.body.as_ref(),
        other => other,
    };
    match body {
        Expr::StructProj { items, .. } => {
            let mut fields = Vec::new();
            for item in items {
                match item {
                    ProjItem::Star { .. } => {
                        return Err(query_error(
                            "Phase 1 projection does not expand `*` yet",
                        ));
                    }
                    ProjItem::Field(init) => {
                        let from = match &init.value {
                            None => None,
                            Some(Expr::Name { name, .. }) | Some(Expr::Member { name, .. }) => {
                                if name == &init.name {
                                    None
                                } else {
                                    Some(name.clone())
                                }
                            }
                            Some(_) => {
                                return Err(query_error(
                                    "Phase 1 projection values must be field refs",
                                ));
                            }
                        };
                        fields.push(ProjectField {
                            name: init.name.clone(),
                            from,
                        });
                    }
                    _ => return Err(query_error("unsupported projection item")),
                }
            }
            Ok(fields)
        }
        _ => Err(query_error("Phase 1 `.map` expects `x => x.{ ... }`")),
    }
}

fn literal_u64(expr: Option<&Expr>, method: &str) -> Result<u64> {
    let Some(expr) = expr else {
        return Err(query_error(format!("`.{method}` requires a count argument")));
    };
    match expr {
        Expr::Literal(Literal::Int(t)) => t.parse::<u64>().map_err(|_| {
            query_error(format!("invalid `{method}` count"))
        }),
        _ => Err(query_error(format!(
            "`.{method}` count must be an integer literal"
        ))),
    }
}
