# Browser integration

The browser uses `@yydb/light-db`. It compiles the same Rust semantic core to WASM and writes session data to the origin-private file system (OPFS).

## Install and open

```bash
npm install @yydb/light-db
```

```ts
import { openLightDatabase } from "@yydb/light-db";

const db = await openLightDatabase("app");
```

`openLightDatabase` returns a handle with the same shape as the Node version: `ensureSchema`, `query`, `execute`, `transaction`, `prepare` and the key-value API are all available.

## In-memory sessions

```ts
import { Database } from "@yydb/light-db";

const db = await Database.open("scratch", { memory: true });
```

In-memory sessions never touch disk. They suit demos, tests and temporary computation. Data disappears when the page reloads.

## Persistent sessions

Persistent sessions use OPFS. Keep these boundaries in mind:

- Data belongs to the current origin. A different domain or protocol is a different storage bucket.
- Private windows and clearing site data delete the database.
- The browser manages quota. A full quota returns a quota-related error.
- Tabs in the same origin share storage, but each tab has its own session. Do not assume cross-tab visibility is immediate.

## Differences from the Node version

| Capability | Node (`@yydb/yydb`) | Browser (`@yydb/light-db`) |
|------------|--------------------|----------------------------|
| Open a local path | Yes | No |
| OPFS persistence | Not applicable | Yes |
| Native binding | Required | Not required |
| In-process UDFs | Yes | Restricted to the WASM session |
| Backup to an arbitrary path | Yes | No, export is left to the application |
| Wire protocol client | Via `@yydb/yydb-client` | Same package over WebSocket |

## Turning a session into a downloadable file

The browser cannot map a session onto a user-chosen local path. To export, the application reads key-values or query results itself, serializes them, and triggers a download from the page. Do not present an OPFS path to users as a portable `.yydb` file.

## Connecting to a remote serve

Browsers also use `@yydb/yydb-client`:

```ts
import { Client } from "@yydb/yydb-client";

const client = await Client.connect("ws://127.0.0.1:7700/wire");
```

The wire protocol covers only Hello, Info, schema, key-values and micro UDFs. It does not include general VOS queries. For full record queries, keep data in a local `light-db` session or build your own API on the server side.

## Recommendations

- Open the session during first paint and defer publishing a schema until a write is actually needed.
- Check remaining quota before writing large objects.
- Wrap multi-step writes in `transaction` to avoid partial success.
