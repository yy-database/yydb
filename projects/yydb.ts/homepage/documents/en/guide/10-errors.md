# Troubleshooting

## The error object

```ts
import { YydbError, isYydbError } from "@yydb/yydb";

try {
    await db.execute('User { id: 1, name: "Ada" }.insert()');
} catch (err) {
    if (isYydbError(err)) {
        console.error(err.code, err.message, err.params);
    }
}
```

`code` is the stable diagnostic and `params` carries structured context. Never branch on `message` text.

## Engine error codes

This table mirrors `Error::code()` in the Rust crate `yydb-types`, which is the only source. Remote errors pass the peer's code through untouched, so a client may also receive a code minted elsewhere.

### Storage and integrity

| Error code | Meaning | Action |
|------------|---------|--------|
| `yydb.io` | File read or write failed | Check path, permissions and disk space |
| `yydb.corrupt` | Page CRC32C check failed | Stop writing, locate the issue with `yydb verify` and `doctor()`, restore from backup |
| `yydb.object.not_found` | A content-addressed object is missing | Check whether the backup copied only the main file and missed object chunks |
| `yydb.object.corrupt` | Object chunk checksum failed | Restore from backup. Do not try to skip the bad chunk |
| `yydb.unsupported` | The current file or platform does not support the requested capability | Follow the message and use a supported entry, for example skip `compact()` on a single-file `.yydb` |

### Schema and queries

| Error code | Meaning | Action |
|------------|---------|--------|
| `yydb.schema.invalid` | Schema document, VOS program or bound value is invalid | Fix the VOS syntax and validate first with `yydb init --schema-file` |
| `yydb.schema.conflict` | Schema shape is incompatible with existing records | Use `migrateSchema`, supplying `renames` when needed |
| `yydb.schema.invalidated` | The schema revision a statement was built on has been replaced | Drop prepared statements and old result shapes, then `prepare` again |
| `yydb.schema.vector_dimension_exceeded` | Vector dimension exceeds the limit | Split into multiple vectors or use object storage |
| `yydb.unique.violation` | Unique constraint violated | Read `params.columns` to find the field, then rewrite or delete the conflicting record |
| `yydb.query.bind` | Parameters missing, extra or of the wrong type | Compare the bound object against `stmt.parameters` |
| `yydb.scalar.invalid` | Invalid exact scalar format | Read `params.kind` and rewrite per date/time/timestamp/decimal rules |
| `yydb.result.format` | The requested result format cannot represent a column value | Read `params.column` and switch formats or read that column row by row |

### UDF

| Error code | Meaning | Action |
|------------|---------|--------|
| `yydb.udf.not_found` | The UDF being called is not registered | Register before calling, or check whether the session was recreated |
| `yydb.udf.arity` | Argument count does not match the registered signature | Match the `args` supplied at registration |
| `yydb.udf.version_mismatch` | Registered and call versions differ | Register again with a single version |
| `yydb.udf.failed` | The host callback raised or returned an unexpected value | Read `params.message` and fix the host implementation |

### Transactions, concurrency and leases

| Error code | Meaning | Action |
|------------|---------|--------|
| `yydb.cas.conflict` | Compare-and-swap expected value did not match | Re-read and retry |
| `yydb.transaction.conflict` | Durable state advanced under the transaction | Roll back and start a new transaction |
| `yydb.lease.unavailable` | The key's lease is already held | Wait for release or pick another key |
| `yydb.fencing.mismatch` | Lease token does not match the current fencing value | Acquire the lease again |
| `yydb.lease.expired` | The lease has expired | Acquire the lease again |
| `yydb.session.closed` | The session or connection is closed | Open a new session. Do not reuse a released handle |
| `yydb.quota.exceeded` | Namespace usage exceeds its quota | Read `params.namespace`, clean up or raise the quota |

### Wire protocol and security

| Error code | Meaning | Action |
|------------|---------|--------|
| `yydb.protocol` | Invalid wire frame | Verify magic, version, `msg_type` and body length |
| `yydb.auth.token_invalid` | Invalid token | Issue a new token |
| `yydb.auth.session_expired` | The session expired | Sign in again |
| `yydb.auth.principal_not_found` | The principal does not exist | Check principal registration |
| `yydb.auth.anonymous_remote_denied` | Anonymous remote access denied | Supply credentials or stay on loopback |
| `yydb.auth.config_missing` | Authentication configuration missing | Complete the server auth configuration |
| `yydb.auth.insufficient_permission` | Insufficient permission | Request `params.required` from an administrator |

## Host-local error codes

These codes are minted by the TypeScript and Python hosts, not by `yydb-types`, and only describe host boundaries.

| Error code | Meaning | Action |
|------------|---------|--------|
| `yydb.client.invalid_endpoint` | The client endpoint cannot be parsed | Check the `yydb://` or `ws://` spelling |
| `yydb.client.transport_failure` | The transport closed or timed out | Check the server process and network, then retry |
| `yydb.client.request_id_mismatch` | Response request id differs from the request | Drop the response and reconnect |
| `yydb.client.unexpected_response` | An unexpected message type arrived | Verify the protocol version |
| `yydb.client.decode_failure` | The frame body cannot be decoded | Verify both protocol versions and the body length |
| `yydb.statement.closed` | A closed statement was executed again | Call `prepare` again. Do not reuse a handle after closing |

## Public limits

- `walPath()` / `wal_path()` only reports the sidecar path. It **does not guarantee the file exists**: the WAL may not be created yet, may already be folded, or the session may not use one at all. Do not use its presence as a commit signal. Use `journalMode()` / `walFrameCount()`, or `await db.checkpoint()` first.
- `doctor()` is read-only and **never repairs damaged pages**. It does not rewrite the WAL, reclaim orphans or modify a single byte. Reclaim orphans with `compact()` on `.yydx`. Damaged pages must be restored from a backup with `restore()` or rebuilt.
- `restore()` verifies page CRCs for `.yydb` or the object closure for `.yydx` before returning a handle. It does not repair the backup: a failed check refuses to open.
- `yydb import` and `yydb export` are explicit ETL, not a SQLite compatibility layer. The declared subset is exactly `records(key TEXT PRIMARY KEY, value BLOB NOT NULL)`. Import refuses other column shapes, non-`TEXT` primary keys, non-`BLOB` values and keys in the reserved `__yydb/` namespace. Export skips `__yydb/` keys. Import replaces keys and preserves no SQLite types, indexes, constraints, triggers or row order.

## Open failures

| Symptom | Cause | Action |
|---------|-------|--------|
| `ENOENT` or a permission error | Parent directory is not writable | Use a writable path, or confirm `mkdir` was not disabled |
| Format mismatch | The file is not `YDPG` | A SQLite file cannot be renamed into a `.yydb` |
| Write-lock conflict | Another process holds the writer lock | Close the duplicate process |

## Native binding not found

The module reports a clear resolution failure. Check:

1. Whether the platform is in the supported list.
2. Whether the install used a configuration that prunes optional dependencies.
3. Whether bundling or pruning `@yydb/yydb` dropped the platform package from the output.

Fix it by explicitly installing the matching `@yydb/yydb-*` package, or by switching to the browser entry.

## Data seems not to be on disk

- An unfolded WAL is a normal state. Use `yydb info` to inspect the journal mode.
- When consistency is required before copying the file, run `await db.checkpoint()` first.
- Copying only the main file and ignoring the WAL loses the most recent commits. `backup()` already handles this.

## verify failures

| Exit code | Action |
|-----------|--------|
| `13` | Record the warning and schedule a review. Do not treat it as "all clear" |
| `1` | Stop writing, preserve the scene, restore from backup or locate the damaged page from the report |

## Still stuck

Bring the following to GitHub Discussions:

- The output of `yydb version`
- Your operating system and platform
- The output of `yydb info ./app.yydb --doctor` (confirm it contains no sensitive business data first)
- The smallest sequence of calls that reproduces the problem

If it is a reproducible defect, file an Issue instead.
