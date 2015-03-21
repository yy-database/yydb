# Serve 与 CLI

`yydb` CLI 随 `@yydb/yydb` 发布，可用 `npx` 运行，无需全局安装。

## 命令一览

| 命令 | 作用 |
|------|------|
| `yydb version` | 打印引擎版本 |
| `yydb init ./app.yydb --schema-file schema.vos` | 创建文件并可选择发布 schema |
| `yydb info ./app.yydb --doctor` | 输出路径、schema、记录数、journal 模式；`--doctor` 追加一致性报告 |
| `yydb verify ./app.yydb` | 只读一致性检查 |
| `yydb serve ./app.yydb --bind 127.0.0.1:7700` | 启动线协议端点 |
| `yydb webui ./app.yydb` | 启动本地 WebUI 并打开浏览器 |

## verify 退出码

| 退出码 | 含义 |
|--------|------|
| `0` | 未发现问题 |
| `13` | 仅有警告 |
| `1` | 存在错误 |

CI 中可以据此区分「必须停止」与「需要人工复核」。

## serve 的选项

| 选项 | 默认 | 说明 |
|------|------|------|
| `--bind <host:port>` | `127.0.0.1:7700` | 监听地址；端口与路径由部署决定 |
| `--insecure-bind` | 关闭 | 允许绑定非回环地址 |

服务同时接受 TCP 与 WebSocket `/wire`。

## WebUI

```bash
npx @yydb/yydb webui ./app.yydb
```

| 选项 | 默认 | 说明 |
|------|------|------|
| `--bind <host:port>` | `127.0.0.1:7700` | 底层 serve 的绑定地址 |
| `--port <number>` | `8780` | WebUI 页面端口 |
| `--endpoint <url>` | 由 bind 推导 | 连接已有 serve，而不是新起一个 |
| `--no-open` | 关闭 | 不自动打开浏览器 |

WebUI 是本地调试界面，不是多用户控制台。

## 安全模型

> 警告：参考 `yydb serve` 没有登录、租户、角色、授权或审计。任何能连上端点的客户端都可以执行协议允许的全部操作。

因此：

1. 默认只绑定 `127.0.0.1`，不要为了图方便改成 `0.0.0.0`。
2. 需要跨机器访问时，用 SSH 隧道、VPN 或反向代理限制来源，而不是依赖 `serve` 自身。
3. 需要身份、权限、审计与配额时使用 YYDS。
4. 操作系统文件权限才是数据的真正边界：能读 `.yydb` 文件的人不需要经过服务。

## 什么时候需要 serve

- 浏览器或另一进程需要访问 Node 侧数据。
- 需要单写者模型，避免多进程写锁冲突。
- 需要统一在本地暴露一个受控端点。

如果应用本身就在 Node 进程内，直接 `Database.open` 更简单，也没有网络攻击面。
