# yydb-server

Loopback-first **YY wire server** for out-of-process clients (TCP + WebSocket `/wire`).

Depends on [`yydb`](../yydb/readme.md) only. Started by language bindings (`yydb-napi` / `yydb-pyo3` `serve`) or custom
hosts. This is **not** an
ACL-protected network service — bind loopback by default.

| Host                     | Surface                          |
|--------------------------|----------------------------------|
| Embedded Rust            | [`yydb`](../yydb) (`Connection`) |
| Node / TS                | `@yydb/yydb` (`yydb-napi`)       |
| Python                   | `yydb-pyo3`                      |
| Rust remote client       | [`yydb-client`](../yydb-client)  |
| TS / browser wire client | `@yydb/yydb-client`              |

See [`yydb-serve-protocol`](../../packages/yydb-skills/skills/yydb-serve-protocol/SKILL.md) in `@yydb/yydb-skills`.
