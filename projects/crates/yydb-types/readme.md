# `yydb-types`

Bottom shared types for the YYDB workspace: `Error`, `Value`, schema identity, and CAS refs. Engine crates build on this
package.

Depend on [`yydb`](https://crates.io/crates/yydb) unless you extend engine internals or author a binding layer — the
facade re-exports these types (`yydb::Value`, `yydb::Error`, `yydb::types`, …).

[docs.rs](https://docs.rs/yydb-types) · [YYDB overview](https://github.com/yy-database/yydb.rs)
