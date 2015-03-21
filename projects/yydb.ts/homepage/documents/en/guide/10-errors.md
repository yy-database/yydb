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

## Common error codes

| Error code | Meaning | Action |
|------------|---------|--------|
| `yydb.io` | File read or write failed | Check path, permissions and disk space |
| `yydb.corrupt` | CRC32C check failed | Stop writing, locate the issue with `yydb verify` and `doctor()`, restore from backup |
| `yydb.schema.invalid` | Invalid schema document | Fix the VOS syntax and validate first with `yydb init --schema-file` |
| `yydb.schema.conflict` | Schema shape is incompatible with existing records | Use `migrateSchema`, supplying `renames` when needed |
| `yydb.unique.violation` | Unique constraint violated | Read `params.columns` to find the field, then rewrite or delete the conflicting record |
| `yydb.query.bind` | Prepared statement parameters missing, extra or mistyped | Compare the bound object against `stmt.parameters` |
| `yydb.statement.closed` | A closed statement was executed again | Call `prepare` again. Do not reuse a handle after closing |
| `yydb.object.not_found` | A content-addressed object is missing | Check whether the backup copied only the main file and missed object chunks |
| `yydb.object.corrupt` | Object chunk checksum failed | Restore from backup. Do not try to skip the bad chunk |
| `yydb.protocol` | Invalid wire frame | Verify magic, version, `msg_type` and body length |
| `yydb.udf.not_found` | The UDF being called is not registered | Register before calling, or check whether the session was recreated |
| `yydb.scalar.invalid` | Invalid exact scalar format | Read `params.kind` and rewrite per date/time/timestamp/decimal rules |

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
