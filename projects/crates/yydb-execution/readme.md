# yydb-execution

Stable local execution IR owned by YYDB. VOS frontends lower into this model
through the [`yydb`](../yydb/readme.md) binder.

Applications should depend on **`yydb`** (`yydb::execution`). Take a direct
`yydb-execution` dependency only when extending the execution layer.
