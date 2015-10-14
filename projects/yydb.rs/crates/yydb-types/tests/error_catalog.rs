//! Keeps the published error-code catalog in sync with `Error::code()`.
//!
//! `Error::code()` in `src/lib.rs` is the only place that mints engine codes.
//! The human catalog in the homepage guide must list every one of them, so a
//! new variant cannot ship without a public explanation.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> Option<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest.ancestors().nth(4).map(Path::to_path_buf)
}

fn engine_codes() -> BTreeSet<String> {
    let source = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("crate source is readable");
    source
        .lines()
        .filter(|line| line.contains("=> \"yydb."))
        .filter_map(|line| {
            let start = line.find("=> \"")? + 4;
            let rest = &line[start..];
            let end = rest.find('"')?;
            Some(rest[..end].to_string())
        })
        .collect()
}

fn catalog_codes(doc: &str) -> BTreeSet<String> {
    doc.lines()
        .filter(|line| line.starts_with("| `yydb."))
        .filter_map(|line| {
            let start = line.find('`')? + 1;
            let rest = &line[start..];
            let end = rest.find('`')?;
            Some(rest[..end].to_string())
        })
        .collect()
}

#[test]
fn engine_codes_are_unique() {
    let codes = engine_codes();
    assert_eq!(codes.len(), 31, "code count changed: {codes:?}");
    for code in &codes {
        assert!(
            code.starts_with("yydb."),
            "`{code}` must stay inside the `yydb.` namespace"
        );
    }
}

#[test]
fn published_catalogs_list_every_engine_code() {
    let Some(root) = repo_root() else {
        return;
    };
    let docs = [
        root.join("projects/yydb.ts/homepage/documents/zh-hans/guide/10-errors.md"),
        root.join("projects/yydb.ts/homepage/documents/en/guide/10-errors.md"),
    ];

    let codes = engine_codes();
    for doc in docs {
        if !doc.exists() {
            continue;
        }
        let text = fs::read_to_string(&doc).expect("catalog is readable");
        let documented = catalog_codes(&text);
        let missing: Vec<_> = codes.difference(&documented).collect();
        assert!(
            missing.is_empty(),
            "{} is missing engine codes: {missing:?}",
            doc.display()
        );
    }
}
