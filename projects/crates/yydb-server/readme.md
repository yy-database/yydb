# `yydb-server`

Loopback-first YY wire server for out-of-process clients (TCP + WebSocket `/wire`). Depends on [
`yydb`](https://crates.io/crates/yydb) only.

Started by language bindings (`yydb-napi` / `yydb-pyo3` `serve`) or custom hosts. **Not** an ACL-protected network
service — bind loopback by default.

| Host                | Surface                                                                |
|---------------------|------------------------------------------------------------------------|
| Embedded Rust       | [`yydb`](https://crates.io/crates/yydb) (`Connection`)                 |
| Node / TS           | [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb)               |
| Rust remote client  | [`yydb-client`](https://crates.io/crates/yydb-client)                  |
| TS / browser client | [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) |

Wire protocol: [
`yydb-serve-protocol`](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/yydb-skills/skills/yydb-serve-protocol/SKILL.md)
in [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills).

[YYDB overview](https://github.com/yy-database/yydb.rs)
