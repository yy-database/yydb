//! Single integration-test binary for OPFS acceptance + session regression.
//!
//! Filter OPFS fixtures: `cargo test -p yydb-wasm --test main yydb_opfs_`

mod acceptance;
mod fixtures;
mod session;
