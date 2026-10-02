# `yydb`

**An embedded, single-file database for Rust applications.**

The crate opens a portable `.yydb` file, stores a versioned
[VOS](https://github.com/voml/vos-language) schema with host-side validation, and exposes durable local records through
a small Rust API. It uses the same storage engine as the YYDB CLI and TypeScript package.

[![crates.io](https://img.shields.io/crates/v/yydb)](https://crates.io/crates/yydb)
[![docs.rs](https://docs.rs/yydb/badge.svg)](https://docs.rs/yydb)
[![CI](https://github.com/yy-database/yydb.rs/actions/workflows/ci.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb.rs/actions/workflows/ci.yml)
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
    db.ensure_schema(1, "table Project { @@id: uuid, title: utf8 }")?;
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

## Included in the 0.1 API

- versioned VOS schema validation and persistence;
- durable key/value records;
- delete and WAL journal modes with recovery;
- content-addressed binary objects with chunked file and range access;
- vector payloads stored through the same object references;
- process-local native scalar UDFs;
- shared wire frame types for hosts and clients.

The query engine and distributed services are separate parts of the YY product roadmap. For a Node.js application, use
[`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb); for a browser or an existing server endpoint, use
[`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client).

## Documentation

- [Rust API reference](https://docs.rs/yydb)
- [YYDB user guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
- [Repository](https://github.com/yy-database/yydb.rs)
- [License](https://github.com/yy-database/yydb.rs/blob/dev/License.md)
