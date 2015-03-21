# 浏览器接入

浏览器使用 `@yydb/light-db`。它把同一套 Rust 语义核心编译为 WASM，会话数据写入源私有文件系统（OPFS）。

## 安装与打开

```bash
npm install @yydb/light-db
```

```ts
import { openLightDatabase } from "@yydb/light-db";

const db = await openLightDatabase("app");
```

`openLightDatabase` 返回与 Node 版本同形的数据库句柄：`ensureSchema`、`query`、`execute`、`transaction`、`prepare` 与键值接口都可用。

## 内存会话

```ts
import { Database } from "@yydb/light-db";

const db = await Database.open("scratch", { memory: true });
```

内存会话不落盘，适合演示、测试与临时计算。页面刷新后数据消失。

## 持久会话

持久会话使用 OPFS。注意以下边界：

- 数据属于当前源（origin），换域名或协议即视为另一个存储空间。
- 无痕窗口与清理站点数据会删除数据库。
- 浏览器配额由用户代理管理，写满时返回配额相关错误。
- 同一源内多个标签页共享存储，但每个标签页有独立会话；不要假设跨标签页立即可见。

## 与 Node 版本的区别

| 能力 | Node（`@yydb/yydb`） | 浏览器（`@yydb/light-db`） |
|------|----------------------|----------------------------|
| 打开本地路径 | 支持 | 不支持 |
| OPFS 持久化 | 不适用 | 支持 |
| 原生绑定 | 需要 | 不需要 |
| 进程内 UDF | 支持 | 受限，仅 WASM 会话内 |
| 文件备份到任意路径 | 支持 | 不支持，导出交由应用处理 |
| 线协议客户端 | 通过 `@yydb/yydb-client` | 同一包，走 WebSocket |

## 转换为可下载文件

浏览器不能把会话直接映射到用户可选的本地路径。需要导出时，应用自行读取键值或查询结果并序列化，再由页面触发下载。不要把 OPFS 路径暴露给用户当作可移植的 `.yydb` 文件。

## 连接远程 serve

浏览器中也使用 `@yydb/yydb-client`：

```ts
import { Client } from "@yydb/yydb-client";

const client = await Client.connect("ws://127.0.0.1:7700/wire");
```

线协议只覆盖 Hello、Info、schema、键值与微 UDF，不包含通用 VOS 查询。需要完整记录查询时，应把数据放在 `light-db` 本地会话，或通过服务端 API 自行封装。

## 建议

- 首屏启动时只打开会话，schema 发布延后到确实需要写入时。
- 大对象写入前先确认剩余配额。
- 用 `transaction` 包裹多步写入，避免部分成功。
