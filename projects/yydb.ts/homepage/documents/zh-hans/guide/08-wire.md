# 线协议参考

YY 线协议是 `yydb serve` 与客户端之间的二进制帧协议。它不是 JSON REST，也不是 SQL 传输。

## 帧结构

| 偏移 | 长度 | 字段 | 取值 |
|------|------|------|------|
| 0 | 4 | `magic` | `YYDB` 或 `YYDS` |
| 4 | 4 | `version` | 四位 ASCII，当前 `0000` |
| 8 | 1 | `msg_type` | 见消息表，`255` 为 `Error` |
| 9 | 3 | `flags` | `0000` 下必须为 0 |
| 12 | 4 | `request_id` | 无符号，响应必须与请求一致 |
| 16 | 4 | `body_len` | body 字节数，上限 16 MiB |
| 20 | `body_len` | `body` | 按消息定义编码 |

头部固定 20 字节，`version` 之后的所有整数按小端解释。

## 编码规则

1. `magic` 只表示后端自称，前端接受两者且不要求与请求一致。
2. 未知 magic、未知版本、未知 `msg_type` 一律拒绝，不做兼容解析。
3. body 超过 16 MiB 直接拒绝，不截断、不分片。
4. 校验失败或解析失败的输入不得产生部分应用。

## 消息表

| 值 | 名称 | 方向 | 用途 |
|----|------|------|------|
| 1 | `Hello` | 请求 | 建立会话并协商版本 |
| 2 | `HelloOk` | 响应 | 会话建立成功 |
| 3 | `Info` | 请求 | 读取数据库基本信息 |
| 4 | `InfoOk` | 响应 | 返回信息 |
| 5 | `SchemaGet` | 请求 | 读取当前 schema |
| 6 | `SchemaGetOk` | 响应 | 返回 schema 版本与文档 |
| 7 | `SchemaEnsure` | 请求 | 发布 schema |
| 8 | `SchemaEnsureOk` | 响应 | 发布成功 |
| 9 | `KvGet` | 请求 | 按键读取 |
| 10 | `KvGetOk` | 响应 | 返回键值或不存在 |
| 11 | `KvPut` | 请求 | 写入键值 |
| 12 | `KvPutOk` | 响应 | 写入成功 |
| 13 | `MicroRegister` | 请求 | 注册会话级微 UDF |
| 14 | `MicroRegisterOk` | 响应 | 注册成功 |
| 15 | `MicroHostInvoke` | 请求 | 宿主调用客户端实现 |
| 16 | `MicroHostInvokeOk` | 响应 | 宿主调用返回 |
| 17 | `ScalarCall` | 请求 | 调用标量 UDF |
| 18 | `ScalarCallOk` | 响应 | 标量结果 |
| 255 | `Error` | 响应 | 统一错误体 |

当前线协议覆盖会话、信息、schema、键值与微 UDF。通用 VOS 读管道与写程序只在进程内嵌入路径提供。

## 错误体

```
u32 code_len        (小端)
code                (UTF-8)
u32 message_len     (小端)
message             (UTF-8)
```

`code` 是稳定诊断，例如：

| 错误码 | 含义 |
|--------|------|
| `yydb.protocol` | 帧或消息体不合法 |
| `yydb.schema.invalid` | schema 文档不合法 |
| `yydb.unique.violation` | 违反唯一约束 |
| `yydb.udf.not_found` | UDF 未注册 |
| `yydb.corrupt` | 文件校验失败 |
| `yydb.io` | 文件读写失败 |

`yydb.unique.violation` 附带 `table`、`constraint`、`columns` 参数；`yydb.scalar.invalid` 附带 `kind` 与展示用 `message`。

## 传输

- TCP：`yydb serve` 的直接监听端口。
- WebSocket：路径 `/wire`，用于浏览器与 WebUI。

客户端库 `@yydb/yydb-client` 同时提供 `encodeFrame` / `decodeFrame` 与错误体编解码，应用不需要手写解析。

## 调试

本站的线协议演示页（`/wire`）在浏览器内完成 Hello 帧编码与解码，不连接任何服务。可以用它确认字段顺序与字节序，再把结果与自己的实现对照。
