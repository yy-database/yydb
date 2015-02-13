# yydb (Python)

Python binding for the YYDB native VOS database. The package exposes version
lookup, a thin embedded database handle, and VOS schema file reads. It does not
expose a SQL API or a second VOS authoring syntax.

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
database.ensure_schema("table Project { @@id: uuid, title: utf8 }")
print(database.info())
database.close()
```

Failures raise `yydb.YydbError` with a stable `code` and a `message`.
