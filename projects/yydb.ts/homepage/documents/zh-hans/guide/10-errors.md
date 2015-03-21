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

## 常见错误码

| 错误码 | 含义 | 处理 |
|--------|------|------|
| `yydb.io` | 文件读写失败 | 检查路径、权限与磁盘空间 |
| `yydb.corrupt` | CRC32C 校验失败 | 停止写入，用 `yydb verify` 与 `doctor()` 定位，从备份恢复 |
| `yydb.schema.invalid` | schema 文档不合法 | 按 VOS 语法修正，先用 `yydb init --schema-file` 验证 |
| `yydb.schema.conflict` | schema 形状与现有记录不兼容 | 使用 `migrateSchema`，必要时提供 `renames` |
| `yydb.unique.violation` | 违反唯一约束 | 读取 `params.columns` 定位字段，改写或先删除冲突记录 |
| `yydb.query.bind` | 预处理语句参数缺失、多余或类型不符 | 对照 `stmt.parameters` 检查绑定对象 |
| `yydb.statement.closed` | 已关闭的语句被再次执行 | 重新 `prepare`，不要在关闭后复用句柄 |
| `yydb.object.not_found` | 内容寻址对象缺失 | 检查备份是否只复制了主文件而漏掉对象分块 |
| `yydb.object.corrupt` | 对象分块校验失败 | 从备份恢复；不要尝试跳过坏块 |
| `yydb.protocol` | 线协议帧不合法 | 核对 magic、版本、`msg_type` 与 body 长度 |
| `yydb.udf.not_found` | 调用的 UDF 未注册 | 先注册再调用，或检查会话是否已重建 |
| `yydb.scalar.invalid` | 精确标量格式不合法 | 读取 `params.kind`，按 date/time/timestamp/decimal 规则重写 |

## 打开失败

| 症状 | 原因 | 处理 |
|------|------|------|
| `ENOENT` 或权限错误 | 父目录不可写 | 改用可写路径，或确认 `mkdir` 未被关闭 |
| 格式不匹配 | 文件不是 `YDPG` | SQLite 文件不能改后缀当作 `.yydb` |
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
