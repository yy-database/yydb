# 包与版本

## npm 包

| 包 | 职责 | 典型使用方 |
|----|------|------------|
| `@yydb/yydb` | Node.js 嵌入式引擎、原生绑定解析与 `yydb` CLI | Node 应用、脚本、CI |
| `@yydb/light-db` | 浏览器引擎，WASM 会话运行在 OPFS 或内存 | 前端应用 |
| `@yydb/yydb-client` | 线协议客户端与帧编解码 | 连接 `serve` 的客户端、WebUI |
| `@yydb/webui` | 本地数据库浏览界面 | 开发与排查 |
| `@yydb/yydb-skills` | 面向集成方的 Agent Skills | 代码代理与工具集成 |
| `@yydb/sqlite` | 只读 SQLite format-3 快照读取器 | 数据迁移与互操作校验 |

## 平台包

| 包 | 目标 |
|----|------|
| `@yydb/yydb-win32-x64` | Windows x64 |
| `@yydb/yydb-linux-x64` | Linux x64 |
| `@yydb/yydb-darwin-x64` | macOS x64 |
| `@yydb/yydb-darwin-arm64` | macOS arm64 |

平台包是 `@yydb/yydb` 的可选依赖。它们不包含 TypeScript 门面，也不应该被应用直接依赖。

## Rust crate

| crate | 作用 |
|-------|------|
| `yydb` | 统一门面：`Connection`、`Value`、错误、预导入模块 |
| `yydb-client` | 到 `yydb serve` 的 TCP 客户端 |
| `yydb-server` | 本地服务实现 |

应用只需依赖 `yydb`。其余 crate 属于门面内部或高级集成场景。

## 版本同步

- 所有 npm 包使用同一版本号一起发布，避免门面与平台包错配。
- 线协议版本独立于包版本，当前为 `0000`。包升级不会自动改变协议版本。
- 文件格式版本独立于包版本与协议版本。旧文件不会因为升级包而自动改写。
- `1.0.0` 之前不承诺旧 API、旧协议形状或旧文件布局的兼容；升级前应重新生成派生数据并跑一致性检查。

## 检查实际版本

```bash
npx @yydb/yydb version
```

```ts
console.log(db.serverVersion());
```

两者输出同一个引擎版本字符串。

## 选择建议

- 只在 Node 中使用：装 `@yydb/yydb`。
- 只在浏览器中使用：装 `@yydb/light-db`。
- 浏览器还要连本地服务：再加 `@yydb/yydb-client`。
- 需要读取 SQLite 快照做迁移：加 `@yydb/sqlite`，并注意它不执行 SQL。
