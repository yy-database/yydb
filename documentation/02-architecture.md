# 分层与能力归属

## 1. 依赖方向

依赖只能自上而下，禁止回指或环形：

```text
yydb-types       共享合同：错误、值、对象引用、报告类型与 UDF 合同（`types::udf`）。无 IO。
yydb-formats     物理格式：页、WAL、shm、blob、pager、CRC 的解析与校验。
yydb-storage     持久化访问：文件生命周期与锁、快照提交、checkpoint/WAL 协调、CAS blob IO、存储指标。
yydb-execution   本地执行模型：类型、程序、字段句柄、记录布局、UDF 验证。
yydb-udf         UDF registry、调用调度、host adapter。
yydb-query       VOS 语言整合、DML / DDL 定义、plan / optimizer / runtime。
        ↑
yydb-engine       数据库运行时：Connection、事务、catalog 发布、query 调用、CAS 协调、
                  lease/TTL、UDF session、backup/doctor/compact 协调与 tuning 入口。
        ↑                          ↑
yydb-protocol                  yydb-server
帧与消息体编解码                dispatch、绑定策略、serve 循环、host wire
        ↑
yydb-client       远程客户端（只依赖 `yydb-protocol` 与 `yydb-types`）
        ↑
yydb             总门面：模块包裹式重导出、精选根类型、prelude、版本与文档。
        ↑
yydb-napi / yydb-pyo3 / yydb-wasm        语言与浏览器宿主绑定。
```

规则：

1. 底层 crate 不依赖上层 crate，也不依赖宿主（WASM、N-API、Python）。
2. 门面依赖实现，实现不反向依赖门面。`yydb-client`、`yydb-server`、bindings 都不得依赖 `yydb`。
3. 绑定层只依赖门面，不直接依赖实现 crate 或内部模块。
4. 下游产品只依赖门面公开路径。内部 crate 不是稳定 ABI。
5. `yydb-protocol` 只依赖 `yydb-types`，不依赖 `yydb-engine`，因此不包含 `dispatch` 与绑定策略。
6. `yydb-client` 只依赖 `yydb-protocol` 与 `yydb-types`，不依赖 `yydb`、`yydb-engine`、`yydb-formats` 或 VOS。
7. `yydb-storage` 不理解 VOS、query、UDF 或 `Connection`，也不接收 `Connection::State`。
8. `yydb-engine` 调用 `yydb-query`；`yydb-query` 不反向依赖 engine。

## 2. crate 职责

| crate            | 拥有                                                                        | 不拥有                                           |
|------------------|-----------------------------------------------------------------------------|--------------------------------------------------|
| `yydb-types`     | 错误与错误码、`Value`、`ObjectRef`、lease/配额/诊断报告、`types::udf` 纯合同 | 文件 IO、连接、协议、格式解析                    |
| `yydb-formats`   | `YDPG` 页、`YYWL` WAL、`YYSH` shm、`YYBB` blob 的编码/解析/校验、pager      | 连接生命周期、用户记录语义、schema               |
| `yydb-storage`    | 文件打开与写锁、快照读写、checkpoint/WAL 协调、CAS blob IO、文件复制与存储指标 | VOS、query、UDF、`Connection` 事务或状态编码     |
| `yydb-execution` | 本地类型、程序、字段句柄、记录布局、UDF 效果与放置验证                      | VOS 语法、文件格式、网络                         |
| `yydb-udf`       | UDF registry、调用调度、宿主适配                                            | 持久化、连接、查询语言                           |
| `yydb-query`     | VOS 语言整合、DML/DDL 定义、plan / optimizer / runtime                      | 存储格式、连接、传输、engine 事务                |
| `yydb-engine`    | 连接与后端、KV/事务、CAS 协调、lease/TTL/配额、journal/backup/compact 协调、catalog 发布、query 调用、UDF 会话桥 | 门面组装、协议编解码、serve 策略、宿主映射 |
| `yydb-protocol`  | YY 帧、消息体 encode/decode、错误体、`split_bind`                           | 数据库操作、`Connection`、dispatch、绑定安全策略 |
| `yydb`           | 模块包裹式重导出、精选根类型、`prelude`、版本与文档                         | 字节级格式、持久化、协议编解码、健康检查实现     |
| `yydb-server`    | serve 循环、帧分发、绑定策略、host 回调通道                                 | 存储实现、页格式、门面                          |
| `yydb-client`    | 远程调用与错误解码                                                          | 本地存储、serve 策略、门面                       |
| 绑定 crate       | 类型转换、生命周期映射、宿主卷适配                                          | 数据库语义、格式、查询规则                       |

## 3. 门面规则

`yydb` 是总门面。判定标准如下，任一条不满足就不算门面：

1. 不拥有字节级实现：页、WAL、blob、CAS、lease/TTL 元数据、帧编解码都不在门面里。
2. 不拥有宿主策略：绑定地址、loopback 判定、进程模型属于 server 或绑定层。
3. 只提供模块包裹式重导出、精选根类型、`prelude`、版本字符串与文档。
4. 新增能力先落到对应实现 crate，再由门面重导出；不允许“先塞进门面再说”。
5. 公开模块使用真实领域名，不保留同义别名。

现状：持久化访问已归 `yydb-storage`，数据库运行时归 `yydb-engine`，协议编解码归 `yydb-protocol`，`dispatch` 与 loopback 判定归 `yydb-server`。门面只保留 `pub mod { pub use ... }` 包裹、精选根类型与 `version()`。

公开模块：`types`、`protocol`、`client`、`server`、`udf`、`catalog`、`connection`、`batch`、`journal`、`objects`、`query`、`execution`、`vos`。`client` / `server` 在 `wasm32` 上不编译。`schema`、`wire` 与原始内部 crate 名出口已删除。

已知残留：`yydb-engine::udf` 仍包含旧 `ScalarUdf` 闭包体系与 `yydb-udf` registry 之间的双注册表桥，尚未收敛为单一合同。

## 4. 能力归属判定

新增能力时依次回答：

| 问题                                      | 归属                               |
|-------------------------------------------|------------------------------------|
| 只是数据结构、错误或报告类型？            | `yydb-types`                       |
| 只关乎字节布局、校验和、页或 WAL 解析？   | `yydb-formats`                     |
| 只关乎文件生命周期、锁、快照、checkpoint 或 blob IO？ | `yydb-storage`         |
| 只关乎本地执行语义、类型布局或 UDF 验证？ | `yydb-execution`                   |
| 只关乎 UDF 合同、注册、调用或宿主适配？   | `yydb-udf`                         |
| 只关乎 VOS 语言整合、DML/DDL 定义？       | `yydb-query`                       |
| 关乎连接、记录、事务、CAS、配额、lease？  | `yydb-engine`                      |
| 关乎帧、消息体、错误体编码？              | `yydb-protocol`                    |
| 关乎监听、绑定策略、连接循环？            | `yydb-server`                      |
| 关乎语言类型映射与宿主卷？                | 对应绑定 crate                     |
| 以上都不是，只是公开组装？                | `yydb`                             |

## 5. 扩展点

只在确实需要时增加扩展点，并且必须落在已定义的层：

1. **新宿主**：新增绑定 crate，只做映射；不得把宿主 API 泄漏到引擎。
2. **新传输**：复用同一帧编码，新增 transport crate；不得改变 `0000` 帧布局。
3. **新存储后端**：先确认是否仍满足 `.yydb` / `.yydx` 不变量；后端差异只能出现在 `yydb-storage` 与状态发布层。
4. **新查询能力**：进入 `yydb-query` 与执行模型，不进入门面或协议。
5. **新对象类型**：进入 CAS 与 `ObjectKind`，不得新增旁路 sidecar。

## 6. 禁止事项

1. 在绑定层重写数据库格式或事务语义。
2. `yydb-server` 绕过 `Connection` 直接读写文件。
3. 门面直接持有 pager、文件句柄或帧编解码。
4. 下游产品依赖内部 crate 或复制内部类型来规避门面。
5. 为同一能力保留两套并行公开 API。
6. 在门面恢复 `schema` / `wire` 别名或原始内部 crate 名出口。
7. 让 `yydb-storage` 接收 `Connection::State`，或让它理解 VOS、query 与 UDF。
8. 让 `yydb-server` 为每个 peer 重新打开数据库文件；每个 peer 只获得一个 `Connection::session()`。
