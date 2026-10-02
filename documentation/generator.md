# YYDB generator (`yydb generate`)

VOS schema → typed TypeScript for `@yydb/yydb` clients.

This is a **component of `yydb-tools`** (the `yydb` binary), not a separate published crate. Implementation lives under
`projects/crates/yydb-tools/src/generator/`.

## Pipeline

1. **Parse** — interim VOS *subset* lexer (one or more `table` decls, `@@id`, scalar fields). Replace with `vos::parser`
   when the language crate exposes a stable check/parse API.
2. **IR** — `TsSchemaIr` (tables / fields / scalars / embedded VOS source + schema version).
3. **Emit** — build an OXC `Program` with `AstBuilder`, print with
   `oxc_codegen`. Declaration text is not string-concatenated.

## CLI

```text
yydb generate <schema.vos> -o schema.generated.ts [--schema-version N]
```

Emits:

- `APP_SCHEMA_VERSION` / `APP_SCHEMA` (database truth for `ensureSchema`)
- `{Table}Row` interfaces
- `AppDb` with `tables: { … }`
- `AppDbTableName = keyof AppDb["tables"]`

## Fixture

`projects/crates/yydb-tools/fixtures/generator/setting.vos` →
`setting.generated.ts` (snapshot in `cargo test -p yydb-tools`).

## Out of scope (MVP)

- Full VOS grammar, relations, indexes, enums
- Wiring `Database.open<AppDb>` in the npm package (consumes this output later)
- Dejavu multi-target artifacts (see `vos-language/docs/generator.md` for non-TS targets)
