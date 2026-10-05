# 二进制协议摘要

传输是 **YY 二进制帧**，不是 JSON REST，也不是 SQL。DDL/查询语言侧是 **VOS**。

- 前缀：产品魔数 **`YYDB` \| `YYDS`**（后端自称，效果相同，前端不强求一致）+ 四位版本 **`0000`**（演进 **`0001`**、…）
- 头：`msg_type` / `flags` / `request_id` / `body_len`（小端）
- 运输：TCP，或 WebSocket `/wire`（帧体相同）

前端库 **`@yydb/yydb-client`**（以及本站 / WebUI）按协议说话， **不探测**后端是 YYDB 还是 YYDS。包名带 `yydb` 只因为当前仓库提供参考实现。

Rust：

- [`yydb`](https://crates.io/crates/yydb)：YYDB 嵌入式门面
- [`yydb-client`](https://crates.io/crates/yydb-client)：YYDB 远程 TCP 客户端

完整说明（Agent skill）：[
`yydb-serve-protocol`](https://github.com/yy-database/yydb.rs/tree/dev/projects/packages/yydb-skills/skills/yydb-serve-protocol/SKILL.md)（[
`@yydb/yydb-skills`](https://www.npmjs.com/package/@yydb/yydb-skills)）。

v1：Hello、Info、SchemaGet / SchemaEnsure、KvGet / KvPut。
