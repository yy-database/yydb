# 读写与查询

YYDB 有两条数据通路：VOS 记录路径与键值路径。二者共用同一个文件与事务边界。

## VOS 读管道

```ts
const all = await db.query("User.all().collect()");
```

- `query` 接受读管道，返回普通对象数组。
- 结果顺序由管道决定，`all()` 不承诺物理顺序。
- 读取需要 schema 已发布，否则返回 schema 相关错误。

## VOS 写程序

```ts
await db.execute('User { id: 1, name: "Ada" }.insert()');
```

- `execute` 接受写程序，返回 `void`。
- 写入在返回前已提交；失败时不会留下部分记录。
- 字符串使用双引号，字段名必须与 schema 一致。

## 键值路径

不需要 schema 的场景可直接使用键值接口：

```ts
await db.put("config/theme", "dark");
const value = await db.get("config/theme"); // Uint8Array | null
await db.delete("config/theme");
```

`put` 会覆盖同键值。`get` 在键不存在时返回 `null`，不抛错。

### 前缀扫描

```ts
const rows = await db.scan({ prefix: "config/", limit: 100 });
for (const row of rows) {
    console.log(row.key, row.value);
}
```

`scan` 按字典序返回，支持 `prefix`、`after` 与 `limit`。`limit` 默认 1000。

### 批量写入

```ts
await db.batch([
    { key: "counter", value: "1" },
    { key: "stale", value: null }, // null 表示删除
]);
```

批量写入在单次调用内原子应用。

### 比较并交换

```ts
const swapped = await db.compareExchange("counter", "1", "2");
```

仅当当前值严格等于期望值时才替换。`expected` 与 `replacement` 都可以是 `null`，分别表示「不存在」与「删除」。

## 事务

```ts
await db.transaction(async (tx) => {
    await tx.execute('User { id: 3, name: "Lin" }.insert()');
    await tx.put("audit/last", "3");
});
```

- 正常返回时提交，抛出异常时回滚。
- 不支持嵌套事务；需要嵌套语义时应在上层合并逻辑。
- 事务内的读取看到本事务已做的写入。

显式控制也可用 `begin` / `commit` / `rollback`，但回调式事务更不容易漏掉回滚。

## 预处理语句

```ts
const stmt = await db.prepare("User.all().take($limit).collect()");
const first = await stmt.execute({ limit: 100 });
const second = await stmt.execute({ limit: 50 });
stmt.close();
```

- `prepare` 会解析并降级一次读管道，之后每次执行不再重复解析。
- 参数以命名参数对象传入，名称与管道中的 `$name` 对应。
- 执行前必须已发布 schema。
- 句柄使用完应 `close()`，否则会持续占用降级计划。

## 一致性检查

```ts
const report = await db.doctor();
console.log(report);
```

`doctor` 执行只读探针，报告页、WAL 与对象引用的一致性。它不会修改文件。

## 何时 checkpoint

WAL 会在正常关闭与达到阈值时折叠。跨进程交接、打包备份或发布数据文件之前，显式执行：

```ts
await db.checkpoint();
```

checkpoint 会合并 WAL 到主文件并清理伴生文件。它不是必需步骤，但能让文件边界更明确。
