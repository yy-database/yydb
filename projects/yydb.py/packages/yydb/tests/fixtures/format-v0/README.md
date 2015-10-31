# v0 golden bytes

本目录是 **YYDB 产品仓**对 format-v0 的验收语料与生成脚本；内核 `database.rs` 不再托管这些二进制 / Python。

格式编解码实现仍在 `database-formats` / `database-storage`；此处冻结 page / WAL / SHM / blob 的独立 golden 与损坏样例。

```text
manifest.toml              case 列表与期望 doctor 结果
page0_empty_dual_slot.bin  空库 page 0（双 slot）
wal_header.bin             仅 WAL 文件头
wal_txn_commit_minimal.bin 单事务 PageImage + TxnCommit
shm_empty.bin              空 SHM
blob_chunk_header_min.bin  0 字节 payload 的 blob 头
page0_bad_checksum.bin     slot A checksum 故意损坏
crc32c_reference.txt       空输入 CRC32C 参考值
gen_golden.py              重新生成上述 bytes
```

```bash
# 在本目录重新生成（会覆盖 .bin）
python gen_golden.py
```
