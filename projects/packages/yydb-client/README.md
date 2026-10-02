# `@yydb/yydb-client`

**A TypeScript client for the YY wire protocol over WebSocket or TCP.**

Use it from a browser, WebUI, Electron renderer, or a Node application that already owns a `yydb serve` process.

[![npm](https://img.shields.io/npm/v/@yydb/yydb-client)](https://www.npmjs.com/package/@yydb/yydb-client)
[![Node.js](https://img.shields.io/node/v/@yydb/yydb-client)](https://www.npmjs.com/package/@yydb/yydb-client)
[![CI](https://github.com/yy-database/yydb.rs/actions/workflows/ci.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb.rs/actions/workflows/ci.yml)

## Install

```bash
npm install @yydb/yydb-client
```

## Browser or WebSocket

Start a local host first:

```text
yydb serve app.yydb --bind 127.0.0.1:7700
```

Then connect from browser-compatible code:

```ts
import {Client} from "@yydb/yydb-client";

const db = await Client.connect("ws://127.0.0.1:7700/wire");
await db.ensureSchema(1, "table Setting { @@id: uuid, key: utf8, value: utf8 }");
await db.put("theme", "dark");
console.log(await db.get("theme"));
db.close();
```

## Node TCP

The `/node` entry point supports a TCP endpoint and `ws://` URLs:

```ts
import {connect} from "@yydb/yydb-client/node";

const db = await connect("127.0.0.1:7700");
console.log(await db.info());
db.close();
```

If a Node application only needs to open a local `.yydb` file, use
[`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb). It manages the local engine for you; this package is for an
existing host or a custom transport integration.

## Exports

| Import                   | Transport         | Typical use        |
|--------------------------|-------------------|--------------------|
| `@yydb/yydb-client`      | WebSocket `/wire` | Browsers and WebUI |
| `@yydb/yydb-client/node` | TCP or WebSocket  | Node and Electron  |

The high-level client includes `info`, schema ensure/get, key/value get/put, server version, and close operations. Frame
helpers such as `encodeFrame` and
`MsgType` are exported for tools and tests.

## Compatibility and security

The client accepts hosts that announce either `YYDB` or `YYDS` on the shared wire. The reference `yydb serve` endpoint
has no authentication, so connect only to a host you control, normally `127.0.0.1` or `localhost`.

Protocol details for implementers are in the
[serve protocol reference](https://github.com/yy-database/yydb.rs/blob/dev/documentation/serve-protocol.md).

## Links

- [Node file API](https://www.npmjs.com/package/@yydb/yydb)
- [Local WebUI](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/yydb-webui)
- [YYDB user guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
- [License](https://github.com/yy-database/yydb.rs/blob/dev/License.md)
