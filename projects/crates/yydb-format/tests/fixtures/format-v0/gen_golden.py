#!/usr/bin/env python3
"""Generate v0 golden bytes. Run from repo root when updating fixtures."""

from __future__ import annotations

import struct
import uuid
from pathlib import Path

OUT = Path(__file__).resolve().parent
DB_ID = uuid.UUID("00000000-0000-4000-8000-000000000001").bytes
MAGIC = b"YDPG\x00"


def crc32c(data: bytes) -> int:
    # Reflected Castagnoli polynomial used by the byte-oriented CRC32C form.
    poly = 0x82F63B78
    crc = 0xFFFFFFFF
    for b in data:
        crc ^= b
        for _ in range(8):
            crc = (crc >> 1) ^ poly if crc & 1 else crc >> 1
    return (crc ^ 0xFFFFFFFF) & 0xFFFFFFFF


def build_header_slot(slot_kind: int, generation: int, checksum_override: int | None = None) -> bytes:
    buf = bytearray(2048)
    buf[0:5] = MAGIC
    buf[5] = slot_kind
    struct.pack_into("<Q", buf, 6, generation)
    struct.pack_into("<Q", buf, 14, 0)
    buf[22:38] = DB_ID
    buf[38] = 0x01
    buf[39] = 12
    struct.pack_into("<I", buf, 40, 0)
    struct.pack_into("<Q", buf, 72, 0)
    struct.pack_into("<Q", buf, 80, 0)
    if checksum_override is not None:
        struct.pack_into("<I", buf, 2044, checksum_override)
    else:
        struct.pack_into("<I", buf, 2044, crc32c(bytes(buf[0:2044])))
    return bytes(buf)


def wal_frame(ftype: int, lsn: int, body: bytes) -> bytes:
    frame = bytearray()
    frame.append(ftype)
    frame += struct.pack("<Q", lsn)
    frame_len = 1 + 8 + len(body)
    frame += struct.pack("<I", frame_len)
    frame += body
    frame += struct.pack("<I", crc32c(bytes(frame)))
    return bytes(frame)


def main() -> None:
    slot_a = build_header_slot(0x41, 1)
    slot_b = build_header_slot(0x42, 1)
    (OUT / "page0_empty_dual_slot.bin").write_bytes(slot_a + slot_b)

    bad = build_header_slot(0x41, 1, checksum_override=0xDEADBEEF)
    (OUT / "page0_bad_checksum.bin").write_bytes(bad + slot_b)

    wal = bytearray()
    wal += b"YYWL\x00"
    wal += DB_ID
    wal += struct.pack("<Q", 0)
    wal += struct.pack("<I", 0)
    wal += struct.pack("<I", crc32c(bytes(wal)))
    (OUT / "wal_header.bin").write_bytes(bytes(wal))

    page1 = bytearray(4096)
    page1[0] = 0x10
    struct.pack_into("<I", page1, 1, 1)
    struct.pack_into("<Q", page1, 5, 1)
    struct.pack_into("<H", page1, 13, 4)
    page1[15] = 0x02
    struct.pack_into("<I", page1, 4092, crc32c(bytes(page1[0:4092])))

    txn = wal_frame(0x01, 1, struct.pack("<QQ", 1, 0))
    txn += wal_frame(0x02, 2, struct.pack("<QIQ", 1, 1, 1) + bytes(page1))
    txn += wal_frame(0x04, 3, struct.pack("<QQII", 1, 2, 1, 0) + bytes(32))
    (OUT / "wal_txn_commit_minimal.bin").write_bytes(bytes(wal) + txn)

    shm = bytearray(4096)
    shm[0:5] = b"YYSH\x00"
    struct.pack_into("<I", shm, 5, 0)
    struct.pack_into("<I", shm, 4092, crc32c(bytes(shm[0:4092])))
    (OUT / "shm_empty.bin").write_bytes(bytes(shm))

    blob = bytearray()
    blob += b"YBLO\x00"
    blob.append(0x01)
    blob += bytes(32)
    blob += struct.pack("<QI", 0, 0)
    blob += bytes(32)
    (OUT / "blob_chunk_header_min.bin").write_bytes(bytes(blob))

    empty_crc = crc32c(b"")
    (OUT / "crc32c_reference.txt").write_text(
        f"empty CRC32C = 0x{empty_crc:08X}\n", encoding="utf-8"
    )
    print(f"wrote fixtures to {OUT}, empty CRC32C=0x{empty_crc:08X}")


if __name__ == "__main__":
    main()
