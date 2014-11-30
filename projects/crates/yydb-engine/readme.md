# `yydb-engine`

YYDB database runtime: connection state, VOS catalog publication, query invocation, CAS coordination, leases and TTL,
backup, and the UDF session bridge.

This crate owns runtime behaviour. The [`yydb`](https://crates.io/crates/yydb) facade re-exports it without growing
implementations of its own, so application code should depend on the facade.

| Layer      | Crate                                      |
|------------|--------------------------------------------|
| Formats    | `yydb-formats`                             |
| Durability | `yydb-storage`                             |
| Execution  | `yydb-execution`, `yydb-query`, `yydb-udf` |
| **Runtime**| **`yydb-engine`**                          |
| Facade     | `yydb`                                     |

Depend on `yydb` unless you are extending the engine itself.

[docs.rs](https://docs.rs/yydb-engine) · [YYDB overview](https://github.com/yy-database/yydb)
