# YYDB

[![Check Rust](https://github.com/yy-database/yydb/actions/workflows/check-rust.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb/actions/workflows/check-rust.yml)
[![Check TypeScript](https://github.com/yy-database/yydb/actions/workflows/check-typescript.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb/actions/workflows/check-typescript.yml)
[![npm](https://img.shields.io/npm/v/@yydb/yydb?label=%40yydb%2Fyydb)](https://www.npmjs.com/package/@yydb/yydb)
[![crates.io](https://img.shields.io/crates/v/yydb)](https://crates.io/crates/yydb)
[![docs.rs](https://docs.rs/yydb/badge.svg)](https://docs.rs/yydb)
[![License](https://img.shields.io/badge/license-MPL--2.0-blue)](./License.md)

## 💡 What is YYDB?

YYDB is a local database whose **schema travels with the data**. A project centres on one portable `.yydb` file — copy
it, back it up, or keep it beside the application that owns it. There is no separate schema registry:
versioned [VOS](https://github.com/voml/vos-language) documents live inside the file and remain the source of truth.

YYDB is **not** a hosted cluster or multi-node control plane. For distributed deployments with permissions and fleet
operations, see [YYDS](https://github.com/yy-database/yyds).

## 🚀 Getting started

### 1. Agent / prompt (recommended)

Install integrator skills when an agent must implement clients or `yydb serve`:

```bash
npx skills add @yydb/yydb-skills --skill yydb-serve-protocol -y
```

```text
Load yydb-serve-protocol. Review my WebSocket client against YY wire 0000 framing and the loopback threat model.
```

See [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills) for wire protocol and legacy storage notes.

### 2. Manual install

| You are using                            | Start with                                                                 |
|------------------------------------------|----------------------------------------------------------------------------|
| TypeScript or Node.js                    | [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb)                   |
| Rust                                     | [`yydb`](https://crates.io/crates/yydb) · [API docs](https://docs.rs/yydb) |
| Browser or an existing `yydb serve` host | [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client)     |
| CLI                                      | `npx @yydb/yydb` (platform binding resolved via optional dependencies)     |

**Node.js**

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

**Rust**

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

The reference `yydb serve` endpoint binds to `127.0.0.1` by default and has no authentication layer. Use it on a machine
or private network you control.

## ✨ Highlights

| Capability              | What it means in practice                                                                                     |
|-------------------------|---------------------------------------------------------------------------------------------------------------|
| **Structured schema**   | Versioned VOS documents live in the `.yydb` file.                                                             |
| **Native file storage** | Large binaries live in an adjacent content-addressed `objects/` store instead of being copied into every row. |
| **Chunking & ranges**   | Files split into chunks; callers read byte ranges without loading entire assets.                              |
| **Vector values**       | Vector payloads share the same object store and CAS references as other binary data.                          |
| **Crash recovery**      | Delete and WAL journal modes keep a file reopenable after an interrupted write.                               |
| **Native extensions**   | Rust hosts can register process-local scalar UDFs without a separate service.                                 |
| **Several hosts**       | Rust embed, the `yydb` CLI, Node.js, browsers, and WebUI share the same file and wire model.                  |

YYDB 0.1 ships durable records, VOS query/DML on embedded connections, and a wire client for a separately managed local
engine. ANN indexing and distributed services remain on the roadmap.

## 📊 How YYDB compares

| Concept           | YYDB                                                        |
|-------------------|-------------------------------------------------------------|
| Data model        | Typed VOS schema + durable records                          |
| Source of truth   | The `.yydb` file itself                                     |
| Deployment        | Embedded in a process, or opened by the local `yydb` engine |
| Network           | Optional local wire endpoint, not a requirement             |
| Operational shape | One file, no cluster control plane                          |

## 📝 VOS syntax primer

VOS describes data shape explicitly instead of leaving field names and types implicit in application code:

```vos
table Account {
    @@account_id: uuid,
    @email: utf8,
    manager: &Account? = null,
}
```

| VOS feature          | Meaning                                           |
|----------------------|---------------------------------------------------|
| `table`              | A persisted data shape                            |
| `@@account_id: uuid` | Typed primary key (`@@` marks primary identity)   |
| `@email: utf8`       | Typed unique field (`@` marks uniqueness)         |
| `manager: &Account`  | Reference to another `Account` value              |
| `&Account?`          | Optional reference; `= null` supplies the default |
| `[T]`                | List of values of type `T`                        |

The same VOS contract is available from the Rust API, the CLI, and the TypeScript client.

## 🧪 Examples

| Demo / surface                    | Link                                                                                                              |
|-----------------------------------|-------------------------------------------------------------------------------------------------------------------|
| Node all-in-one (`Database.open`) | [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb)                                                          |
| Wire client (browser / TCP)       | [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client)                                            |
| Local browser explorer            | [`@yydb/yydb-webui`](https://www.npmjs.com/package/@yydb/yydb-webui)                                              |
| Rust embed API                    | [`yydb`](https://crates.io/crates/yydb) · [docs.rs](https://docs.rs/yydb)                                         |
| User guides (中文)                | [documentation](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/homepage/documentation/zh-hans) |

## 📜 License

[Mozilla Public License 2.0](./License.md)
