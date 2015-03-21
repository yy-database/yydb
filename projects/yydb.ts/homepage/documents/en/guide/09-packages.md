# Packages and versions

## npm packages

| Package | Role | Typical consumers |
|---------|------|-------------------|
| `@yydb/yydb` | Node.js embedded engine, native binding resolution and the `yydb` CLI | Node applications, scripts, CI |
| `@yydb/light-db` | Browser engine. WASM sessions on OPFS or in memory | Frontend applications |
| `@yydb/yydb-client` | Wire protocol client and frame codecs | Clients connecting to `serve`, the WebUI |
| `@yydb/webui` | Local database browsing interface | Development and inspection |
| `@yydb/yydb-skills` | Agent Skills for integrators | Coding agents and tool integration |
| `@yydb/sqlite` | Read-only SQLite format-3 snapshot reader | Data migration and interoperability checks |

## Platform packages

| Package | Target |
|---------|--------|
| `@yydb/yydb-win32-x64` | Windows x64 |
| `@yydb/yydb-linux-x64` | Linux x64 |
| `@yydb/yydb-darwin-x64` | macOS x64 |
| `@yydb/yydb-darwin-arm64` | macOS arm64 |

Platform packages are optional dependencies of `@yydb/yydb`. They contain no TypeScript facade and should not be depended on directly.

## Rust crates

| Crate | Purpose |
|-------|---------|
| `yydb` | Unified facade: `Connection`, `Value`, errors and prelude |
| `yydb-client` | TCP client for `yydb serve` |
| `yydb-server` | Local service implementation |

Applications only need `yydb`. The other crates are internal to the facade or serve advanced integration scenarios.

## Version sync

- All npm packages share one version and are published together, so the facade and platform packages never mismatch.
- The wire protocol version is independent of the package version and is currently `0000`. Upgrading a package never changes the protocol version automatically.
- The file format version is independent of both the package version and the protocol version. An upgrade never rewrites old files.
- Before `1.0.0` there is no compatibility promise for old APIs, old protocol shapes or old file layouts. Regenerate derived data and run a consistency check before upgrading.

## Check the actual version

```bash
npx @yydb/yydb version
```

```ts
console.log(db.serverVersion());
```

Both print the same engine version string.

## Choosing packages

- Node only: install `@yydb/yydb`.
- Browser only: install `@yydb/light-db`.
- Browser plus a local service: add `@yydb/yydb-client`.
- Reading a SQLite snapshot for migration: add `@yydb/sqlite`, and note that it does not execute SQL.
