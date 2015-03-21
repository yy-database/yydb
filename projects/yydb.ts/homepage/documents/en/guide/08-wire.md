# Wire protocol reference

The YY wire protocol is the binary framing between `yydb serve` and its clients. It is neither JSON REST nor SQL transport.

## Frame layout

| Offset | Length | Field | Value |
|--------|--------|-------|-------|
| 0 | 4 | `magic` | `YYDB` or `YYDS` |
| 4 | 4 | `version` | Four ASCII digits, currently `0000` |
| 8 | 2 | `msg_type` | See the message table. `255` is `Error` |
| 10 | 2 | `flags` | Must be 0 under `0000` |
| 12 | 4 | `request_id` | Unsigned. Responses must echo the request value |
| 16 | 4 | `body_len` | Body length in bytes, at most 16 MiB |
| 20 | `body_len` | `body` | Encoded per message definition |

The header is fixed at 20 bytes. Every integer after `version` is little-endian.

## Encoding rules

1. `magic` only identifies the backend. Frontends accept both values and do not require them to match the request.
2. Unknown magic, unknown version and unknown `msg_type` are rejected outright with no compatibility parsing.
3. A body larger than 16 MiB is rejected. It is never truncated or split.
4. Input that fails validation or parsing must not produce a partial application.

## Messages

| Value | Name | Direction | Purpose |
|-------|------|-----------|---------|
| 1 | `Hello` | Request | Establish a session and negotiate the version |
| 2 | `HelloOk` | Response | Session established |
| 3 | `Info` | Request | Read basic database information |
| 4 | `InfoOk` | Response | Information returned |
| 5 | `SchemaGet` | Request | Read the current schema |
| 6 | `SchemaGetOk` | Response | Schema version and document |
| 7 | `SchemaEnsure` | Request | Publish a schema |
| 8 | `SchemaEnsureOk` | Response | Publish succeeded |
| 9 | `KvGet` | Request | Read one key |
| 10 | `KvGetOk` | Response | Value or absent |
| 11 | `KvPut` | Request | Write one key |
| 12 | `KvPutOk` | Response | Write succeeded |
| 13 | `MicroRegister` | Request | Register a session-level micro UDF |
| 14 | `MicroRegisterOk` | Response | Registration succeeded |
| 15 | `MicroHostInvoke` | Request | Host invokes a client implementation |
| 16 | `MicroHostInvokeOk` | Response | Host invocation result |
| 17 | `ScalarCall` | Request | Call a scalar UDF |
| 18 | `ScalarCallOk` | Response | Scalar result |
| 255 | `Error` | Response | Unified error body |

The current protocol covers sessions, info, schemas, key-values and micro UDFs. General VOS read pipelines and write programs are available only on the in-process embedding path.

## Error body

```
u32 code_len        (little-endian)
code                (UTF-8)
u32 message_len     (little-endian)
message             (UTF-8)
```

`code` is a stable diagnostic, for example:

| Error code | Meaning |
|------------|---------|
| `yydb.protocol` | Invalid frame or message body |
| `yydb.schema.invalid` | Invalid schema document |
| `yydb.unique.violation` | Unique constraint violated |
| `yydb.udf.not_found` | UDF not registered |
| `yydb.corrupt` | File checksum failed |
| `yydb.io` | File read or write failed |

`yydb.unique.violation` carries `table`, `constraint` and `columns` parameters. `yydb.scalar.invalid` carries `kind` and a display `message`.

## Transport

- TCP: the direct listen port of `yydb serve`.
- WebSocket: the `/wire` path, used by browsers and the WebUI.

The `@yydb/yydb-client` library provides `encodeFrame` / `decodeFrame` plus error body codecs, so applications never need to hand-write parsing.

## Debugging

The wire demo page on this site (`/wire`) encodes and decodes a Hello frame entirely in the browser without connecting to any service. Use it to confirm field order and byte order, then compare the result with your own implementation.
