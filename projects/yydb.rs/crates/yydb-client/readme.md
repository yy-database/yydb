# `yydb-client`

Rust remote client for `yydb serve` over the YY binary wire (TCP). Message types and framing come from
[`yydb-types`](https://crates.io/crates/yydb-types) and [`yydb-protocol`](https://crates.io/crates/yydb-protocol);
this crate never depends on the embedded facade.

## When to use

- Your Rust process runs next to a `yydb serve` endpoint and must talk to it over the wire.
- You need the same record and schema operations as the TypeScript client, with the same error contract.

For in-process access use `yydb::Connection`. TypeScript and browser hosts use
[`@yydb/yydb-client`](https://www.npmjs.com/package/@yydb/yydb-client).

## Example

```rust
use yydb_client::Client;

let client = Client::connect("127.0.0.1:7070")?;
client.ensure_schema("table Setting { @@id: uuid, key: utf8 }")?;
client.put("theme", b"dark")?;
assert_eq!(client.get("theme")?, Some(b"dark".to_vec()));
```

`yydb serve` has **no ACL**. Connect only to loopback endpoints you control.

[docs.rs](https://docs.rs/yydb-client) · [YYDB overview](https://github.com/yy-database/yydb)
