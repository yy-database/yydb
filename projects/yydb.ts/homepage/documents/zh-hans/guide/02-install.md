# 安装与平台

## 环境要求

| 项目 | 要求 | 说明 |
|------|------|------|
| Node.js | 18 或更高 | `@yydb/yydb` 与 `@yydb/yydb-client` 的 `engines` 要求 |
| 包管理器 | npm / pnpm / yarn | 必须支持可选依赖 |
| 操作系统 | Windows x64、Linux x64、macOS x64、macOS arm64 | 原生绑定按平台解析 |
| 浏览器 | 支持 OPFS 与 WebAssembly | 仅 `@yydb/light-db` 需要 |

## Node.js

```bash
npm install @yydb/yydb
```

主包包含 TypeScript 门面、CLI 与按平台解析原生绑定的逻辑。

## 平台二进制

以下包是 `@yydb/yydb` 的可选依赖，正常安装无需手动处理。

| 包 | 目标平台 |
|----|----------|
| `@yydb/yydb-win32-x64` | Windows x64 |
| `@yydb/yydb-linux-x64` | Linux x64 |
| `@yydb/yydb-darwin-x64` | macOS x64（Intel） |
| `@yydb/yydb-darwin-arm64` | macOS arm64（Apple Silicon） |

如果没有匹配平台包，模块会给出明确的解析失败信息，不会静默退回到另一种实现。可以用 `pnpm add --allow-build` 之类的选项检查安装日志，确认可选依赖没有被裁剪。

## Rust

```bash
cargo add yydb
```

```rust
use yydb::prelude::*;

fn main() {
    let db = Connection::open("app.yydb").expect("open database");
    // 每个 peer 使用 db.session() 建立独立会话
}
```

`Connection` 由单个进程持有；多连接场景在同一 `Connection` 上创建会话，而不是重复打开文件。

## 浏览器

```bash
npm install @yydb/light-db
```

```ts
import { openLightDatabase } from "@yydb/light-db";

const db = await openLightDatabase("app");
const rows = await db.query("User.all().collect()");
```

浏览器入口使用 WASM，不包含 Node 内置模块，也不暴露 SQLite 快照读取器。

## 远程客户端

```bash
npm install @yydb/yydb-client
```

```ts
import { Client } from "@yydb/yydb-client";

const client = await Client.connect("ws://127.0.0.1:7700/wire");
```

Node.js 的 TCP 入口从子路径导入：`@yydb/yydb-client/node`。

## 验证安装

```bash
npx @yydb/yydb version
npx @yydb/yydb init ./app.yydb
npx @yydb/yydb info ./app.yydb
```

`info` 输出路径、schema 版本、记录数与 journal 模式。追加 `--doctor` 会执行只读一致性检查。

## 卸载

```bash
npm uninstall @yydb/yydb
```

数据库文件不会被自动删除。备份或移除 `.yydb`、`.yydb-wal` 与 `.yydb-shm` 需要自行操作。

> 警告：`serve` 没有身份校验层。不要在未受信任网络中暴露绑定地址。
