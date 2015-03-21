# Quickstart

This page completes one full round trip: install, open a file, publish a schema, write a record, read it back.

## 1. Install

```bash
npm install @yydb/yydb
```

Node.js 18 or later is required. The native binding is selected automatically from optional dependencies, so there is nothing to download separately.

## 2. Open a file

```ts
import { Database } from "@yydb/yydb";

const db = await Database.open("app.yydb");
```

`Database.open` creates missing parent directories and creates a new database when the file does not exist. The path is resolved to an absolute path and readable from `db.path`.

## 3. Publish a schema

```ts
await db.ensureSchema(`
table User {
    @@id: i64,
    name: utf8,
    email: utf8,
}
`);
```

The schema document is persisted inside the file with a version number. Publishing the same document again does not create a new version.

## 4. Write

```ts
await db.execute('User { id: 1, name: "Ada", email: "ada@example.com" }.insert()');
```

Writes use VOS write programs, not SQL. Strings use double quotes and field names must match the schema.

## 5. Read

```ts
const users = await db.query<User[]>("User.all().collect()");
console.log(users);
```

Read pipelines return plain JavaScript objects. Numbers, booleans and strings keep their original type. Exact scalars (`timestamp`, `decimal`) are returned as tagged objects so precision is not lost.

## 6. Close

```ts
await db.close();
```

Closing is best-effort, and the native handle is also reclaimed when the process exits. When data must be guaranteed on disk across processes, run `await db.checkpoint()` first.

## Full example

```ts
import { Database } from "@yydb/yydb";

const db = await Database.open("app.yydb");

await db.ensureSchema(`
table User {
    @@id: i64,
    name: utf8,
    email: utf8,
}
`);

await db.execute(
    'User { id: 1, name: "Ada", email: "ada@example.com" }.insert()',
);

const rows = await db.query("User.all().collect()");
console.log(rows.length); // 1

await db.checkpoint();
await db.close();
```

## Next

- [Schema and VOS): types, primary keys and evolution
- [Reading and writing): transactions, prepared statements and scans
- [Node.js integration): backup, restore and diagnostics
