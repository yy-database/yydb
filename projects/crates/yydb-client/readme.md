# yydb-client

Rust remote client for `yydb serve` (TCP binary wire v1).

Depends on [`yydb`](../yydb/readme.md) only — types and wire codecs are taken
from the facade (`yydb::Error`, `yydb::wire`, …). For in-process access use
`yydb::Connection` instead.

TypeScript and browser hosts should use `@yydb/yydb-client`.
