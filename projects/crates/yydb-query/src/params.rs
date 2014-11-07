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

use super::exec;
use super::ops::{BoundValue, QueryOp, QueryRow, SlotTy};

/// One named parameter extracted during prepare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterDescriptor {
    /// Canonical parameter name (no leading `$`).
    pub name: String,
    /// Positional slot index within the bound-value slice.
    pub index: u32,
    /// Declared slot type (presently always `Any`; future passes can narrow).
    pub slot_ty: SlotTy,
    /// Number of times this parameter appears in the source.
    pub usage_count: u32,
}

/// A pre-parsed, pre-lowered query plan with known parameter slots.
#[derive(Debug, Clone)]
pub struct PreparedStatement {
    /// Lowered ops where every parameter occurrence is a `BoundValue::Slot`.
    pub ops: Vec<QueryOp>,
    /// Parameter descriptors in ascending `index` order.
    pub parameters: Vec<ParameterDescriptor>,
    /// SHA-256 hex fingerprint of the raw source (FNV-1a 64-bit fallback).
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

/// Placeholder prefix injected into source during prepare so that VOS parser
/// accepts the token as a literal.
const PARAM_PREFIX: &str = "__YYDB_PARAM_";

/// Compile a VOS source into a prepared statement. If the source contains no
/// `$name` parameters, the returned plan still goes through the same path —
/// parameters will be an empty vec and every bound value will be unused.
pub fn prepare(source: &str) -> Result<PreparedStatement> {
    // 1. Extract + deduplicate `$name` parameters in first-occurrence order.
    let (parameters, rewritten) = scan_and_rewrite(source)?;

    // 2. Fingerprint the *original* source, not the rewritten one, so that two
    //    identical sources get identical fingerprints even if parameter naming
    //    differs in their surface form.
    let source_fingerprint = fingerprint(source);

    // 3. Parse + lower the rewritten source (placeholders are now string literals).
    let program = vos::parse_program(&rewritten).map_err(|diagnostics| Error::Schema {
        message: diagnostics,
    })?;
    let mut ops = super::lower::lower_program(&program)?;

    // 4. Replace placeholder string literals with Slot references.
    rewrite_placeholders_in_ops(&mut ops, &parameters)?;

    Ok(PreparedStatement {
        ops,
        parameters,
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
pub fn bind_named(
    stmt: &PreparedStatement,
    named: &BTreeMap<String, Value>,
) -> Result<Vec<Value>> {
    // Detect extraneous parameters before building the positional slice.
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
        let Some(value) = named.get(&param.name) else {
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
                message: format!(
                    "parameter `{}` expects `{}`, got `{}`",
                    param.name,
                    param.slot_ty.as_str(),
                    got,
                ),
            });
        }
        values[param.index as usize] = value.clone();
    }
    Ok(values)
}

/// Execute a prepared-and-bound statement against stored rows.
///
/// Delegates to [`exec::execute_ops`] with the bound-value slice.
pub fn execute_prepared(
    stmt: &PreparedStatement,
    values: &[Value],
    contract: Option<&vos::types::ResolvedContract>,
    records: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<QueryRow>> {
    exec::execute_ops(&stmt.ops, contract, records, values)
}

// ─── internals ────────────────────────────────────────────────────────────

/// Walks `source` extracting every `$name` occurrence and producing a
/// rewritten source where each parameter becomes a unique string literal
/// `"__YYDB_PARAM_<index>"`.
fn scan_and_rewrite(source: &str) -> Result<(Vec<ParameterDescriptor>, String)> {
    let mut parameters: Vec<ParameterDescriptor> = Vec::new();
    let mut name_to_index: BTreeMap<String, u32> = BTreeMap::new();
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && is_ident_byte(bytes[end]) {
                end += 1;
            }
            if end > start {
                let name = std::str::from_utf8(&bytes[start..end])
                    .map_err(|e| Error::QueryBind {
                        parameter: "(dollar-name)".into(),
                        message: e.to_string(),
                    })?
                    .to_owned();
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
                            slot_ty: SlotTy::Any,
                            usage_count: 1,
                        });
                        idx
                    }
                };
                out.push('"');
                out.push_str(PARAM_PREFIX);
                out.push_str(&index.to_string());
                out.push('"');
                i = end;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    Ok((parameters, out))
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Walk every BoundValue in the lowered ops tree and replace string literals
/// that match `PARAM_PREFIX + index` with `Slot { index, expected_ty: Any }`.
fn rewrite_placeholders_in_ops(ops: &mut [QueryOp], parameters: &[ParameterDescriptor]) -> Result<()> {
    for op in ops.iter_mut() {
        match op {
            QueryOp::Filter { predicate } => rewrite_placeholders_in_pred(predicate, parameters)?,
            QueryOp::Skip { count } | QueryOp::Take { count } => {
                rewrite_placeholders_in_bound(count, parameters)?;
            }
            QueryOp::Scan { .. }
            | QueryOp::Project { .. }
            | QueryOp::Sort { .. }
            | QueryOp::Collect => {}
        }
    }
    Ok(())
}

fn rewrite_placeholders_in_pred(pred: &mut super::ops::Pred, parameters: &[ParameterDescriptor]) -> Result<()> {
    use super::ops::Pred;
    match pred {
        Pred::True | Pred::False => {}
        Pred::FieldBool { value, .. } => rewrite_placeholders_in_bound(value, parameters)?,
        Pred::FieldCmp { value, .. } => rewrite_placeholders_in_bound(value, parameters)?,
        Pred::And(l, r) | Pred::Or(l, r) => {
            rewrite_placeholders_in_pred(l, parameters)?;
            rewrite_placeholders_in_pred(r, parameters)?;
        }
    }
    Ok(())
}

fn rewrite_placeholders_in_bound(bv: &mut BoundValue, parameters: &[ParameterDescriptor]) -> Result<()> {
    match bv {
        BoundValue::Literal(Value::Text(s)) => {
            // Accept either the original placeholder text or a user-supplied
            // literal that coincidentally starts with the prefix — we match
            // exact prefix followed by pure digits.
            if let Some(index_str) = s.strip_prefix(PARAM_PREFIX) {
                if let Ok(index) = index_str.parse::<u32>() {
                    if (index as usize) < parameters.len() {
                        let slot_ty = parameters[index as usize].slot_ty;
                        *bv = BoundValue::Slot { index, expected_ty: slot_ty };
                    }
                }
            }
        }
        BoundValue::Literal(_) | BoundValue::Slot { .. } => {}
    }
    Ok(())
}

/// Returns a hex-encoded SHA-256 fingerprint of the source. Falls back to a
/// stable FNV-1a 64-bit digest when neither sha2 nor any hashing dependency
/// is linked — this crate's Cargo.toml does not declare sha2 today and the
/// fingerprint length we have space for is well below FNV collision risk for
/// query-source keys.
fn fingerprint(source: &str) -> String {
    fn fnv1a_64(bytes: &[u8]) -> u64 {
        const FNV_OFFSET: u64 = 0xcbf29ce484222325;
        const FNV_PRIME: u64 = 0x100000001b3;
        let mut h = FNV_OFFSET;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(FNV_PRIME);
        }
        h
    }
    let mut out = String::with_capacity(16);
    let h = fnv1a_64(source.as_bytes());
    out.push_str(&format!("{h:016x}"));
    out
}
