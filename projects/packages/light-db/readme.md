# `@yydb/light-db`

**Light DB** — the pure TypeScript database engine for environments without native bindings. Pages, WAL, shared-memory
index blocks, checkpoints, crash recovery and secondary indexes are implemented in TypeScript, so the same source can
run in Node and in browsers.

`@yydb/yydb` is the Node product (native engine plus CLI). Light DB is the browser counterpart and is maintained in this
repository.

## Status

Light DB was transplanted from the standalone `LightDB.ts` project and is not release-ready yet. On Node file I/O the
current baseline is **252 passing and 34 failing** assertions (`pnpm --filter @yydb/light-db test:unit`); the failures
cluster in serialization checksums, WAL record counts, checkpoint / recovery and transaction rollback.

Browser persistence is the next milestone: file access currently goes through a Node `fs` adapter in
`src/utils/file.ts`, and the planned change is a platform-neutral storage interface with an OPFS adapter
(worker-backed for positional writes) so the public surface stays identical across hosts.

## Usage

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
