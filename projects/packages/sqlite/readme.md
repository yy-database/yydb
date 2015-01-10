# @yydb/sqlite

Read-only inspection of SQLite format-3 main files. Copies bytes into memory, lists schema objects, and reads raw table records. Does not execute SQL, open a live database, or recover WAL sidecars.

SQLite files are not YYDS storage. They are not routed through `.yyds` catalogs or `.yykv` shards.

## Exports

| Subpath | Use |
|---------|-----|
| `@yydb/sqlite` | Types only |
| `@yydb/sqlite/node` | `SqliteSnapshot` (requires the `@yydb/yyds` native addon) |

The `sqlite3` command supports `.tables` and `.schema` on a file path. It does not run SQL statements.

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
