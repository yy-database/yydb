# @yydb/sqlite

Read-only inspection of SQLite format-3 main files. Copies bytes into memory, lists schema objects, and reads raw table records. Does not execute SQL, open a live database, or recover WAL sidecars.

SQLite files are not YYDS storage. They are not routed through `.yyds` catalogs or `.yykv` shards.

## Scope

Supported:

- Format-3 main files with page sizes from 512 to 65536 bytes, including UTF-8, UTF-16le, and UTF-16be text.
- `sqlite_master` / `sqlite_schema` reads with `CREATE` source preserved as opaque text.
- Rowid-table B-tree scans, including interior pages, implicit rowids, `INTEGER PRIMARY KEY` aliases, composite-key rowid tables, and overflow payloads.
- Storage-class record decoding (`NULL`, integer, real, text, blob) through `tableDecoded` and `decodeRecord`.

Not supported:

- SQL execution of any kind, including `SELECT`, views, triggers, and virtual tables.
- Writes, transactional paging, journals, WAL recovery, and live-database consistency. A snapshot is the copied main-file bytes at the moment of reading.
- Encrypted or non-format-3 files, and index B-tree scans such as `WITHOUT ROWID` tables.

Values are on-disk storage classes, not SQL-projected results. `NULL` and the empty string stay distinct, an `INTEGER PRIMARY KEY` alias column is returned as `NULL` rather than substituted with the rowid, and REAL affinity is not applied, so an integral REAL input can decode as an integer. Use the schema `sql` source to map aliases and declared types.

## Exports

| Subpath | Use |
|---------|-----|
| `@yydb/sqlite` | Types plus `SqliteSnapshot`, `SqliteDatabase`, and `decodeRecord` |
| `@yydb/sqlite/node` | The same reader surface (requires the `@yydb/yyds` native addon at call time) |

The `sqlite3` command supports `.tables`, `.schema`, `.dump`, `.ping`, `.encoding`, and `.version` on a file path. It does not run SQL statements.

## Example

```ts
import { readFileSync } from "node:fs";
import { SqliteSnapshot } from "@yydb/sqlite/node";

const snapshot = new SqliteSnapshot(readFileSync("app.sqlite"));
const tables = snapshot.schema().filter((o) => o.kind === "table");
const rows = snapshot.table("samples", { maxRows: 100 });
```

Each row is `{ rowid: bigint, payload: Buffer }` — storage bytes, not SQL-projected values. Schema `sql` fields are opaque CREATE text. The default snapshot size cap is 256 MiB; each scan accepts `maxPages`, `maxRows`, `maxPayloadBytes`, and `maxTotalPayloadBytes`.

## Related packages

- [`@yydb/yyds`](https://github.com/yy-database/yyds/tree/dev/projects/packages/yyds) — hosts the native `SqliteSnapshot` binding
