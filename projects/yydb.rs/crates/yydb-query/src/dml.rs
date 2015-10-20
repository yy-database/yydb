//! Phase 1 VOS write executor (`TypedObject.insert()`, `Type::insert({ … })`,
//! keyed `Type::update({ … })` / `Type::delete({ … })`, schema macros).
//!
//! ## Phase 1 write shapes
//!
//! | Shape | Effect |
//! |-------|--------|
//! | `Type::insert({ … })` / `Type { … }.insert()` | Upsert the whole row |
//! | `Type::update({ pk: …, field: … })` | Patch a keyed row, omitted fields keep their stored value |
//! | `Type::delete({ pk: … })` | Remove a keyed row, a missing key is a no-op |
//!
//! Every shape takes one plain object literal whose primary-key field selects
//! the row. Collection forms (`Type.filter(…).update(…)`), lambda patches and
//! projection patches stay [`Error::Unsupported`]: Phase 1 has no predicate
//! evaluator here, so it never guesses which rows a filter would match.

use std::collections::BTreeMap;

use vos::{
    syntax::{VosDeclarationKind, VosExpr, VosFieldInit, VosLiteral, VosProgram, VosRoot, VosStmt},
    types::{ResolvedCanonicalType, ResolvedContract, ResolvedTypeContract},
};

use yydb_types::{Error, Result, TableUniqueConstraint, Value};

use super::{
    catalog::{field_on_table, primary_field_name, table_storage_name, table_type_by_name},
    coerce, constraints,
    ops::QueryRow,
    store,
};

fn dml_error(message: impl Into<String>) -> Error {
    Error::Schema { message: message.into() }
}

/// Quick AST-level check: does this VOS program look like a write /
/// mutation (insert, update, delete, macro call)? Used by
/// [`crate::params::prepare`] to short-circuit DML programs past the
/// read-pipeline lowerer.
pub(crate) fn is_write_program(program: &VosProgram) -> bool {
    let is_write = |expr: &VosExpr| is_macro_call(expr) || is_write_call(expr);
    for stmt in &program.statements {
        if let VosStmt::Expr(expr) = stmt {
            if is_write(expr) {
                return true;
            }
        }
    }
    if let Some(result) = &program.result {
        if is_write(result) {
            return true;
        }
    }
    false
}

/// Execute unit-valued VOS write programs against stored table rows.
pub fn execute(
    source: &str,
    contract: &ResolvedContract,
    schema_document: Option<&str>,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    execute_with_constraints(source, contract, &[], schema_document, records)
}

/// Execute writes with host-declared constraints merged into document rules.
pub fn execute_with_constraints(
    source: &str,
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    schema_document: Option<&str>,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> Result<()> {
    execute_with_params(source, contract, declared, schema_document, records, &[])
}

/// Execute a write program whose `__yydb_param_<index>` slots read `params`.
///
/// The caller rewrote `$name` parameters before parsing, so the program text
/// already carries slot identifiers and this function only resolves them. A
/// slot value is coerced against its declared field type exactly like a
/// literal, which is how a host-supplied `bytes` value reaches a `bytes`
/// column without a text detour.
pub fn execute_with_params(
    source: &str,
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    schema_document: Option<&str>,
    records: &mut BTreeMap<String, Vec<u8>>,
    params: &[Value],
) -> Result<()> {
    let program = vos::parse_program(source).map_err(|diagnostics| Error::Schema { message: diagnostics })?;
    let constraints = constraints::merge(constraints::from_contract(contract)?, declared);
    execute_program_with_constraints(&program, contract, &constraints, schema_document, records, params)
}

/// If `source` is a single `Type::insert({ … })` program, run it and return the stored row.
pub fn try_insert_returning(
    source: &str,
    contract: &ResolvedContract,
    records: &mut BTreeMap<String, Vec<u8>>,
) -> Result<Option<Vec<QueryRow>>> {
    let program = vos::parse_program(source).map_err(|diagnostics| Error::Schema { message: diagnostics })?;
    if !program.statements.is_empty() || program.result.is_none() {
        return Ok(None);
    }
    let result = program.result.as_ref().expect("checked above");
    let is_zero_arg_insert = matches!(
        result,
        VosExpr::Call {
            callee,
            args,
            ..
        } if args.is_empty() && matches!(callee.as_ref(), VosExpr::Member { name, .. } if name == "insert")
    );
    if is_macro_call(result) || is_zero_arg_insert {
        return Ok(None);
    }
    let (table, row) = match lower_insert(result, contract, &[], &[]) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let constraints = constraints::from_contract(contract)?;
    let stored = upsert_row_for_table(contract, &constraints, records, &table, &row)?;
    Ok(Some(vec![stored]))
}

fn execute_program_with_constraints(
    program: &VosProgram,
    contract: &ResolvedContract,
    constraints: &[TableUniqueConstraint],
    schema_document: Option<&str>,
    records: &mut BTreeMap<String, Vec<u8>>,
    params: &[Value],
) -> Result<()> {
    let expanded = expand_program(program, schema_document)?;
    let mut bindings: Vec<(String, VosExpr)> = Vec::new();
    let mut wrote = false;

    for stmt in &expanded.statements {
        match stmt {
            VosStmt::Let(let_) => bindings.push((let_.name.clone(), let_.value.clone())),
            VosStmt::Expr(expr) => {
                apply_write(expr, contract, constraints, records, &bindings, params)?;
                wrote = true;
            }
        }
    }

    if let Some(result) = &expanded.result {
        if is_macro_call(result) {
            return Err(dml_error("macro call could not be expanded (schema document required)"));
        }
        apply_write(result, contract, constraints, records, &bindings, params)?;
        wrote = true;
    }

    if !wrote {
        return Err(dml_error("execute program has no write statements"));
    }
    Ok(())
}

/// Replace every schema-macro call with the body the schema document declares.
///
/// Write binding runs this before it decides whether a program is a write: a
/// macro call is not a `.insert(…)` until it has been expanded, and an
/// unexpandable macro must stay an error rather than become an accepted write.
pub(crate) fn expand_program(program: &VosProgram, schema_document: Option<&str>) -> Result<VosProgram> {
    let mut statements = Vec::new();
    for stmt in &program.statements {
        match stmt {
            VosStmt::Let(_) => statements.push(stmt.clone()),
            VosStmt::Expr(expr) => expand_expr_into_program(expr, schema_document, &mut statements)?,
        }
    }

    let result = match &program.result {
        None => None,
        Some(expr) if is_macro_call(expr) => {
            let body = expand_macro_call(expr, schema_document)?;
            for stmt in body.statements {
                match stmt {
                    VosStmt::Expr(inner) => {
                        expand_expr_into_program(&inner, schema_document, &mut statements)?;
                    }
                    VosStmt::Let(let_) => statements.push(VosStmt::Let(let_)),
                }
            }
            body.result
        }
        Some(expr) => Some(expr.clone()),
    };

    Ok(VosProgram { micros: program.micros.clone(), statements, result, span: program.span.clone() })
}

fn expand_expr_into_program(expr: &VosExpr, schema_document: Option<&str>, statements: &mut Vec<VosStmt>) -> Result<()> {
    if is_macro_call(expr) {
        let body = expand_macro_call(expr, schema_document)?;
        for stmt in body.statements {
            match stmt {
                VosStmt::Expr(inner) => {
                    expand_expr_into_program(&inner, schema_document, statements)?;
                }
                VosStmt::Let(let_) => statements.push(VosStmt::Let(let_)),
            }
        }
        if let Some(result) = body.result {
            if is_macro_call(&result) {
                expand_expr_into_program(&result, schema_document, statements)?;
            }
            else {
                statements.push(VosStmt::Expr(result));
            }
        }
        return Ok(());
    }
    statements.push(VosStmt::Expr(expr.clone()));
    Ok(())
}

fn expand_macro_call(expr: &VosExpr, schema_document: Option<&str>) -> Result<VosProgram> {
    let VosExpr::Call { callee, args, .. } = expr
    else {
        return Err(dml_error("expected macro call"));
    };
    let VosExpr::Name { name, .. } = callee.as_ref()
    else {
        return Err(dml_error("expected bare macro name call"));
    };
    if !args.is_empty() {
        return Err(dml_error(format!("Phase 1 macro expansion does not bind parameters for `{name}()`")));
    }
    let document = schema_document.ok_or_else(|| dml_error("macro expansion requires an installed schema document"))?;
    let document = vos::syntax::parse(document).map_err(|diagnostics| Error::Schema { message: diagnostics })?;
    find_macro_body(&document, name).ok_or_else(|| dml_error(format!("unknown schema macro `{name}`")))
}

fn find_macro_body(document: &VosRoot, name: &str) -> Option<VosProgram> {
    document
        .declarations
        .iter()
        .find(|declaration| declaration.kind == VosDeclarationKind::Macro && declaration.name.as_deref() == Some(name))
        .and_then(|declaration| declaration.body_program.clone())
}

/// Whether a program contains at least one `.insert(…)`, `.update(…)` or
/// `.delete(…)` expression.
///
/// Used by parameter binding to refuse a read pipeline before it can enter the
/// DML executor. Broader than [`is_write_program`], which only recognizes the
/// zero-argument `Type { … }.insert()` shape consumed by `prepare`.
pub(crate) fn has_write_call(program: &VosProgram) -> bool {
    program.statements.iter().any(|stmt| matches!(stmt, VosStmt::Expr(expr) if is_write_call(expr)))
        || program.result.as_ref().is_some_and(is_write_call)
}

fn is_macro_call(expr: &VosExpr) -> bool {
    matches!(
        expr,
        VosExpr::Call {
            callee,
            args,
            ..
        } if args.is_empty() && matches!(callee.as_ref(), VosExpr::Name { .. })
    )
}

/// Whether `expr` is `.insert(…)`, `.update(…)` or `.delete(…)`.
fn is_write_call(expr: &VosExpr) -> bool {
    matches!(
        expr,
        VosExpr::Call {
            callee,
            ..
        } if matches!(
            callee.as_ref(),
            VosExpr::Member { name, .. } if matches!(name.as_str(), "insert" | "update" | "delete")
        )
    )
}

/// Record keys a point write needs to read before it runs, when every write in
/// the program is a keyed write on a table without uniqueness rules.
///
/// The engine uses this to publish only the records a statement changed instead
/// of materializing the database. `None` means "cannot prove this is a point
/// write": the caller must fall back to the full path and let the real executor
/// report any error.
///
/// Every keyed write names exactly the primary-key record it addresses. Reads
/// need it because they merge or resolve against the stored row, and inserts
/// want it so an insert that rewrites the same bytes publishes no page at all.
///
/// Any table carrying a uniqueness rule falls back, because its conflict check
/// loads the whole table. Macros are expanded first, so a macro that expands to
/// keyed writes is accepted and one that expands to anything else is not.
pub fn point_write_keys(
    source: &str,
    contract: &ResolvedContract,
    declared: &[TableUniqueConstraint],
    schema_document: Option<&str>,
    params: &[Value],
) -> Result<Option<Vec<String>>> {
    let program = vos::parse_program(source).map_err(|diagnostics| Error::Schema { message: diagnostics })?;
    let constraints = constraints::merge(constraints::from_contract(contract)?, declared);
    let Ok(expanded) = expand_program(&program, schema_document)
    else {
        return Ok(None);
    };
    let mut bindings: Vec<(String, VosExpr)> = Vec::new();
    let mut keys = Vec::new();
    let mut wrote = false;
    for stmt in &expanded.statements {
        match stmt {
            VosStmt::Let(let_) => bindings.push((let_.name.clone(), let_.value.clone())),
            VosStmt::Expr(expr) => {
                let Ok(Some(key)) = plan_point_key(expr, contract, &constraints, &bindings, params)
                else {
                    return Ok(None);
                };
                keys.push(key);
                wrote = true;
            }
        }
    }
    if let Some(result) = &expanded.result {
        let Ok(Some(key)) = plan_point_key(result, contract, &constraints, &bindings, params)
        else {
            return Ok(None);
        };
        keys.push(key);
        wrote = true;
    }
    if !wrote {
        return Ok(None);
    }
    keys.sort();
    keys.dedup();
    Ok(Some(keys))
}

/// One keyed write's record key, or `None` when the expression is not a Phase 1
/// point write on a rule-free table.
fn plan_point_key(
    expr: &VosExpr,
    contract: &ResolvedContract,
    constraints: &[TableUniqueConstraint],
    bindings: &[(String, VosExpr)],
    params: &[Value],
) -> Result<Option<String>> {
    let VosExpr::Call { callee, .. } = expr
    else {
        return Ok(None);
    };
    let VosExpr::Member { name, .. } = callee.as_ref()
    else {
        return Ok(None);
    };
    let (table, pk_text) = match name.as_str() {
        "insert" => {
            let Ok((table, row)) = lower_insert(expr, contract, bindings, params)
            else {
                return Ok(None);
            };
            let Ok(entry) = table_type_by_name(contract, &table)
            else {
                return Ok(None);
            };
            let Ok(pk_field) = primary_field_name(entry)
            else {
                return Ok(None);
            };
            let Some(pk) = row.get(pk_field)
            else {
                return Ok(None);
            };
            let Ok(pk_text) = constraints::value_text(pk)
            else {
                return Ok(None);
            };
            (table, pk_text)
        }
        "update" | "delete" => {
            let Ok((table, pk_text, _)) = lower_keyed_write(expr, contract, name.as_str(), bindings, params)
            else {
                return Ok(None);
            };
            (table, pk_text)
        }
        _ => return Ok(None),
    };
    let Ok(entry) = table_type_by_name(contract, &table)
    else {
        return Ok(None);
    };
    let storage_name = table_storage_name(entry)?;
    if constraints::has_rules(constraints, storage_name) {
        return Ok(None);
    }
    Ok(Some(store::row_key(storage_name, &pk_text)))
}

fn apply_write(
    expr: &VosExpr,
    contract: &ResolvedContract,
    constraints: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    bindings: &[(String, VosExpr)],
    params: &[Value],
) -> Result<()> {
    let VosExpr::Call { callee, .. } = expr
    else {
        return Err(dml_error("Phase 1 execute expects a write expression"));
    };
    let VosExpr::Member { name, .. } = callee.as_ref()
    else {
        return Err(dml_error("Phase 1 execute expects a write expression"));
    };
    match name.as_str() {
        "insert" => apply_insert(expr, contract, constraints, records, bindings, params),
        "update" => {
            let (table, pk_text, patch) = lower_keyed_write(expr, contract, "update", bindings, params)?;
            apply_update(contract, constraints, records, &table, &pk_text, &patch)
        }
        "delete" => {
            let (table, pk_text, _) = lower_keyed_write(expr, contract, "delete", bindings, params)?;
            apply_delete(contract, records, &table, &pk_text)
        }
        _ => Err(Error::Unsupported("Phase 1 execute supports `.insert()`, `.update()` and `.delete()` only")),
    }
}

/// Lower `Type::update({ … })` / `Type::delete({ … })` to a table, primary key
/// text and the object argument.
fn lower_keyed_write(
    expr: &VosExpr,
    contract: &ResolvedContract,
    verb: &str,
    bindings: &[(String, VosExpr)],
    params: &[Value],
) -> Result<(String, String, QueryRow)> {
    let VosExpr::Call { callee, args, .. } = expr
    else {
        return Err(dml_error("Phase 1 execute expects a write expression"));
    };
    let VosExpr::Member { object, .. } = callee.as_ref()
    else {
        return Err(dml_error("Phase 1 execute expects a write expression"));
    };
    let table = match object.as_ref() {
        VosExpr::Name { name, .. } => name.clone(),
        _ => {
            return Err(Error::Unsupported(
                "Phase 1 keyed writes target a table type (`Type::update({ … })` or `Type::delete({ … })`), not a \
                 filtered query",
            ));
        }
    };
    if args.len() != 1 {
        return Err(Error::Unsupported("Phase 1 keyed writes require exactly one plain object argument"));
    }
    let entry = table_type_by_name(contract, &table)?;
    let VosExpr::AnonObject { fields, .. } = expand_bindings(&args[0], bindings)?
    else {
        return Err(Error::Unsupported("Phase 1 keyed writes require a plain `{ … }` object, not a lambda or projection"));
    };
    let object = lower_field_inits(entry, &fields, params)?;
    let pk_field = primary_field_name(entry)?;
    let pk = object
        .get(pk_field)
        .ok_or_else(|| dml_error(format!("`.{verb}()` object must carry the primary key field `{pk_field}`")))?;
    let pk_text = constraints::value_text(pk)?;
    Ok((table, pk_text, object))
}

fn apply_update(
    contract: &ResolvedContract,
    constraints: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    table: &str,
    pk_text: &str,
    patch: &QueryRow,
) -> Result<()> {
    let entry = table_type_by_name(contract, table)?;
    let pk_field = primary_field_name(entry)?;
    let storage_name = table_storage_name(entry)?.to_string();
    let mut merged = {
        if !records.contains_key(&store::row_key(&storage_name, pk_text)) {
            return Err(dml_error(format!("update target `{table}` primary key `{pk_text}` does not exist")));
        }
        store::load_row(records, &storage_name, pk_text)?
    };
    for (field, value) in patch {
        merged.insert(field.clone(), value.clone());
    }
    constraints::check(records, constraints, &storage_name, pk_field, pk_text, &merged)?;
    store::upsert_row(records, &storage_name, pk_text, &merged)
}

fn apply_delete(
    contract: &ResolvedContract,
    records: &mut BTreeMap<String, Vec<u8>>,
    table: &str,
    pk_text: &str,
) -> Result<()> {
    let entry = table_type_by_name(contract, table)?;
    let storage_name = table_storage_name(entry)?.to_string();
    // The object argument may carry more than the key, but only the key is
    // matched, so a stale caller still deletes exactly one row and a key that
    // is already gone is a no-op.
    store::remove_row(records, &store::row_key(&storage_name, pk_text));
    Ok(())
}

fn apply_insert(
    expr: &VosExpr,
    contract: &ResolvedContract,
    constraints: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    bindings: &[(String, VosExpr)],
    params: &[Value],
) -> Result<()> {
    let (table, row) = lower_insert(expr, contract, bindings, params)?;
    upsert_row_for_table(contract, constraints, records, &table, &row)?;
    Ok(())
}

fn upsert_row_for_table(
    contract: &ResolvedContract,
    constraints: &[TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
    table: &str,
    row: &QueryRow,
) -> Result<QueryRow> {
    let entry = table_type_by_name(contract, table)?;
    let pk_field = primary_field_name(entry)?;
    let pk = row.get(pk_field).ok_or_else(|| dml_error(format!("insert row is missing primary key field `{pk_field}`")))?;
    let pk_text = constraints::value_text(pk)?;
    let storage_name = table_storage_name(entry)?.to_string();
    constraints::check(records, constraints, &storage_name, pk_field, &pk_text, row)?;
    store::upsert_row(records, &storage_name, &pk_text, row)?;
    Ok(row.clone())
}

fn lower_insert(
    expr: &VosExpr,
    contract: &ResolvedContract,
    bindings: &[(String, VosExpr)],
    params: &[Value],
) -> Result<(String, QueryRow)> {
    let VosExpr::Call { callee, args, .. } = expr
    else {
        return Err(dml_error("Phase 1 execute expects an insert expression"));
    };
    let VosExpr::Member { object, name, .. } = callee.as_ref()
    else {
        return Err(dml_error("Phase 1 execute expects an insert expression"));
    };
    if name != "insert" {
        return Err(dml_error(format!("unsupported execute expression `.{}()`", name)));
    }

    if args.len() == 1 {
        let table = type_name_from_expr(object)?;
        let entry = table_type_by_name(contract, &table)?;
        let VosExpr::AnonObject { fields, .. } = expand_bindings(&args[0], bindings)?
        else {
            return Err(dml_error("Phase 1 execute expects `Type::insert({ … })` with an object literal"));
        };
        let row = lower_field_inits(entry, &fields, params)?;
        return Ok((table, row));
    }
    if !args.is_empty() {
        return Err(dml_error("`.insert()` accepts zero or one object argument"));
    }

    let object = expand_bindings(object, bindings)?;
    let VosExpr::TypedObject { ty, fields, .. } = object
    else {
        return Err(dml_error("Phase 1 execute supports `Type { … }.insert()` or `Type::insert({ … })`"));
    };
    let entry = table_type_by_name(contract, &ty)?;
    let row = lower_field_inits(entry, &fields, params)?;
    Ok((ty, row))
}

fn type_name_from_expr(expr: &VosExpr) -> Result<String> {
    match expr {
        VosExpr::Name { name, .. } => Ok(name.clone()),
        VosExpr::Member { object, .. } => type_name_from_expr(object.as_ref()),
        _ => Err(dml_error("insert target must be a table type name")),
    }
}

fn table_display_name(entry: &ResolvedTypeContract) -> String {
    entry.canonical_path.join("::")
}

fn lower_field_inits(entry: &ResolvedTypeContract, fields: &[VosFieldInit], params: &[Value]) -> Result<QueryRow> {
    let mut row = QueryRow::new();
    for init in fields {
        let slot = field_on_table(entry, &init.name)
            .map_err(|_| dml_error(format!("unknown field `{}` on `{}`", init.name, table_display_name(entry))))?;
        let value =
            init.value.as_ref().ok_or_else(|| dml_error(format!("field `{}` requires an explicit initializer", init.name)))?;
        row.insert(init.name.clone(), eval_literal(value, &slot.canonical_type, params)?);
    }
    Ok(row)
}

fn eval_literal(expr: &VosExpr, ty: &ResolvedCanonicalType, params: &[Value]) -> Result<Value> {
    match expr {
        VosExpr::Literal(literal) => {
            let raw = match literal {
                VosLiteral::Null => Value::Null,
                VosLiteral::Bool(value) => Value::Bool(*value),
                VosLiteral::Int(text) => {
                    Value::I64(text.parse().map_err(|_| dml_error(format!("invalid integer literal `{text}`")))?)
                }
                VosLiteral::String(text) => Value::Text(text.clone()),
                VosLiteral::Float(text) => {
                    Value::F64(text.parse().map_err(|_| dml_error(format!("invalid REAL literal `{text}`")))?)
                }
            };
            coerce::value_to_scalar(raw, ty)
        }
        VosExpr::Name { name, .. } => {
            let Some(slot) = name.strip_prefix(super::params::PARAM_PREFIX)
            else {
                return Err(dml_error("Phase 1 execute only supports literal field initializers and bound parameters"));
            };
            let index: usize = slot.parse().map_err(|_| dml_error(format!("malformed parameter slot `{name}`")))?;
            let value = params.get(index).ok_or_else(|| dml_error(format!("parameter slot {index} has no bound value")))?;
            coerce::value_to_scalar(value.clone(), ty)
        }
        _ => Err(dml_error("Phase 1 execute only supports literal field initializers and bound parameters")),
    }
}

fn expand_bindings(expr: &VosExpr, bindings: &[(String, VosExpr)]) -> Result<VosExpr> {
    match expr {
        VosExpr::Name { name, .. } => {
            if let Some((_, value)) = bindings.iter().rev().find(|(n, _)| n == name) {
                expand_bindings(value, bindings)
            }
            else {
                Ok(expr.clone())
            }
        }
        VosExpr::Member { object, name, name_span, sep, span } => Ok(VosExpr::Member {
            object: Box::new(expand_bindings(object, bindings)?),
            name: name.clone(),
            name_span: name_span.clone(),
            sep: *sep,
            span: span.clone(),
        }),
        VosExpr::Call { callee, args, span } => Ok(VosExpr::Call {
            callee: Box::new(expand_bindings(callee, bindings)?),
            args: args.iter().map(|arg| expand_bindings(arg, bindings)).collect::<Result<Vec<_>>>()?,
            span: span.clone(),
        }),
        VosExpr::TypedObject { ty, ty_span, fields, span } => {
            let mut expanded = Vec::with_capacity(fields.len());
            for init in fields {
                expanded.push(VosFieldInit {
                    name: init.name.clone(),
                    name_span: init.name_span.clone(),
                    value: init.value.as_ref().map(|value| expand_bindings(value, bindings)).transpose()?,
                    span: init.span.clone(),
                });
            }
            Ok(VosExpr::TypedObject { ty: ty.clone(), ty_span: ty_span.clone(), fields: expanded, span: span.clone() })
        }
        VosExpr::AnonObject { fields, span } => {
            let mut expanded = Vec::with_capacity(fields.len());
            for init in fields {
                expanded.push(VosFieldInit {
                    name: init.name.clone(),
                    name_span: init.name_span.clone(),
                    value: init.value.as_ref().map(|value| expand_bindings(value, bindings)).transpose()?,
                    span: init.span.clone(),
                });
            }
            Ok(VosExpr::AnonObject { fields: expanded, span: span.clone() })
        }
        other => Ok(other.clone()),
    }
}
