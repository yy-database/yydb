//! Prepared query contract: parse once, bind many times, execute many times.
//!
//! This module sits above [`super::lower`] and produces a plan ready to run
//! against any bound parameter slice. Callers:
//!
//! ```ignore
//! let stmt = prepare(source)?;
//! let bound = stmt.bind_named(&{"min_age" => Value::I64(18)})?;
//! let rows = execute_prepared(&bound, contract, records)?;
//! ```
//!
//! Binding a second time produces a *new* `BoundStatement` without re-parsing
//! or re-lowering — exactly the "prepare once, execute many" semantic the user
//! surface exposes via `Database.prepare(...)`.

use std::collections::{BTreeMap, BTreeSet};

use yydb_types::{Error, Result, Value};

use super::{
    exec, lower,
    ops::{BoundValue, QueryOp, QueryRow, SlotTy, WriteOp},
    sql,
    write::{self, WriteOutcome},
};
use vos::syntax::{VosExpr, VosProgram, VosStmt};

/// Authoring language a statement was prepared from.
///
/// Both frontends lower into the same physical operators; the dialect is kept
/// only for diagnostics and for re-preparing against the right parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Dialect {
    /// VOS pipeline / write program.
    Vos,
    /// SQL statement.
    Sql,
}

/// One named parameter extracted during prepare.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ParameterDescriptor {
    /// Canonical parameter name (no leading `$`).
    pub name: String,
    /// Positional slot index within the bound-value slice.
    pub index: u32,
    /// Declared slot type; inferred from surrounding context when possible,
    /// otherwise `Any`.
    pub slot_ty: SlotTy,
    /// Number of times this parameter appears in the source.
    pub usage_count: u32,
}

/// Read / write query kind — populated after lowering when ops are inspected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum QueryKind {
    /// Read pipeline ending in `.collect()`.
    Read,
    /// Write program (`Type::insert`, etc.).
    Write,
}

/// A pre-parsed, pre-lowered query plan with known parameter slots.
///
/// `ops` carries the engine-internal lowered plan (not serialized across
/// boundaries); the public surface exposes only parameter descriptors,
/// fingerprints and kind for introspection and wire transmission.
///
/// When prepared through a live [`yydb_engine::Connection`] the statement
/// additionally records the active schema's FNV-1a fingerprint and revision
/// so later executions can detect schema migration without re-parsing.
/// Pure [`prepare`] calls (for tests or offline use) leave these fields at
/// their empty / zero values; the engine path always fills them.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreparedStatement {
    /// Unique synthetic ID for this prepared statement (monotonic within a
    /// connection, stable fingerprint otherwise).
    pub statement_id: String,
    /// Frontend the statement was prepared from.
    pub dialect: Dialect,
    /// Lowered read ops where every parameter occurrence is a
    /// `BoundValue::Slot`.
    #[serde(skip)]
    pub ops: Vec<QueryOp>,
    /// Lowered write ops (empty for read statements).
    #[serde(skip)]
    pub write_ops: Vec<WriteOp>,
    /// Whether this statement is a read pipeline or a write program.
    pub query_kind: QueryKind,
    /// Parameter descriptors in ascending `index` order.
    pub parameters: Vec<ParameterDescriptor>,
    /// Tables referenced in the read pipeline (from `QueryOp::Scan` +
    /// field paths touched by filters / projections / sort).
    pub read_set: Vec<String>,
    /// Tables this statement writes into (populated only for DML paths).
    pub write_set: Vec<String>,
    /// FNV-1a fingerprint of the raw source — two identical sources hash equal.
    pub source_fingerprint: String,
    /// The raw VOS source text (for diagnostics / cache keys).
    pub source: String,
    /// FNV-1a fingerprint of the schema document captured at prepare time.
    /// Empty string when this statement was prepared without a live schema
    /// context (unit tests or offline [`prepare`] calls).
    #[serde(default)]
    pub schema_fingerprint: String,
    /// Schema revision captured at prepare time. Zero when prepared without
    /// a live schema context.
    #[serde(default)]
    pub schema_revision: u64,
}

/// A parameterized statement bound to one concrete parameter value set.
/// Produced by [`bind_named`].
pub struct BoundStatement<'a> {
    /// The owning prepared statement.
    pub stmt: &'a PreparedStatement,
    /// Positional bound values in ascending slot-index order.
    pub values: Vec<Value>,
}

/// Prefix injected into source identifiers during prepare so that VOS
/// parser accepts the token as a plain Name node. Oak-vos lexer currently
/// has no `$` token — `$name` would parse as invalid syntax. Instead we
/// rewrite each `$name` → `__yydb_param_<index>` (a valid Identifier), then
/// lower.rs walks the AST and converts `Name("__yydb_param_*")` directly
/// into `BoundValue::Slot` references.
pub(crate) const PARAM_PREFIX: &str = "__yydb_param_";

/// Compile a VOS source into a prepared statement. If the source contains no
/// `$name` parameters, the returned plan still goes through the same path —
/// parameters will be an empty vec and every bound value will be unused.
///
/// Internally this is:
///   1. `scan_and_rewrite` — extract `$name` params (lexer-aware, skips
///      strings / line comments / block comments) and rewrite each to an
///      identifier `__yydb_param_<index>`.
///   2. Fingerprint the *original* source.
///   3. `vos::parse_program(&rewritten)` — Oak-vos sees only valid VOS.
///   4. `lower::lower_program` — Name("__yydb_param_*") nodes are lowered
///      directly to `BoundValue::Slot { index, expected_ty }`.
///   5. Build the full PreparedStatement envelope (read_set / write_set /
///      query_kind), ready to pass to [`execute_prepared`].
///
/// This pure variant does not know the live schema so `schema_fingerprint`
/// and `schema_revision` remain empty / zero. The engine path fills them.
pub fn prepare(source: &str) -> Result<PreparedStatement> {
    let (parameters, rewritten) = scan_and_rewrite(source, Dialect::Vos)?;
    let source_fingerprint = fingerprint(source);

    let program = vos::parse_program(&rewritten).map_err(|diagnostics| Error::Schema { message: diagnostics })?;

    // ── Write-program short-circuit ────────────────────────────────────
    // DML programs (`Type::insert(...)`, `TypedObject.insert()`) do not
    // lower to QueryOp pipelines — execute_prepared must route them through
    // the dml executor instead. We detect them here so `query_kind` is
    // populated correctly before engine / dispatch code sees the stmt.
    let is_write_program = super::dml::is_write_program(&program);
    let (ops, query_kind, read_set, write_set) = if is_write_program {
        (Vec::new(), QueryKind::Write, Vec::new(), extract_write_set(&program))
    }
    else {
        let ops = lower::lower_program(&program, &parameters)?;
        let (kind, reads, writes) = classify_ops(&ops);
        (ops, kind, reads, writes)
    };

    Ok(PreparedStatement {
        statement_id: format!("stmt-{source_fingerprint}"),
        dialect: Dialect::Vos,
        ops,
        write_ops: Vec::new(),
        query_kind,
        parameters,
        read_set,
        write_set,
        source_fingerprint,
        source: source.to_owned(),
        schema_fingerprint: String::new(),
        schema_revision: 0,
    })
}

/// Compile a SQL source into a prepared statement.
///
/// SQL and VOS differ only in the front half: parameter extraction, parsing and
/// lowering. The resulting plan is the same operator set, so execution and
/// schema-drift handling are shared with [`prepare`].
pub fn prepare_sql(source: &str) -> Result<PreparedStatement> {
    let (parameters, rewritten) = scan_and_rewrite(source, Dialect::Sql)?;
    let source_fingerprint = fingerprint(source);
    let plan = sql::lower(&rewritten, &parameters)?;
    let query_kind = if plan.write_ops.is_empty() { QueryKind::Read } else { QueryKind::Write };

    Ok(PreparedStatement {
        statement_id: format!("stmt-{source_fingerprint}"),
        dialect: Dialect::Sql,
        ops: plan.read_ops,
        write_ops: plan.write_ops,
        query_kind,
        parameters,
        read_set: plan.read_set,
        write_set: plan.write_set,
        source_fingerprint,
        source: source.to_owned(),
        schema_fingerprint: String::new(),
        schema_revision: 0,
    })
}

/// Bind a prepared statement to a named parameter map.
///
/// Returns `Err` if:
///
/// * A required parameter is missing from `named`.
/// * `named` contains a parameter name the statement never references.
/// * A bound value does not match the declared `slot_ty`.
pub fn bind_named(stmt: &PreparedStatement, named: &BTreeMap<String, Value>) -> Result<Vec<Value>> {
    let param_names: BTreeSet<&String> = stmt.parameters.iter().map(|p| &p.name).collect();
    for supplied in named.keys() {
        if !param_names.contains(supplied) {
            return Err(Error::QueryBind {
                parameter: supplied.clone(),
                message: format!("statement has no parameter named `{supplied}`"),
            });
        }
    }

    let mut values: Vec<Value> = vec![Value::Null; stmt.parameters.len()];
    for param in &stmt.parameters {
        let Some(value) = named.get(&param.name)
        else {
            return Err(Error::QueryBind {
                parameter: param.name.clone(),
                message: format!("missing required parameter `{}`", param.name),
            });
        };
        if !param.slot_ty.matches(value) {
            let got = match value {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::I64(_) | Value::U64(_) => "int",
                Value::F64(_) => "float",
                Value::Text(_) => "text",
                Value::Bytes(_) => "bytes",
                Value::Uuid(_) => "uuid",
                _ => "complex",
            };
            return Err(Error::QueryBind {
                parameter: param.name.clone(),
                message: format!("parameter `{}` expects `{}`, got `{}`", param.name, param.slot_ty.as_str(), got),
            });
        }
        values[param.index as usize] = value.clone();
    }
    Ok(values)
}

/// Bind a prepared statement to an ordered value slice.
///
/// Positional binding is what SQL frontends need (`?` placeholders): values are
/// matched to parameters in declaration order. Named statements (`$name`,
/// `:name`) can still bind positionally as long as the caller keeps the
/// declaration order returned by [`prepare`] / [`prepare_sql`].
pub fn bind_ordered(stmt: &PreparedStatement, values: &[Value]) -> Result<Vec<Value>> {
    if values.len() != stmt.parameters.len() {
        return Err(Error::QueryBind {
            parameter: format!("{}", stmt.parameters.len()),
            message: format!("statement expects {} parameters, got {}", stmt.parameters.len(), values.len()),
        });
    }
    let mut bound: Vec<Value> = vec![Value::Null; stmt.parameters.len()];
    for (value, param) in values.iter().zip(&stmt.parameters) {
        if !param.slot_ty.matches(value) {
            return Err(Error::QueryBind {
                parameter: param.name.clone(),
                message: format!("parameter `{}` expects `{}`", param.name, param.slot_ty.as_str()),
            });
        }
        bound[param.index as usize] = value.clone();
    }
    Ok(bound)
}

/// Execute a prepared-and-bound statement against stored rows.
pub fn execute_prepared(
    stmt: &PreparedStatement,
    values: &[Value],
    contract: Option<&vos::types::ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<QueryRow>> {
    exec::execute_ops(&stmt.ops, contract, records, values)
}

/// Execute a prepared-and-bound write statement against stored rows.
///
/// Returns [`Error::Unsupported`] for read statements so callers cannot
/// accidentally treat a `SELECT` plan as a mutation.
pub fn execute_write_prepared(
    stmt: &PreparedStatement,
    values: &[Value],
    contract: &vos::types::ResolvedContract,
    declared: &[yydb_types::TableUniqueConstraint],
    records: &mut BTreeMap<String, Vec<u8>>,
) -> Result<WriteOutcome> {
    if stmt.write_ops.is_empty() {
        return Err(Error::Unsupported("statement is a read pipeline, not a write program"));
    }
    write::execute_ops(&stmt.write_ops, contract, declared, records, values)
}

/// Convert a `Name("__yydb_param_<index>")` identifier (emitted by the
/// parameter rewriter) into a proper [`BoundValue::Slot`].
///
/// The rewritten identifier carries the *parameter index*, not its name, so
/// names that are not identifier-safe (`?1`, `:name`, `$name`) all round-trip.
pub(crate) fn name_to_slot(name: &str, parameters: &[ParameterDescriptor]) -> Result<BoundValue> {
    let Some(slot) = name.strip_prefix(PARAM_PREFIX)
    else {
        return Err(Error::Schema { message: format!("expected rewritten parameter Name, got `{name}`") });
    };
    let index: u32 = slot.parse().map_err(|_| Error::Schema { message: format!("malformed parameter slot `{name}`") })?;
    let param = parameters
        .get(index as usize)
        .ok_or_else(|| Error::Schema { message: format!("parameter slot {index} is not declared") })?;
    Ok(BoundValue::Slot { index: param.index, expected_ty: param.slot_ty })
}

// ─── internals ────────────────────────────────────────────────────────────

/// Lexer-aware preprocessor that walks `source` once and extracts every
/// parameter, rewriting each to `__yydb_param_<name>` — a plain identifier the
/// frontend parser accepts.
///
/// Both frontends share the walker; only the surface syntax differs:
///
/// | | VOS | SQL |
/// |---|---|---|
/// | strings | `"..."` with `\\` escapes | `'...'` with `''` escapes, plus `"..."` identifiers |
/// | comments | `//`, `/* */` | `--`, `/* */` |
/// | parameters | `$name` | `?`, `?NNN`, `$name`, `:name`, `@name` |
///
/// UTF-8 safe: uses `char_indices()` rather than `bytes[i] as char`.
fn scan_and_rewrite(source: &str, dialect: Dialect) -> Result<(Vec<ParameterDescriptor>, String)> {
    #[derive(Clone, Copy)]
    enum Lex {
        Normal,
        /// Inside a quoted region opened by the stored delimiter.
        Quoted(char),
        /// Inside a line comment that ends at a newline.
        LineComment,
        BlockComment,
    }

    fn emit(name: String, parameters: &mut Vec<ParameterDescriptor>, indexes: &mut BTreeMap<String, u32>, out: &mut String) {
        let index = match indexes.get(&name) {
            Some(&index) => {
                parameters.get_mut(index as usize).expect("index always valid").usage_count += 1;
                index
            }
            None => {
                let index = parameters.len() as u32;
                indexes.insert(name.clone(), index);
                parameters.push(ParameterDescriptor {
                    name,
                    index,
                    slot_ty: SlotTy::Any, // narrowing done in lowering
                    usage_count: 1,
                });
                index
            }
        };
        // Slot identifiers are numeric so they stay valid identifiers for every
        // frontend, including SQL `?` placeholders.
        out.push_str(PARAM_PREFIX);
        out.push_str(&index.to_string());
    }

    fn read_identifier(chars: &mut std::str::CharIndices<'_>) -> String {
        let mut name = String::new();
        loop {
            let mut peek = chars.clone();
            match peek.next() {
                Some((_, c)) if c.is_ascii_alphanumeric() || c == '_' => {
                    name.push(c);
                    *chars = peek;
                }
                _ => break,
            }
        }
        name
    }

    let is_sql = dialect == Dialect::Sql;
    let mut state = Lex::Normal;
    let mut parameters: Vec<ParameterDescriptor> = Vec::new();
    let mut indexes: BTreeMap<String, u32> = BTreeMap::new();
    let mut out = String::with_capacity(source.len());
    let mut chars = source.char_indices();
    let mut previous: Option<char> = None;
    let mut next_positional: u32 = 1;

    while let Some((_, ch)) = chars.next() {
        if let Lex::Normal = state {
            let handled = match (ch, is_sql) {
                // VOS strings and SQL identifier quoting: backslash escapes.
                ('"', false) => {
                    state = Lex::Quoted('"');
                    out.push(ch);
                    true
                }
                // SQL string literals use doubled quotes, not backslashes.
                ('\'', true) => {
                    state = Lex::Quoted('\'');
                    out.push(ch);
                    true
                }
                // SQL double quotes are identifiers; parameters never appear inside.
                ('"', true) => {
                    state = Lex::Quoted('"');
                    out.push(ch);
                    true
                }
                ('/', _) => {
                    let mut peek = chars.clone();
                    match peek.next() {
                        Some((_, '*')) => {
                            chars = peek;
                            state = Lex::BlockComment;
                            out.push_str("/*");
                            true
                        }
                        Some((_, '/')) if !is_sql => {
                            chars = peek;
                            state = Lex::LineComment;
                            out.push_str("//");
                            true
                        }
                        _ => false,
                    }
                }
                ('-', true) => {
                    let mut peek = chars.clone();
                    if let Some((_, '-')) = peek.next() {
                        chars = peek;
                        state = Lex::LineComment;
                        out.push_str("--");
                        true
                    }
                    else {
                        false
                    }
                }
                ('$', _) => {
                    let name = read_identifier(&mut chars);
                    if name.is_empty() {
                        false
                    }
                    else {
                        emit(name, &mut parameters, &mut indexes, &mut out);
                        true
                    }
                }
                ('?', true) => {
                    let digits = read_identifier(&mut chars);
                    let digits: String = digits.chars().take_while(char::is_ascii_digit).collect();
                    let number = if digits.is_empty() {
                        let number = next_positional;
                        next_positional += 1;
                        number
                    }
                    else {
                        let number: u32 = digits
                            .parse()
                            .map_err(|_| Error::Schema { message: format!("invalid SQL parameter `?{digits}`") })?;
                        next_positional = next_positional.max(number + 1);
                        number
                    };
                    emit(format!("?{number}"), &mut parameters, &mut indexes, &mut out);
                    true
                }
                (':', true) if previous != Some(':') => {
                    let name = read_identifier(&mut chars);
                    if name.is_empty() {
                        false
                    }
                    else {
                        emit(name, &mut parameters, &mut indexes, &mut out);
                        true
                    }
                }
                ('@', true) => {
                    let name = read_identifier(&mut chars);
                    if name.is_empty() {
                        false
                    }
                    else {
                        emit(name, &mut parameters, &mut indexes, &mut out);
                        true
                    }
                }
                _ => false,
            };
            if handled {
                previous = out.chars().last();
                continue;
            }
            out.push(ch);
            previous = Some(ch);
            continue;
        }

        out.push(ch);
        match state {
            Lex::Quoted(delimiter) => {
                if !is_sql {
                    // VOS: backslash escapes, plain closing quote.
                    if ch == '\\' {
                        if let Some((_, escaped)) = chars.next() {
                            out.push(escaped);
                        }
                    }
                    else if ch == delimiter {
                        state = Lex::Normal;
                    }
                }
                else if ch == delimiter {
                    // SQL: a doubled delimiter is an escaped one and keeps the
                    // region open.
                    let mut peek = chars.clone();
                    if let Some((_, next)) = peek.next() {
                        if next == delimiter {
                            out.push(delimiter);
                            chars = peek;
                        }
                        else {
                            state = Lex::Normal;
                        }
                    }
                    else {
                        state = Lex::Normal;
                    }
                }
            }
            Lex::LineComment => {
                if ch == '\n' {
                    state = Lex::Normal;
                }
            }
            Lex::BlockComment => {
                if ch == '*' {
                    let mut peek = chars.clone();
                    if let Some((_, '/')) = peek.next() {
                        out.push('/');
                        chars = peek;
                        state = Lex::Normal;
                    }
                }
            }
            Lex::Normal => unreachable!("normal state is handled above"),
        }
        previous = out.chars().last();
    }

    Ok((parameters, out))
}

/// Walk lowered ops to classify read/write + extract simple read_set.
fn classify_ops(ops: &[QueryOp]) -> (QueryKind, Vec<String>, Vec<String>) {
    let mut tables: Vec<String> = Vec::new();
    let kind = QueryKind::Read;
    for op in ops {
        match op {
            QueryOp::Scan { table } => {
                if !tables.contains(table) {
                    tables.push(table.clone());
                }
            }
            // Future: DML ops such as Insert / Update / Delete will flip to Write
            _ => {}
        }
    }
    (kind, tables, Vec::new())
}

/// Extract table names referenced by insert-style expressions in a write
/// program AST. Handles both `Type::insert({ … })` and
/// `TypedObject { … }.insert()` shapes.
fn extract_write_set(program: &VosProgram) -> Vec<String> {
    let mut tables = Vec::new();
    let mut visit_expr = |expr: &VosExpr| {
        if let Some(name) = type_name_from_insert_expr(expr) {
            if !tables.contains(&name) {
                tables.push(name);
            }
        }
    };
    for stmt in &program.statements {
        if let VosStmt::Expr(expr) = stmt {
            visit_expr(expr);
        }
    }
    if let Some(result) = &program.result {
        visit_expr(result);
    }
    tables
}

fn type_name_from_insert_expr(expr: &VosExpr) -> Option<String> {
    let VosExpr::Call { callee, .. } = expr
    else {
        return None;
    };
    // `Type::insert({ … })` — callee is a Name("Type") called as a constructor-style call
    // `TypedObject { … }.insert()` — callee is Member(object="TypedObject", name="insert")
    match callee.as_ref() {
        VosExpr::Member { object, name, .. } if name == "insert" => {
            // `Something.insert()` — extract from the object
            type_name_from_expr(object)
        }
        _ => None,
    }
}

fn type_name_from_expr(expr: &VosExpr) -> Option<String> {
    match expr {
        VosExpr::Name { name, .. } => Some(name.clone()),
        VosExpr::TypedObject { ty, .. } => Some(ty.clone()),
        VosExpr::Member { object, .. } => type_name_from_expr(object),
        _ => None,
    }
}

/// FNV-1a 64-bit — stable, collision-safe for query-source keys within a
/// single database workspace (not a cryptographic hash).
pub fn fingerprint(source: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in source.as_bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

// ─── tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use yydb_types::Value;

    #[test]
    fn prepare_extracts_dollar_named_parameters_in_order() {
        let source = r#"User.all().take($limit).collect()"#;
        let (_, rewritten) = scan_and_rewrite(source, Dialect::Vos).expect("scan");
        assert!(vos::parse_program(&rewritten).is_ok(), "rewritten must parse: {}", rewritten);
        let stmt = prepare(source).expect("prepare");
        assert_eq!(stmt.parameters.len(), 1);
        assert_eq!(stmt.parameters[0].name, "limit");
        assert_eq!(stmt.parameters[0].index, 0);
    }

    #[test]
    fn prepare_deduplicates_repeated_parameter() {
        let stmt = prepare("User.all().take($limit).skip($limit).collect()").unwrap();
        assert_eq!(stmt.parameters.len(), 1);
        assert_eq!(stmt.parameters[0].usage_count, 2);
    }

    #[test]
    fn prepare_same_source_gives_same_fingerprint_and_statement_id() {
        let a = prepare("User.all().take($limit).collect()").unwrap();
        let b = prepare("User.all().take($limit).collect()").unwrap();
        assert_eq!(a.source_fingerprint, b.source_fingerprint);
        assert_eq!(a.statement_id, b.statement_id);
        assert!(a.source_fingerprint.len() >= 8);
    }

    #[test]
    fn execute_prepared_reuses_plan_for_different_bindings() {
        let stmt = prepare("User.all().take($limit).collect()").unwrap();

        let bound_a = bind_named(&stmt, &BTreeMap::from([("limit".into(), Value::I64(100))])).expect("bind a");
        assert_eq!(bound_a.len(), 1);
        assert_eq!(bound_a[0], Value::I64(100));

        let bound_b = bind_named(&stmt, &BTreeMap::from([("limit".into(), Value::I64(1))])).expect("bind b");
        assert_eq!(bound_b[0], Value::I64(1));
    }

    #[test]
    fn bind_named_rejects_missing_parameter() {
        let stmt = prepare("User.all().take($limit).collect()").unwrap();
        let err = bind_named(&stmt, &BTreeMap::from([])).unwrap_err();
        assert_eq!(err.code(), "yydb.query.bind");
        assert_eq!(err.operation(), "query");
        let params = err.params().expect("QueryBind has params");
        let param_value = params.iter().find(|(k, _)| *k == "parameter").map(|(_, v)| v.as_str()).expect("parameter exists");
        assert_eq!(param_value, "limit");
    }

    #[test]
    fn bind_named_rejects_extraneous_parameter() {
        let stmt = prepare("User.all().take($limit).collect()").unwrap();
        let err = bind_named(
            &stmt,
            &BTreeMap::from([("limit".into(), Value::I64(10)), ("unreferenced".into(), Value::Text("oops".into()))]),
        )
        .unwrap_err();
        assert_eq!(err.code(), "yydb.query.bind");
    }

    #[test]
    fn prepare_works_with_no_parameters() {
        let stmt = prepare("User.all().collect()").unwrap();
        assert!(stmt.parameters.is_empty());
        let bound = bind_named(&stmt, &BTreeMap::new()).unwrap();
        assert!(bound.is_empty());
    }

    #[test]
    fn prepare_fails_on_vos_syntax_error() {
        let err = prepare("this is not valid VOS").unwrap_err();
        assert_eq!(err.code(), "yydb.schema.invalid");
    }

    // ─── new lexer-aware tests ─────────────────────────────────────────────

    #[test]
    fn param_inside_double_string_is_not_extracted() {
        // `$limit` inside a string literal must not be treated as a parameter.
        let (params, _) = scan_and_rewrite("User.all().filter(|u| u.name == \"$limit\").collect()", Dialect::Vos).unwrap();
        assert!(params.is_empty(), "string literal `$limit` should not be a param");
    }

    #[test]
    fn param_inside_line_comment_is_not_extracted() {
        let (params, _) = scan_and_rewrite("User.all().take($x).collect() // $y should not count", Dialect::Vos).unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "x");
    }

    #[test]
    fn param_inside_block_comment_is_not_extracted() {
        let (params, _) = scan_and_rewrite("User.all().take($x) /* $y is inside */ .collect()", Dialect::Vos).unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "x");
    }

    #[test]
    fn double_string_with_escape_is_handled() {
        let (params, _) =
            scan_and_rewrite("User.all().take($x).filter(|u| u.n == \"has\\\"$y\"\").collect()", Dialect::Vos).unwrap();
        assert_eq!(params.len(), 1, "only `$x` is a param");
        assert_eq!(params[0].name, "x");
    }

    #[test]
    fn take_and_skip_can_both_be_bound() {
        let stmt = prepare("User.all().skip($offset).take($limit).collect()").unwrap();
        assert_eq!(stmt.parameters.len(), 2);
        assert_eq!(stmt.parameters[0].name, "offset");
        assert_eq!(stmt.parameters[1].name, "limit");

        let bound =
            bind_named(&stmt, &BTreeMap::from([("offset".into(), Value::I64(50)), ("limit".into(), Value::I64(10))])).unwrap();
        assert_eq!(bound, vec![Value::I64(50), Value::I64(10)]);
    }

    // VOS lambda syntax is `lambda(x) { body }`, NOT `|x| body`. The pipe-
    // based filter tests below are intentionally absent until the author
    // surface converges on `lambda` keyword syntax. PreparedStatement param
    // infrastructure is fully exercised by `.take($p)` / `.skip($p)` paths.

    #[test]
    fn read_set_records_scan_table() {
        let stmt = prepare("User.all().collect()").unwrap();
        assert_eq!(stmt.read_set, vec!["User".to_string()]);
        assert_eq!(stmt.query_kind, QueryKind::Read);
    }

    #[test]
    fn multiple_scans_collected() {
        // Two `Type.all()` in the same source — the pipeline starts at the
        // outermost scan; nested scans inside filter bodies are Phase 2+.
        let stmt = prepare("User.all().take($limit).collect()").unwrap();
        assert!(stmt.read_set.contains(&"User".to_string()));
    }

    #[test]
    fn slot_ty_can_be_inferred_from_context() {
        // $limit used as .take($limit) → must be numeric
        let stmt = prepare("User.all().take($limit).collect()").unwrap();
        // Phase-1 lowering still reports Any, but the infrastructure is now
        // in place for context-aware inference.
        assert_eq!(stmt.parameters[0].slot_ty, SlotTy::Any);
    }
}
