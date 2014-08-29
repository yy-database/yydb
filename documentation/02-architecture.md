# 分层与能力归属

## 1. 依赖方向

依赖只能自上而下，禁止回指或环形：

```text
yydb-types       模型：错误、值、对象引用、报告类型。无 IO。
yydb-format      页、WAL、shm、blob、pager、CRC 的解析与校验。
yydb-execution   本地执行模型：类型、程序、字段句柄、记录布局、UDF 验证。
yydb-udf         UDF 合同、registry、host adapter、scalar 合同。
yydb-query       VOS 语言整合、DML / DDL 定义与本地查询降级。
        ↑
yydb             门面：公开作者面、重导出、prelude、版本与文档。
        ↑
yydb-server / yydb-client     serve 与远程客户端。
yydb-napi / yydb-pyo3 / yydb-wasm   语言与浏览器宿主绑定。
```

规则：

1. 底层 crate 不依赖上层 crate，也不依赖宿主（WASM、N-API、Python）。
2. 门面依赖实现，实现不反向依赖门面。
3. 绑定层依赖门面（以及 serve 需要的 server），不直接依赖 `yydb-format` 或内部模块。
4. 下游产品只依赖门面公开路径。内部 crate 不是稳定 ABI。

## 2. crate 职责

| crate | 拥有 | 不拥有 |
|-------|------|--------|
| `yydb-types` | 错误与错误码、`Value`、`ObjectRef`、lease/配额/诊断报告等共享模型 | 文件 IO、连接、协议、格式解析 |
| `yydb-format` | `YDPG` 页、`YYWL` WAL、`YYSH` shm、`YYBB` blob 的编码/解析/校验、内存 pager | 连接生命周期、用户记录语义、schema |
| `yydb-execution` | 本地类型、程序、字段句柄、记录布局、UDF 效果与放置验证 | VOS 语法、文件格式、网络 |
| `yydb-udf` | UDF 合同、注册表、调用、宿主适配 | 持久化、连接、查询语言 |
| `yydb-query` | VOS 语言整合、DML/DDL 定义、查询降级 | 存储格式、连接、传输 |
| `yydb` | 公开作者面与跨层组装 | 字节级格式、记录元数据、协议编解码、健康检查实现 |
| `yydb-server` | serve 循环、帧分发、绑定策略、host 回调通道 | 存储实现、页格式 |
| `yydb-client` | 远程调用与错误解码 | 本地存储、serve 策略 |
| 绑定 crate | 类型转换、生命周期映射、宿主卷适配 | 数据库语义、格式、查询规则 |

## 3. 门面规则

`yydb` 是门面。判定标准如下，任一条不满足就不算门面：

1. 不拥有字节级实现：页、WAL、blob、CAS、lease/TTL 元数据、帧编解码都不在门面里。
2. 不拥有宿主策略：绑定地址、loopback 判定、进程模型属于 server 或绑定层。
3. 只提供重导出、`prelude`、版本字符串、文档与必要的跨层组装。
4. 新增能力先落到对应实现 crate，再由门面重导出；不允许“先塞进门面再说”。
5. 公开路径稳定优先于内部结构调整。移动实现时保持门面路径不变。

当前偏离：`yydb` 仍承载连接、存储、对象、lease/TTL、协议与健康检查实现。这是已记录的结构债，不是新的分层许可。收口时以本节的判定标准为准。

## 4. 能力归属判定

新增能力时依次回答：

| 问题 | 归属 |
|------|------|
| 只是数据结构、错误或报告类型？ | `yydb-types` |
| 只关乎字节布局、校验和、页或 WAL 恢复？ | `yydb-format` |
| 只关乎本地执行语义、类型布局或 UDF 验证？ | `yydb-execution` |
| 只关乎 UDF 合同、注册、调用或宿主适配？ | `yydb-udf` |
| 只关乎 VOS 语言整合、DML/DDL 定义？ | `yydb-query` |
| 关乎连接、记录、事务、CAS、配额、lease？ | 存储实现层（门面之外的引擎 crate） |
| 关乎帧、消息体、错误体编码？ | 协议层 |
| 关乎监听、绑定策略、连接循环？ | `yydb-server` |
| 关乎语言类型映射与宿主卷？ | 对应绑定 crate |
| 以上都不是，只是公开组装？ | `yydb` |

## 5. 扩展点

只在确实需要时增加扩展点，并且必须落在已定义的层：

1. **新宿主**：新增绑定 crate，只做映射；不得把宿主 API 泄漏到引擎。
2. **新传输**：复用同一帧编码，新增 transport crate；不得改变 `0000` 帧布局。
3. **新存储后端**：先确认是否仍满足 `.yydb` / `.yydx` 不变量；后端差异只能出现在 pager 与状态发布层。
4. **新查询能力**：进入 `yydb-query` 与执行模型，不进入门面或协议。
5. **新对象类型**：进入 CAS 与 `ObjectKind`，不得新增旁路 sidecar。

## 6. 禁止事项

1. 在绑定层重写数据库格式或事务语义。
2. `yydb-server` 绕过 `Connection` 直接读写文件。
3. 门面直接持有 pager、文件句柄或帧编解码。
4. 下游产品依赖内部 crate 或复制内部类型来规避门面。
5. 为同一能力保留两套并行公开 API。
