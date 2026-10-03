//! Phase 1 VOS write executor (`TypedObject.insert()`).

use std::collections::BTreeMap;

use vos::ast::catalog::{CatalogSnapshot, TypeKind};
use vos::ast::expr::{Expr, FieldInit, Stmt};
use vos::ast::{FieldAttribute, Literal, Program};

use yydb_types::{Error, Result, Value};

use super::ops::QueryRow;
use super::store;

fn dml_error(message: impl Into<String>) -> Error {
    Error::Schema {
        message: message.into(),
    }
}

/// Execute unit-valued VOS write programs against stored table rows.
pub fn execute(
    source: &str,
    catalog: &CatalogSnapshot,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let program = vos::parse_program(source).map_err(|diagnostics| Error::Schema {
        message: diagnostics.to_string(),
    })?;
    execute_program(&program, catalog, records)
}

pub fn execute_program(
    program: &Program,
    catalog: &CatalogSnapshot,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let mut bindings: Vec<(String, Expr)> = Vec::new();
    let mut wrote = false;

    for stmt in &program.statements {
        match stmt {
            Stmt::Let(let_) => bindings.push((let_.name.clone(), let_.value.clone())),
            Stmt::Expr(expr) => {
                apply_insert(expr, catalog, records, &bindings)?;
                wrote = true;
            }
            _ => {
                return Err(dml_error(
                    "unsupported statement in Phase 1 execute program",
                ));
            }
        }
    }

    if let Some(result) = &program.result {
        apply_insert(result, catalog, records, &bindings)?;
        wrote = true;
    }

    if !wrote {
        return Err(dml_error("execute program has no insert statements"));
    }
    Ok(())
}

fn apply_insert(
    expr: &Expr,
    catalog: &CatalogSnapshot,
    records: &mut BTreeMap<String, Vec<u8>>,
    bindings: &[(String, Expr)],
) -> Result<()> {
    let (table, row) = lower_insert(expr, catalog, bindings)?;
    let pk_field = primary_field_name(table_entry(catalog, &table)?)?;
    let pk = row
        .get(pk_field)
        .ok_or_else(|| dml_error(format!("insert row is missing primary key field `{pk_field}`")))?;
    let pk_text = value_to_key(pk)?;
    store::upsert_row(records, &table, &pk_text, &row)
}

fn lower_insert(
    expr: &Expr,
    catalog: &CatalogSnapshot,
    bindings: &[(String, Expr)],
) -> Result<(String, QueryRow)> {
    let Expr::Call { callee, args, .. } = expr else {
        return Err(dml_error("Phase 1 execute expects `.insert()` expression statements"));
    };
    if !args.is_empty() {
        return Err(dml_error("`.insert()` takes no arguments"));
    }
    let Expr::Member { object, name, .. } = callee.as_ref() else {
        return Err(dml_error("Phase 1 execute expects `.insert()` expression statements"));
    };
    if name != "insert" {
        return Err(dml_error(format!(
            "unsupported execute expression `.{}()`",
            name
        )));
    }
    let object = expand_bindings(object, bindings)?;
    let Expr::TypedObject { ty, fields, .. } = object else {
        return Err(dml_error(
            "Phase 1 execute only supports `Type { … }.insert()`",
        ));
    };
    let entry = table_entry(catalog, &ty)?;
    let row = lower_typed_object(entry, &fields)?;
    Ok((ty.clone(), row))
}

fn table_entry<'a>(catalog: &'a CatalogSnapshot, name: &str) -> Result<&'a vos::ast::catalog::TypeEntry> {
    catalog
        .types
        .iter()
        .find(|entry| entry.name == name && entry.kind == TypeKind::Table)
        .ok_or_else(|| dml_error(format!("unknown table `{name}`")))
}

fn primary_field_name(entry: &vos::ast::catalog::TypeEntry) -> Result<&str> {
    entry
        .fields
        .iter()
        .find(|field| field.attrs.contains(&FieldAttribute::Primary))
        .map(|field| field.current_name.as_str())
        .ok_or_else(|| dml_error(format!("table `{}` has no primary key field", entry.name)))
}

fn lower_typed_object(
    entry: &vos::ast::catalog::TypeEntry,
    fields: &[FieldInit],
) -> Result<QueryRow> {
    let mut row = QueryRow::new();
    for init in fields {
        let slot = entry
            .fields
            .iter()
            .find(|field| field.current_name == init.name)
            .ok_or_else(|| dml_error(format!("unknown field `{}` on `{}`", init.name, entry.name)))?;
        let value = init
            .value
            .as_ref()
            .ok_or_else(|| dml_error(format!("field `{}` requires an explicit initializer", init.name)))?;
        row.insert(init.name.clone(), eval_literal(value, &slot.ty)?);
    }
    Ok(row)
}

fn eval_literal(expr: &Expr, ty: &vos::ast::TypeExpr) -> Result<Value> {
    match expr {
        Expr::Literal(literal) => literal_to_value(literal, ty),
        _ => Err(dml_error(
            "Phase 1 execute only supports literal field initializers",
        )),
    }
}

fn literal_to_value(literal: &Literal, _ty: &vos::ast::TypeExpr) -> Result<Value> {
    Ok(match literal {
        Literal::Null => Value::Null,
        Literal::Bool(value) => Value::Bool(*value),
        Literal::Int(text) => Value::I64(text.parse().map_err(|_| {
            dml_error(format!("invalid integer literal `{text}`"))
        })?),
        Literal::String(text) | Literal::Ident(text) => Value::Text(text.clone()),
        Literal::Float(text) => Value::Text(text.clone()),
        _ => {
            return Err(dml_error(
                "Phase 1 execute only supports null, bool, integer, string, and float literals",
            ));
        }
    })
}

fn value_to_key(value: &Value) -> Result<String> {
    Ok(match value {
        Value::Text(text) => text.clone(),
        Value::Uuid(id) => id.to_string(),
        Value::I64(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => return Err(dml_error("primary key cannot be null")),
        _ => {
            return Err(dml_error(
                "primary key must be text, uuid, integer, or bool in Phase 1 execute",
            ));
        }
    })
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
        Expr::TypedObject { ty, fields, span } => {
            let mut expanded = Vec::with_capacity(fields.len());
            for init in fields {
                expanded.push(FieldInit {
                    name: init.name.clone(),
                    value: init
                        .value
                        .as_ref()
                        .map(|value| expand_bindings(value, bindings))
                        .transpose()?,
                    span: init.span,
                });
            }
            Ok(Expr::TypedObject {
                ty: ty.clone(),
                fields: expanded,
                span: *span,
            })
        }
        other => Ok(other.clone()),
    }
}
