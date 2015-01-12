# yyds-sqlite

Pure-Rust SQLite format-3 snapshot reader for the YYDS workspace. Reads main-file bytes with explicit limits; no external SQLite library, no SQL engine, no WAL recovery.

Use the `yyds` crate or `@yydb/sqlite` unless you need this reader directly. This crate is not a disguise gateway and not part of cluster KV execution.

## Example

```rust
use yyds_sqlite::{SchemaObjectKind, SqliteDatabase};

let db = SqliteDatabase::open("app.sqlite")?;
let tables = db
    .schema(Default::default())?
    .into_iter()
    .filter(|o| o.kind == SchemaObjectKind::Table);
let rows = db.table("samples", Default::default())?;
```

`read_schema` / `read_table` return raw records and opaque CREATE source. Index b-trees, WITHOUT ROWID tables, and virtual tables are out of scope for table scans.
