# `@yydb/yydb-win32-x64`

**The YYDB native binding for Windows x64.**

This package is selected automatically when you install
[`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb) on `win32/x64`. Most applications should install the parent
package rather than depend on this platform package directly.

```bash
npm install @yydb/yydb
```

The TypeScript `yydb` CLI ships from `@yydb/yydb` and loads this `.node` binding:

```text
npx yydb --help
npx yydb serve app.yydb --bind 127.0.0.1:7700
```

| Field       | Value                     |
|-------------|---------------------------|
| npm package | `@yydb/yydb-win32-x64`    |
| Platform    | Windows x64 (`win32/x64`) |
| Binding     | `yydb.win32-x64.node`     |

Other platforms install as `@yydb/yydb-linux-x64`, `@yydb/yydb-darwin-x64`, and `@yydb/yydb-darwin-arm64` via `@yydb/yydb` optionalDependencies.

[YYDB overview](https://github.com/yy-database/yydb.rs) ·
[License](https://github.com/yy-database/yydb.rs/blob/dev/License.md)
