# `@yydb/yydb`

Node all-in-one and browser wasm core from one package. On Node.js, `Database.open` starts a private loopback `yydb serve`
process and talks over the YY wire protocol. In browsers, `@yydb/yydb/wasm` loads the in-process wasm binding for schema
checks today and persistent sessions next.

For a wire client only (renderer or an app that already runs `yydb serve`), use
[`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client).

[![npm](https://img.shields.io/npm/v/@yydb/yydb)](https://www.npmjs.com/package/@yydb/yydb)
[![Node.js](https://img.shields.io/node/v/@yydb/yydb)](https://www.npmjs.com/package/@yydb/yydb)

## Exports

| Import | Runtime | Role |
|--------|---------|------|
| `@yydb/yydb` | Node (`node` export condition) | `Database.open`, micro helpers |
| `@yydb/yydb` | Browser / bundler (`default`) | wasm `Database` |
| `@yydb/yydb/node` | Node (explicit) | Same as Node default |
| `@yydb/yydb/wasm` | Browser (explicit) | wasm `Database`, `initWasm`, `checkSchema` |
| `@yydb/yydb/cli` | Node | CLI (`version`, `init`, `info`, `serve`) |

Node.js 18+. Platform `.node` bindings install via optional dependencies.

Schema truth lives in the `.yydb` file via `ensureSchema` and VOS documents on Node. Wasm exposes `checkSchema` today.

## Node example

```ts
import { Database } from '@yydb/yydb';

const db = await Database.open('./data/app.yydb');

await db.ensureSchema(
    1,
    `table Setting {
        @@id: uuid,
        key: utf8,
        value: utf8,
    }`,
);

await db.put('theme', 'dark');
console.log(await db.get('theme'));
await db.close();
```

The file is created when missing. Schema version and VOS document persist with the data.

## Browser example

```ts
import { Database } from '@yydb/yydb/wasm';

const db = await Database.openInMemory();
console.log(db.version());
console.log(db.checkSchema('table Setting { @@id: uuid, key: utf8 }'));
```

Call `Database.init()` or `openInMemory()` once before semantic methods. Persistent OPFS `Database.open` is planned on
this entry.

## Node API

```ts
const db = await Database.open(path, options?);

await db.ensureSchema(version, vosDocument);
await db.getSchema();
await db.put(key, stringOrBytes);
await db.get(key);
await db.info();
await db.serverVersion();
await db.close();
```

`OpenOptions`: `cli` (explicit CLI script path), `port` (preferred loopback port), `serveArgs` (extra `yydb serve`
arguments). `binary` is a deprecated alias for `cli`.

## Related packages

| Need | Package |
|------|---------|
| Node all-in-one (this package) | **`@yydb/yydb`** or `@yydb/yydb/node` |
| Browser wasm entry | **`@yydb/yydb/wasm`** |
| Wire client only | [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) |
| In-process Rust embed | [`yydb`](https://crates.io/crates/yydb) |
| Agent skills (wire protocol) | [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills) |

Electron: main process can use `@yydb/yydb/node`; renderer code can use `@yydb/yydb-client` against the main-process
endpoint.

The managed engine listens on `127.0.0.1` with no authentication — keep the endpoint private.

[YYDB overview](https://github.com/yy-database/yydb.rs) · [User guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
