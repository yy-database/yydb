//! Lower VOS expression pipelines into Phase 1 query ops.

use vos::syntax::{
    VosBinaryOp, VosExpr, VosFieldInit, VosLiteral, VosProgram, VosProjItem, VosStmt,
};

use yydb_types::{Error, Result};

use super::ops::{CmpOp, LiteralKind, Pred, ProjectExpr, ProjectField, QueryOp, SortKey};

fn query_error(message: impl Into<String>) -> Error {
    Error::Schema {
        message: message.into(),
    }
}

/// Lower a VOS program into an ordered physical op list.
pub fn lower_program(program: &VosProgram) -> Result<Vec<QueryOp>> {
    let mut bindings: Vec<(String, VosExpr)> = Vec::new();
    for stmt in &program.statements {
        match stmt {
            VosStmt::Let(let_) => bindings.push((let_.name.clone(), let_.value.clone())),
            VosStmt::Expr(expr) => return lower_exec_expr(expr, &bindings),
        }
    }
    let Some(result) = &program.result else {
        return Err(query_error("query program has no result expression"));
    };
    lower_exec_expr(result, &bindings)
}

fn lower_exec_expr(expr: &VosExpr, bindings: &[(String, VosExpr)]) -> Result<Vec<QueryOp>> {
    let expr = expand_bindings(expr, bindings)?;
    let pipeline = split_collect(&expr)?;
    let mut ops = lower_pipeline(&pipeline)?;
    ops.push(QueryOp::Collect);
    Ok(ops)
}

fn expand_bindings(expr: &VosExpr, bindings: &[(String, VosExpr)]) -> Result<VosExpr> {
    match expr {
        VosExpr::Name { name, .. } => {
            if let Some((_, value)) = bindings.iter().rev().find(|(n, _)| n == name) {
                expand_bindings(value, bindings)
            } else {
                Ok(expr.clone())
            }
        }
        VosExpr::Member {
            object,
            name,
            name_span,
            sep,
            span,
        } => Ok(VosExpr::Member {
            object: Box::new(expand_bindings(object, bindings)?),
            name: name.clone(),
            name_span: name_span.clone(),
            sep: *sep,
            span: span.clone(),
        }),
        VosExpr::Call { callee, args, span } => Ok(VosExpr::Call {
            callee: Box::new(expand_bindings(callee, bindings)?),
            args: args
                .iter()
                .map(|arg| expand_bindings(arg, bindings))
                .collect::<Result<Vec<_>>>()?,
            span: span.clone(),
        }),
        other => Ok(other.clone()),
    }
}

fn split_collect(expr: &VosExpr) -> Result<VosExpr> {
    match expr {
        VosExpr::Call {
            callee,
            args,
            span: _,
        } => match callee.as_ref() {
            VosExpr::Member { object, name, .. } if name == "collect" => {
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

fn lower_pipeline(expr: &VosExpr) -> Result<Vec<QueryOp>> {
    let mut methods: Vec<(&str, &[VosExpr])> = Vec::new();
    let mut cur = expr;
    loop {
        match cur {
            VosExpr::Call { callee, args, .. } => match callee.as_ref() {
                VosExpr::Member { object, name, .. } => {
                    methods.push((name.as_str(), args.as_slice()));
                    cur = object.as_ref();
                }
                _ => return Err(query_error("unsupported call shape in query pipeline")),
            },
            VosExpr::Member { object, name, .. } if name == "all" => {
                methods.push(("all", &[]));
                cur = object.as_ref();
            }
            VosExpr::Name { name, .. } => {
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

fn push_method(ops: &mut Vec<QueryOp>, method: &str, args: &[VosExpr]) -> Result<()> {
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
        other => Err(query_error(format!(
            "unsupported pipeline method `.{other}`"
        ))),
    }
}

fn lower_predicate(expr: &VosExpr) -> Result<Pred> {
    match expr {
        VosExpr::Lambda(lambda) => {
            let param = lambda.params.first().map(String::as_str);
            lower_pred_body(lambda.body.as_ref(), param)
        }
        other => lower_pred_body(other, None),
    }
}

fn lower_pred_body(expr: &VosExpr, param: Option<&str>) -> Result<Pred> {
    match expr {
        VosExpr::Literal(VosLiteral::Bool(true)) => Ok(Pred::True),
        VosExpr::Literal(VosLiteral::Bool(false)) => Ok(Pred::False),
        VosExpr::Binary {
            op: VosBinaryOp::And,
            left,
            right,
            ..
        } => Ok(Pred::And(
            Box::new(lower_pred_body(left, param)?),
            Box::new(lower_pred_body(right, param)?),
        )),
        VosExpr::Binary {
            op: VosBinaryOp::Or,
            left,
            right,
            ..
        } => Ok(Pred::Or(
            Box::new(lower_pred_body(left, param)?),
            Box::new(lower_pred_body(right, param)?),
        )),
        VosExpr::Binary {
            op, left, right, ..
        } => {
            let path = lower_field_path(left.as_ref(), param)?;
            if path.is_empty() {
                return Err(query_error(
                    "Phase 1 filters must compare a field access on the left",
                ));
            }
            let (literal, kind) = match right.as_ref() {
                VosExpr::Literal(VosLiteral::Bool(b)) => (b.to_string(), LiteralKind::Bool),
                VosExpr::Literal(VosLiteral::Int(t)) => (t.clone(), LiteralKind::Int),
                VosExpr::Literal(VosLiteral::String(s)) => (s.clone(), LiteralKind::Str),
                VosExpr::Literal(VosLiteral::Null) => ("null".into(), LiteralKind::Null),
                VosExpr::Literal(VosLiteral::Float(t)) => (t.clone(), LiteralKind::Str),
                _ => {
                    return Err(query_error(
                        "Phase 1 filters require a literal right-hand side",
                    ));
                }
            };
            if matches!(op, VosBinaryOp::Eq) && kind == LiteralKind::Bool {
                let value = literal == "true";
                return Ok(Pred::FieldBool { path, value });
            }
            let cmp = match op {
                VosBinaryOp::Eq => CmpOp::Eq,
                VosBinaryOp::Ne => CmpOp::Ne,
                VosBinaryOp::Lt => CmpOp::Lt,
                VosBinaryOp::Le => CmpOp::Le,
                VosBinaryOp::Gt => CmpOp::Gt,
                VosBinaryOp::Ge => CmpOp::Ge,
                _ => return Err(query_error("unsupported comparison in filter")),
            };
            Ok(Pred::FieldCmp {
                path,
                op: cmp,
                literal,
                kind,
            })
        }
        _ => {
            let path = lower_field_path(expr, param)?;
            if path.is_empty() {
                return Err(query_error("unsupported predicate shape"));
            }
            Ok(Pred::FieldBool { path, value: true })
        }
    }
}

fn lower_field_path(expr: &VosExpr, param: Option<&str>) -> Result<Vec<String>> {
    match expr {
        VosExpr::Member { object, name, .. } => {
            let mut path = lower_field_path(object.as_ref(), param)?;
            path.push(name.clone());
            Ok(path)
        }
        VosExpr::Name { name, .. } if Some(name.as_str()) == param => Ok(Vec::new()),
        _ => Err(query_error(
            "Phase 1 filters must use field paths rooted at the lambda parameter",
        )),
    }
}

fn lower_field_lambda(expr: &VosExpr) -> Result<String> {
    let body = match expr {
        VosExpr::Lambda(lambda) => lambda.body.as_ref(),
        other => other,
    };
    match body {
        VosExpr::Member { name, .. } => Ok(name.clone()),
        _ => Err(query_error(
            "Phase 1 sort key must be a field access lambda",
        )),
    }
}

fn lower_projection(expr: &VosExpr) -> Result<Vec<ProjectField>> {
    let (body, param) = match expr {
        VosExpr::Lambda(lambda) => (
            lambda.body.as_ref(),
            lambda.params.first().map(String::as_str),
        ),
        other => (other, None),
    };
    match body {
        VosExpr::StructProj { items, .. } => {
            let mut fields = Vec::new();
            for item in items {
                match item {
                    VosProjItem::Star { .. } => {
                        return Err(query_error("Phase 1 projection does not expand `*` yet"));
                    }
                    VosProjItem::Field(init) => {
                        fields.push(lower_project_field(init, param)?);
                    }
                }
            }
            Ok(fields)
        }
        _ => Err(query_error("Phase 1 `.map` expects `x => x.{ ... }`")),
    }
}

fn lower_project_field(init: &VosFieldInit, param: Option<&str>) -> Result<ProjectField> {
    let expr = match &init.value {
        None => ProjectExpr::Scalar {
            path: vec![init.name.clone()],
        },
        Some(value) => lower_project_expr(value, param)?,
    };
    Ok(ProjectField {
        name: init.name.clone(),
        expr,
    })
}

fn lower_project_expr(expr: &VosExpr, param: Option<&str>) -> Result<ProjectExpr> {
    match expr {
        VosExpr::StructProj {
            receiver, items, ..
        } => {
            let path = lower_field_path(receiver.as_ref(), param)?;
            let mut fields = Vec::new();
            for item in items {
                match item {
                    VosProjItem::Star { .. } => {
                        return Err(query_error(
                            "Phase 1 nested projection does not expand `*` yet",
                        ));
                    }
                    VosProjItem::Field(init) => fields.push(lower_project_field(init, param)?),
                }
            }
            Ok(ProjectExpr::Nested { path, fields })
        }
        other => {
            let path = lower_field_path(other, param)?;
            if path.is_empty() {
                return Err(query_error(
                    "Phase 1 projection values must be field paths rooted at the lambda parameter",
                ));
            }
            Ok(ProjectExpr::Scalar { path })
        }
    }
}

fn literal_u64(expr: Option<&VosExpr>, method: &str) -> Result<u64> {
    let Some(expr) = expr else {
        return Err(query_error(format!(
            "`.{method}` requires a count argument"
        )));
    };
    match expr {
        VosExpr::Literal(VosLiteral::Int(t)) => t
            .parse::<u64>()
            .map_err(|_| query_error(format!("invalid `{method}` count"))),
        _ => Err(query_error(format!(
            "`.{method}` count must be an integer literal"
        ))),
    }
}
