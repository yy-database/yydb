# sqlite

SQLite product facade. It re-exports `sqlite-storage` and `sqlite-formats` as one
public entry point and adds the product constants clients import. It owns no
byte-level behaviour of its own.

The implementation is split in two: `sqlite-formats` owns SQLite format-3 byte
facts (header validation, record decoding, encoding primitives), while
`sqlite-storage` owns the read-only main-file runtime (file/memory handle,
schema table reads, rowid B-tree scans, resource limits). There is no external
SQLite library, no SQL engine, and no WAL recovery yet.

Use this crate unless you need the format primitives directly. It is not a
disguise gateway and not part of cluster KV execution.

## Example

```rust
use sqlite::{SchemaObjectKind, SqliteDatabase};

let db = SqliteDatabase::open("app.sqlite")?;
let tables = db
    .schema(Default::default())?
    .into_iter()
    .filter(|o| o.kind == SchemaObjectKind::Table);
let rows = db.table("samples", Default::default())?;
```

`read_schema` / `read_table` return raw records and opaque CREATE source. Index
B-trees, WITHOUT ROWID tables, and virtual tables are out of scope for table
scans.
