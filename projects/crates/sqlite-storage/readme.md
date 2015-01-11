# sqlite-storage

SQLite read-only storage runtime, modeled after `yydb-storage`.

This crate owns SQLite main-file runtime behaviour:

- `SqliteDatabase` — file-backed or in-memory handle; opens and validates SQLite main files
- `read_table` — rowid-table B-tree traversal, including overflow page assembly
- `read_schema` / `read_named_table` — catalog reads from SQLite's main-file schema table
- Resource limits for scans and record decoding

**Depends on `sqlite-formats`** for pure header/record/encoding primitives.

**Not yet here**: WAL recovery, journal rollback, index scan, write path,
transactional pager, or cross-process locking. Those follow the same staged
roadmap as `yydb-storage` already has.
