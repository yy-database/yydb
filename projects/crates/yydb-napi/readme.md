# yydb-napi

Thin Node-API binding for TypeScript (`@yydb/yydb`). Engine logic lives in
[`yydb`](../yydb/readme.md); serve loops through [`yydb-server`](../yydb-server/readme.md).
This crate must not depend on Python/WASM bindings.
