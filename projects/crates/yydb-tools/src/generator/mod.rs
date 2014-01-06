//! VOS → TypeScript codegen (`yydb generate`).
//!
//! Pipeline: VOS source → [`TsSchemaIr`] → OXC AST → `oxc_codegen` text.
//! String-concatenation of TypeScript declarations is intentionally forbidden
//! in the emit path (see [`emit_oxc`]).

mod emit_oxc;
mod ir;
mod parse_vos;

use emit_oxc::emit_typescript;
use parse_vos::parse_vos_subset;

/// Generate TypeScript from a VOS document.
pub fn generate_typescript(vos_source: &str, schema_version: u32) -> Result<String, Error> {
    let ir = parse_vos_subset(vos_source, schema_version)?;
    emit_typescript(&ir)
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("VOS parse error: {0}")]
    Parse(String),
    #[allow(dead_code)]
    #[error("TypeScript emit error: {0}")]
    Emit(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn setting_fixture_snapshot() {
        let vos = include_str!("../../fixtures/generator/setting.vos");
        let expected = include_str!("../../fixtures/generator/setting.generated.ts");
        let got = generate_typescript(vos, 1).unwrap();
        assert_eq!(normalize_newlines(&got), normalize_newlines(expected));
    }

    fn normalize_newlines(s: &str) -> String {
        s.replace("\r\n", "\n")
    }
}
