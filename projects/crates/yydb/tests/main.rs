//! Single integration-test binary for acceptance + regression suites.
//!
//! Filter acceptance fixtures: `cargo test -p yydb --test main yydb_`

mod acceptance;
mod fixtures;
mod integration;
