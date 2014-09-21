# `yydb-napi`

Thin Node-API binding consumed by [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb). Engine logic lives in [
`yydb`](https://crates.io/crates/yydb).

The binding exposes one `Database` handle per open file plus single-purpose methods (`ensure_schema`, `record_count`,
`journal_mode`, `checkpoint`, `get` / `put`, …). The TypeScript CLI composes user-facing commands from those methods.

Application authors use the npm package, not this crate directly.

```ts
import {Database} from "@yydb/yydb";

const db = await Database.open("app.yydb");
await db.close();
```

[YYDB overview](https://github.com/yy-database/yydb.rs)
