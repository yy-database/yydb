# `yydb-wasm`

Browser WebAssembly build of the YYDB core. It shares semantic entry points with `yydb-napi` where applicable and
never introduces a parallel TypeScript parser.

Prebuilt artifacts ship in [`@yydb/yydb-unknown-wasm32`](https://www.npmjs.com/package/@yydb/yydb-unknown-wasm32).
Browser applications normally use [`@yydb/light-db`](https://www.npmjs.com/package/@yydb/light-db) instead of calling
this crate's glue directly.

```ts
import {checkSchema, initWasm} from "@yydb/yydb-unknown-wasm32";

await initWasm();
checkSchema("table Setting { @@id: uuid, key: utf8 }");
```

[YYDB overview](https://github.com/yy-database/yydb)
