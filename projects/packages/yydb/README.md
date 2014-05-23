# `@yydb/yydb`

Opens a `.yydb` file in Node.js with one dependency: a private loopback `yydb serve` process starts automatically and
the package talks to it over the YY wire protocol. For a lightweight client only (browser, renderer, or an app that
already runs `yydb serve`), use [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client).

[![npm](https://img.shields.io/npm/v/@yydb/yydb)](https://www.npmjs.com/package/@yydb/yydb)
[![Node.js](https://img.shields.io/node/v/@yydb/yydb)](https://www.npmjs.com/package/@yydb/yydb)

## Exports

| Import           | Role                                     |
|------------------|------------------------------------------|
| `@yydb/yydb`     | `Database` API and default entry         |
| `@yydb/yydb/cli` | CLI (`version`, `init`, `info`, `serve`) |

Node.js 18+. The matching platform `.node` binding installs via optional dependencies.

Schema truth lives in the `.yydb` file via `ensureSchema` and VOS documents.

## Example

```ts
import {Database} from "@yydb/yydb";

const db = await Database.open("./data/app.yydb");

await db.ensureSchema(
    1,
    `table Setting {
        @@id: uuid,
        key: utf8,
        value: utf8,
    }`,
);

await db.put("theme", "dark");
console.log(await db.get("theme"));
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
await db.close();
```

`OpenOptions`: `cli` (explicit CLI script path), `port` (preferred loopback port), `serveArgs` (extra `yydb serve`
arguments). `binary` is a deprecated alias for `cli`.

## Related packages

| Need                           | Package                                                                |
|--------------------------------|------------------------------------------------------------------------|
| Node all-in-one (this package) | **`@yydb/yydb`**                                                       |
| Wire client only               | [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) |
| In-process Rust embed          | [`yydb`](https://crates.io/crates/yydb)                                |
| Agent skills (wire protocol)   | [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills) |

Electron: main process can use `@yydb/yydb`; renderer code can use `@yydb/yydb-client` against the main-process
endpoint.

The managed engine listens on `127.0.0.1` with no authentication — keep the endpoint private.

[YYDB overview](https://github.com/yy-database/yydb.rs) · [User guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
