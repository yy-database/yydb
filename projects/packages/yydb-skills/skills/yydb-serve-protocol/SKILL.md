---
name: yydb-serve-protocol
description: >-
    YY wire protocol for yydb serve: frame layout, message types, threat model, TCP/WebSocket.
    Load when changing yydb-server, @yydb/yydb-client wire codec, or serve security.
---

# YY wire protocol (VOS-native serve)

Binary request/response framing for **YY-family** servers that speak VOS-native serve semantics.

## Prefix: product magic + version digits

| Bytes | Field         | Values                                                      |
|-------|---------------|-------------------------------------------------------------|
| 0–3   | product magic | ASCII `YYDB` or `YYDS`                                      |
| 4–7   | wire version  | four ASCII digits: **`0000`** (current), then **`0001`**, … |

`YYDB` / `YYDS` are a **backend self-claim** only. Wire semantics are the same; frontends accept either. Version digits
gate compatibility.

Reference host: `yydb serve` (`yydb-server`, `@yydb/yydb` CLI) encodes `YYDB` + `0000`. `@yydb/yydb-client` and WebUI
use the same layout. **Not** HTTP JSON or SQL.

## Threat model (no ACL)

Reference `yydb serve` has **no** login, tenant, role, or grants. Anyone who can open the socket can read/write the
database.

- Default bind: **loopback only** (`127.0.0.1` / `::1`)
- Non-loopback requires `--insecure-bind` with loud warning
- Trust boundary = process placement + OS permissions on `.yydb`
- Public homepage must not open a live serve socket

**YYDS** is the distributed product with permissions/audit. Do not implement that stack inside YYDB.

## Transport

| Mode      | Endpoint              | Notes                      |
|-----------|-----------------------|----------------------------|
| TCP       | `host:port`           | Rust / Node native clients |
| WebSocket | `ws://host:port/wire` | Same port; browser WebUI   |

WebSocket carries **identical binary frames** after HTTP upgrade.

## Frame layout (little-endian after 8-byte prefix)

| Offset | Size | Field                                |
|--------|------|--------------------------------------|
| 0      | 4    | product magic `YYDB` \| `YYDS`       |
| 4      | 4    | version digits `0000` \| `0001` \| … |
| 8      | 2    | `msg_type`                           |
| 10     | 2    | `flags` (reserved, `0` in `0000`)    |
| 12     | 4    | `request_id`                         |
| 16     | 4    | `body_len`                           |
| 20     | N    | `body`                               |

Max body: **16 MiB**. Unknown magic or unsupported version → reject.

## Message types (`0000`)

| Code | Name                | Direction | Body                                                |
|------|---------------------|-----------|-----------------------------------------------------|
| 1    | `Hello`             | C→S       | empty                                               |
| 2    | `HelloOk`           | S→C       | UTF-8 server version                                |
| 3    | `Info`              | C→S       | empty                                               |
| 4    | `InfoOk`            | S→C       | UTF-8 diagnostics                                   |
| 5    | `SchemaGet`         | C→S       | empty                                               |
| 6    | `SchemaGetOk`       | S→C       | `u8 present`; if 1: version + VOS doc               |
| 7    | `SchemaEnsure`      | C→S       | `u32 version` + VOS doc (server uses document only) |
| 8    | `SchemaEnsureOk`    | S→C       | empty                                               |
| 9    | `KvGet`             | C→S       | key                                                 |
| 10   | `KvGetOk`           | S→C       | optional value                                      |
| 11   | `KvPut`             | C→S       | key + value                                         |
| 12   | `KvPutOk`           | S→C       | empty                                               |
| 13   | `MicroRegister`     | C→S       | session TS micro metadata                           |
| 14   | `MicroRegisterOk`   | S→C       | empty                                               |
| 15   | `MicroHostInvoke`   | S→C       | host callback during another request                |
| 16   | `MicroHostInvokeOk` | C→S       | scalar result                                       |
| 17   | `ScalarCall`        | C→S       | scalar UDF invoke                                   |
| 18   | `ScalarCallOk`      | S→C       | scalar result                                       |
| 255  | `Error`             | S→C       | UTF-8 message                                       |

TCP clients must handle server-pushed `MicroHostInvoke` before their own response completes.

## Out of scope for `0000`

Authentication/RBAC, full VOS query RPC, CAS object streaming, TLS as product surface, `0001` layout (when shipped).
