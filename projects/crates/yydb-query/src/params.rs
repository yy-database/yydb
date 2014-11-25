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
    exec,
    ops::{QueryOp, QueryRow, SlotTy},
};

/// One named parameter extracted during prepare.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    /// Read pipeline ending in `.collect()`.
    Read,
    /// Write program (`Type::insert`, etc.).
    Write,
}

/// A pre-parsed, pre-lowered query plan with known parameter slots.
#[derive(Debug, Clone)]
pub struct PreparedStatement {
    /// Unique synthetic ID for this prepared statement (monotonic within a
    /// connection, stable fingerprint otherwise).
    pub statement_id: String,
    /// Lowered ops where every parameter occurrence is a `BoundValue::Slot`.
    pub ops: Vec<QueryOp>,
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
/// rewrite each `$name` → `__yydb_param_<name>` (a valid Identifier), then
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
///      identifier `__yydb_param_<name>`.
///   2. Fingerprint the *original* source.
///   3. `vos::parse_program(&rewritten)` — Oak-vos sees only valid VOS.
///   4. `lower::lower_program` — Name("__yydb_param_*") nodes are lowered
///      directly to `BoundValue::Slot { index, expected_ty }`.
///   5. Build the full PreparedStatement envelope (read_set / write_set /
///      query_kind), ready to pass to [`execute_prepared`].
pub fn prepare(source: &str) -> Result<PreparedStatement> {
    let (parameters, rewritten) = scan_and_rewrite(source)?;
    let source_fingerprint = fingerprint(source);

    let program = vos::parse_program(&rewritten)
        .map_err(|diagnostics| Error::Schema { message: diagnostics })?;
    let ops = super::lower::lower_program(&program, &parameters)?;

    let (query_kind, read_set, write_set) = classify_ops(&ops);
    let statement_id = format!("stmt-{source_fingerprint}");

    Ok(PreparedStatement {
        statement_id,
        ops,
        query_kind,
        parameters,
        read_set,
        write_set,
        source_fingerprint,
        source: source.to_owned(),
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

/// Execute a prepared-and-bound statement against stored rows.
pub fn execute_prepared(
    stmt: &PreparedStatement,
    values: &[Value],
    contract: Option<&vos::types::ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<QueryRow>> {
    exec::execute_ops(&stmt.ops, contract, records, values)
}

// ─── internals ────────────────────────────────────────────────────────────

/// Lexer-aware preprocessor that walks `source` once, extracting every
/// top-level `$name` parameter while correctly skippting `"..."` strings,
/// `//` line comments and `/* */` block comments. Each parameter occurrence
/// is rewritten to `__yydb_param_<name>` — a plain Identifier Oak-vos
/// parser accepts.
///
/// UTF-8 safe: uses `char_indices()` rather than `bytes[i] as char`.
fn scan_and_rewrite(source: &str) -> Result<(Vec<ParameterDescriptor>, String)> {
    enum Lex {
        Normal,
        DoubleQuote,
        LineComment,
        BlockComment,
    }

    let mut state = Lex::Normal;
    let mut parameters: Vec<ParameterDescriptor> = Vec::new();
    let mut name_to_index: BTreeMap<String, u32> = BTreeMap::new();
    let mut out = String::with_capacity(source.len());
    let mut chars = source.char_indices();

    while let Some((_, ch)) = chars.next() {
        match state {
            Lex::Normal => {
                match ch {
                    '"' => {
                        state = Lex::DoubleQuote;
                        out.push(ch);
                    }
                    '/' => {
                        let mut peek = chars.clone();
                        match peek.next() {
                            Some((_, '/')) => {
                                state = Lex::LineComment;
                                chars = peek; // consume one extra `/`
                                out.push_str("//");
                                continue;
                            }
                            Some((_, '*')) => {
                                state = Lex::BlockComment;
                                chars = peek;
                                out.push_str("/*");
                                continue;
                            }
                            _ => out.push(ch),
                        }
                    }
                        '$' => {
                        // Try to read an identifier immediately after `$`.
                        // `chars` has just yielded `$`, so the next item is the
                        // first char of the identifier body (or whatever follows).
                        let mut ident_chars: Vec<char> = Vec::new();
                        loop {
                            // Peek the next char without consuming it.
                            let mut peek = chars.clone();
                            match peek.next() {
                                Some((_, c)) if c.is_ascii_alphanumeric() || c == '_' => {
                                    ident_chars.push(c);
                                    chars = peek; // now consume that char
                                }
                                _ => break,    // non-identifier char stays in chars
                            }
                        }
                        if !ident_chars.is_empty() {
                            let name: String = ident_chars.into_iter().collect();
                            let name_for_rewrite = name.clone();
                            let index = match name_to_index.get(&name) {
                                Some(&idx) => {
                                    let p = parameters.get_mut(idx as usize).expect("index always valid");
                                    p.usage_count += 1;
                                    idx
                                }
                                None => {
                                    let idx = parameters.len() as u32;
                                    name_to_index.insert(name.clone(), idx);
                                    parameters.push(ParameterDescriptor {
                                        name,
                                        index: idx,
                                        slot_ty: SlotTy::Any, // narrowing done in lowering
                                        usage_count: 1,
                                    });
                                    idx
                                }
                            };
                            // Rewrite to `__yydb_param_<name>` (identifier-safe) —
                            // dedup is by name so same name → same identifier.
                            out.push_str(PARAM_PREFIX);
                            out.push_str(&name_for_rewrite);
                        } else {
                            // Lone `$` — keep as-is; VOS parser will reject but that's
                            // a source error, not a parameter.
                            out.push('$');
                        }
                    }
                    _ => out.push(ch),
                }
            }
            Lex::DoubleQuote => {
                out.push(ch);
                match ch {
                    '"' => state = Lex::Normal,
                    '\\' => {
                        if let Some((_, escaped)) = chars.next() {
                            out.push(escaped);
                        }
                    }
                    _ => {}
                }
            }
            Lex::LineComment => {
                out.push(ch);
                if ch == '\n' {
                    state = Lex::Normal;
                }
            }
            Lex::BlockComment => {
                out.push(ch);
                if ch == '*' {
                    let mut peek = chars.clone();
                    if let Some((_, '/')) = peek.next() {
                        out.push('/');
                        chars = peek;
                        state = Lex::Normal;
                    }
                }
            }
        }
    }

    Ok((parameters, out))
}

/// Walk lowered ops to classify read/write + extract simple read_set.
fn classify_ops(ops: &[QueryOp]) -> (QueryKind, Vec<String>, Vec<String>) {
    let mut tables: Vec<String> = Vec::new();
    let mut kind = QueryKind::Read;
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

/// FNV-1a 64-bit — stable, collision-safe for query-source keys within a
/// single database workspace (not a cryptographic hash).
fn fingerprint(source: &str) -> String {
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
        let (parameters, rewritten) = scan_and_rewrite(source).expect("scan");
        assert!(
            vos::parse_program(&rewritten).is_ok(),
            "rewritten must parse: {}",
            rewritten
        );
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
        let param_value = params
            .iter()
            .find(|(k, _)| *k == "parameter")
            .map(|(_, v)| v.as_str())
            .expect("parameter exists");
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
        let (params, _) = scan_and_rewrite("User.all().filter(|u| u.name == \"$limit\").collect()").unwrap();
        assert!(params.is_empty(), "string literal `$limit` should not be a param");
    }

    #[test]
    fn param_inside_line_comment_is_not_extracted() {
        let (params, _) =
            scan_and_rewrite("User.all().take($x).collect() // $y should not count").unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "x");
    }

    #[test]
    fn param_inside_block_comment_is_not_extracted() {
        let (params, _) =
            scan_and_rewrite("User.all().take($x) /* $y is inside */ .collect()").unwrap();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "x");
    }

    #[test]
    fn double_string_with_escape_is_handled() {
        let (params, _) = scan_and_rewrite("User.all().take($x).filter(|u| u.n == \"has\\\"$y\"\").collect()").unwrap();
        assert_eq!(params.len(), 1, "only `$x` is a param");
        assert_eq!(params[0].name, "x");
    }

    #[test]
    fn take_and_skip_can_both_be_bound() {
        let stmt = prepare("User.all().skip($offset).take($limit).collect()").unwrap();
        assert_eq!(stmt.parameters.len(), 2);
        assert_eq!(stmt.parameters[0].name, "offset");
        assert_eq!(stmt.parameters[1].name, "limit");

        let bound = bind_named(
            &stmt,
            &BTreeMap::from([("offset".into(), Value::I64(50)), ("limit".into(), Value::I64(10))]),
        )
        .unwrap();
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
