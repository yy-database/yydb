# `@yydb/light-db`

**Light DB** is the browser edition of YYDB: a database written in pure TypeScript that stores `.yydb`-style
snapshots, WAL records and checkpoints in browser storage. It has no Node built-ins, no N-API bindings and no WASM in
its runtime path.

## Product relationship

| Package | Role |
|---------|------|
| `@yydb/yydb` | Node product: native Rust engine, N-API, local `.yydb` / `.yydx` files and CLI |
| `@yydb/light-db` | Browser product: pure TypeScript, OPFS / IndexedDB storage, Web APIs, same author surface |
| `@yydb/yydb-client` | Remote wire client for a running `yydb serve` |

## Status

Light DB is a migration of the original `LightDB.ts` engine, rebuilt around a browser-native architecture. It is not
release-ready and should not be published as a stable version yet.

Current coverage:

- `pnpm test` runs the Node-side runner over codec, adapter and database logic (29 cases).
- `pnpm test:browser` runs the conformance suite in an installed Chrome (16 cases): OPFS persistence across reopen, WAL
  recovery, checkpoint truncation, corruption detection, single-writer exclusion, transaction rollback and commit,
  schema and close notifications, and two-tab lease and broadcast behaviour.

## Usage

```ts
import { LightDbError, openLightDatabase } from '@yydb/light-db';

const db = await openLightDatabase('app');

await db.ensureSchema('// @yydb-schema-version: 1\nkv users');
await db.put('user:1', new TextEncoder().encode('Ada'));

const value = await db.get('user:1');
await db.checkpoint();
await db.close();
```

Public data types are browser-native: keys and values use `Uint8Array`, databases are addressed by name rather than by
file path, and every lifecycle call is asynchronous.

## Author surface

```ts
const db: LightDatabase = await openLightDatabase('app', options);

await db.schema();
await db.ensureSchema(document);
await db.get(key);
await db.put(key, value);
await db.delete(key);
await db.begin();
await db.commit();
await db.rollback();
await db.checkpoint();
db.stats();
await db.close();
```

`schema()` returns the persisted schema version and document. The document is the single source of truth: a second
`ensureSchema` call must pass the identical document, otherwise it fails with `yydb.schema.conflict`. Keys starting with
`__yydb__/` are reserved for the database.

## Storage backends

| Adapter | Use |
|---------|-----|
| `OpfsStorageAdapterFactory` | Default. Origin Private File System, positional reads and writes, atomic snapshot publication |
| `IndexedDbStorageAdapterFactory` | Compatibility backend for hosts without OPFS |
| `MemoryStorageBackend` | Tests and throwaway sessions |

`StorageAdapter` is the only platform boundary. Transactions, recovery and checkpoint semantics live in `core`, so
backends implement files, not database behaviour.

## Concurrency

A writer holds a session-scoped `Web Locks` lease. A second tab that opens the same database fails with
`yydb.lease.unavailable` and `retryable: true` instead of silently writing alongside the first one. When a lock manager
is missing the call fails the same way; it never degrades to multiple writers.

`BroadcastChannel` carries `commit`, `schema` and `close` events so other same-origin contexts can discard stale views
and retry after the lease is released. `listenDatabaseEvents(name, handler)` subscribes without opening the database.

## Layout

| Path | Role |
|------|------|
| `src/api/` | Public database, transaction, schema and shared types |
| `src/core/` | Records, WAL, checkpoint, recovery, writer lease and broadcast |
| `src/storage/` | `StorageAdapter` plus OPFS, IndexedDB and memory backends |
| `src/codec/` | Page, snapshot and WAL record encoding over `Uint8Array` |
| `src/errors/` | Stable `yydb.*` error codes |

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

[YYDB overview](https://github.com/yy-database/yydb.rs)
