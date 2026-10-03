# VOS → TypeScript schema codegen

The former Rust `yydb generate` command lived in the removed `yydb-tools` crate.

Schema codegen is **not** part of the Rust workspace anymore. The only supported
CLI is the TypeScript `yydb` command shipped by `@yydb/yydb`. A TypeScript
generator will land in that package when the Oak/VOS codegen contract is wired
for Node hosts.

Until then, author VOS schema documents directly or generate types from your
application build pipeline outside this repository.
