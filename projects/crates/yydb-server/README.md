# `yydb-server`

Loopback-first **YY wire server** for out-of-process clients (TCP + WebSocket `/wire`).

| Host | Crate / package |
|------|-----------------|
| Embedded Rust | [`yydb`](../yydb) (`Connection`) |
| Managed Node (CLI + engine + serve) | [`@yydb/yydb`](../../packages/yydb) |
| Lightweight remote client | [`@yydb/yydb-client`](../../packages/yydb-client) or [`yydb-client`](../yydb-client) |

This crate is the Rust serve loop used by `@yydb/yydb` (`yydb serve` via N-API). It is **not** an ACL-protected
network service — bind loopback by default.

See [`documentation/serve-protocol.md`](../../../documentation/serve-protocol.md).
