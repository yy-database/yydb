"""format-v0 golden 语料验收（产品侧 Python 测试）。

内核只保留最小冒烟；权威字节与生成脚本落在本目录的 ``fixtures/format-v0``。
"""

from __future__ import annotations

import struct
import unittest
from pathlib import Path

FIXTURES = Path(__file__).resolve().parent / "fixtures" / "format-v0"


class FormatV0FixturesTest(unittest.TestCase):
    def test_required_files_exist(self) -> None:
        required = [
            "page0_empty_dual_slot.bin",
            "page0_bad_checksum.bin",
            "wal_header.bin",
            "wal_txn_commit_minimal.bin",
            "shm_empty.bin",
            "blob_chunk_header_min.bin",
            "crc32c_reference.txt",
            "gen_golden.py",
            "manifest.toml",
        ]
        for name in required:
            path = FIXTURES / name
            self.assertTrue(path.is_file(), f"missing fixture {path}")

    def test_page0_magic_and_size(self) -> None:
        raw = (FIXTURES / "page0_empty_dual_slot.bin").read_bytes()
        self.assertEqual(len(raw), 4096)
        self.assertEqual(raw[0:5], b"YDPG\x00")
        self.assertEqual(raw[5], 0x41)
        self.assertEqual(raw[2048:2053], b"YDPG\x00")
        self.assertEqual(raw[2053], 0x42)

    def test_wal_header_magic(self) -> None:
        raw = (FIXTURES / "wal_header.bin").read_bytes()
        self.assertEqual(raw[0:5], b"YYWL\x00")

    def test_shm_magic(self) -> None:
        raw = (FIXTURES / "shm_empty.bin").read_bytes()
        self.assertEqual(len(raw), 4096)
        self.assertEqual(raw[0:5], b"YYSH\x00")

    def test_blob_header_magic_and_min_size(self) -> None:
        raw = (FIXTURES / "blob_chunk_header_min.bin").read_bytes()
        self.assertEqual(raw[0:5], b"YYBB\x00")
        self.assertEqual(len(raw), 82)

    def test_crc32c_reference_mentions_empty(self) -> None:
        text = (FIXTURES / "crc32c_reference.txt").read_text(encoding="utf-8")
        self.assertIn("empty CRC32C", text)


if __name__ == "__main__":
    unittest.main()
