# yydb-types

Bottom shared types for YYDB: `Error`, `Value`, schema identity, and CAS refs.
Engine crates (`yydb-query`, `yydb-udf`, …) build on this package.

**Applications should depend on [`yydb`](../yydb/readme.md)** — the facade re-exports
these types at the crate root (`yydb::Value`, `yydb::Error`, `yydb::types`, …).
Take a direct `yydb-types` dependency only when extending engine internals or
authoring a new binding layer.
