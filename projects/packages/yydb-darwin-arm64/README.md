# `@yydb/yydb-darwin-arm64`

**The YYDB native binding for Apple Silicon Macs.**

This package is selected automatically when you install
[`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb) on `darwin/arm64`. Most applications should install the parent
package rather than depend on this platform package directly.

```bash
npm install @yydb/yydb
```

The TypeScript `yydb` CLI ships from `@yydb/yydb` and loads this `.node` binding:

```text
npx yydb --help
npx yydb serve app.yydb --bind 127.0.0.1:7700
```

| Field       | Value                                 |
|-------------|---------------------------------------|
| npm package | `@yydb/yydb-darwin-arm64`             |
| Platform    | macOS, Apple Silicon (`darwin/arm64`) |
| Binding     | `lib/yydb-darwin-arm64.node`              |

For Intel Macs, use [`@yydb/yydb-darwin-x64`](https://www.npmjs.com/package/@yydb/yydb-darwin-x64).

[YYDB overview](https://github.com/yy-database/yydb.rs) ·
[`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb) ·
[License](https://github.com/yy-database/yydb.rs/blob/dev/License.md)
