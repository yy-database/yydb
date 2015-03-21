# Node.js 接入

`@yydb/yydb` 在进程内直接打开文件。热路径上没有子进程，也没有回环 TCP。

## 打开与生命周期

```ts
import { Database } from "@yydb/yydb";

const db = await Database.open("data/app.yydb", { mkdir: true });
```

- `mkdir: false` 可关闭自动创建父目录。
- 打开失败时抛出 `YydbError`，不会返回半初始化对象。
- 同一进程内不要对同一文件打开多个 `Database`；写锁会阻止第二个写入者。
- 进程退出前执行 `await db.checkpoint()` 可确保 WAL 已折叠。

## 文件布局

| 文件 | 作用 |
|------|------|
| `app.yydb` | `YDPG` 页文件，页大小 4096 字节 |
| `app.yydb-wal` | `YYWL` 事务日志，提交后由 checkpoint 清理 |
| `app.yydb-shm` | `YYSH` WAL 共享状态 |
| `app-objects/` | 内容寻址对象分块（仅在写入对象时出现） |

移动数据库时，如果 WAL 尚未折叠，必须连同 `.yydb-wal` 一起移动。更稳妥的做法是先 `checkpoint()` 再移动单个主文件。

## 诊断

```ts
console.log(db.info());
const report = await db.doctor();
```

`info()` 返回路径、schema 版本、记录数与 journal 模式。`doctor()` 返回结构化一致性报告，只读，不修改文件。

## 备份

```ts
await db.backup("backup/app.yydb");
```

- `backup(dest)` 先做 checkpoint，再复制出自包含主文件。
- `backupWithWal(dest)` 不 checkpoint，连同 WAL 与 SHM 一起复制；目标端首次打开时会重放 WAL 尾部。
- 备份目标必须是另一个路径，不会覆盖当前打开的文件。

## 精确类型

读取 `timestamp` 与 `decimal` 时得到带标签的对象：

```ts
type Timestamp = { $yydb: "timestamp"; utcMicros: string };
type Decimal = { $yydb: "decimal"; precision: number; scale: number; unscaled: string };
```

需要展示时自行格式化。不要用 `Number()` 直接转换 `utcMicros` 或 `unscaled`，超出安全整数范围会丢精度。

## 进程内 UDF

```ts
const def = db.defineMicro(/* 名称、版本、参数、返回类型与实现 */);
const handle = await db.registerMicro(def);
const result = await db.callScalar("myScalar", 1, [42]);
```

- UDF 只在当前进程会话内有效，不写进数据库文件。
- 参数与返回值限标量类型；复杂结构请先序列化。
- 名称与版本用于在协议路径上定位实现，两者必须一致。

## 并发

- 单进程内多个异步任务可以共享一个 `Database`，写操作会串行化。
- 事务不支持嵌套。需要多个操作原子生效时，统一放进一个 `transaction` 回调。
- 多进程写入同一文件会被写锁拒绝。需要并发写入时，改用 `yydb serve` 的单写者模型。

## 关闭

```ts
await db.close();
```

`close()` 是尽力而为的释放。需要确定性落盘时，先 `checkpoint()`，再关闭。
