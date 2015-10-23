"""SQLite format-3 acceptance cases for the Python binding.

Capability evidence comes from external engines only: Python `sqlite3` and
`PRAGMA integrity_check`. Each case covers both directions, bytes written here
and read externally, and bytes written externally and read here.
"""

from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parents[1]

try:
    import yydb_pyo3
except ImportError:  # running the suite from a source checkout
    sys.path.insert(0, str(PACKAGE_ROOT / "src"))
    import yydb_pyo3

COLUMNS = [
    {"name": "id", "declaredType": "INTEGER", "primaryKey": True},
    {"name": "label", "declaredType": "TEXT"},
    {"name": "score", "declaredType": "REAL"},
    {"name": "payload", "declaredType": "BLOB"},
]


def query(path, sql: str):
    connection = sqlite3.connect(path)
    try:
        return connection.execute(sql).fetchall()
    finally:
        connection.close()


def integrity(path) -> str:
    return query(path, "pragma integrity_check")[0][0]


def page_size(path) -> int:
    return query(path, "pragma page_size")[0][0]


def header_page_size(bytes_like) -> int:
    raw = int.from_bytes(bytes_like[16:18], "big")
    return 65536 if raw == 1 else raw


class Workspace:
    def __init__(self, label: str) -> None:
        self.root = Path(tempfile.mkdtemp(prefix=f"yydb-sqlite-{label}-"))
        self.path = self.root / "main.sqlite3"

    def __enter__(self) -> "Workspace":
        return self

    def __exit__(self, *args) -> None:
        for entry in self.root.glob("*"):
            try:
                entry.unlink()
            except OSError:
                pass
        try:
            self.root.rmdir()
        except OSError:
            pass


def writer_for(path, columns=COLUMNS, page_size=4096):
    writer = yydb_pyo3.SqliteWriter.create(page_size)
    writer.create_table("t", json.dumps(columns))
    return writer


def rows_of(db, name="t"):
    return [(row["rowid"], [value.get("value") for value in row["columns"]]) for row in db.table(name)]


def python_value(value):
    """Converts one tagged writer value into a value the stdlib module accepts."""
    if isinstance(value, dict):
        if "$i64" in value:
            return int(value["$i64"])
        if "$blob" in value:
            return bytes.fromhex(value["$blob"])
    return value


class Format3Test(unittest.TestCase):
    def test_page_header_and_page_size(self) -> None:
        with Workspace("header") as space:
            writer = writer_for(space.path)
            writer.insert("t", json.dumps([{"$omit": True}, "one", 1.25, None]))
            writer.save(str(space.path))

            self.assertEqual(page_size(space.path), 4096)
            self.assertEqual(query(space.path, "pragma journal_mode")[0][0], "delete")
            self.assertEqual(query(space.path, "select count(*) from sqlite_master")[0][0], 1)
            self.assertEqual(integrity(space.path), "ok")

            external = space.root / "external.sqlite3"
            connection = sqlite3.connect(external)
            try:
                connection.execute("pragma page_size = 2048")
                connection.execute("create table e (id integer primary key, label text)")
                connection.execute("insert into e (label) values ('external-one')")
                connection.commit()
            finally:
                connection.close()

            expected = page_size(external)
            reader = yydb_pyo3.SqliteDatabase.open(str(external))
            self.assertEqual(header_page_size(reader.pages()), expected)
            self.assertEqual(len(reader.pages()) % expected, 0)
            rows = reader.table("e")
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]["columns"][1]["value"], "external-one")

    def test_leaf_split_divider_and_rowid_order(self) -> None:
        with Workspace("split") as space:
            writer = writer_for(space.path, page_size=1024)
            for index in range(1, 1201):
                writer.insert("t", json.dumps([{"$omit": True}, f"row-{index}", index / 2, None]))
            writer.save(str(space.path))

            external_rowids = [row[0] for row in query(space.path, "select rowid from t order by rowid")]
            self.assertEqual(len(external_rowids), 1200)
            self.assertEqual(query(space.path, "select rowid, label from t order by rowid desc limit 1")[0], (1200, "row-1200"))
            self.assertEqual(integrity(space.path), "ok")

            reader = yydb_pyo3.SqliteDatabase.open(str(space.path))
            self.assertEqual([rowid for rowid, _ in rows_of(reader)], external_rowids)

            mirrored = space.root / "external.sqlite3"
            connection = sqlite3.connect(mirrored)
            try:
                connection.execute("pragma page_size = 1024")
                connection.execute("create table t (id integer primary key, label text, score real, payload blob)")
                connection.executemany("insert into t (label, score) values (?, ?)", [(f"row-{i}", i / 2) for i in range(1, 1201)])
                connection.commit()
            finally:
                connection.close()

            expected = [row[0] for row in query(mirrored, "select rowid from t order by rowid")]
            mirror_reader = yydb_pyo3.SqliteDatabase.open(str(mirrored))
            self.assertEqual(len(rows_of(mirror_reader)), 1200)
            self.assertEqual([rowid for rowid, _ in rows_of(mirror_reader)], expected)

    def test_overflow_payload_round_trip(self) -> None:
        with Workspace("overflow") as space:
            writer = writer_for(space.path, page_size=512)
            writer.insert("t", json.dumps([{"$omit": True}, "big", 1, {"$blob": "07" * 3000}]))
            writer.save(str(space.path))

            expected = query(space.path, "select length(payload), hex(payload) from t")[0]
            self.assertEqual(integrity(space.path), "ok")

            rows = yydb_pyo3.SqliteDatabase.open(str(space.path)).table("t")
            self.assertEqual(rows[0]["columns"][3]["kind"], "blob")
            self.assertEqual(rows[0]["columns"][3]["value"], bytes([7]) * expected[0])
            self.assertEqual(rows[0]["columns"][3]["value"].hex(), expected[1].lower())

            mirrored = space.root / "external.sqlite3"
            blob = bytes([9]) * 3000
            connection = sqlite3.connect(mirrored)
            try:
                connection.execute("pragma page_size = 512")
                connection.execute("create table t (id integer primary key, label text, score real, payload blob)")
                connection.execute("insert into t (label, score, payload) values ('external-big', 1, ?)", (blob,))
                connection.commit()
            finally:
                connection.close()

            mirror = yydb_pyo3.SqliteDatabase.open(str(mirrored)).table("t")
            self.assertEqual(mirror[0]["columns"][3]["kind"], "blob")
            self.assertEqual(mirror[0]["columns"][3]["value"], blob)

    def test_serial_types_preserve_storage_class(self) -> None:
        with Workspace("serial") as space:
            payloads = [
                ["null", None],
                ["empty-text", ""],
                ["min-int", {"$i64": "-9223372036854775808"}],
                ["max-int", {"$i64": "9223372036854775807"}],
                ["real", 2.5],
                ["unicode", "案例-明"],
                ["empty-blob", {"$blob": ""}],
                ["blob", {"$blob": "010203"}],
            ]
            writer = writer_for(
                space.path,
                [
                    {"name": "id", "declaredType": "INTEGER", "primaryKey": True},
                    {"name": "kind", "declaredType": "TEXT"},
                    {"name": "value", "declaredType": "BLOB"},
                ],
            )
            for kind, value in payloads:
                writer.insert("t", json.dumps([{"$omit": True}, kind, value]))
            writer.save(str(space.path))

            kinds = [row[0] for row in query(space.path, "select typeof(value) from t order by rowid")]
            self.assertEqual(kinds, ["null", "text", "integer", "integer", "real", "text", "blob", "blob"])
            self.assertEqual(
                [row[0] for row in query(space.path, "select value from t where rowid in (3, 4) order by rowid")],
                [-9223372036854775808, 9223372036854775807],
            )
            self.assertEqual(integrity(space.path), "ok")

            rows = yydb_pyo3.SqliteDatabase.open(str(space.path)).table("t")
            self.assertEqual([row["columns"][2]["kind"] for row in rows], kinds)
            self.assertEqual(rows[5]["columns"][2]["value"], "案例-明")

            mirrored = space.root / "external.sqlite3"
            connection = sqlite3.connect(mirrored)
            try:
                connection.execute("create table t (id integer primary key, kind text, value blob)")
                for kind, value in payloads:
                    connection.execute("insert into t (kind, value) values (?, ?)", (kind, python_value(value)))
                connection.commit()
            finally:
                connection.close()

            external_kinds = [row[0] for row in query(mirrored, "select typeof(value) from t order by rowid")]
            mirror = yydb_pyo3.SqliteDatabase.open(str(mirrored)).table("t")
            self.assertEqual([row["columns"][2]["kind"] for row in mirror], external_kinds)
            self.assertEqual(mirror[5]["columns"][2]["value"], "案例-明")

    def test_real_affinity_on_numeric_text(self) -> None:
        with Workspace("affinity") as space:
            writer = yydb_pyo3.SqliteWriter.create(4096)
            writer.create_table(
                "t",
                json.dumps(
                    [
                        {"name": "id", "declaredType": "INTEGER", "primaryKey": True},
                        {"name": "score", "declaredType": "REAL"},
                        {"name": "maybe", "declaredType": "NUMERIC"},
                    ]
                ),
            )
            writer.insert("t", json.dumps([{"$omit": True}, 7, 3]))
            writer.insert("t", json.dumps([{"$omit": True}, "7.5", "3.5"]))
            writer.save(str(space.path))

            self.assertEqual(
                query(space.path, "select typeof(score), typeof(maybe), score, maybe from t order by rowid"),
                [("real", "integer", 7.0, 3), ("real", "real", 7.5, 3.5)],
            )

            mirrored = space.root / "external.sqlite3"
            connection = sqlite3.connect(mirrored)
            try:
                connection.execute("create table t (id integer primary key, score real, maybe numeric)")
                connection.execute("insert into t (score, maybe) values (7, 3)")
                connection.execute("insert into t (score, maybe) values ('7.5', '3.5')")
                connection.commit()
            finally:
                connection.close()

            external_values = query(mirrored, "select score, maybe from t order by rowid")
            mirror = yydb_pyo3.SqliteDatabase.open(str(mirrored)).table("t")
            # Integral REAL values live on disk as integers, so the record-level reader
            # reports `integer` where a SQL read reports `real`. Values must agree.
            self.assertEqual(
                [(float(row["columns"][1]["value"]), float(row["columns"][2]["value"])) for row in mirror],
                [(float(score), float(maybe)) for score, maybe in external_values],
            )
            self.assertEqual(
                [row["columns"][2]["kind"] for row in mirror],
                [row[0] for row in query(mirrored, "select typeof(maybe) from t order by rowid")],
            )

    def test_integer_primary_key_is_rowid_alias(self) -> None:
        with Workspace("rowid") as space:
            writer = writer_for(space.path)
            self.assertEqual(writer.insert("t", json.dumps([{"$omit": True}, "auto", 1, None])), 1)
            self.assertEqual(writer.insert("t", json.dumps([100, "explicit", 2, None])), 100)
            writer.save(str(space.path))

            self.assertEqual(query(space.path, "select rowid, id from t order by rowid"), [(1, 1), (100, 100)])
            reader = yydb_pyo3.SqliteDatabase.open(str(space.path))
            self.assertEqual([rowid for rowid, _ in rows_of(reader)], [1, 100])

            mirrored = space.root / "external.sqlite3"
            connection = sqlite3.connect(mirrored)
            try:
                connection.execute("create table t (id integer primary key, label text, score real, payload blob)")
                connection.execute("insert into t (id, label) values (1, 'auto')")
                connection.execute("insert into t (id, label) values (250, 'explicit')")
                connection.commit()
            finally:
                connection.close()

            expected = [row[0] for row in query(mirrored, "select rowid from t order by rowid")]
            mirror = yydb_pyo3.SqliteDatabase.open(str(mirrored)).table("t")
            self.assertEqual([row["rowid"] for row in mirror], expected)
            self.assertEqual(mirror[1]["columns"][1]["value"], "explicit")

    def test_sqlite_master_index_and_autoindex(self) -> None:
        with Workspace("schema") as space:
            writer = yydb_pyo3.SqliteWriter.create(4096)
            writer.create_table(
                "t",
                json.dumps(
                    [
                        {"name": "id", "declaredType": "INTEGER", "primaryKey": True},
                        {"name": "slug", "declaredType": "TEXT", "unique": True},
                        {"name": "name", "declaredType": "TEXT"},
                    ]
                ),
            )
            writer.create_index("t_name", "t", ["name"], False)
            writer.create_index("t_name_unique", "t", ["name"], True)
            writer.insert("t", json.dumps([{"$omit": True}, "a", "alpha"]))
            writer.save(str(space.path))

            master = query(space.path, "select name, type from sqlite_master order by name")
            self.assertEqual(
                master,
                [("sqlite_autoindex_t_1", "index"), ("t", "table"), ("t_name", "index"), ("t_name_unique", "index")],
            )
            self.assertEqual(
                query(space.path, "select name, origin from pragma_index_list('t') order by name"),
                [("sqlite_autoindex_t_1", "u"), ("t_name", "c"), ("t_name_unique", "c")],
            )
            names = sorted(f"{item['kind']}:{item['name']}" for item in yydb_pyo3.SqliteDatabase.open(str(space.path)).schema())
            self.assertEqual(names, sorted(f"{kind}:{name}" for name, kind in master))

            mirrored = space.root / "external.sqlite3"
            connection = sqlite3.connect(mirrored)
            try:
                connection.execute("create table t (id integer primary key, slug text unique, name text)")
                connection.execute("create index t_name on t (name)")
                connection.execute("create unique index t_name_unique on t (name)")
                connection.execute("insert into t (slug, name) values ('a', 'alpha')")
                connection.commit()
            finally:
                connection.close()

            external_master = query(mirrored, "select name, type from sqlite_master order by name")
            mirror = sorted(f"{item['kind']}:{item['name']}" for item in yydb_pyo3.SqliteDatabase.open(str(mirrored)).schema())
            self.assertEqual(mirror, sorted(f"{kind}:{name}" for name, kind in external_master))

    def test_update_delete_rollback(self) -> None:
        with Workspace("mutate") as space:
            writer = writer_for(space.path)
            for index in range(1, 21):
                writer.insert("t", json.dumps([{"$omit": True}, f"r{index}", index, None]))
            writer.begin()
            writer.insert("t", json.dumps([{"$omit": True}, "rolled-back", 0, None]))
            writer.rollback()
            writer.update("t", 5, json.dumps([{"keep": True}, {"set": "updated"}, {"set": 5.5}, {"keep": True}]))
            writer.delete("t", 7)
            writer.save(str(space.path))

            self.assertEqual(query(space.path, "select count(*) from t")[0][0], 19)
            self.assertEqual(query(space.path, "select label, score from t where rowid = 5"), [("updated", 5.5)])
            self.assertEqual(query(space.path, "select count(*) from t where rowid = 7")[0][0], 0)
            self.assertEqual(query(space.path, "select count(*) from t where label = 'rolled-back'")[0][0], 0)
            self.assertEqual(integrity(space.path), "ok")

            reader = yydb_pyo3.SqliteDatabase.open(str(space.path))
            rows = rows_of(reader)
            self.assertEqual(len(rows), 19)
            self.assertEqual(rows[4][1][1], "updated")
            self.assertEqual(rows[4][1][2], 5.5)

            mirrored = space.root / "external.sqlite3"
            connection = sqlite3.connect(mirrored, isolation_level=None)
            try:
                connection.execute("create table t (id integer primary key, label text, score real, payload blob)")
                connection.execute("begin")
                connection.executemany("insert into t (label, score) values (?, ?)", [(f"r{i}", i) for i in range(1, 21)])
                connection.execute("rollback")
                connection.execute("begin")
                connection.executemany("insert into t (label, score) values (?, ?)", [(f"r{i}", i) for i in range(1, 21)])
                connection.execute("commit")
                connection.execute("begin")
                connection.execute("insert into t (label, score) values ('rolled-back', 0)")
                connection.execute("rollback")
                connection.execute("update t set label = 'updated', score = 5.5 where rowid = 5")
                connection.execute("delete from t where rowid = 7")
            finally:
                connection.close()

            expected = query(mirrored, "select rowid, label from t order by rowid")
            mirror = rows_of(yydb_pyo3.SqliteDatabase.open(str(mirrored)))
            self.assertEqual([(rowid, values[1]) for rowid, values in mirror], expected)

    def test_without_rowid_rows_are_read_in_key_order(self) -> None:
        with Workspace("without-rowid") as space:
            connection = sqlite3.connect(space.path)
            try:
                connection.execute("create table wo (k text primary key, v integer) without rowid")
                connection.executemany("insert into wo values (?, ?)", [("b", 2), ("a", 1), ("c", 3)])
                connection.commit()
            finally:
                connection.close()

            external = query(space.path, "select k, v from wo order by k")
            rows = yydb_pyo3.SqliteDatabase.open(str(space.path)).table_without_rowid("wo")
            self.assertEqual([(row[0]["value"], row[1]["value"]) for row in rows], external)
            self.assertEqual([value["kind"] for value in rows[0]], ["text", "integer"])

    def test_secondary_index_matches_external_indexed_by(self) -> None:
        with Workspace("index") as space:
            writer = writer_for(space.path, page_size=1024)
            writer.create_index("i_label", "t", ["label"], False)
            for index in range(1, 1201):
                writer.insert("t", json.dumps([{"$omit": True}, f"label-{(index * 37) % 1200:04d}", index / 2, None]))
            writer.save(str(space.path))

            reader = yydb_pyo3.SqliteDatabase.open(str(space.path))
            self.assertGreater(len(reader.pages()) / header_page_size(reader.pages()), 4)
            external_order = query(space.path, "select rowid, label from t indexed by i_label order by label, rowid")
            self.assertEqual(len(external_order), 1200)
            self.assertEqual(integrity(space.path), "ok")
            self.assertEqual([(row["rowid"], row["columns"][0]["value"]) for row in reader.index("i_label")], external_order)

            connection = sqlite3.connect(space.path)
            try:
                connection.execute("drop index i_label")
                connection.execute("create index i_label on t (label)")
                connection.commit()
            finally:
                connection.close()
            self.assertEqual(
                query(space.path, "select rowid, label from t indexed by i_label order by label, rowid"),
                external_order,
            )

            mirrored = space.root / "external.sqlite3"
            connection = sqlite3.connect(mirrored)
            try:
                connection.execute("pragma page_size = 1024")
                connection.execute("create table t (id integer primary key, label text, score real, payload blob)")
                connection.execute("create index i_label on t (label)")
                connection.executemany(
                    "insert into t (label, score) values (?, ?)",
                    [(f"label-{(i * 53) % 1200:04d}", i / 2) for i in range(1, 1201)],
                )
                connection.commit()
            finally:
                connection.close()

            expected = query(mirrored, "select rowid, label from t indexed by i_label order by label, rowid")
            mirror = yydb_pyo3.SqliteDatabase.open(str(mirrored)).index("i_label")
            self.assertEqual([(row["rowid"], row["columns"][0]["value"]) for row in mirror], expected)

    def test_committed_wal_frames_are_visible(self) -> None:
        with Workspace("wal") as space:
            connection = sqlite3.connect(space.path)
            try:
                connection.execute("pragma journal_mode = wal")
                connection.execute("create table w (id integer primary key, label text)")
                connection.executemany("insert into w (label) values (?)", [(f"w-{i}",) for i in range(1, 6)])
                connection.commit()

                wal_path = f"{space.path}-wal"
                self.assertTrue(os.path.exists(wal_path))
                self.assertGreater(os.path.getsize(wal_path), 32)

                rows = yydb_pyo3.SqliteDatabase.open(str(space.path)).table("w")
                self.assertEqual(len(rows), 5)
                self.assertEqual(rows[-1]["rowid"], 5)
                self.assertEqual(connection.execute("select count(*) from w").fetchone()[0], 5)
            finally:
                connection.close()

    def test_hot_journal_rolls_partial_pages_back(self) -> None:
        with Workspace("journal") as space:
            connection = sqlite3.connect(space.path)
            try:
                connection.execute("create table j (id integer primary key, label text)")
                connection.executemany("insert into j (label) values (?)", [("committed-one",), ("committed-two",)])
                connection.commit()
            finally:
                connection.close()

            script = (
                "import os, sqlite3, sys\n"
                "connection = sqlite3.connect(sys.argv[1], isolation_level=None)\n"
                "connection.execute('pragma journal_mode = delete')\n"
                "connection.execute('begin immediate')\n"
                "connection.execute(\"insert into j (label) values ('uncommitted')\")\n"
                "os._exit(9)\n"
            )
            result = subprocess.run([sys.executable, "-c", script, str(space.path)], capture_output=True)
            self.assertNotEqual(result.returncode, 0)

            journal_path = f"{space.path}-journal"
            self.assertTrue(os.path.exists(journal_path))
            self.assertGreaterEqual(os.path.getsize(journal_path), 512)

            rows = yydb_pyo3.SqliteDatabase.open(str(space.path)).table("j")
            self.assertEqual(len(rows), 2)
            self.assertEqual([row["columns"][1]["value"] for row in rows], ["committed-one", "committed-two"])
            self.assertEqual(integrity(space.path), "ok")


if __name__ == "__main__":
    unittest.main()
