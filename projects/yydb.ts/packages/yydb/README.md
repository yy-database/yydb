# `@yydb/yydb`

Node embedded engine and CLI. `Database.open` starts a private loopback `yydb serve` process and talks over the YY
wire protocol; platform `.node` bindings install through optional dependencies.

Browsers are served by the pure TypeScript [`@yydb/light-db`](../light-db/readme.md) package. This package no longer
ships a WebAssembly entry.

For a wire client only (renderer or an app that already runs `yydb serve`), use
[`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client).

[![npm](https://img.shields.io/npm/v/@yydb/yydb)](https://www.npmjs.com/package/@yydb/yydb)
[![Node.js](https://img.shields.io/node/v/@yydb/yydb)](https://www.npmjs.com/package/@yydb/yydb)

## Exports

| Import | Runtime | Role |
|--------|---------|------|
| `@yydb/yydb` | Node | `Database.open`, micro helpers, VOS re-exports |
| `@yydb/yydb/cli` | Node | CLI (`version`, `init`, `info`, `verify`, `serve`) |

Node.js 18+.

```bash
npm install @yydb/yydb
npx yydb version
```

Schema truth lives in the `.yydb` file via `ensureSchema` and VOS documents.

## Example

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

## API

```ts
const db = await Database.open(path, options?);

await db.ensureSchema(version, vosDocument);
await db.getSchema();
await db.put(key, stringOrBytes);
await db.get(key);
await db.info();
await db.serverVersion();
await db.registerMicro(microDefinition);
await db.callScalar('double', 1, [21]);
await db.close();
```

`OpenOptions`: `cli` (explicit CLI script path), `port` (preferred loopback port), `serveArgs` (extra `yydb serve`
arguments). `binary` is a deprecated alias for `cli`.

### Prepared statements

`db.prepare` parses and lowers a VOS query once; the plan stays in a native thread-local registry and re-executes
with different bindings.

```ts
const stmt = await db.prepare('User.all().take($limit).collect()');
const rows = await stmt.execute({ limit: 100 });
const more = await stmt.execute({ limit: 50 });
stmt.close();
```

`stmt.parameters` lists the expected bindings, and `stmt.info` exposes the read/write sets and source fingerprint.
Missing, extra, or mistyped parameters raise `YydbError` with code `yydb.query.bind`.

The native binding exposes additive operations (`schemaVersion`, `schemaDocument`, `recordCount`, `journalMode`,
`checkpoint`, `walFrameCount`, `get`, `put`); the CLI composes commands from them.

## Related packages

| Need | Package |
|------|---------|
| Node embedded engine and CLI (this package) | **`@yydb/yydb`** |
| Browser engine (pure TS and WASM sessions) | **`@yydb/light-db`** |
| Wire client only | [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) |
| In-process Rust embed | [`yydb`](https://crates.io/crates/yydb) |
| Agent skills (wire protocol) | [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills) |

Electron: main process can use `@yydb/yydb`; renderer code can use `@yydb/yydb-client` against the main-process
endpoint.

The managed engine listens on `127.0.0.1` with no authentication â€?keep the endpoint private.

[YYDB overview](https://github.com/yy-database/yydb) Â· [User guides](https://github.com/yy-database/yydb/tree/dev/projects/yydb.ts/homepage/documentation/zh-hans)
