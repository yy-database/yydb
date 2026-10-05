# `@yydb/yydb-linux-x64`

**The YYDB native binding for Linux x64 (glibc).**

This package is selected automatically when you install
[`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb) on `linux/x64`. Most applications should install the parent
package rather than depend on this platform package directly.

```bash
npm install @yydb/yydb
```

The TypeScript `yydb` CLI ships from `@yydb/yydb` and loads this `.node` binding:

```text
npx yydb --help
npx yydb serve app.yydb --bind 127.0.0.1:7700
```

| Field       | Value                              |
|-------------|------------------------------------|
| npm package | `@yydb/yydb-linux-x64`             |
| Platform    | Linux x64 with glibc (`linux/x64`) |
| Binding     | `lib/yydb-linux-x64-gnu.node`              |

Install via `@yydb/yydb` optionalDependencies or add `@yydb/yydb-linux-x64` directly.

[YYDB overview](https://github.com/yy-database/yydb.rs) ·
[License](https://github.com/yy-database/yydb.rs/blob/dev/License.md)
