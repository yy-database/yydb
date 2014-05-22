# `yydb-napi`

Thin Node-API binding consumed by [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb). Engine logic lives in [
`yydb`](https://crates.io/crates/yydb); serve loops through [`yydb-server`](https://crates.io/crates/yydb-server).

Application authors use the npm package, not this crate directly.

```ts
import {Database} from "@yydb/yydb";

const db = await Database.open("app.yydb");
await db.close();
```

[YYDB overview](https://github.com/yy-database/yydb.rs)
