# yydb-query

Phase 1 VOS query lowering, read pipelines, and insert/write DML against the
YYDB record map. Depends on [`yydb-types`](../yydb-types/readme.md) and `vos`.

Embedded hosts should depend on [`yydb`](../yydb/readme.md) and use
`yydb::query` (this crate re-exported). Direct `yydb-query` is for engine work
and tests only.
