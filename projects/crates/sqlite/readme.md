# sqlite

SQLite product facade with two distinct capabilities.

## 1. Writable SQL dialect over YYDB

`SqliteConnection` runs SQL statements on a `.yydb` database using YYDB's own
engine. `oak-sql` parses, `yydb-query` lowers the AST into the internal operator
set VOS uses, and `yydb-engine` provides catalog, transactions, locking and
durability. No SQLite library, C sys-binding or SQLite bytecode engine is
involved, so indexing and durability are exactly the YYDB ones.

```rust
use sqlite::SqliteConnection;

let conn = SqliteConnection::open("app.yydb")?;
conn.ensure_schema("table Account { @@account_id: i64, email: utf8 }")?;
conn.execute("INSERT INTO Account (account_id, email) VALUES (?, ?)")?;
let rows = conn.query("SELECT email FROM Account WHERE account_id > ?")?;
```

Schema creation stays VOS-owned: SQL covers rows (`INSERT`, `UPDATE`, `DELETE`,
`SELECT` with `WHERE` / `ORDER BY` / `LIMIT` / `OFFSET`), while tables and
columns are declared by one VOS schema document so migrations and unique
constraints have a single source of truth. `JOIN`, `GROUP BY`, `LIKE` and DDL
are rejected explicitly rather than approximated.

## 2. Read-only upstream file inspection

`SqliteDatabase` reads an existing SQLite format-3 file so its schema and rows
can be adopted into a YYDB database. `sqlite-storage` owns the main-file runtime
(file/memory handle, schema table reads, rowid B-tree scans, resource limits)
and `sqlite-formats` owns the byte-level format facts (header validation,
record decoding, encoding primitives). There is no SQL engine and no WAL
recovery on this path.

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
