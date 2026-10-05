# `@yydb/yydb-unknown-wasm32`

Low-level wasm-pack artifacts for the YYDB browser core. **App authors should use
[`@yydb/yydb/wasm`](https://www.npmjs.com/package/@yydb/yydb)** instead of this package.

Import here only when you need the raw glue or bundled `.wasm` path. For durable reads and writes in production, prefer
[`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) against `yydb serve` until wasm persistence
lands.

## Example

```ts
import {initWasm, checkSchema, yydbVersion} from "@yydb/yydb/wasm";

await initWasm();

console.log(yydbVersion());
console.log(
    checkSchema("table Setting { @@id: uuid, key: utf8 }"),
);
```

Call `initWasm()` once before semantic methods. Omit `options.module` to load the bundled `lib/yydb_wasm_bg.wasm` asset.

Low-level glue: `@yydb/yydb-unknown-wasm32/wasm` (raw `.wasm` path).

[YYDB overview](https://github.com/yy-database/yydb.rs)
