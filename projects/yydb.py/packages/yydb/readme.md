# yydb (Python)

Python binding for the YYDB native VOS database. The package exposes schema
management, VOS reads and writes, the raw key/value layer with scans and
batches, explicit transactions with savepoints, consistency probes, backup,
compaction and verified restore. It does not expose a SQL API or a second VOS
authoring syntax.

## Install

```bash
pip install .
```

Building from source needs a Rust toolchain and `maturin>=1.7`, the wheel is
built from the `yydb-pyo3` crate in this repository.

## Usage

```python
import yydb

database = yydb.Database.open("app.yydb")
database.ensure_schema("table Blob { @@blob_id: i64, body: bytes }")
database.execute(
    "Blob::insert({ blob_id: $id, body: $body })",
    {"id": 1, "body": b"\x00\xffraw octets"},
)
print(database.query("Blob.all().collect()"))  # body comes back as bytes
database.close()
```

A `bytes` value moves as raw octets in both directions, so NUL bytes and
payloads that are not valid UTF-8 reach the database and come back unchanged.

Failures raise `yydb.YydbError` with a stable `code` and a `message`. Engine
codes come from the Rust core, the full catalog is the guide page
`guide/10-errors.md`.

## Records and sessions

```python
connection.put("t/key", b"payload")
connection.get("t/key")            # bytes, or None when absent
connection.delete("t/key")
connection.scan("t/", limit=100, after=None)   # [(key, value), ...]
connection.write_batch([("t/a", b"1"), ("t/b", None)])  # None deletes

connection.begin()
connection.savepoint("sp1")
connection.rollback_to("sp1")
connection.release("probe")
connection.commit()
```

`Connection.close()` releases the underlying handle, which also drops the
writer lock. A closed handle fails with `yydb.session.closed` instead of
silently succeeding.

## Recovery and maintenance

```python
connection.doctor()      # read-only report, no repairs
connection.backup("backup.yydb")
connection.compact()     # `.yydx` only
yydb.restore("backup.yydb")   # verified handle
```

## Limits

- `Connection.wal_path()` reports the sidecar path and returns `None` when the
  session has none. It does not guarantee the file exists: the WAL may not be
  created yet, may already be folded, or may be unused. Use `journal_mode()` /
  `wal_frame_count()`, or `checkpoint()` first, instead of probing the file.
- `Connection.doctor()` is read-only. It never repairs a damaged page, rewrites
  the WAL or reclaims orphans. Damaged pages must be restored from a verified
  backup or the file rebuilt.
- `yydb.restore()` verifies `.yydb` page CRCs or the `.yydx` object closure
  before returning a handle. A failed check raises instead of opening.
- `compact()` refuses single-file `.yydb` with `yydb.unsupported`. Orphan
  reclamation belongs to `.yydx` layouts.
