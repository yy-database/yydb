# Node.js integration

`@yydb/yydb` opens the file directly inside the process. The hot path has no subprocess and no loopback TCP.

## Open and lifecycle

```ts
import { Database } from "@yydb/yydb";

const db = await Database.open("data/app.yydb", { mkdir: true });
```

- `mkdir: false` disables automatic creation of parent directories.
- A failed open throws `YydbError` and never returns a half-initialized object.
- Do not open multiple `Database` instances for the same file in one process. The write lock blocks a second writer.
- Run `await db.checkpoint()` before the process exits to ensure the WAL is folded.

## File layout

| File | Role |
|------|------|
| `app.yydb` | `YDPG` page file, 4096-byte pages |
| `app.yydb-wal` | `YYWL` transaction log, cleaned by checkpoint after commit |
| `app.yydb-shm` | `YYSH` WAL shared state |
| `app-objects/` | Content-addressed object chunks, present only when objects were written |

When moving a database whose WAL has not been folded, move `.yydb-wal` along with it. The safer option is to `checkpoint()` first and then move the single main file.

## Diagnostics

```ts
console.log(db.info());
const report = await db.doctor();
```

`info()` returns a multi-line string with path, schema version, record count and journal mode. `doctor()` returns a structured consistency report. It is read-only and does not modify the file.

## Backup

```ts
await db.backup("backup/app.yydb");
```

- `backup(dest)` checkpoints first, then copies a self-contained main file.
- `backupWithWal(dest)` skips the checkpoint and copies the WAL and SHM as well. The destination replays the WAL tail on first open.
- The destination must be a different path. It never overwrites the file currently open.

## Exact types

Reading `timestamp` and `decimal` yields tagged objects:

```ts
type Timestamp = { $yydb: "timestamp"; utcMicros: string };
type Decimal = { $yydb: "decimal"; precision: number; scale: number; unscaled: string };
```

Format them yourself for display. Do not convert `utcMicros` or `unscaled` with `Number()` directly, since values beyond safe-integer range lose precision.

## In-process UDFs

```ts
const def = db.defineMicro(/* name, version, arguments, return type and implementation */);
const handle = await db.registerMicro(def);
const result = await db.callScalar("myScalar", 1, [42]);
```

- UDFs are valid only inside the current process session and are never written into the database file.
- Arguments and return values are limited to scalar types. Serialize complex structures first.
- The name and version locate the implementation over the protocol path, so both must match.

## Concurrency

- Multiple async tasks in one process can share a `Database`. Writes are serialized.
- Transactions do not nest. Put every operation that must apply atomically into a single `transaction` callback.
- A second process writing to the same file is rejected by the write lock. For concurrent writes, switch to the single-writer model of `yydb serve`.

## Close

```ts
await db.close();
```

`close()` is a best-effort release. When deterministic flush is required, call `checkpoint()` first, then close.
