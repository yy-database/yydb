# `yydb-server`

Loopback-first YY wire server for out-of-process clients (TCP plus WebSocket `/wire`). It depends on
[`yydb-engine`](https://crates.io/crates/yydb-engine) for execution and
[`yydb-protocol`](https://crates.io/crates/yydb-protocol) for framing.

Started by language bindings (`yydb-napi` / `yydb-pyo3` `serve`) or custom hosts. **Not** an ACL-protected network
service: bind loopback by default and require an explicit opt-in for any other address.

| Host                | Surface                                                                |
|---------------------|------------------------------------------------------------------------|
| Embedded Rust       | [`yydb`](https://crates.io/crates/yydb) (`Connection`)                 |
| Node / TS           | [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb)               |
| Rust remote client  | [`yydb-client`](https://crates.io/crates/yydb-client)                  |
| TS / browser client | [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) |

Wire protocol: [`yydb-serve-protocol`](https://github.com/yy-database/yydb/tree/dev/projects/yydb.ts/packages/yydb-skills/skills/yydb-serve-protocol/SKILL.md)
in [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills).

[YYDB overview](https://github.com/yy-database/yydb)
