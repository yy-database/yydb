//! VOS schema documents shared with [`vos`](https://github.com/voml/vos-language)
//! (`git`, branch `dev`).
//!
//! YYDB uses **VOS for DDL and query**. It does **not** invent a private schema
//! dialect. Database-truth documents stored by
//! [`crate::Connection::ensure_schema`] are VOS source text; semantic checking
//! goes through the `vos` facade so YYDB / YYDS / tooling stay aligned as
//! `vos-parser` grows.

use yydb_types::{Error, Result};

/// Canonical remote used by this workspace for shared VOS semantics.
pub const VOS_GIT_DEV: &str = "https://github.com/voml/vos-language.git#branch=dev";

/// Validate a schema document before it becomes database truth.
///
/// Today `vos-parser` on `dev` is still scaffolding, so this performs host-side
/// checks and keeps the git-backed `vos` crate linked as the language
/// authority. When `vos::parser` exposes a stable check/parse API, this
/// function will call into it and map diagnostics to [`Error::Schema`].
pub fn validate_document(document: &str) -> Result<()> {
    // Force-link the voml/vos-language @dev facade (shared semantics).
    let _span = vos::ast::Span {
        start: 0,
        end: document.len(),
    };
    let _ = core::any::type_name::<vos::ast::Span>();

    if document.contains('\0') {
        return Err(Error::Schema {
            message: "VOS schema document must not contain NUL bytes".into(),
        });
    }
    Ok(())
}
