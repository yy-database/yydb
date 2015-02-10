# `@yydb/yydb-unknown-wasm32`

Low-level wasm-pack artifacts for the YYDB browser core: init glue, schema checks, and memory / persistent sessions.

Application authors should use [`@yydb/light-db`](https://www.npmjs.com/package/@yydb/light-db), which wraps these
artifacts as `WasmDatabase` and `openLightDatabase`. Import this package directly only when you need the raw glue or
the bundled `.wasm` asset path.

## Example

```ts
import { checkSchema, initWasm, yydbVersion } from '@yydb/yydb-unknown-wasm32';

await initWasm();

console.log(yydbVersion());
console.log(checkSchema('table Setting { @@id: uuid, key: utf8 }'));
```

Call `initWasm()` once before semantic methods. Omit `options.module` to load the bundled `lib/yydb_wasm_bg.wasm` asset.

Raw asset path: `@yydb/yydb-unknown-wasm32/wasm`.

[YYDB overview](https://github.com/yy-database/yydb) · [Browser product](https://www.npmjs.com/package/@yydb/light-db)
