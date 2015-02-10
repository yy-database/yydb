# `yydb-execution`

Stable local execution model owned by YYDB. VOS frontends lower into this IR through the
[`yydb`](https://crates.io/crates/yydb) binder, so planners never share an unstable AST across crates.

The crate deliberately has no dependency on Oak, VOS, YYDS, or a remote execution protocol.

Depend on **`yydb`** (`yydb::execution`) unless you extend the execution layer directly.

[docs.rs](https://docs.rs/yydb-execution) · [YYDB overview](https://github.com/yy-database/yydb)
