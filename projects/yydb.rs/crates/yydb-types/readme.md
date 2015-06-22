# `yydb-types`

Bottom shared types for the YYDB workspace: `Error`, `Value`, schema identity, and CAS refs. Engine crates build on this
package.

Binary payloads, UUIDs, temporal values, fixed-point decimals, and `vector<N>` embeddings travel as a tagged JSON
object that keeps their exact bits. A vector tag carries its declared dimension alongside the little-endian `f32`
components, so a payload whose length disagrees with its own dimension, a non-finite component, or an unknown tag is
refused instead of being decoded into a smaller vector or a silently truncated value.

Depend on [`yydb`](https://crates.io/crates/yydb) unless you extend engine internals or author a binding layer — the
facade re-exports these types (`yydb::Value`, `yydb::Error`, `yydb::types`, …).

[docs.rs](https://docs.rs/yydb-types) · [YYDB overview](https://github.com/yy-database/yydb)
