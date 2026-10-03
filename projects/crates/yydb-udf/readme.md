# yydb-udf

UDF contract, layered registry, and host adapters. Sits above
[`yydb-execution`](../yydb-execution/readme.md) and below the
[`yydb`](../yydb/readme.md) facade. Not shared with YYDS.

Applications should depend on **`yydb`** for host UDF types (`HostMicroDefinition`,
`UdfValue`, …). Use `yydb::udf` for both Rust scalar traits and
`yydb::udf::UdfRegistry`. The `yydb::yydb_udf` crate re-export is for engine
authors only.
