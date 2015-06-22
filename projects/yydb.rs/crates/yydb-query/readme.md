# `yydb-query`

Phase 1 VOS query lowering, read pipelines, and insert/write DML against the YYDB record map.

Every write value is coerced to the column type the schema declares. A `vector<N>` column accepts exactly `N` finite
`f32` components and refuses another dimension, an empty vector, or a non-finite component before the row is staged,
so a stored embedding always matches its declared shape. Rows persist the tagged form, so a corrupt or padded component
payload fails the read rather than decoding into a wider or shorter vector.

Embedded hosts should depend on [`yydb`](https://crates.io/crates/yydb) and use `yydb::query` (this crate re-exported).
Direct `yydb-query` is for engine work and tests only.

[docs.rs](https://docs.rs/yydb-query) · [YYDB overview](https://github.com/yy-database/yydb)
