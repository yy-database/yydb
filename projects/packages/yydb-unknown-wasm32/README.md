# `@yydb/yydb-unknown-wasm32`

Browser WebAssembly artifacts for schema validation and version checks in WASM hosts. **Not a full database runtime** —
prefer [`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client) against `yydb serve` for durable reads and
writes.

Pulled when a browser bundle depends on this package directly. Most apps should not import it unless they embed the WASM
core explicitly.

## Example

```ts
import {initWasm, checkSchema, yydbVersion} from "@yydb/yydb-unknown-wasm32";

await initWasm();

console.log(yydbVersion());
console.log(
    checkSchema("table Setting { @@id: uuid, key: utf8 }"),
);
```

Call `initWasm()` once before semantic methods. Omit `options.module` to load the bundled `lib/yydb_wasm_bg.wasm` asset.

Low-level glue: `@yydb/yydb-unknown-wasm32/wasm` (raw `.wasm` path).

[YYDB overview](https://github.com/yy-database/yydb.rs)
