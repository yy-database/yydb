---
name: yydb-generator
description: >-
    Maintainer notes for the optional VOS-to-TypeScript typing helper (yydb
    generate, @yydb/yydb/generator). YYDB is not an ORM. Load only when hacking
    that helper or its snapshot tests — not for normal database integration.
---

# Optional VOS → TypeScript typing helper

**YYDB is an embedded database, not a generative ORM.** Runtime schema truth is a VOS document stored in the `.yydb`
file via `ensureSchema`. Applications talk to the engine with keys and values (and Phase 1 VOS query/DML on the Rust
side) — there is no ORM layer and no required codegen step.

`yydb generate` and `@yydb/yydb/generator` are a **small, optional dev convenience**: emit TypeScript interfaces
(`*Row`, `AppDb`) and an embedded `APP_SCHEMA` string so app code can type-check against a checked-in `.vos` file. They
do **not** generate queries, migrations, repositories, or runtime database code.

## When to load

- Changing the typing helper implementation, CLI flags, or snapshot tests
- Reviewing output shape (`APP_SCHEMA`, `*Row`, `AppDb`) for the helper only

## Do not load

- Normal YYDB integration (`Database.open`, `ensureSchema`, wire client)
- Positioning YYDB as an ORM or codegen-first product
- Wire protocol (`yydb-serve-protocol`) or on-disk format

## CLI (optional helper)

```bash
yydb generate schema.vos -o src/schema.generated.ts
yydb generate schema.vos -o src/schema.generated.ts --schema-version 2
```

Positional argument: VOS schema file path. `-o` / `--output` is required.

## Supported VOS subset (MVP)

- One or more `table Name { ... }` declarations
- Primary key via `@@id: <scalar>`
- Scalar fields: `uuid`, `utf8`, `bool`, `i64`, `f64`
- UTF-8 input with optional BOM strip and CRLF normalization

## Generated output (types only)

| Symbol               | Meaning                                    |
|----------------------|--------------------------------------------|
| `APP_SCHEMA_VERSION` | from `--schema-version` (default `1`)      |
| `APP_SCHEMA`         | embedded VOS source (single-quoted string) |
| `<Table>Row`         | one interface per table                    |
| `AppDb`              | `tables` map of row types                  |
| `AppDbTableName`     | `keyof AppDb['tables']`                    |

Files include a `// @generated` banner — do not edit manually. Regenerate when the `.vos` source changes; the database
file does not depend on this output.

## Library API

```ts
import { generateTypeScript, parseVosSubset } from '@yydb/yydb/generator';

const ts = generateTypeScript(vosSource, 1);
```

## Example prompt

```text
Load yydb-generator. I am changing the optional typing helper — extend the VOS subset parser for optional utf8 fields and update the snapshot test only.
```
