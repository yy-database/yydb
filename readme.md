# YYDB

A local, single-file database whose **schema travels with the data**.

[![Check Rust](https://github.com/yy-database/yydb/actions/workflows/check-rust.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb/actions/workflows/check-rust.yml)
[![Check TypeScript](https://github.com/yy-database/yydb/actions/workflows/check-typescript.yml/badge.svg?branch=dev)](https://github.com/yy-database/yydb/actions/workflows/check-typescript.yml)
[![npm](https://img.shields.io/npm/v/@yydb/yydb?label=%40yydb%2Fyydb)](https://www.npmjs.com/package/@yydb/yydb)
[![crates.io](https://img.shields.io/crates/v/yydb)](https://crates.io/crates/yydb)
[![docs.rs](https://docs.rs/yydb/badge.svg)](https://docs.rs/yydb)
[![License](https://img.shields.io/badge/license-MPL--2.0-blue)](./License.md)

[Rust API](https://docs.rs/yydb) · [npm packages](https://www.npmjs.com/org/yydb) · [User guides (Chinese)](./projects/packages/homepage/documentation/zh-hans/index.md) · [Releases](https://github.com/yy-database/yydb/releases)

## 💡 What is YYDB?

YYDB is a local database whose **schema travels with the data**. A project centers on one portable `.yydb` file —
copy it, back it up, or keep it beside the application that owns it. There is no separate schema registry: versioned
[VOS](https://github.com/voml/vos-language) documents live inside the file and remain the source of truth.

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

See [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills) for the wire protocol and legacy storage
notes.

### 2. Manual install

| You are using                            | Start with                                                                 |
|------------------------------------------|----------------------------------------------------------------------------|
| TypeScript or Node.js                    | [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb)                   |
| Rust                                     | [`yydb`](https://crates.io/crates/yydb) · [API docs](https://docs.rs/yydb) |
| Browser                                  | [`@yydb/light-db`](https://www.npmjs.com/package/@yydb/light-db)           |
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
console.log(await db.get("theme"));
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

**Browser**

```ts
import {openLightDatabase} from "@yydb/light-db";

const db = await openLightDatabase("app");
await db.ensureSchema("table Setting { @@id: uuid, key: utf8 }");
await db.put("user:1", new TextEncoder().encode("Ada"));
await db.close();
```

The reference `yydb serve` endpoint binds to `127.0.0.1` by default and has no authentication layer. Use it on a
machine or private network you control.

## ✨ Highlights

| Capability              | What it means in practice                                                              |
|-------------------------|----------------------------------------------------------------------------------------|
| **Structured schema**   | Versioned VOS documents live in the `.yydb` file.                                      |
| **Durable records**     | Page, WAL, and checkpoint machinery is built in; files reopen after an interrupted write. |
| **Native file storage** | Large binaries are stored as content-addressed chunks instead of being copied into every row. |
| **Chunking & ranges**   | Files split into chunks; callers read byte ranges without loading entire assets.        |
| **Vector values**       | Vector payloads use the same content-addressed chunk storage as other binary data.     |
| **Native extensions**   | Rust hosts can register process-local scalar and micro UDFs without a separate service. |
| **Several hosts**       | Rust embed, the `yydb` CLI, Node.js, browsers, and the WebUI share the same file and wire model. |

The current release ships durable records, VOS query and DML on embedded connections, prepared statements, and a wire
client for a separately managed local engine. ANN indexing and distributed services remain on the roadmap.

## 📊 Design choices

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

The same VOS contract is available from the Rust API, the CLI, and the TypeScript clients.

## 📦 Packages

| Surface                        | Package                                                                 | Use                                                   |
|--------------------------------|-------------------------------------------------------------------------|-------------------------------------------------------|
| Node.js engine and CLI         | [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb)                 | Open a local `.yydb` file from Node.js                |
| Browser engine                 | [`@yydb/light-db`](https://www.npmjs.com/package/@yydb/light-db)         | Pure-TS `Database` and WASM sessions over OPFS, IndexedDB, or memory |
| Wire client                    | [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client)   | WebSocket / TCP client for a running `yydb serve`     |
| Local browser explorer         | [`@yydb/yydb-webui`](https://www.npmjs.com/package/@yydb/yydb-webui)     | Inspect info, schema, and key/value data              |
| Agent skills                   | [`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills)   | Wire protocol and legacy storage notes for agents     |
| Rust embed API                 | [`yydb`](https://crates.io/crates/yydb)                                 | `Connection`, `client`, and `server` in one facade    |
| Raw wasm artifacts             | [`@yydb/yydb-unknown-wasm32`](https://www.npmjs.com/package/@yydb/yydb-unknown-wasm32) | Low-level wasm glue and bundled `.wasm` |
| Platform bindings              | `@yydb/yydb-{win32-x64,linux-x64,darwin-x64,darwin-arm64}`               | Installed automatically by `@yydb/yydb`               |

The CLI (`yydb version`, `init`, `info`, `verify`, `serve`) ships from `@yydb/yydb`; `light-db` and `yydb-client`
have no CLI.

## 🗂 Repository layout

| Path                 | Contents                                                       |
|----------------------|----------------------------------------------------------------|
| `projects/yydb.rs/`  | Rust workspace: crates, ffis, toolchain                         |
| `projects/yydb.ts/`  | TypeScript: packages, platforms, emulators, homepage            |
| `projects/examples/` | Runnable examples (if present)                                  |
| `documentation/`     | Developer design guidance (invariants, layering, encoding)     |

## 📚 Documentation

| Need                     | Where                                                                                                     |
|--------------------------|-----------------------------------------------------------------------------------------------------------|
| Install and use          | [User guides (Chinese)](./projects/yydb.ts/homepage/documentation/zh-hans/index.md)                        |
| Wire protocol            | [`yydb-serve-protocol`](./projects/yydb.ts/packages/yydb-skills/skills/yydb-serve-protocol/SKILL.md)                |
| Rust API                 | [docs.rs/yydb](https://docs.rs/yydb)                                                                       |
| Format and compatibility | [`documentation/03-encoding-and-compatibility.md`](./documentation/03-encoding-and-compatibility.md)        |
| Release history          | [GitHub Releases](https://github.com/yy-database/yydb/releases) and [`releases/`](./documentation/maintenance/releases/) |

## 📜 License

[Mozilla Public License 2.0](./License.md)
