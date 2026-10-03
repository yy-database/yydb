# yydb-pyo3

Thin PyO3 binding for Python hosts. Engine logic lives in
[`yydb`](../yydb/readme.md); serve loops through [`yydb-server`](../yydb-server/readme.md).
This crate must not depend on N-API / TypeScript packages.
