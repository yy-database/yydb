---
name: yydb-generator
description: >-
    VOS to TypeScript schema codegen via `yydb generate` and `@yydb/yydb/generator`.
    Load when extending generator, CLI flags, or generated row types.
---

# VOS → TypeScript schema codegen

`yydb generate` ships with `@yydb/yydb` and replaces the removed Rust `yydb-tools` generator.

## Usage

```bash
yydb generate schema.vos -o src/schema.generated.ts
yydb generate schema.vos -o src/schema.generated.ts --schema-version 2
```

## Supported VOS subset (MVP)

- One or more `table Name { ... }` declarations
- Primary key via `@@id: <scalar>`
- Scalar fields: `uuid`, `utf8`, `bool`, `i64`, `f64`
- UTF-8 input with optional BOM strip and CRLF normalization

## Output

- `APP_SCHEMA_VERSION` — from `--schema-version`
- `APP_SCHEMA` — embedded VOS source
- `<Table>Row` interfaces per table
- `AppDb` with `tables` map
- `AppDbTableName` — `keyof AppDb["tables"]`

Generated files include a banner — do not edit manually.

## Library API

```ts
import {generateTypeScript} from "@yydb/yydb/generator";
```

CLI and library share `projects/packages/yydb/src/generator/`.
