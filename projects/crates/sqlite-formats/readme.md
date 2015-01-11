# sqlite-formats

Pure-Rust SQLite format-3 binary format primitives, modeled after `yydb-formats`.

This crate owns the **pure format facts** of SQLite 3 main files:

- File magic, page-size header decoding, format/encoding validation
- Blank zero-page database materialization
- Record payload serial-type decoding (varint, storage-class values)
- Text encoding selection from the format-3 header

**Not in this crate** — these belong in `sqlite-storage`:

- File I/O, page traversal, or B-tree navigation
- Catalog/schema table reads
- Row scan, row ordering checks, or overflow page chain assembly
- WAL or journal recovery
- SQL execution

All structs and functions here operate on borrowed byte slices. Nothing opens paths, holds handles, or requires a database connection.
