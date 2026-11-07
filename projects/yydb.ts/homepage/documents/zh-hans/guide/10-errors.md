# 故障排查

## 错误对象

```ts
import { YydbError, isYydbError } from "@yydb/yydb";

try {
    await db.execute('User { id: 1, name: "Ada" }.insert()');
} catch (err) {
    if (isYydbError(err)) {
        console.error(err.code, err.message, err.params);
    }
}
```

`code` 是稳定诊断，`params` 携带结构化上下文。不要匹配 `message` 文本做分支。

## 引擎错误码

本表与 Rust `yydb-types` 的 `Error::code()` 一一对应，那里是唯一来源。远端错误经 `Self::Remote` 原样透传对端错误码，因此客户端也可能收到本仓未铸造的码。

### 存储与完整性

| 错误码 | 含义 | 处理 |
|--------|------|------|
| `yydb.io` | 文件读写失败 | 检查路径、权限与磁盘空间 |
| `yydb.corrupt` | 页 CRC32C 校验失败 | 停止写入，用 `yydb verify` 与 `doctor()` 定位，从备份恢复 |
| `yydb.object.not_found` | 内容寻址对象缺失 | 检查备份是否只复制了主文件而漏掉对象分块 |
| `yydb.object.corrupt` | 对象分块校验失败 | 从备份恢复，不要尝试跳过坏块 |
| `yydb.unsupported` | 请求了当前文件或平台不支持的能力 | 按错误文本改走受支持入口，例如对单文件 `.yydb` 不要调 `compact()` |

### Schema 与查询

| 错误码 | 含义 | 处理 |
|--------|------|------|
| `yydb.schema.invalid` | schema 文档、VOS 程序或绑定值不合法 | 按 VOS 语法修正，先用 `yydb init --schema-file` 验证 |
| `yydb.schema.conflict` | schema 形状与现有记录不兼容 | 使用 `migrateSchema`，必要时提供 `renames` |
| `yydb.schema.invalidated` | 语句基于的 schema revision 已被替换 | 丢弃预处理语句与旧结果形状，重新 `prepare` |
| `yydb.schema.vector_dimension_exceeded` | 向量维度超过上限 | 拆分向量或改用对象存储 |
| `yydb.unique.violation` | 违反唯一约束 | 读取 `params.columns` 定位字段，改写或先删除冲突记录 |
| `yydb.query.bind` | 参数缺失、多余或不符 | 对照 `stmt.parameters` 检查绑定对象 |
| `yydb.scalar.invalid` | 精确标量格式不合法 | 读取 `params.kind`，按 date/time/timestamp/decimal 规则重写 |
| `yydb.result.format` | 请求的结果格式无法表示某列值 | 读取 `params.column` 换格式，或改为逐行读取该列 |

### UDF

| 错误码 | 含义 | 处理 |
|--------|------|------|
| `yydb.udf.not_found` | 调用的 UDF 未注册 | 先注册再调用，或检查会话是否已重建 |
| `yydb.udf.arity` | 实参个数与注册签名不符 | 对照注册时的 `args` 调整调用 |
| `yydb.udf.version_mismatch` | 注册版本与调用版本不符 | 统一版本后重新注册 |
| `yydb.udf.failed` | 宿主回调抛错或返回非预期值 | 读取 `params.message`，修复宿主实现 |

### 事务、并发与租约

| 错误码 | 含义 | 处理 |
|--------|------|------|
| `yydb.cas.conflict` | 比较交换的期望值不匹配 | 重新读取后重试 |
| `yydb.transaction.conflict` | 事务基于的持久状态已被推进 | 回滚并重新开始事务 |
| `yydb.lease.unavailable` | 目标键的租约已被占用 | 等待释放或换键 |
| `yydb.fencing.mismatch` | 租约令牌与当前 fencing 值不符 | 重新获取租约 |
| `yydb.lease.expired` | 租约已过期 | 重新获取租约 |
| `yydb.session.closed` | 会话或连接已关闭 | 打开新会话，不要复用已释放句柄 |
| `yydb.quota.exceeded` | 命名空间用量超配额 | 读取 `params.namespace`，清理或调高配额 |

### 线协议与安全

| 错误码 | 含义 | 处理 |
|--------|------|------|
| `yydb.protocol` | 线协议帧不合法 | 核对 magic、版本、`msg_type` 与 body 长度 |
| `yydb.auth.token_invalid` | 令牌无效 | 重新签发令牌 |
| `yydb.auth.session_expired` | 会话已过期 | 重新登录 |
| `yydb.auth.principal_not_found` | 主体不存在 | 检查主体注册 |
| `yydb.auth.anonymous_remote_denied` | 匿名访问远端被拒 | 提供凭据，或只在本机回环使用 |
| `yydb.auth.config_missing` | 缺少鉴权配置 | 补齐服务端鉴权配置 |
| `yydb.auth.insufficient_permission` | 权限不足 | 按 `params.required` 向管理员申请 |

## 宿主本地错误码

以下码由 TypeScript / Python 宿主自己铸造，不在 `yydb-types` 表内，语义只覆盖宿主边界。

| 错误码 | 含义 | 处理 |
|--------|------|------|
| `yydb.client.invalid_endpoint` | 客户端地址无法解析 | 检查 `yydb://` 或 `ws://` 端点拼写 |
| `yydb.client.transport_failure` | 传输层断开或超时 | 检查服务进程与网络，重试 |
| `yydb.client.request_id_mismatch` | 响应请求号与请求不符 | 丢弃该响应，重建连接 |
| `yydb.client.unexpected_response` | 收到未预期的消息类型 | 核对协议版本 |
| `yydb.client.decode_failure` | 帧体无法解码 | 核对双方协议版本与 body 长度 |
| `yydb.statement.closed` | 已关闭的语句被再次执行 | 重新 `prepare`，不要在关闭后复用句柄 |

## 公开限制

- `walPath()` / `wal_path()` 只返回伴生路径，**不保证文件存在**。WAL 可能尚未创建、已折叠，或该会话根本不落 WAL。不要用它的存在性判断提交是否生效，改用 `journalMode()` / `walFrameCount()`，或先 `await db.checkpoint()`。
- `doctor()` 是只读检查，**不修复损坏页**。它不会重写 WAL、不会回收孤儿对象、不会改写任何字节。孤儿对象回收用 `.yydx` 上的 `compact()`，损坏页只能从备份 `restore()` 或重建文件。
- `restore()` 在返回句柄前校验 `.yydb` 的页 CRC 或 `.yydx` 的对象闭包。它不修复备份本身：校验失败直接拒绝打开。

## 打开失败

| 症状 | 原因 | 处理 |
|---------|------|------|
| `ENOENT` 或权限错误 | 父目录不可写 | 改用可写路径，或确认 `mkdir` 未被关闭 |
| 格式不匹配 | 文件不是 `YDPG` | 外部数据库文件不能改后缀当作 `.yydb` |
| 写锁冲突 | 另一个进程持有写者锁 | 关闭重复打开的进程 |

## 找不到原生绑定

模块会明确报告解析失败。检查：

1. 平台是否在支持列表内。
2. 安装时是否用了会裁剪可选依赖的配置。
3. 是否在对 `@yydb/yydb` 做打包或裁剪，导致平台包未进入产物。

处理方式是显式安装对应的 `@yydb/yydb-*` 包，或改用浏览器入口。

## 数据看起来没落盘

- WAL 尚未折叠是正常状态。用 `yydb info` 查看 journal 模式。
- 需要在复制文件前保证一致性时，先 `await db.checkpoint()`。
- 只复制主文件而忽略 WAL 会丢失最近提交，`backup()` 已处理该问题。

## verify 失败

| 退出码 | 处理 |
|--------|------|
| `13` | 记录警告，安排复核；不要把它当作「全部正常」 |
| `1` | 停止写入，保留现场，从备份恢复或按报告定位损坏页 |

## 仍无法定位

携带以下信息到 GitHub Discussions：

- `yydb version` 输出
- 操作系统与平台
- `yydb info ./app.yydb --doctor` 输出（先确认不含敏感业务数据）
- 最小的可复现调用序列

如果是可复现缺陷，改为提交 Issue。
