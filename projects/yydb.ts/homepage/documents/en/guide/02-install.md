# Install and platforms

## Requirements

| Item | Requirement | Notes |
|------|-------------|-------|
| Node.js | 18 or later | `engines` requirement of `@yydb/yydb` and `@yydb/yydb-client` |
| Package manager | npm / pnpm / yarn | Must support optional dependencies |
| Operating system | Windows x64, Linux x64, macOS x64, macOS arm64 | Native bindings resolve per platform |
| Browser | OPFS and WebAssembly support | Only needed by `@yydb/light-db` |

## Node.js

```bash
npm install @yydb/yydb
```

The main package contains the TypeScript facade, the CLI, and the logic that resolves the native binding for the current platform.

## Platform binaries

These packages are optional dependencies of `@yydb/yydb`. A normal install needs no manual handling.

| Package | Target |
|---------|--------|
| `@yydb/yydb-win32-x64` | Windows x64 |
| `@yydb/yydb-linux-x64` | Linux x64 |
| `@yydb/yydb-darwin-x64` | macOS x64 (Intel) |
| `@yydb/yydb-darwin-arm64` | macOS arm64 (Apple Silicon) |

When no matching platform package is present, the module reports a clear resolution failure instead of silently falling back to another implementation. Check install logs with options such as `pnpm add --allow-build` to confirm optional dependencies were not pruned.

## Rust

```bash
cargo add yydb
```

```rust
use yydb::prelude::*;

fn main() {
    let db = Connection::open("app.yydb").expect("open database");
    // Each peer uses db.session() to establish its own session
}
```

`Connection` is owned by a single process. For multiple connections, create sessions on the same `Connection` instead of opening the file again.

## Browser

```bash
npm install @yydb/light-db
```

```ts
import { openLightDatabase } from "@yydb/light-db";

const db = await openLightDatabase("app");
const rows = await db.query("User.all().collect()");
```

The browser entry runs on WASM. It contains no Node built-ins and does not expose the SQLite snapshot reader.

## Remote client

```bash
npm install @yydb/yydb-client
```

```ts
import { Client } from "@yydb/yydb-client";

const client = await Client.connect("ws://127.0.0.1:7700/wire");
```

The TCP entry for Node.js is imported from the subpath `@yydb/yydb-client/node`.

## Verify the install

```bash
npx @yydb/yydb version
npx @yydb/yydb init ./app.yydb
npx @yydb/yydb info ./app.yydb
```

`info` prints the path, schema version, record count and journal mode. Add `--doctor` to run a read-only consistency check.

## Uninstall

```bash
npm uninstall @yydb/yydb
```

Database files are never deleted automatically. Removing or backing up `.yydb`, `.yydb-wal` and `.yydb-shm` is your responsibility.

> Warning: `serve` has no authentication layer. Do not expose a bind address on an untrusted network.
