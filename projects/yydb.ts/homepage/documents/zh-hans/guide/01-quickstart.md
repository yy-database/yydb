# 快速开始

本页完成一次完整往返：安装、打开文件、发布 schema、写入记录、读取记录。

## 1. 安装

```bash
npm install @yydb/yydb
```

需要 Node.js 18 或更高版本。原生绑定由可选依赖自动选择，不需要单独下载。

## 2. 打开文件

```ts
import { Database } from "@yydb/yydb";

const db = await Database.open("app.yydb");
```

`Database.open` 会创建缺失的父目录，并在文件不存在时创建新数据库。路径会解析为绝对路径，可从 `db.path` 读取。

## 3. 发布 schema

```ts
await db.ensureSchema(`
table User {
    @@id: i64,
    name: utf8,
    email: utf8,
}
`);
```

schema 文档随文件持久化，并带有版本号。重复发布相同文档不会产生新版本。

## 4. 写入

```ts
await db.execute('User { id: 1, name: "Ada", email: "ada@example.com" }.insert()');
```

写入使用 VOS 写程序，不是 SQL。字符串使用双引号，字段名与 schema 声明一致。

## 5. 读取

```ts
const users = await db.query<User[]>("User.all().collect()");
console.log(users);
```

读管道返回普通 JavaScript 对象数组。数值、布尔与字符串按原类型返回；精确标量（`timestamp`、`decimal`）以带标签的对象形式返回，避免精度丢失。

## 6. 关闭

```ts
await db.close();
```

关闭是尽力而为的操作，进程退出后原生句柄也会被回收。需要跨进程保证数据已落盘时，先执行 `await db.checkpoint()`。

## 完整示例

```ts
import { Database } from "@yydb/yydb";

const db = await Database.open("app.yydb");

await db.ensureSchema(`
table User {
    @@id: i64,
    name: utf8,
    email: utf8,
}
`);

await db.execute(
    'User { id: 1, name: "Ada", email: "ada@example.com" }.insert()',
);

const rows = await db.query("User.all().collect()");
console.log(rows.length); // 1

await db.checkpoint();
await db.close();
```

## 下一步

- [Schema 与 VOS)：类型、主键与演进
- [读写与查询)：事务、预处理语句与扫描
- [Node.js 接入)：备份、恢复与诊断
