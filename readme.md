# YYDB

**A local database whose schema travels with the data.**

[![Check Rust](https://github.com/yy-database/yydb/actions/workflows/check-rust.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb/actions/workflows/check-rust.yml)
[![Check TypeScript](https://github.com/yy-database/yydb/actions/workflows/check-typescript.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb/actions/workflows/check-typescript.yml)
[![npm](https://img.shields.io/npm/v/@yydb/yydb?label=%40yydb%2Fyydb)](https://www.npmjs.com/package/@yydb/yydb)
[![crates.io](https://img.shields.io/crates/v/yydb)](https://crates.io/crates/yydb)
[![docs.rs](https://docs.rs/yydb/badge.svg)](https://docs.rs/yydb)
[![License](https://img.shields.io/badge/license-MPL--2.0-blue)](./License.md)

## 🧭 What is YYDB?

YYDB is a database for software that wants local, durable data without a database service to operate. A project is
centred on one portable `.yydb`
file: copy it, back it up, or keep it beside the application that owns it.

The important idea is that the data file also contains its **versioned schema**. There is no second schema registry to
keep in sync. The schema is written in
[VOS](https://github.com/voml/vos-language), a typed language shared by the YYDB tools and clients.

If you are comparing options, think of YYDB as:

| Concept           | YYDB                                                        |
|-------------------|-------------------------------------------------------------|
| Data model        | Typed VOS schema + durable records                          |
| Source of truth   | The `.yydb` file itself                                     |
| Deployment        | Embedded in a process, or opened by the local `yydb` engine |
| Network           | Optional local wire endpoint, not a requirement             |
| Operational shape | One file, no cluster control plane                          |

## 🧩 The VOS language in one minute

VOS describes the shape of your data explicitly instead of leaving field names and types implicit in application code:

```vos
table User {
    @@user_id: uuid,
    @user_name: utf8,
    manager: &User? = null,
}
```

This small document gives YYDB a table name, an explicit primary key, a unique field, and a nullable reference to
another `User`. A schema version is stored with the document, and YYDB validates the document before accepting it as
database truth.

The VOS vocabulary is intentionally readable:

| VOS feature        | Meaning                                                     |
|--------------------|-------------------------------------------------------------|
| `table`            | A persisted data shape                                      |
| `@@user_id: uuid`  | A typed primary key (`@@` marks the primary identity)       |
| `@user_name: utf8` | A typed unique field (`@` marks uniqueness)                 |
| `manager: &User`   | A reference to another `User` value                         |
| `&User?`           | An optional `User` reference; `= null` supplies its default |
| `[T]`              | A list of values of type `T`                                |

YYDB currently centres on `table` schemas and durable records. The broader VOS language also defines classes, enums,
unions, and service contracts; those constructs are shared with the surrounding VOS tooling as the database query and
generation surfaces grow.

The same VOS contract is available from the Rust API, the CLI, and the TypeScript client. That keeps a schema
understandable when a project moves between a desktop tool, an edge process, and a browser-facing host.

## ✨ Features

YYDB is more than a key/value file. Its storage model covers the pieces that local applications commonly need around
their structured data:

| Capability                    | What it means in practice                                                                                                     |
|-------------------------------|-------------------------------------------------------------------------------------------------------------------------------|
| **Structured schema**         | Versioned VOS documents live in the `.yydb` file and remain the database truth.                                               |
| **Native file storage**       | Large files and binary values can live in the adjacent content-addressed `objects/` store instead of being copied into a row. |
| **Chunking and range reads**  | Files are split into chunks and can be read by byte range, so a caller does not need to load an entire asset at once.         |
| **Vector values**             | Vector payloads use the same CAS and object references as other binary data; hot/cold residency is available at runtime.      |
| **Crash recovery**            | Delete and WAL journal modes keep a file reopenable after an interrupted write.                                               |
| **Native extensions**         | Rust applications can register process-local scalar UDFs without a separate service.                                          |
| **One engine, several hosts** | Rust embed, the `yydb` CLI, Node.js, browsers, and WebUI share the same file and wire model.                                  |

The object store is deliberately unified: files, bytes, vectors, and future ANN segments use one
`<db>.objects/objects/…` layout. YYDB 0.1 provides vector and object persistence and a Phase 1 VOS query/DML executor
(`yydb::query`). ANN indexing/search and distributed services remain separate roadmap items.

## 📦 What the current release provides

YYDB 0.1 is deliberately small and usable today:

- a reopenable `.yydb` file with schema/version metadata;
- VOS schema persistence with host-side validation;
- durable key/value records for application state;
- delete and WAL journal modes with recovery;
- content-addressed, chunked storage for larger binary objects, including range reads for native files;
- vector payload persistence in the same object store;
- native Rust scalar UDFs;
- Phase 1 VOS query/DML on embedded connections;
- a shared wire client for a separately managed local engine.

Distributed multi-node services are a separate product surface ([YYDS](#yydb-and-yyds)). This repository does not turn
YYDB into a hosted cluster control plane.

## 🚀 Pick a way to use it

| You are using                            | Start with                                                                                                                                                |
|------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------|
| TypeScript or Node.js                    | [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb)                                                                                                  |
| Rust                                     | [`yydb`](https://crates.io/crates/yydb) and its [API docs](https://docs.rs/yydb)                                                                          |
| Browser or an existing `yydb serve` host | [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client)                                                                                    |
| CLI                                      | `npx @yydb/yydb` / [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb) (platform `.node` via optionalDependencies) |

### Node.js

```bash
npm install @yydb/yydb
```

```ts
import {Database} from "@yydb/yydb";

const db = await Database.open("app.yydb");
await db.ensureSchema(1, "table Setting { @@id: uuid, key: utf8 }");
await db.put("theme", "dark");
await db.close();
```

The package installs the matching platform native binding and starts a private loopback `yydb serve` process when
`Database.open()` runs.

### Rust

```bash
cargo add yydb
```

```rust
use yydb::prelude::*;

fn main() -> Result<()> {
    let db = Connection::open("app.yydb")?;
    db.ensure_schema("table Project { @@id: uuid, title: utf8 }")?;
    db.put("project/title", b"Spark")?;
    Ok(())
}
```

### Rust crate layout

| Layer      | Crate                                      | Role                                                  |
|------------|--------------------------------------------|-------------------------------------------------------|
| Bottom     | `yydb-types`                               | `Value`, `Error`, CAS refs                            |
| Engine     | `yydb-execution`, `yydb-udf`, `yydb-query` | Execution IR, UDF contract, VOS query/DML             |
| **Facade** | **`yydb`**                                 | **`Connection` + re-exports — normal app dependency** |
| Transport  | `yydb-client`, `yydb-server`               | Remote wire client / serve loop                       |
| Bindings   | `yydb-napi`, `yydb-pyo3`                   | Node / Python hosts (thin layers on `yydb`)           |

See [`projects/crates/yydb/readme.md`](./projects/crates/yydb/readme.md) for the facade contract.

## 🔒 Local by default

The reference `yydb serve` endpoint binds to `127.0.0.1` by default and has no authentication layer. Use it on a machine
or private network you control, and let the operating system and surrounding network define access to the file or
endpoint.

## YYDB and YYDS

YYDB is the lightweight, local product: a single `.yydb` file and an embedded
engine. [YYDS](https://github.com/yy-database/yyds) is the separate distributed product for multi-node deployments that
need permissions, audit, and fleet operations.

## Learn more

- [YYDB user guides](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans)
- [Rust API documentation](https://docs.rs/yydb)
- [Repository and issue tracker](https://github.com/yy-database/yydb.rs)
- [Agent skills (`@yydb/yydb-skills`)](./projects/packages/yydb-skills)

## License

[Mozilla Public License 2.0](./License.md)
