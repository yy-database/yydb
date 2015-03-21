# Reading and writing

YYDB has two data paths: the VOS record path and the key-value path. Both share the same file and transaction boundary.

## VOS read pipelines

```ts
const all = await db.query("User.all().collect()");
```

- `query` accepts a read pipeline and returns an array of plain objects.
- Result order is defined by the pipeline. `all()` makes no promise about physical order.
- Reading requires a published schema. Otherwise you get a schema-related error.

## VOS write programs

```ts
await db.execute('User { id: 1, name: "Ada" }.insert()');
```

- `execute` accepts a write program and returns `void`.
- The write is committed before it returns. A failure leaves no partial records.
- Strings use double quotes. Field names must match the schema.

## Key-value path

When a schema is not needed, use the key-value API directly:

```ts
await db.put("config/theme", "dark");
const value = await db.get("config/theme"); // Uint8Array | null
await db.delete("config/theme");
```

`put` overwrites an existing key. `get` returns `null` for a missing key instead of throwing.

### Prefix scan

```ts
const rows = await db.scan({ prefix: "config/", limit: 100 });
for (const row of rows) {
    console.log(row.key, row.value);
}
```

`scan` returns rows in lexical order and accepts `prefix`, `after` and `limit`. `limit` defaults to 1000.

### Batch writes

```ts
await db.batch([
    { key: "counter", value: "1" },
    { key: "stale", value: null }, // null deletes the key
]);
```

A batch is applied atomically within a single call.

### Compare and exchange

```ts
const swapped = await db.compareExchange("counter", "1", "2");
```

The value is replaced only when the current value strictly equals the expected one. Both `expected` and `replacement` may be `null`, meaning "absent" and "delete" respectively.

## Transactions

```ts
await db.transaction(async (tx) => {
    await tx.execute('User { id: 3, name: "Lin" }.insert()');
    await tx.put("audit/last", "3");
});
```

- Returning normally commits, throwing rolls back.
- Nested transactions are not supported. Merge the logic at the outer level when nesting semantics are needed.
- Reads inside a transaction observe writes already made by that transaction.

Explicit `begin` / `commit` / `rollback` is also available, but the callback form makes it harder to miss a rollback.

## Prepared statements

```ts
const stmt = await db.prepare("User.all().take($limit).collect()");
const first = await stmt.execute({ limit: 100 });
const second = await stmt.execute({ limit: 50 });
stmt.close();
```

- `prepare` parses and lowers a read pipeline once. Later executions skip parsing.
- Parameters are passed as a named object whose keys match `$name` in the pipeline.
- The schema must be published before execution.
- Call `close()` when done, otherwise the lowered plan stays allocated.

## Consistency checks

```ts
const report = await db.doctor();
console.log(report);
```

`doctor` runs read-only probes and reports consistency across pages, the WAL and object references. It never modifies the file.

## When to checkpoint

The WAL is folded on a clean close and when a threshold is reached. Before handing the file to another process, archiving a backup, or publishing a data file, run explicitly:

```ts
await db.checkpoint();
```

A checkpoint merges the WAL into the main file and cleans up companion files. It is not mandatory, but it makes the file boundary unambiguous.
