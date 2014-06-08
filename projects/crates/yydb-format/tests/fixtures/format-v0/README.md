# v0 golden bytes

设计合同：[`../09-v0页与WAL字节合同.md`](../09-v0页与WAL字节合同.md) · 架构：[`../07-存储模式与统一存储合同.md`](../07-存储模式与统一存储合同.md)

本目录冻结 v0 page file、WAL、SHM、blob 的独立 golden 与损坏样例。实现仓 `yydb.rs` 须逐文件断言这些样例。

```text
manifest.toml              case 列表与期望 doctor 结果
page0_empty_dual_slot.bin  空库 page 0（双 slot，generation=0）
wal_header.bin             仅 WAL 文件头
wal_txn_commit_minimal.bin 单事务 PageImage + TxnCommit（合成）
shm_empty.bin              空 SHM
blob_chunk_header_min.bin  0 字节 payload 的 blob 头（82 B）
page0_bad_checksum.bin     slot A checksum 故意损坏
crc32c_reference.txt       空输入 CRC32C 参考值
```

生成工具可在实现仓落地为 `yydb-format-fixture-gen`；设计仓只保留权威 bytes，不以生成器输出覆盖 golden 而不更新 manifest。
