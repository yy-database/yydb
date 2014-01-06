# YY wire protocol (VOS-native serve)

Binary request/response framing for **YY-family** servers that speak VOS-native serve semantics.

## Prefix: product magic + version digits

| Bytes | Field         | Values                                                      |
|-------|---------------|-------------------------------------------------------------|
| 0–3   | product magic | ASCII `YYDB` or `YYDS`                                      |
| 4–7   | wire version  | four ASCII digits: **`0000`** (current), then **`0001`**, … |

`YYDB` / `YYDS` are a **backend self-claim** only. Wire semantics are the same; frontends accept either and **do not
strongly require** the magic to match any preferred product name. Version digits are what actually gate compatibility.

In this repository the reference host is `yydb serve` and encodes `YYDB` +
`0000`. The TypeScript client (`@yydb/yydb-client`) and WebUI/homepage speak the same layout. This is **not** an HTTP
JSON API and **not** a SQL dialect.

## Threat model (no ACL) — YYDB reference serve

The reference `yydb serve` has **no** login, tenant, role, or grant system. Anyone who can open the socket can read and
write the opened database.

Therefore:

- Default bind is **loopback only** (`127.0.0.1` / `::1`).
- Binding a non-loopback address requires `--insecure-bind` and prints a loud warning. Do **not** expose `yydb serve` on
  the public internet.
- Trust boundary = process placement + OS file permissions on the `.yydb` file.
- The public homepage must not open a live serve socket.

**YYDS** is the heavy product for distributed large customers: built-in permission management, automatic audit, tuning /
ops reporting, and related control-plane defaults. Do not implement that stack inside YYDB; keep the embed path light. A
peer may still claim magic `YYDS` on this shared wire — frontends treat that as an equivalent self-claim, not as proof
that ACL is present.

## Transport

| Mode      | Endpoint              | Notes                        |
|-----------|-----------------------|------------------------------|
| TCP       | `host:port`           | Native clients (Rust / Node) |
| WebSocket | `ws://host:port/wire` | Same port; browser WebUI     |

WebSocket carries **identical binary frames** after the HTTP upgrade. No parallel JSON control channel.

## Frame layout (little-endian after the 8-byte prefix)

| Offset | Size | Field                                            |
|--------|------|--------------------------------------------------|
| 0      | 4    | product magic `YYDB` \| `YYDS`                   |
| 4      | 4    | version digits `0000` \| `0001` \| …             |
| 8      | 2    | `msg_type`                                       |
| 10     | 2    | `flags` (reserved, must be `0` in `0000`)        |
| 12     | 4    | `request_id` (client-chosen; echoed on response) |
| 16     | 4    | `body_len`                                       |
| 20     | N    | `body`                                           |

Maximum body length accepted by the reference server: **16 MiB**.

Unknown product magic, or a version digit string this peer does not implement (including future `0001` before support
lands), must be rejected.

## Message types (version `0000`)

| Code | Name             | Direction | Body                                                                |
|------|------------------|-----------|---------------------------------------------------------------------|
| 1    | `Hello`          | C→S       | empty                                                               |
| 2    | `HelloOk`        | S→C       | UTF-8 server version string                                         |
| 3    | `Info`           | C→S       | empty                                                               |
| 4    | `InfoOk`         | S→C       | UTF-8 text: `path=…\nschema.version=…\nrecords=…\njournal_mode=…\n` |
| 5    | `SchemaGet`      | C→S       | empty                                                               |
| 6    | `SchemaGetOk`    | S→C       | `u8 present`; if 1: `u32 version` + `u32 doc_len` + UTF-8 VOS       |
| 7    | `SchemaEnsure`   | C→S       | `u32 version` + `u32 doc_len` + UTF-8 VOS                           |
| 8    | `SchemaEnsureOk` | S→C       | empty                                                               |
| 9    | `KvGet`          | C→S       | `u32 key_len` + key UTF-8                                           |
| 10   | `KvGetOk`        | S→C       | `u8 present`; if 1: `u32 val_len` + bytes                           |
| 11   | `KvPut`          | C→S       | `u32 key_len` + key + `u32 val_len` + bytes                         |
| 12   | `KvPutOk`        | S→C       | empty                                                               |
| 255  | `Error`          | S→C       | UTF-8 error message                                                 |

Unknown `msg_type` → `Error` with the same `request_id`.

## Out of scope for version `0000`

- Authentication / RBAC
- Full VOS query execution
- CAS object streaming RPCs
- TLS termination as a product surface
- Version `0001` layout changes (defined when that generation ships)
