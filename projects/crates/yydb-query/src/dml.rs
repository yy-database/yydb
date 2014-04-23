//! Phase 1 VOS write executor (`TypedObject.insert()`, `Type::insert({ … })`, schema macros).

use std::collections::BTreeMap;

use vos::ast::catalog::{CatalogSnapshot, TypeKind};
use vos::ast::expr::{Expr, FieldInit, Stmt};
use vos::ast::{Document, FieldAttribute, Item, Literal, Program};

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
    schema_document: Option<&str>,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let program = vos::parse_program(source).map_err(|diagnostics| Error::Schema {
        message: diagnostics.to_string(),
    })?;
    execute_program(&program, catalog, schema_document, records)
}

/// If `source` is a single `Type::insert({ … })` program, run it and return the stored row.
pub fn try_insert_returning(
    source: &str,
    catalog: &CatalogSnapshot,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> Result<Option<Vec<QueryRow>>> {
    let program = vos::parse_program(source).map_err(|diagnostics| Error::Schema {
        message: diagnostics.to_string(),
    })?;
    if !program.statements.is_empty() || program.result.is_none() {
        return Ok(None);
    }
    let result = program.result.as_ref().expect("checked above");
    if is_macro_call(result) || is_typed_object_insert(result) {
        return Ok(None);
    }
    let (table, row) = match lower_insert(result, catalog, &[]) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let stored = upsert_row_for_table(catalog, records, &table, &row)?;
    Ok(Some(vec![stored]))
}

pub fn execute_program(
    program: &Program,
    catalog: &CatalogSnapshot,
    schema_document: Option<&str>,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    let expanded = expand_program(program, schema_document)?;
    let mut bindings: Vec<(String, Expr)> = Vec::new();
    let mut wrote = false;

    for stmt in &expanded.statements {
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

    if let Some(result) = &expanded.result {
        if is_macro_call(result) {
            return Err(dml_error(
                "macro call could not be expanded (schema document required)",
            ));
        }
        apply_insert(result, catalog, records, &bindings)?;
        wrote = true;
    }

    if !wrote {
        return Err(dml_error("execute program has no insert statements"));
    }
    Ok(())
}

fn expand_program(program: &Program, schema_document: Option<&str>) -> Result<Program> {
    let mut statements = Vec::new();
    for stmt in &program.statements {
        match stmt {
            Stmt::Let(_) => statements.push(stmt.clone()),
            Stmt::Expr(expr) => expand_expr_into_program(expr, schema_document, &mut statements)?,
            _ => {
                return Err(dml_error(
                    "unsupported statement in Phase 1 execute program",
                ));
            }
        }
    }

    let result = match &program.result {
        None => None,
        Some(expr) if is_macro_call(expr) => {
            let body = expand_macro_call(expr, schema_document)?;
            for stmt in body.statements {
                match stmt {
                    Stmt::Expr(inner) => {
                        expand_expr_into_program(&inner, schema_document, &mut statements)?;
                    }
                    Stmt::Let(let_) => statements.push(Stmt::Let(let_.clone())),
                    _ => {
                        return Err(dml_error("unsupported statement in expanded macro body"));
                    }
                }
            }
            body.result
        }
        Some(expr) => Some(expr.clone()),
    };

    Ok(Program {
        micros: program.micros.clone(),
        statements,
        result,
        span: program.span,
    })
}

fn expand_expr_into_program(
    expr: &Expr,
    schema_document: Option<&str>,
    statements: &mut Vec<Stmt>,
) -> Result<()> {
    if is_macro_call(expr) {
        let body = expand_macro_call(expr, schema_document)?;
        for stmt in body.statements {
            match stmt {
                Stmt::Expr(inner) => {
                    expand_expr_into_program(&inner, schema_document, statements)?;
                }
                Stmt::Let(let_) => statements.push(Stmt::Let(let_.clone())),
                _ => {
                    return Err(dml_error("unsupported statement in expanded macro body"));
                }
            }
        }
        if let Some(result) = body.result {
            if is_macro_call(&result) {
                expand_expr_into_program(&result, schema_document, statements)?;
            } else {
                statements.push(Stmt::Expr(result));
            }
        }
        return Ok(());
    }
    statements.push(Stmt::Expr(expr.clone()));
    Ok(())
}

fn expand_macro_call(expr: &Expr, schema_document: Option<&str>) -> Result<Program> {
    let Expr::Call { callee, args, .. } = expr else {
        return Err(dml_error("expected macro call"));
    };
    let Expr::Name { name, .. } = callee.as_ref() else {
        return Err(dml_error("expected bare macro name call"));
    };
    if !args.is_empty() {
        return Err(dml_error(format!(
            "Phase 1 macro expansion does not bind parameters for `{name}()`"
        )));
    }
    let document = schema_document
        .ok_or_else(|| dml_error("macro expansion requires an installed schema document"))?;
    let document = vos::parser::parse_document(document).map_err(|diagnostics| Error::Schema {
        message: diagnostics.to_string(),
    })?;
    find_macro_body(&document, name)
        .ok_or_else(|| dml_error(format!("unknown schema macro `{name}`")))
}

fn find_macro_body(document: &Document, name: &str) -> Option<Program> {
    document.items.iter().find_map(|item| match item {
        Item::Macro(macro_def) if macro_def.name == name => Some(macro_def.body.clone()),
        _ => None,
    })
}

fn is_macro_call(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Call {
            callee,
            args,
            ..
        } if args.is_empty() && matches!(callee.as_ref(), Expr::Name { .. })
    )
}

fn is_typed_object_insert(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Call {
            callee,
            args,
            ..
        } if args.is_empty()
            && matches!(
                callee.as_ref(),
                Expr::Member { name, .. } if name == "insert"
            )
    )
}

fn apply_insert(
    expr: &Expr,
    catalog: &CatalogSnapshot,
    records: &mut BTreeMap<String, Vec<u8>>,
    bindings: &[(String, Expr)],
) -> Result<()> {
    let (table, row) = lower_insert(expr, catalog, bindings)?;
    upsert_row_for_table(catalog, records, &table, &row)?;
    Ok(())
}

fn upsert_row_for_table(
    catalog: &CatalogSnapshot,
    records: &mut BTreeMap<String, Vec<u8>>,
    table: &str,
    row: &QueryRow,
) -> Result<QueryRow> {
    let pk_field = primary_field_name(table_entry(catalog, table)?)?;
    let pk = row.get(pk_field).ok_or_else(|| {
        dml_error(format!(
            "insert row is missing primary key field `{pk_field}`"
        ))
    })?;
    let pk_text = value_to_key(pk)?;
    store::upsert_row(records, table, &pk_text, row)?;
    Ok(row.clone())
}

fn lower_insert(
    expr: &Expr,
    catalog: &CatalogSnapshot,
    bindings: &[(String, Expr)],
) -> Result<(String, QueryRow)> {
    let Expr::Call { callee, args, .. } = expr else {
        return Err(dml_error("Phase 1 execute expects an insert expression"));
    };
    let Expr::Member { object, name, .. } = callee.as_ref() else {
        return Err(dml_error("Phase 1 execute expects an insert expression"));
    };
    if name != "insert" {
        return Err(dml_error(format!(
            "unsupported execute expression `.{}()`",
            name
        )));
    }

    if args.len() == 1 {
        let table = type_name_from_expr(object)?;
        let entry = table_entry(catalog, &table)?;
        let Expr::AnonObject { fields, .. } = expand_bindings(&args[0], bindings)? else {
            return Err(dml_error(
                "Phase 1 execute expects `Type::insert({ … })` with an object literal",
            ));
        };
        let row = lower_field_inits(entry, &fields)?;
        return Ok((table, row));
    }
    if !args.is_empty() {
        return Err(dml_error("`.insert()` accepts zero or one object argument"));
    }

    let object = expand_bindings(object, bindings)?;
    let Expr::TypedObject { ty, fields, .. } = object else {
        return Err(dml_error(
            "Phase 1 execute supports `Type { … }.insert()` or `Type::insert({ … })`",
        ));
    };
    let entry = table_entry(catalog, &ty)?;
    let row = lower_field_inits(entry, &fields)?;
    Ok((ty.clone(), row))
}

fn type_name_from_expr(expr: &Expr) -> Result<String> {
    match expr {
        Expr::Name { name, .. } => Ok(name.clone()),
        Expr::Member { object, .. } => type_name_from_expr(object.as_ref()),
        _ => Err(dml_error("insert target must be a table type name")),
    }
}

fn table_entry<'a>(
    catalog: &'a CatalogSnapshot,
    name: &str,
) -> Result<&'a vos::ast::catalog::TypeEntry> {
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

fn lower_field_inits(
    entry: &vos::ast::catalog::TypeEntry,
    fields: &[FieldInit],
) -> Result<QueryRow> {
    let mut row = QueryRow::new();
    for init in fields {
        let slot = entry
            .fields
            .iter()
            .find(|field| field.current_name == init.name)
            .ok_or_else(|| {
                dml_error(format!("unknown field `{}` on `{}`", init.name, entry.name))
            })?;
        let value = init.value.as_ref().ok_or_else(|| {
            dml_error(format!(
                "field `{}` requires an explicit initializer",
                init.name
            ))
        })?;
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
        Literal::Int(text) => Value::I64(
            text.parse()
                .map_err(|_| dml_error(format!("invalid integer literal `{text}`")))?,
        ),
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
        Expr::AnonObject { fields, span } => {
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
            Ok(Expr::AnonObject {
                fields: expanded,
                span: *span,
            })
        }
        other => Ok(other.clone()),
    }
}
