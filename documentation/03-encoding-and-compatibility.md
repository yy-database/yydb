# 编码、格式与兼容

YYDB 有三层独立编码。任何一层都不能借用另一层的版本号。

## 1. 磁盘格式

| 对象          | magic    | 说明                                                          |
|---------------|----------|---------------------------------------------------------------|
| 主文件        | `YDPG\0` | 页文件。页大小 4096 字节，page 0 使用双槽描述当前与上一代头部 |
| WAL companion | `YYWL\0` | 页镜像事务日志                                                |
| SHM companion | `YYSH\0` | WAL 共享状态                                                  |
| 对象段        | `YYBB\0` | CAS 的 `.blob` 段                                             |

规则：

1. magic 与格式版本是显式的。读取器遇到未知 magic 或未知版本必须失败，不得尝试兼容解析。
2. CRC32C 是硬校验。校验失败即损坏，不提供“跳过坏页继续读”的选项。
3. 校验失败的输入不得产生部分应用。要么整段接受，要么整段拒绝。
4. WAL 恢复只重放完整事务；尾部不完整事务可丢弃。已提交事务不得被改写、重排或跳过。
5. 新格式必须提升格式版本并使用新的 magic 或版本字段。禁止在旧版本号下改变字节布局。
6. 旧文件不会被静默升级。任何迁移必须是显式动作，并在迁移前后都能被诊断工具识别。
7. 页内空闲空间、槽位与指针的相对位置属于格式合同；压缩、分页或缓存策略不得改变它们在文件中的表达。

## 2. 线协议

参考 `yydb serve` 使用 `YYDB` 作为后端自述，客户端接受 `YYDB` 与 `YYDS` 两种 magic，语义相同。

| 偏移 | 长度 | 字段                             |
|------|------|----------------------------------|
| 0    | 4    | product magic                    |
| 4    | 4    | 四位 ASCII 版本数字，当前 `0000` |
| 8    | 2    | `msg_type`                       |
| 10   | 2    | `flags`，`0000` 中为 0           |
| 12   | 4    | `request_id`                     |
| 16   | 4    | `body_len`                       |
| 20   | N    | body                             |

规则：

1. 前缀之后按小端解释。头部固定 20 字节。
2. body 上限 16 MiB。超限即拒绝，不截断、不分片。
3. 未知 magic、未知版本、未知 `msg_type` 一律拒绝。
4. 新增消息类型不得改变已发布类型的 body 编码。破坏性变化进入新的版本数字。
5. `flags` 在 `0000` 中保留为 0；读取方不得依赖未定义位。
6. 服务端可以在同一连接上推送 host 回调请求，客户端必须能处理请求与响应交错。

### 错误体

`Error` 帧的 body 是结构化的：`u32 code_len` + code + `u32 message_len` + message，两者都是 UTF-8。

1. `code` 是稳定诊断，例如 `yydb.schema.invalid`、`yydb.corrupt`、`yydb.protocol`、`yydb.udf.not_found`。
2. `message` 只用于展示，可随版本变化。
3. 客户端只能基于 `code` 与结构化字段分支，禁止解析 `message`。
4. 新增错误码是兼容变更；删除或改变既有错误码含义是破坏变更。

当前错误码集合：

```text
yydb.io                      yydb.corrupt
yydb.schema.conflict          yydb.schema.invalid
yydb.object.not_found         yydb.object.corrupt
yydb.udf.not_found            yydb.udf.arity
yydb.udf.failed               yydb.udf.version_mismatch
yydb.unsupported              yydb.protocol
yydb.cas.conflict             yydb.quota.exceeded
yydb.lease.unavailable        yydb.lease.expired
yydb.fencing.mismatch         yydb.session.closed
```

## 3. 内部 JSON

schema identity、catalog 快照等以 JSON 存放在主文件或 WAL 快照中。

1. 这些 JSON 是 **内部编码**，不是跨产品 ABI，也不是编辑器或第三方工具可以直接依赖的稳定接口。
2. 需要对外交换的合同必须单独定义，并带自己的版本字段。
3. 内部编码变化仍然要遵守“整段接受或整段拒绝”，不能靠默认值吞掉缺失字段。

## 4. 编码的单一来源

1. 字段名与枚举值以实现中的类型定义为唯一来源。
2. 不得为迁就旧 fixture、旧断言或旧客户端而增加逐字段兼容层或兼容别名。
3. 过时的 fixture 与测试断言应当修正，而不是让运行时承担兼容分支。
4. 需要保持的兼容只有一种：已发布格式版本与已发布线协议。其余都属于内部实现，可以改。

## 5. 变更判定

| 变更               | 需要做什么                                       |
|--------------------|--------------------------------------------------|
| 新增错误码         | 兼容。补文档与客户端映射                         |
| 新增 `msg_type`    | 兼容。不得改动既有 body 编码                     |
| 修改既有 body 编码 | 破坏。新版本数字                                 |
| 修改页内布局       | 破坏。新格式版本，显式迁移                       |
| 修改内部 JSON 字段 | 内部变更。同步改读取器与 fixture，不保留兼容别名 |
| 修改 magic         | 破坏。必须提供迁移与诊断路径                     |
