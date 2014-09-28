# `@yydb/light-db`

**Light DB** — the pure TypeScript database engine for environments without native bindings. Pages, WAL, shared-memory
index blocks, checkpoints, crash recovery and secondary indexes are implemented in TypeScript, so the same source can
run in Node and in browsers.

`@yydb/yydb` is the Node product (native engine plus CLI). Light DB is the browser counterpart and is maintained in this
repository.

## Status

Light DB was transplanted from the standalone `LightDB.ts` project and is not release-ready yet. On Node file I/O the
current baseline is **300 passing assertions** (`pnpm --filter @yydb/light-db test:unit`).

The package exposes two author surfaces:

- `LightDB` / `collection()` — typed documents, queries, secondary indexes and transactions.
- `Database` — the same `open` / `get` / `put` / `getSchema` / `ensureSchema` / `checkpoint` / `close` surface as
  `@yydb/yydb`, so hosts only switch package names. Schema records and KV values share the storage engine and stable
  `yydb.*` error codes.

Browser persistence is the next milestone: file access currently goes through a Node `fs` adapter in
`src/utils/file.ts`, and the planned change is a platform-neutral storage interface with an OPFS adapter
(worker-backed for positional writes) so the public surface stays identical across hosts.

## Usage

```ts
import { Database } from '@yydb/light-db';

const db = await Database.open('./data/app');
await db.put('user:1', 'Ada');
const schema = await db.getSchema();
await db.checkpoint();
await db.close();
```

Collections stay available for document-style workloads:

```ts
import { openDatabase } from '@yydb/light-db';

const db = await openDatabase('./data/app', { dbName: 'app' });
const users = db.collection<{ id: number; name: string }>('users', { primaryKey: 'id' });

await users.insert({ id: 1, name: 'Ada' });
await db.checkpoint();
await db.close();
```

## Layout

| Path | Role |
|------|------|
| `src/storage/` | `LightFile`, WAL, SHM, checkpoint and recovery managers |
| `src/indexes/` | B+ tree, primary and secondary indexes |
| `src/collection/` | Documents, query parser / builder, collection API |
| `src/performance/` | Columnar storage, buffer pool, RW locks, lazy queries |
| `src/api/` | `LightDB`, transactions, shared API types |
| `src/utils/` | Binary encoding and file access |

[YYDB overview](https://github.com/yy-database/yydb.rs)
