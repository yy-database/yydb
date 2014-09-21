# 分层与能力归属

## 1. 依赖方向

依赖只能自上而下，禁止回指或环形。下图中 `A ← B` 表示 **B 依赖 A**：

```text
yydb-types  ←  yydb-formats  ←  yydb-storage  ←  yydb-engine
yydb-types  ←  yydb-query     ←  yydb-engine
yydb-types  ←  yydb-execution ←  yydb-engine
yydb-types  ←  yydb-udf       ←  yydb-engine
yydb-engine + yydb-protocol  ←  yydb-server
yydb-types + yydb-protocol   ←  yydb-client
yydb-engine + yydb-protocol + yydb-client + yydb-server  ←  yydb
yydb  ←  yydb-napi / yydb-pyo3 / yydb-wasm
```

| crate            | 角色                                                                 |
|------------------|----------------------------------------------------------------------|
| `yydb-types`     | 共享合同：错误、值、对象引用、报告类型与 `udf` 纯合同。无 IO。       |
| `yydb-formats`   | 物理格式：页、B-tree、WAL、shm、blob、pager、CRC 的解析与校验。       |
| `yydb-storage`   | 持久化访问：文件生命周期与锁、快照提交、checkpoint/WAL、CAS blob IO。 |
| `yydb-execution` | 本地执行模型：类型、程序、字段句柄、记录布局、UDF 验证。               |
| `yydb-udf`       | UDF registry、调用调度、host adapter。                                 |
| `yydb-query`     | VOS 语言整合、DML / DDL 定义、plan / optimizer / runtime。             |
| `yydb-engine`    | 数据库运行时：Connection、事务、catalog 发布、query 调用、CAS 协调、lease/TTL、UDF session、backup/doctor/compact 协调与 tuning 入口。 |
| `yydb-protocol`  | YY 帧、消息体编解码、错误体、`split_bind`。                            |
| `yydb-client`    | Rust 远程客户端（只依赖 `yydb-protocol` 与 `yydb-types`）。           |
| `yydb-server`    | serve 循环、帧分发、绑定策略、host 回调通道（依赖 engine 与 protocol）。 |
| `yydb`           | 总门面：模块包裹式重导出、精选根类型、`prelude`、版本与文档。         |
| 绑定 crate       | `yydb-napi`、`yydb-pyo3`、`yydb-wasm`，只依赖门面。                   |

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

## 3. 模块结构

### 3.1 `yydb-types`

| 模块 | 内容 |
|------|------|
| 根   | `Error` / `Result` / `Value` / `ObjectRef` / `ObjectKind` / `HashAlgo`、lease、配额、诊断与压缩报告、`SchemaVersion` |
| `udf` | 纯 UDF 合同：`UdfIdentity`、`Signature`、`UdfPolicy`、`CapabilitySet`、`Effect`、`Placement`、`UdfValue`、`UdfError`、host metadata |

`types::udf` 不依赖 engine、execution 或协议。原 `yydb-udf-types` crate 已删除，不保留 shim。

### 3.2 `yydb-formats`

| 模块 | 内容 |
|------|------|
| `page` / `header` / `leaf` / `internal` / `key` | `YDPG` 页头、页槽、B-tree 叶/内部页、`TreeKey` |
| `pager` / `file` / `memory` | `PageStore`、`FilePager`、`MemoryPager` |
| `wal` / `wal_append` / `blob_refs` | `YYWL` 帧解析与追加、事务提交帧、blob 引用清单 |
| `shm` | `YYSH` 索引块 |
| `blob` | `YYBB` chunk 头与载荷 |
| `crc32c` / `doctor` | 校验和与文件级诊断探针 |

### 3.3 `yydb-storage`

| 模块 | 可见性 | 内容 |
|------|--------|------|
| `blob` | 公开 | `BlobStore`：`<root>/<hash-prefix>/<chunk_hash>.blob` 的 put/read/list/remove |
| `files` | 公开 | `copy_main` / `copy_sidecars` / `copy_tree` |
| `journal` | 公开 | `JournalMode`、`OpenFlags`、`wal_path`、`shm_path`、`sidecar_status` |
| `snapshot` | 公开 | `Snapshot`（metadata + records）、`PagerKv`、`read_snapshot` / `write_snapshot` |
| `database` | 私有 | `Database`：打开、写锁、快照提交、checkpoint、WAL 统计 |
| `lock` | 私有 | `WriterLock`（跨进程独占） |

根导出：`Database`、`BlobStore`、`WriterLock`、`Snapshot`、`PagerKv`、`file_is_ydpg`、`snapshot_from_main_bytes`、`snapshot_to_main_bytes`。

边界：`Snapshot.metadata` 是 `BTreeMap<Vec<u8>, Vec<u8>>`，`records` 是 `BTreeMap<String, Vec<u8>>`。storage 不解析 engine 的 `State`；状态编解码留在 `yydb-engine::storage::snapshot`。

### 3.4 `yydb-engine`

| 模块 | 内容 |
|------|------|
| `connection` | `Connection`、`Backend`（`File` / `Memory`）、事务、KV、lease、doctor、backup、compact、schema / 查询入口 |
| `catalog` | VOS 合同发布、执行目录、identity / rename / version 处理 |
| `objects` | `ObjectStore`：CAS + 内存热/冷层 + 引用扫描 |
| `batch` | `Batch` 原子提交封装 |
| `journal` | 仅重导出 `yydb-storage::journal`，保持 engine 路径稳定 |
| `udf` | `ScalarUdf` 闭包体系、`UdfRegistry` 桥、session UDF |
| `kv`（私有） | `State`、lease、retention、配额 |
| `storage`（私有） | `snapshot`（`State` ↔ `Snapshot` 编解码、blob delta）与 `doctor` 探针 |

`connection` 的子模块都是私有实现文件：`backup`、`compact`、`doctor`、`journal`、`kv`、`lease`、`objects_api`、`query`、`schema_api`、`state`、`transaction`、`udf`。对外只暴露 `Connection` 及其方法。

### 3.5 `yydb-udf` / `yydb-query` / `yydb-execution`

| crate | 公开面 |
|-------|--------|
| `yydb-udf` | `UdfRegistry`、`ImplementationRegistry`、`SessionUdfRegistry`、`UdfImplementation`、`UdfContext`、host adapter；纯合同全部重导出 `yydb_types::udf` |
| `yydb-query` | VOS 解析/校验后的查询结果与 DML/DDL 执行入口；内部 `catalog` / `dml` / `exec` / `lower` / `ops` / `resolve` / `store` 均私有 |
| `yydb-execution` | `Program`、`Node`、`Type`、`RecordValue`、`ValidatedProgram` 等执行模型；无公开子模块 |

### 3.6 `yydb-protocol` / `yydb-client` / `yydb-server`

| crate | 公开面 |
|-------|--------|
| `yydb-protocol` | 帧头常量、`MessageType`、`Frame`、`read_frame` / `write_frame`、各消息体 encode/decode、`split_bind` |
| `yydb-client` | `Client`、错误解码、`HostMicroDefinition` 重导出 |
| `yydb-server` | `dispatch`、`is_loopback_host`、`host_wire`、`run_serve`、`assert_bind_allowed`；`serve` 循环私有 |

### 3.7 门面模块映射

| `yydb` 模块 | 来源 |
|-------------|------|
| `types` | `yydb-types` |
| `protocol` | `yydb-protocol` |
| `client` / `server` | `yydb-client` / `yydb-server`（`wasm32` 不编译） |
| `udf` | `yydb-engine::udf` |
| `catalog` / `connection` / `batch` / `journal` / `objects` | `yydb-engine` 对应模块 |
| `query` | `yydb-query` |
| `execution` | `yydb-execution` |
| `vos` | 上游 `vos` |

根类型：`Connection`、`Batch`、`ObjectStore`、`JournalMode`、`OpenFlags`、`Error`、`Result`、`Value`、`SchemaVersion`、`ObjectRef` 与 UDF 根类型。`prelude` 只放嵌入场景最常用的类型。

## 4. 运行时结构

### 4.1 连接与后端

```text
Connection
├── backend: Arc<Backend>
│   ├── File { path, database: Mutex<Database> }   → yydb-storage
│   └── Memory { state: Mutex<State> }
├── operation_lock: Arc<Mutex<()>>                 // 串行化写路径与 checkpoint
├── objects: Arc<ObjectStore>                      // 进程内热层 + 可选磁盘 CAS
├── txn: Mutex<Option<State>>                      // 会话私有
└── udfs: Mutex<UdfSubsystem>                      // 会话私有
```

- `Database` 持有 `FilePager`、`WriterLock` 与 journal 模式；打开文件、折叠 WAL、提交快照、checkpoint 都在该层完成。
- engine 只通过 `Snapshot` + `BlobManifestDelta` 与 storage 交互；`State` 编解码在 `yydb-engine::storage::snapshot`。
- `ObjectStore` 的磁盘分支使用 `yydb-storage::BlobStore`；内存分支不发文件 IO。

### 4.2 会话隔离

`Connection::session()` 复制 `Arc<Backend>`、`Arc<Mutex<()>>`、`Arc<ObjectStore>`，并新建空的 `txn` 与 `udfs`：

- 数据、pager、写锁、CAS 共享，所以同一进程内只有一个写者、一个文件句柄。
- 事务槽与 UDF registry 按会话隔离，回滚与 UDF 注册不跨 peer。
- host adapter 按会话注册，一个 peer 的 host micro 不会泄漏给另一个 peer。
- `yydb-server` 只调用一次 `Connection::open`，之后每个 peer 用 `conn.session()`；禁止每个 peer 重新打开数据库文件。

## 5. 门面规则

`yydb` 是总门面。判定标准如下，任一条不满足就不算门面：

1. 不拥有字节级实现：页、WAL、blob、CAS、lease/TTL 元数据、帧编解码都不在门面里。
2. 不拥有宿主策略：绑定地址、loopback 判定、进程模型属于 server 或绑定层。
3. 只提供模块包裹式重导出、精选根类型、`prelude`、版本字符串与文档。
4. 新增能力先落到对应实现 crate，再由门面重导出；不允许“先塞进门面再说”。
5. 公开模块使用真实领域名，不保留同义别名。

现状：持久化访问已归 `yydb-storage`，数据库运行时归 `yydb-engine`，协议编解码归 `yydb-protocol`，`dispatch` 与 loopback 判定归 `yydb-server`。门面只保留 `pub mod { pub use ... }` 包裹、精选根类型与 `version()`。

公开模块：`types`、`protocol`、`client`、`server`、`udf`、`catalog`、`connection`、`batch`、`journal`、`objects`、`query`、`execution`、`vos`。`client` / `server` 在 `wasm32` 上不编译。`schema`、`wire` 与原始内部 crate 名出口已删除。

已知残留：`yydb-engine::udf` 仍包含旧 `ScalarUdf` 闭包体系与 `yydb-udf` registry 之间的双注册表桥，尚未收敛为单一合同。

## 6. 能力归属判定

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

## 7. 扩展点

只在确实需要时增加扩展点，并且必须落在已定义的层：

1. **新宿主**：新增绑定 crate，只做映射；不得把宿主 API 泄漏到引擎。
2. **新传输**：复用同一帧编码，新增 transport crate；不得改变 `0000` 帧布局。
3. **新存储后端**：先确认是否仍满足 `.yydb` / `.yydx` 不变量；后端差异只能出现在 `yydb-storage` 与状态发布层。
4. **新查询能力**：进入 `yydb-query` 与执行模型，不进入门面或协议。
5. **新对象类型**：进入 CAS 与 `ObjectKind`，不得新增旁路 sidecar。

## 8. 禁止事项

1. 在绑定层重写数据库格式或事务语义。
2. `yydb-server` 绕过 `Connection` 直接读写文件。
3. 门面直接持有 pager、文件句柄或帧编解码。
4. 下游产品依赖内部 crate 或复制内部类型来规避门面。
5. 为同一能力保留两套并行公开 API。
6. 在门面恢复 `schema` / `wire` 别名或原始内部 crate 名出口。
7. 让 `yydb-storage` 接收 `Connection::State`，或让它理解 VOS、query 与 UDF。
8. 让 `yydb-server` 为每个 peer 重新打开数据库文件；每个 peer 只获得一个 `Connection::session()`。
