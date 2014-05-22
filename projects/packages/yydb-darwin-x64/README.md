# `@yydb/yydb-darwin-x64`

Prebuilt native binding for Intel macOS (`darwin/x64`). **Not a public import target.**

Pulled automatically when installing [`@yydb/yydb`](https://www.npmjs.com/package/@yydb/yydb) on matching hosts. Apple
Silicon Macs use [`@yydb/yydb-darwin-arm64`](https://www.npmjs.com/package/@yydb/yydb-darwin-arm64).

## Example

Application code imports the main package only:

```ts
import {Database} from "@yydb/yydb";

const db = await Database.open("app.yydb");
console.log(await db.info());
await db.close();
```

The CLI also ships from `@yydb/yydb`:

```text
npx yydb serve app.yydb --bind 127.0.0.1:7700
```

[YYDB overview](https://github.com/yy-database/yydb.rs)
