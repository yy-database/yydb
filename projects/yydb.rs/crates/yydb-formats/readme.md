# `yydb-formats`

YYDB format v0 parsing, validation, and in-memory page backend: `YDPG` page files, `YYWL` WAL, `YYSH` SHM, and `YYBB`
blob chunk headers.

Workspace-internal crate. It knows the physical layout but not VOS, query planning, or connection state. Application
authors depend on [`yydb`](https://crates.io/crates/yydb).

[docs.rs](https://docs.rs/yydb-formats) · [YYDB overview](https://github.com/yy-database/yydb)
