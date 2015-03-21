# Schema 与 VOS

YYDB 不接受 SQL。schema、查询与写入统一使用 VOS。

## schema 文档

```vos
table User {
    @@id: i64,
    name: utf8,
    email: utf8,
}
```

- `@@id` 声明主键，必须有且只有一个。
- 字段名区分大小写，写入时必须使用声明过的名字。
- schema 文档通过 `ensureSchema` 发布，随文件保存并带版本号。

## 字段类型

| 类型 | 语义 | JS 返回形态 |
|------|------|-------------|
| `i64` | 64 位有符号整数 | `number`（超出安全整数范围时应按字符串处理） |
| `f64` | 双精度浮点 | `number` |
| `utf8` | UTF-8 文本 | `string` |
| `bytes` | 任意字节 | 字节数组或 `Uint8Array` |
| `bool` | 布尔 | `boolean` |
| `timestamp` | UTC 微秒时间戳 | 带 `$yydb: "timestamp"` 标签的对象 |
| `decimal` | 定点小数 | 带 `$yydb: "decimal"` 标签的对象 |

精确标量不会退化为字符串或浮点近似。`timestamp` 以十进制字符串保存微秒值，`decimal` 保存未缩放整数与 scale。

## 主键与记录键

主键值决定记录的物理键。写入时 `id` 必须显式提供，不存在自增主键。

```ts
await db.execute('User { id: 2, name: "Grace", email: "grace@example.com" }.insert()');
```

同一主键重复写入的行为由写程序选择。不要依赖隐式去重，也不要把主键当作唯一业务字段使用。

## 唯一性

schema 可以声明唯一约束。违反唯一约束时返回 `yydb.unique.violation`，整条写入回滚，不会留下部分记录。

## Schema 演进

```ts
await db.migrateSchema(
    `
table User {
    @@id: i64,
    name: utf8,
    email: utf8,
    createdAt: timestamp,
}
`,
    { renames: [{ type: "User", fromField: "displayName", toField: "name" }] },
);
```

- `ensureSchema` 只发布与当前一致的文档，形状变化会被拒绝。
- `migrateSchema` 允许形状变化，并在需要时迁移既有记录。
- `renames` 告诉引擎旧字段名与新字段名的对应关系；缺少该信息时，结构不相关的记录会被视为不兼容。
- schema 版本随每次成功发布递增，可用 `db.getSchema()` 读取当前版本与原始文档。

## 读取当前 schema

```ts
const current = await db.getSchema();
console.log(current?.version);
console.log(current?.document);
```

未发布过 schema 时返回 `null`。

## 在命令行验证

```bash
npx @yydb/yydb init ./app.yydb --schema-file schema.vos
```

这一步会在写入任何数据前验证 VOS 文档。schema 不合法时返回 `yydb.schema.invalid`，并给出具体位置。
