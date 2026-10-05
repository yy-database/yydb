# `yydb-query`

Phase 1 VOS query lowering, read pipelines, and insert/write DML against the YYDB record map.

Embedded hosts should depend on [`yydb`](https://crates.io/crates/yydb) and use `yydb::query` (this crate re-exported).
Direct `yydb-query` is for engine work and tests only.

[docs.rs](https://docs.rs/yydb-query) · [YYDB overview](https://github.com/yy-database/yydb.rs)
