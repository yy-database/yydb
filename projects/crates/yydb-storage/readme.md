# `yydb-storage`

Durable YYDB file access: main-file lifecycle, writer locking, snapshot commit, WAL and checkpoint coordination,
durable CAS blob I/O, and storage metrics.

The crate knows the physical `YDPG` / `YYWL` / `YYBB` v0 layout. It does not know VOS, query planning, UDFs, or
`Connection`, and it never receives engine state: callers hand over opaque metadata rows plus logical records.

Workspace-internal and not published. Application authors depend on [`yydb`](https://crates.io/crates/yydb).

[YYDB overview](https://github.com/yy-database/yydb)
