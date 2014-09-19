# `yydb-client`

Rust remote client for `yydb serve` (TCP binary wire). Types and wire codecs come from [
`yydb-types`](https://crates.io/crates/yydb-types) and [
`yydb-protocol`](https://crates.io/crates/yydb-protocol); this crate never depends on the embedded facade.

For in-process access use `yydb::Connection`. TypeScript and browser hosts use [
`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client).

[docs.rs](https://docs.rs/yydb-client) · [YYDB overview](https://github.com/yy-database/yydb.rs)
