# `@yydb/yydb`

**All-in-one YYDB for Node.js: CLI, embedded engine, and local `serve`.**

Open a `.yydb` file with one dependency — `@yydb/yydb` spawns a private loopback server and talks to it over the YY
wire protocol. For a **lightweight client only** (browser, renderer, or an app that already runs `yydb serve`), use
[`@yydb/yydb-client`](../yydb-client).

[![npm](https://img.shields.io/npm/v/@yydb/yydb)](https://www.npmjs.com/package/@yydb/yydb)
[![Node.js](https://img.shields.io/node/v/@yydb/yydb)](https://www.npmjs.com/package/@yydb/yydb)
[![CI](https://github.com/yy-database/yydb.rs/actions/workflows/ci.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb.rs/actions/workflows/ci.yml)

## Install

```bash
npm install @yydb/yydb
```

Node.js 18 or newer is supported. The matching `yydb-napi` binding is installed as an optional platform dependency and is
started on a private loopback endpoint when `Database.open()` runs.

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
const value = await db.get("theme");
console.log(value && new TextDecoder().decode(value));

await db.close();
```

The database file is created if it does not exist. The VOS schema and its version are stored with the data, so reopening
the file does not depend on a separate schema registry.

## API

```ts
const db = await Database.open(path, options ?);

await db.ensureSchema(version, vosDocument);
await db.getSchema();
await db.put(key, stringOrBytes);
await db.get(key);
await db.info();
await db.serverVersion();
await db.close();
```

`OpenOptions` supports `cli` (an explicit CLI script path), `port` (a preferred loopback port), and `serveArgs` (extra
arguments forwarded to `yydb serve`). `binary` is a deprecated alias for `cli`.

## Use a different package when

| Need | Package |
|------|---------|
| Node all-in-one (CLI + engine + serve) | **`@yydb/yydb`** (this package) |
| Lightweight wire client only | [`@yydb/yydb-client`](../yydb-client) |
| In-process Rust embed | [`yydb`](https://crates.io/crates/yydb) |
| Rust wire server crate (used by N-API) | [`yydb-server`](../../crates/yydb-server) |
| Rust remote client | [`yydb-client`](../../crates/yydb-client) |

For Electron, the main process can use `@yydb/yydb`; renderer code can use
`@yydb/yydb-client` against the main process endpoint.

## Security

The managed engine listens on `127.0.0.1` and has no authentication layer. Keep the endpoint private and use the
operating system's file permissions for the `.yydb` file.

## Links

- [YYDB overview](https://github.com/yy-database/yydb.rs)
- [User guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
- [API documentation](https://docs.rs/yydb)
- [License](https://github.com/yy-database/yydb.rs/blob/dev/License.md)
