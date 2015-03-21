# Schema and VOS

YYDB does not accept SQL. Schemas, queries and writes all use VOS.

## Schema document

```vos
table User {
    @@id: i64,
    name: utf8,
    email: utf8,
}
```

- `@@id` declares the primary key. Exactly one is required.
- Field names are case-sensitive. Writes must use the declared names.
- The schema document is published through `ensureSchema`, stored in the file, and carries a version number.

## Field types

| Type | Meaning | JavaScript shape |
|------|---------|------------------|
| `i64` | 64-bit signed integer | `number` (treat as a string beyond safe-integer range) |
| `f64` | Double-precision float | `number` |
| `utf8` | UTF-8 text | `string` |
| `bytes` | Arbitrary bytes | Byte array or `Uint8Array` |
| `bool` | Boolean | `boolean` |
| `timestamp` | UTC microsecond timestamp | Object tagged with `$yydb: "timestamp"` |
| `decimal` | Fixed-point decimal | Object tagged with `$yydb: "decimal"` |

Exact scalars never degrade into strings or float approximations. `timestamp` stores microseconds as a decimal string and `decimal` stores the unscaled integer plus its scale.

## Primary keys and record keys

The primary key value determines the physical record key. `id` must be supplied explicitly on write. There is no auto-increment key.

```ts
await db.execute('User { id: 2, name: "Grace", email: "grace@example.com" }.insert()');
```

How a repeated write to the same primary key behaves is up to the write program. Do not rely on implicit deduplication and do not treat the primary key as a unique business field.

## Uniqueness

A schema can declare unique constraints. Violating one returns `yydb.unique.violation`, and the whole write rolls back without leaving partial records.

## Schema evolution

```ts
await db.migrateSchema(
    `
table User {
    @@id: i64,
    name: utf8,
    email: utf8,
    createdAt: timestamp,
}
`,
    { renames: [{ type: "User", fromField: "displayName", toField: "name" }] },
);
```

- `ensureSchema` only publishes a document that matches the current one. Shape changes are rejected.
- `migrateSchema` allows shape changes and migrates existing records when needed.
- `renames` tells the engine which old field names map to new ones. Without it, structurally unrelated records are treated as incompatible.
- The schema version increases with every successful publish. Read the current version and original document with `db.getSchema()`.

## Read the current schema

```ts
const current = await db.getSchema();
console.log(current?.version);
console.log(current?.document);
```

Returns `null` when no schema has ever been published.

## Validate on the command line

```bash
npx @yydb/yydb init ./app.yydb --schema-file schema.vos
```

This validates the VOS document before any data is written. An invalid schema returns `yydb.schema.invalid` with the offending location.
