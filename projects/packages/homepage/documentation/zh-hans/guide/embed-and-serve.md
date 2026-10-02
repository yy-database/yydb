# 嵌入与 Serve

| 角色                  | 包                      | 说明                                                          |
|-----------------------|-------------------------|---------------------------------------------------------------|
| YYDB 进程内嵌入       | `projects/crates/yydb`（Rust） | 仅 Rust 宿主；UDF                                             |
| YYDB Rust 远程客户端  | `projects/crates/yydb-client`  | TCP → `yydb serve`                                            |
| **TS 产品包（推荐）** | `@yydb/yydb`            | **一个依赖**：`Database.open("app.yydb")`，自动管理引擎二进制 |
| TS 线协议             | `@yydb/yydb-client`     | WebUI / 浏览器 / 高级用法                                     |

TypeScript 应用开发者应依赖 **`@yydb/yydb`**，不要自己下载 `yydb.exe`
或手写 `serve`。平台二进制通过 `optionalDependencies` 随 npm 安装。

```ts
import {Database} from "@yydb/yydb";

const db = await Database.open("app.yydb");
await db.ensureSchema(1, "table Setting { @@id: uuid, key: utf8 }");
await db.put("k", "v");
await db.close();
```

底层仍是同一套 Rust 内核（loopback 私有进程），不是第二套 JS 存储实现。 帧头 `YYDB`/`YYDS` 自称等效；需要内置权限/审计等重能力时用
YYDS。
