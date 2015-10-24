"""Same-workload KV comparison from Python: the YYDB engine against `sqlite3`.

Both sides store the same keys and the same 64-byte values, load the same row
count with the same page size and transaction boundary (``KEYS_PER_TXN`` keys
per commit), and read through the same point, range and reopen workloads.

Run: python bench/bench.py
"""

from __future__ import annotations

import os
import pathlib
import shutil
import sqlite3
import statistics
import sys
import tempfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))

from yydb import Connection  # noqa: E402

ROWS = 20_000
KEYS_PER_TXN = 1_000
POINT_OPS = 10_000
RANGE_SCANS = 200
RANGE_LEN = 100
PAGE_SIZE = 4_096
PAYLOAD = b"x" * 64


def key(row: int) -> str:
    return f"row:{row:05d}"


def median(values: list[float]) -> float:
    return statistics.median(values)


def p95(values: list[float]) -> float:
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int(round(len(ordered) * 0.95)) - 1))
    return ordered[index]


def elapsed_ms(start: float) -> float:
    return (time.perf_counter() - start) * 1_000


def our_load(path: pathlib.Path) -> tuple[float, list[float]]:
    connection = Connection.open(path)
    start = time.perf_counter()
    for offset in range(0, ROWS, KEYS_PER_TXN):
        connection.begin()
        connection.write_batch(
            [(key(row), PAYLOAD) for row in range(offset + 1, offset + KEYS_PER_TXN + 1)]
        )
        connection.commit()
    total_ms = elapsed_ms(start)
    connection.close()

    reopen_ms: list[float] = []
    for _ in range(20):
        at = time.perf_counter()
        reopened = Connection.open(path)
        reopened.get(key(1))
        reopen_ms.append(elapsed_ms(at))
        reopened.close()
    return total_ms, reopen_ms


def our_point(path: pathlib.Path) -> list[float]:
    connection = Connection.open(path)
    samples: list[float] = []
    for index in range(POINT_OPS):
        rowid = ((index * 7919) % ROWS) + 1
        start = time.perf_counter()
        connection.get(key(rowid))
        samples.append((time.perf_counter() - start) * 1_000_000)
    connection.close()
    return samples


def our_scan(path: pathlib.Path) -> list[float]:
    connection = Connection.open(path)
    samples: list[float] = []
    cursor = None
    for index in range(RANGE_SCANS):
        start = time.perf_counter()
        rows = connection.scan(prefix="row:", after=cursor, limit=RANGE_LEN)
        if len(rows) != RANGE_LEN:
            raise RuntimeError(f"{len(rows)} scan rows, expected {RANGE_LEN}")
        cursor = rows[-1][0] if index % 2 == 0 else None
        samples.append((time.perf_counter() - start) * 1_000_000)
    connection.close()
    return samples


def our_single(path: pathlib.Path) -> list[float]:
    connection = Connection.open(path)
    samples: list[float] = []
    for row in range(1, 201):
        start = time.perf_counter()
        connection.put(key(row), PAYLOAD)
        samples.append((time.perf_counter() - start) * 1_000_000)
    connection.close()
    return samples


def oracle_load(path: pathlib.Path, synchronous: str) -> float:
    connection = sqlite3.connect(path)
    try:
        connection.execute(f"pragma page_size = {PAGE_SIZE}")
        connection.execute(f"pragma synchronous = {synchronous}")
        connection.execute("pragma journal_mode = memory")
        connection.execute("create table kv (k text primary key, v blob)")
        start = time.perf_counter()
        for offset in range(0, ROWS, KEYS_PER_TXN):
            connection.execute("begin")
            connection.executemany(
                "insert into kv (k, v) values (?, ?)",
                [(key(row), PAYLOAD) for row in range(offset + 1, offset + KEYS_PER_TXN + 1)],
            )
            connection.execute("commit")
        return elapsed_ms(start)
    finally:
        connection.close()


def oracle_point(path: pathlib.Path) -> list[float]:
    connection = sqlite3.connect(path)
    try:
        samples: list[float] = []
        for index in range(POINT_OPS):
            rowid = ((index * 7919) % ROWS) + 1
            start = time.perf_counter()
            connection.execute("select v from kv where k = ?", (key(rowid),)).fetchone()
            samples.append((time.perf_counter() - start) * 1_000_000)
        return samples
    finally:
        connection.close()


def oracle_scan(path: pathlib.Path) -> list[float]:
    connection = sqlite3.connect(path)
    try:
        samples: list[float] = []
        for index in range(RANGE_SCANS):
            start_key = "row:" if index % 2 == 0 else key(((index * 97) % ROWS) + 1)
            start = time.perf_counter()
            rows = connection.execute(
                "select k, v from kv where k >= ? order by k limit ?", (start_key, RANGE_LEN)
            ).fetchall()
            if len(rows) != RANGE_LEN:
                raise RuntimeError(f"{len(rows)} scan rows, expected {RANGE_LEN}")
            samples.append((time.perf_counter() - start) * 1_000_000)
        return samples
    finally:
        connection.close()


def oracle_reopen(path: pathlib.Path) -> list[float]:
    samples: list[float] = []
    for _ in range(20):
        start = time.perf_counter()
        connection = sqlite3.connect(path)
        connection.execute("select v from kv where k = ?", (key(1),)).fetchone()
        connection.close()
        samples.append(elapsed_ms(start))
    return samples


def oracle_single(path: pathlib.Path) -> list[float]:
    connection = sqlite3.connect(path)
    try:
        connection.execute("pragma synchronous = off")
        connection.execute("pragma journal_mode = memory")
        connection.execute("create table kv (k text primary key, v blob)")
        samples: list[float] = []
        for row in range(1, 201):
            start = time.perf_counter()
            connection.execute("insert into kv (k, v) values (?, ?)", (key(row), PAYLOAD))
            samples.append((time.perf_counter() - start) * 1_000_000)
        return samples
    finally:
        connection.close()


def main() -> int:
    root = pathlib.Path(tempfile.mkdtemp(prefix="yydb-py-bench-"))
    try:
        our_path = root / "ours.yydb"
        off_path = root / "sqlite-off.sqlite3"
        full_path = root / "sqlite-full.sqlite3"
        single_our_path = root / "single-ours.yydb"
        single_off_path = root / "single-off.sqlite3"

        load_ms, reopen_ours = our_load(our_path)
        off_ms = oracle_load(off_path, "off")
        full_ms = oracle_load(full_path, "full")
        rows: list[tuple[str, float, float, str]] = [
            ("our batch load 20k keys (64B value, 1k/txn)", load_ms, load_ms, "ms"),
            (
                "python sqlite3 batch load 20k keys (4k page, 1k/txn, synchronous=off)",
                off_ms,
                off_ms,
                "ms",
            ),
            (
                "python sqlite3 batch load 20k keys (4k page, 1k/txn, synchronous=full)",
                full_ms,
                full_ms,
                "ms",
            ),
        ]

        single_ours = our_single(single_our_path)
        rows.append(("our single insert 200 (autocommit put)", median(single_ours), p95(single_ours), "us"))
        single_oracle = oracle_single(single_off_path)
        rows.append(
            (
                "python sqlite3 single insert 200 (autocommit, synchronous=off)",
                median(single_oracle),
                p95(single_oracle),
                "us",
            )
        )

        point_ours = our_point(our_path)
        rows.append(("our point get 10k", median(point_ours), p95(point_ours), "us"))
        point_oracle = oracle_point(off_path)
        rows.append(
            ("python sqlite3 point lookup 10k (where k = ?)", median(point_oracle), p95(point_oracle), "us")
        )

        scan_ours = our_scan(our_path)
        rows.append((f"our range scan {RANGE_SCANS}x{RANGE_LEN}", median(scan_ours), p95(scan_ours), "us"))
        scan_oracle = oracle_scan(off_path)
        rows.append(
            (
                f"python sqlite3 range scan {RANGE_SCANS}x{RANGE_LEN}",
                median(scan_oracle),
                p95(scan_oracle),
                "us",
            )
        )

        rows.append(("our reopen + get (20 rounds)", median(reopen_ours), p95(reopen_ours), "ms"))
        reopen_oracle = oracle_reopen(off_path)
        rows.append(
            ("python sqlite3 reopen + get (20 rounds)", median(reopen_oracle), p95(reopen_oracle), "ms")
        )

        print(f"yydb py bench · rows={ROWS} · txn={KEYS_PER_TXN} · point={POINT_OPS} · range={RANGE_SCANS}x{RANGE_LEN} · page={PAGE_SIZE}")
        print()
        print("| workload | median | p95 | unit |")
        print("| --- | --- | --- | --- |")
        for name, center, tail, unit in rows:
            print(f"| {name} | {center:.3f} | {tail:.3f} | {unit} |")
        print()
        print("| file | bytes |")
        for label, path in (("our", our_path), ("sqlite off", off_path), ("sqlite full", full_path)):
            print(f"| {label} | {path.stat().st_size} |")
        print()
        print(
            "Notes: YYDB exposes no fsync or page-size switch. `synchronous=off` is the closest SQLite setting and `synchronous=full` is the durability reference. Range scans use the same key prefix and the same 100-row window."
        )
        return 0
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
