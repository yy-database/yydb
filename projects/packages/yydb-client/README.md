# `@yydb/yydb-client`

TypeScript client for the YY wire protocol over WebSocket or TCP. Use when `yydb serve` already runs (from [
`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb) CLI or another host) and you only need the client surface —
browser, WebUI, Electron renderer, or a thin Node process.

[![npm](https://img.shields.io/npm/v/@yydb/yydb-client)](https://www.npmjs.com/package/@yydb/yydb-client)
[![Node.js](https://img.shields.io/node/v/@yydb/yydb-client)](https://www.npmjs.com/package/@yydb/yydb-client)

## Exports

| Import                   | Transport         | Typical use        |
|--------------------------|-------------------|--------------------|
| `@yydb/yydb-client`      | WebSocket `/wire` | Browsers and WebUI |
| `@yydb/yydb-client/node` | TCP or WebSocket  | Node and Electron  |

Frame helpers (`encodeFrame`, `MsgType`, …) are exported for tools and tests.

## Example

Start a host (from `@yydb/yydb` or the `yydb` CLI):

```text
yydb serve app.yydb --bind 127.0.0.1:7700
```

**Browser / WebSocket**

```ts
import {Client} from "@yydb/yydb-client";

const db = await Client.connect("ws://127.0.0.1:7700/wire");
await db.ensureSchema(1, "table Setting { @@id: uuid, key: utf8, value: utf8 }");
await db.put("theme", "dark");
console.log(await db.get("theme"));
db.close();
```

**Node TCP**

```ts
import {connect} from "@yydb/yydb-client/node";

const db = await connect("127.0.0.1:7700");
console.log(await db.info());
db.close();
```

To open a local `.yydb` file without managing a server, use [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb)
instead.

The client accepts hosts that announce `YYDB` or `YYDS` on the shared wire. Connect only to hosts you control — normally
`127.0.0.1` or `localhost`.

Protocol details: [
`yydb-serve-protocol`](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/yydb-skills/skills/yydb-serve-protocol/SKILL.md)
in [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills).

[YYDB overview](https://github.com/yy-database/yydb.rs) · [User guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
