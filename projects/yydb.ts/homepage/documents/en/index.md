# YYDB Documentation

YYDB is a single-file local database. One `.yydb` file holds page data, references to its WAL companion files, and a versioned VOS schema document. Applications read and write through VOS, not SQL, and do not need a resident database server.

These guides are written for people using YYDB. They do not discuss internal implementation. Every example runs as-is on Node.js 18 or later.

## Browse by topic

| Topic | Contents |
|-------|----------|
| [Quickstart) | Open a file, publish a schema, write and read the first records |
| [Install and platforms) | npm packages, native bindings, the Rust crate, the browser entry, system requirements |
| [Schema and VOS) | Field types, primary keys, uniqueness, references and schema evolution |
| [Reading and writing) | Read pipelines, write programs, transactions, scans and prepared statements |
| [Node.js integration) | File lifecycle, backup and restore, checkpoint, UDFs |
| [Browser integration) | OPFS sessions and capability boundaries in `@yydb/light-db` |
| [Serve and CLI) | The `yydb` command, bind addresses, threat model and the WebUI |
| [Wire protocol reference) | Frame layout, message types, error bodies and transport |
| [Packages and versions) | Each package's role, optional dependencies and version sync |
| [Troubleshooting) | Stable error codes, common failures and recovery steps |

## Choose an entry point

| Runtime | Entry | Notes |
|---------|-------|-------|
| Node.js application | `@yydb/yydb` | Opens the file in-process, includes the CLI |
| Browser | `@yydb/light-db` | WASM session persisted to OPFS |
| A running local service | `@yydb/yydb-client` | WebSocket or TCP binary frames |
| Rust host | `yydb` crate | `Connection`, `client` and `server` |

## Boundaries

- YYDB exposes no SQL entry point. SQLite files can only be read as read-only snapshots through `@yydb/sqlite`.
- YYDB provides no login, tenancy, roles, audit or clustering. Those belong to YYDS.
- `yydb serve` binds only `127.0.0.1:7700` by default and performs no authentication.
- ANN indexes are not released and must not be depended on in production.
