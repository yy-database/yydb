# `yydb-protocol`

YY wire protocol: frame layout, message bodies, and the `0000` version prefix, shared by
[`yydb-server`](https://crates.io/crates/yydb-server) and [`yydb-client`](https://crates.io/crates/yydb-client).

This crate never opens a database, never dispatches a request, and never depends on the embedded engine. Both
`YYDB` and `YYDS` hosts claim the same wire format; the product magic only identifies the peer.

Using the facade, you normally reach these types through `yydb::protocol`.

[docs.rs](https://docs.rs/yydb-protocol) · [YYDB overview](https://github.com/yy-database/yydb)
