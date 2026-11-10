"""Same-workload KV benchmark for the YYDB engine from Python.

The workloads cover batch load, single-key autocommit writes, point gets,
range scans and reopen + get. Only YYDB is measured.

Run: python bench/bench.py
"""

from __future__ import annotations

import math
import os
import pathlib
import shutil
import statistics
import sys
import tempfile
import time

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "src"))

from yydb import Connection  # noqa: E402

ROWS = int(os.environ.get("BENCH_ROWS", "20000"))
KEYS_PER_TXN = int(os.environ.get("BENCH_TXN", "1000"))
POINT_OPS = int(os.environ.get("BENCH_POINT", "10000"))
RANGE_SCANS = int(os.environ.get("BENCH_RANGE", "200"))
RANGE_LEN = int(os.environ.get("BENCH_RANGE_LEN", "100"))
PAYLOAD = b"x" * 64


def key(row: int) -> str:
    return f"row:{row:05d}"


def median(values: list[float]) -> float:
    return statistics.median(values)


def p95(values: list[float]) -> float:
    """Nearest-rank p95, byte-for-byte the same rule as `database.harness`.

    rank = ceil(n * 0.95) computed in integer arithmetic; invalid samples are
    rejected instead of producing a plausible-looking number.
    """
    if not values:
        raise ValueError("p95 requires at least one sample")
    if any(not math.isfinite(value) or value < 0 for value in values):
        raise ValueError("samples must be finite non-negative milliseconds")
    ordered = sorted(values)
    rank = (len(ordered) * 95 + 99) // 100
    index = min(len(ordered) - 1, max(0, rank - 1))
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


def main() -> int:
    root = pathlib.Path(tempfile.mkdtemp(prefix="yydb-py-bench-"))
    try:
        our_path = root / "ours.yydb"
        single_our_path = root / "single-ours.yydb"

        load_ms, reopen_ours = our_load(our_path)
        rows: list[tuple[str, float, float, str]] = [
            (f"our batch load {ROWS} keys (64B value, {KEYS_PER_TXN}/txn)", load_ms, load_ms, "ms"),
        ]

        single_ours = our_single(single_our_path)
        rows.append(("our single insert 200 (autocommit put)", median(single_ours), p95(single_ours), "us"))

        point_ours = our_point(our_path)
        rows.append((f"our point get {POINT_OPS}", median(point_ours), p95(point_ours), "us"))

        scan_ours = our_scan(our_path)
        rows.append((f"our range scan {RANGE_SCANS}x{RANGE_LEN}", median(scan_ours), p95(scan_ours), "us"))

        rows.append(("our reopen + get (20 rounds)", median(reopen_ours), p95(reopen_ours), "ms"))

        print(f"yydb py bench · rows={ROWS} · txn={KEYS_PER_TXN} · point={POINT_OPS} · range={RANGE_SCANS}x{RANGE_LEN}")
        print()
        print("| workload | median | p95 | unit |")
        print("| --- | --- | --- | --- |")
        for name, center, tail, unit in rows:
            print(f"| {name} | {center:.3f} | {tail:.3f} | {unit} |")
        print()
        print("| file | bytes |")
        print(f"| our | {our_path.stat().st_size} |")
        print()
        print(
            "Notes: YYDB exposes no fsync or page-size switch, so its load is transacted writes followed by close. Range scans use the same key prefix and the same 100-row window."
        )
        return 0
    finally:
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
