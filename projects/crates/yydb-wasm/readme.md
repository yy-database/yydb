# `yydb-wasm`

Browser WebAssembly build of the YYDB core for schema validation. Artifacts ship in [
`@yydb/yydb-unknown-wasm32`](https://www.npmjs.com/package/@yydb/yydb-unknown-wasm32).

```ts
import {initWasm, checkSchema} from "@yydb/yydb-unknown-wasm32";

await initWasm();
checkSchema("table Setting { @@id: uuid, key: utf8 }");
```

[YYDB overview](https://github.com/yy-database/yydb.rs)
