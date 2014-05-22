# `yydb`

Embedded, single-file database for Rust: opens a portable `.yydb` file, stores a
versioned [VOS](https://github.com/voml/vos-language) schema with host-side validation, and exposes durable local
records. Same storage engine as the YYDB CLI and `@yydb/yydb`.

[![crates.io](https://img.shields.io/crates/v/yydb)](https://crates.io/crates/yydb)
[![docs.rs](https://docs.rs/yydb/badge.svg)](https://docs.rs/yydb)

## Example

```rust
use yydb::{Connection, Result};

fn main() -> Result<()> {
    let db = Connection::open("app.yydb")?;
    db.ensure_schema("table Project { @@id: uuid, title: utf8 }")?;
    db.put("project/title", b"Spark")?;

    assert_eq!(
        db.get("project/title")?.as_deref(),
        Some(b"Spark".as_slice()),
    );
    Ok(())
}
```

`Connection::open` creates the file when needed. `Connection::open_in_memory()` keeps object CAS in-process without
filesystem sidecars — useful for tests.

## API surface

`yydb` is the **facade** — the normal Rust dependency for embed hosts.

```rust
use yydb::prelude::*;
// query: yydb::query
// UDF: yydb::udf::UdfRegistry
// wire types: yydb::wire
```

| Layer      | Crate                                      | When to depend                 |
|------------|--------------------------------------------|--------------------------------|
| Bottom     | `yydb-types`                               | Engine internals only          |
| Engine     | `yydb-execution`, `yydb-udf`, `yydb-query` | Facade / query / UDF authors   |
| **Facade** | **`yydb`**                                 | **Default — `Connection`**     |
| Transport  | `yydb-client`, `yydb-server`               | Remote client or serve process |
| Bindings   | `yydb-napi`, `yydb-pyo3`, `yydb-wasm`      | Language hosts only            |

## Related surfaces

| Need               | Surface               |
|--------------------|-----------------------|
| Rust embed         | `yydb` (`Connection`) |
| Rust wire server   | `yydb-server`         |
| Rust remote client | `yydb-client`         |
| Node all-in-one    | `@yydb/yydb`          |
| TS wire client     | `@yydb/yydb-client`   |

[API reference](https://docs.rs/yydb) · [YYDB overview](https://github.com/yy-database/yydb.rs) · [User guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
