# VOS → TypeScript schema codegen

`yydb generate` ships with `@yydb/yydb` and replaces the removed Rust `yydb-tools`
generator. It parses a small VOS subset and writes a TypeScript module with schema
metadata and row types.

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

The generator writes:

- `APP_SCHEMA_VERSION` — numeric version from `--schema-version`
- `APP_SCHEMA` — embedded VOS source string
- `<Table>Row` interfaces for each table
- `AppDb` with a `tables` map
- `AppDbTableName` — `keyof AppDb["tables"]`

Generated files include a banner comment and should not be edited manually.

## Library API

```ts
import { generateTypeScript } from "@yydb/yydb/generator";
```

The CLI and library share the same parser and emitter under
`projects/packages/yydb/src/generator/`.
