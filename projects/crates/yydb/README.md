# `yydb`

**An embedded, single-file database for Rust applications.**

The crate opens a portable `.yydb` file, stores a versioned
[VOS](https://github.com/voml/vos-language) schema with host-side validation, and exposes durable local records through
a small Rust API. It uses the same storage engine as the YYDB CLI and TypeScript package.

[![crates.io](https://img.shields.io/crates/v/yydb)](https://crates.io/crates/yydb)
[![docs.rs](https://docs.rs/yydb/badge.svg)](https://docs.rs/yydb)
[![Check Rust](https://github.com/yy-database/yydb/actions/workflows/check-rust.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb/actions/workflows/check-rust.yml)
[![Check TypeScript](https://github.com/yy-database/yydb/actions/workflows/check-typescript.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb/actions/workflows/check-typescript.yml)
[![License](https://img.shields.io/badge/license-MPL--2.0-blue)](https://github.com/yy-database/yydb.rs/blob/dev/License.md)

## Install

```bash
cargo add yydb
```

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

`Connection::open` creates the file when needed. Use
`Connection::open_in_memory()` for tests and short-lived tools.

## Crate layering

`yydb` is the **facade**: one normal Rust dependency for embed hosts. It owns
`Connection`, re-exports `yydb-types` (`Value`, `Error`, …), folds in
`yydb-query` as `yydb::query`, and exposes execution/UDF/wire surfaces.
`yydb-types` is the **bottom package** — engine internals and bindings build on it,
but applications should not need a direct dependency.

```rust
use yydb::prelude::*;
// advanced UDF: use yydb::udf::UdfRegistry;
```

Bindings (`yydb-napi`, `yydb-pyo3`, future `yydb-wasm`, …) are **thin language
layers** on top of `yydb` / `yydb-server`. They are not folded into the facade and
must not couple to each other.

| Layer      | Crate                                      | When to depend                               |
|------------|--------------------------------------------|----------------------------------------------|
| Bottom     | `yydb-types`                               | Extending engine internals only              |
| Engine     | `yydb-execution`, `yydb-udf`, `yydb-query` | Facade implementation / query or UDF authors |
| **Facade** | **`yydb`**                                 | **Default — embedded `Connection`**          |
| Transport  | `yydb-client`, `yydb-server`               | Remote client or serve process               |
| Bindings   | `yydb-napi`, `yydb-pyo3`, …                | Node / Python / WASM hosts only              |

## Included in the 0.1 API

- versioned VOS schema validation and persistence;
- durable key/value records;
- delete and WAL journal modes with recovery;
- content-addressed binary objects with chunked file and range access;
- vector payloads stored through the same object references;
- process-local native scalar UDFs;
- Phase 1 VOS query/DML (`yydb::query`, backed by `yydb-query`);
- shared wire frame types for hosts and clients (`yydb::wire`).

Distributed services beyond the embedded engine are separate parts of the YY product roadmap.

| Need                                   | Surface                                                   |
|----------------------------------------|-----------------------------------------------------------|
| Rust embed                             | `yydb` (`Connection`)                                     |
| Rust wire server                       | `yydb-server` (used by `yydb-napi` / `yydb-pyo3` `serve`) |
| Rust remote client                     | `yydb-client`                                             |
| Node all-in-one (CLI + engine + serve) | `@yydb/yydb` (`yydb-napi`)                                |
| Python embed / serve                   | `yydb-pyo3`                                               |
| Lightweight TS wire client             | `@yydb/yydb-client`                                       |

## Documentation

- [Rust API reference](https://docs.rs/yydb)
- [YYDB user guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
- [Repository](https://github.com/yy-database/yydb.rs)
- [License](https://github.com/yy-database/yydb.rs/blob/dev/License.md)
