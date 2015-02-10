# `@yydb/light-db`

**Light DB** is the browser-native edition of YYDB. A thin TypeScript facade over
`@yydb/yydb-unknown-wasm32`, which ships the Rust semantic core compiled to
WebAssembly â€?same engine as the Node-side `@yydb/yydb`, same OPFS persistence,
same VOS schema validation, same transaction and UDF semantics.

> No Node built-ins. No filesystem paths. No second semantic implementation.
> One package, one implementation: WASM-backed `Database`.

## Product relationship

| Package | Role |
|---------|------|
| `@yydb/yydb` | Node product: native Rust engine, N-API, local `.yydb` / `.yydx` files, CLI |
| `@yydb/light-db` | **This package** â€?browser product: WASM core over OPFS, same author surface |
| `@yydb/yydb-client` | Remote wire client for a running `yydb serve` (TCP or WebSocket) |

## Usage

```ts
import { Database } from '@yydb/light-db';

const db = await Database.open('app');

// Schema â€?either raw source or @voml/vos descriptors
await db.ensureSchema('table Setting { @@id: uuid, key: utf8 }');
// or: await db.ensureSchema(v.table('Setting', { key: v.text() }));

await db.put('user:1', new TextEncoder().encode('Ada'));
const value = await db.get('user:1');

// VOS read / write
const users = await db.query('User.all().take(10).collect()');
await db.execute('INSERT User { @@id: uuid(), name: "Ada" }');

// Transactions
await db.transaction(async (tx) => {
    await tx.put('user:2', new TextEncoder().encode('Grace'));
    await tx.put('user:3', new TextEncoder().encode('Eve'));
});

await db.close();
```

## Author surface

```ts
await db.serverVersion();            // engine version string
await db.info();                      // session diagnostics

await db.getSchema();                 // { version, document } | null
await db.ensureSchema(document);      // publish schema
await db.migrateSchema(source, { renames: [...] });

await db.get(key);                    // Uint8Array | null
await db.put(key, value);             // value: Uint8Array | string
await db.delete(key);                 // void
await db.scan({ prefix, after, limit });

await db.batch([{ key, value }, ...]);
await db.compareExchange(key, expected, replacement);

await db.begin(); await db.commit(); await db.rollback();
await db.transaction(async (tx) => { ... });

await db.query(source);               // VOS read pipeline â†?typed rows[]
await db.execute(source);             // VOS write program

await db.registerMicro(body);         // host UDF wire-format registration
await db.callScalar(body);            // host UDF invocation
await db.setMicroHostInvoker(fn);     // JS â†?Rust UDF bridge

await db.checkpoint();                // fold WAL into main file
await db.close();
```

Data types are browser-native: keys are strings, values are `Uint8Array`,
databases are addressed by name (no file paths), and every lifecycle call is
`async`.

## Layout

| Path | Role |
|------|------|
| `src/api/wasm-database.ts` | Main {@link Database} facade over WASM sessions |
| `src/api/types.ts` | Public type contract (SchemaVersion, ScanRow, LightDatabase, â€? |
| `src/errors/` | Stable `yydb.*` error codes and envelope parsing |

## Errors

| Code | Meaning |
|------|---------|
| `yydb.io` | Storage operation failed |
| `yydb.corrupt` | Persisted bytes failed validation |
| `yydb.schema.conflict` | Schema document differs from the stored one |
| `yydb.schema.invalid` | Schema document is not usable |
| `yydb.unsupported` | Host or operation is not supported |
| `yydb.session.closed` | Database or handle is closed |
| `yydb.lease.unavailable` | Writer lease is held elsewhere or Web Locks is missing (retryable) |
| `yydb.quota.exceeded` | Browser storage quota is exhausted |

## See also

- [YYDB overview](https://github.com/yy-database/yydb)
- [@yydb/yydb-unknown-wasm32](https://github.com/yy-database/yydb/tree/dev/projects/yydb.ts/packages/yydb-unknown-wasm32) â€?the Rust semantic core
- [Project structure](https://github.com/yy-database/yydb/tree/dev/projects/yydb.ts/homepage/documentation/zh-hans)
