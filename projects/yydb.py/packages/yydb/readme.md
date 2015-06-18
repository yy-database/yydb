# yydb (Python)

Python binding for the YYDB native VOS database. The package exposes version
lookup, a thin embedded database handle with one parameterized VOS write and
one VOS read pipeline, and VOS schema file reads. It does not expose a SQL API
or a second VOS authoring syntax.

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

Failures raise `yydb.YydbError` with a stable `code` and a `message`.
