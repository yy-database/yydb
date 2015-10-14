"""Contract tests for the Python binding.

Native-dependent cases skip when the ``yydb_pyo3`` extension is not built,
the pure import surface still runs everywhere.
"""

from __future__ import annotations

import json
import re
import sys
import tempfile
import unittest
from pathlib import Path
from typing import Any

PACKAGE_ROOT = Path(__file__).resolve().parents[1]

try:
    import yydb
except ImportError:  # running the suite from a source checkout
    sys.path.insert(0, str(PACKAGE_ROOT / "src"))
    import yydb

from yydb import Connection, Database, YydbError, restore
from yydb.errors import DATABASE_CLOSED, INVALID_ARGUMENT, NATIVE_ERROR

SCHEMA = "table Project { @@id: uuid, title: utf8 }"
BYTES_SCHEMA = """
table Blob {
    @@blob_id: i64,
    label: utf8,
    body: bytes,
}
"""
BYTES_INSERT = "Blob::insert({ blob_id: $id, label: $label, body: $body })"
BYTES_READ = "Blob.all().collect()"
FILE_SCHEMA = """
table Asset {
    @@asset_id: i64,
    payload: file,
}
"""
DOC_SCHEMA = """
table Doc {
    @@id: i64,
    name: utf8,
    score: f64,
    embedding: vector<3>,
}
"""
DOC_INSERT = "Doc::insert({ id: $id, name: $name, score: $score, embedding: $embedding })"
DOC_READ = "Doc.all().collect()"
FILE_INSERT = "Asset::insert({ asset_id: $id, payload: $payload })"
FILE_READ = "Asset.all().collect()"
INLINE_BYTES_MAX = 4096
NATIVE_REASON = yydb.native_unavailable_reason() or "yydb_pyo3 is not available"


class SurfaceTest(unittest.TestCase):
    def test_public_surface_imports(self) -> None:
        self.assertTrue(callable(yydb.version))
        self.assertTrue(callable(yydb.read_schema_file))
        self.assertTrue(callable(Database.open))

    def test_error_carries_code_and_message(self) -> None:
        error = YydbError("sample_code", "sample message")
        self.assertIsInstance(error, Exception)
        self.assertEqual(error.code, "sample_code")
        self.assertEqual(error.message, "sample message")
        self.assertIn("sample message", str(error))

    def test_invalid_path_is_rejected_before_the_native_call(self) -> None:
        with self.assertRaises(YydbError) as raised:
            Database.open("")
        self.assertEqual(raised.exception.code, INVALID_ARGUMENT)


@unittest.skipUnless(yydb.is_native_available(), NATIVE_REASON)
class NativeTest(unittest.TestCase):
    def test_version_is_non_empty_string(self) -> None:
        self.assertIsInstance(yydb.version(), str)
        self.assertTrue(yydb.version())

    def test_embedded_open_ensure_schema_info_close(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            path = Path(directory) / "embedded.yydb"
            database = Database.open(path)
            try:
                database.ensure_schema(SCHEMA)
                report = database.info()
                self.assertIn("path=", report)
                self.assertIn("schema.version=", report)
                self.assertIn("Project", report)
                self.assertIn("records=", report)
            finally:
                database.close()
            database.close()

            with self.assertRaises(YydbError) as raised:
                database.info()
            self.assertEqual(raised.exception.code, DATABASE_CLOSED)
            self.assertEqual(raised.exception.message, f"database {str(path)!r} is closed")

    def test_reopen_after_close(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            path = Path(directory) / "embedded.yydb"
            first = Database.open(path)
            first.ensure_schema(SCHEMA)
            first.close()

            second = Database.open(path)
            try:
                report = second.info()
                self.assertIn("schema.version=", report)
                self.assertIn("Project", report)
            finally:
                second.close()

    def test_native_failures_become_yydb_errors(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            missing = Path(directory) / "missing.vos"
            with self.assertRaises(YydbError) as raised:
                yydb.read_schema_file(missing)
            self.assertEqual(raised.exception.code, NATIVE_ERROR)
            self.assertNotIsInstance(raised.exception, RuntimeError)
            self.assertIsNotNone(raised.exception.__cause__)

            database = Database.open(Path(directory) / "schema.yydb")
            try:
                with self.assertRaises(YydbError) as schema_error:
                    database.ensure_schema("table {")
                self.assertEqual(schema_error.exception.code, NATIVE_ERROR)
            finally:
                database.close()

    def test_bytes_round_trip_verbatim_through_the_public_handle(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            path = Path(directory) / "blobs.yydb"
            vectors = {
                1: b"",
                2: b"\x00\x00\x00",
                3: b"\xff\xfe\x00\x80",
                4: bytes(range(256)) * 2,
            }
            database = Database.open(path)
            try:
                database.ensure_schema(BYTES_SCHEMA)
                for blob_id, payload in vectors.items():
                    database.execute(BYTES_INSERT, {"id": blob_id, "label": f"row-{blob_id}", "body": payload})
                rows = {row["blob_id"]: row for row in database.query(BYTES_READ)}
                self.assertEqual(sorted(rows), sorted(vectors))
                for blob_id, payload in vectors.items():
                    stored = rows[blob_id]["body"]
                    self.assertIsInstance(stored, bytes)
                    self.assertEqual(stored, payload)
                    self.assertEqual(stored.hex(), payload.hex())
            finally:
                database.close()

            reopened = Database.open(path)
            try:
                rows = {row["blob_id"]: row for row in reopened.query(BYTES_READ)}
                self.assertEqual(rows[3]["body"], vectors[3])
            finally:
                reopened.close()

    def test_bytes_above_the_inline_boundary_read_back_from_objects(self) -> None:
        # 4097 = INLINE_BYTES_MAX + 1, and the octets cover NUL plus invalid UTF-8.
        payload = bytes((index * 7) % 256 for index in range(INLINE_BYTES_MAX + 1))
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            path = Path(directory) / "blobs.yydx"
            database = Database.open(path)
            try:
                database.ensure_schema(BYTES_SCHEMA)
                database.execute(BYTES_INSERT, {"id": 1, "label": "big", "body": payload})
            finally:
                database.close()

            reopened = Database.open(path)
            try:
                rows = reopened.query(BYTES_READ)
                self.assertEqual(len(rows), 1)
                self.assertEqual(rows[0]["body"], payload)
                self.assertEqual(rows[0]["body"].hex(), payload.hex())
            finally:
                reopened.close()

    def test_bytes_above_the_inline_boundary_are_embedded_in_single_file_mode(self) -> None:
        payload = b"\xff" * (INLINE_BYTES_MAX + 1)
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            path = Path(directory) / "blobs.yydb"
            database = Database.open(path)
            try:
                database.ensure_schema(BYTES_SCHEMA)
                database.execute(BYTES_INSERT, {"id": 1, "label": "big", "body": payload})
                self.assertEqual(database.query(BYTES_READ)[0]["body"], payload)
            finally:
                database.close()

            self.assertFalse(
                (Path(directory) / "blobs-objects").exists(),
                "single-file .yydb must not create a sibling object tree",
            )
            reopened = Database.open(path)
            try:
                self.assertEqual(reopened.query(BYTES_READ)[0]["body"], payload, "reopen must re-read the embedded payload")
            finally:
                reopened.close()

    def test_file_round_trip_through_the_public_handle(self) -> None:
        octets = bytes((index * 7) % 256 for index in range(1000))
        with tempfile.TemporaryDirectory(prefix="yydb-python-file-") as directory:
            path = Path(directory) / "assets.yydx"
            database = Database.open(path)
            try:
                database.ensure_schema(FILE_SCHEMA)
                descriptor = database.put_file(octets, 256, name="asset.bin", media_type="application/octet-stream")
                self.assertEqual(descriptor["$yydb"], "file")
                self.assertRegex(descriptor["digest"], r"^[0-9a-f]{64}$")
                self.assertEqual(descriptor["octets"], len(octets))
                self.assertEqual(descriptor["name"], "asset.bin")
                self.assertEqual(descriptor["mediaType"], "application/octet-stream")

                # Renaming never changes content identity: same octets, same manifest.
                renamed = database.put_file(octets, 256, name="other.bin")
                self.assertEqual(renamed["digest"], descriptor["digest"])
                self.assertEqual(renamed["manifest"], descriptor["manifest"])

                database.execute(FILE_INSERT, {"id": 1, "payload": descriptor})
                rows = database.query(FILE_READ)
                self.assertEqual(len(rows), 1)
                self.assertEqual(rows[0]["payload"], descriptor)

                self.assertEqual(database.read_file(descriptor), octets)
                self.assertEqual(database.read_file_window(descriptor, 200, 200), octets[200:400])
                self.assertEqual(database.read_file_window(descriptor, 900, 500), octets[900:])
                self.assertEqual(database.read_file_window(descriptor, 1000, 10), b"", "end of file is an empty window")
                self.assertEqual(database.read_file_window(descriptor, 1500, 10), b"", "past end of file is an empty window")
            finally:
                database.close()

            reopened = Database.open(path)
            try:
                rows = reopened.query(FILE_READ)
                self.assertEqual(reopened.read_file(rows[0]["payload"]), octets, "reopen must re-read every chunk from disk")
            finally:
                reopened.close()

    def test_file_values_are_embedded_in_single_file_mode(self) -> None:
        octets = bytes((index * 7) % 256 for index in range(600))
        with tempfile.TemporaryDirectory(prefix="yydb-python-file-") as directory:
            path = Path(directory) / "assets.yydb"
            database = Database.open(path)
            try:
                database.ensure_schema(FILE_SCHEMA)
                descriptor = database.put_file(octets, 256, name="asset.bin")
                database.execute(FILE_INSERT, {"id": 1, "payload": descriptor})
                self.assertEqual(database.read_file(descriptor), octets)
            finally:
                database.close()

            self.assertFalse(
                (Path(directory) / "assets-objects").exists(),
                "single-file .yydb must not create a sibling object tree",
            )
            reopened = Database.open(path)
            try:
                rows = reopened.query(FILE_READ)
                self.assertEqual(reopened.read_file(rows[0]["payload"]), octets, "reopen must re-read the embedded chunks")
            finally:
                reopened.close()

    def test_file_arguments_are_validated_before_the_native_call(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-file-") as directory:
            path = Path(directory) / "assets.yydx"
            database = Database.open(path)
            try:
                with self.assertRaises(YydbError) as raised:
                    database.put_file("not-bytes")
                self.assertEqual(raised.exception.code, INVALID_ARGUMENT)
                with self.assertRaises(YydbError) as raised:
                    database.read_file_window({"$yydb": "file"}, -1, 10)
                self.assertEqual(raised.exception.code, INVALID_ARGUMENT)
                with self.assertRaises(YydbError) as raised:
                    database.read_file_window({"$yydb": "file"}, 0, -1)
                self.assertEqual(raised.exception.code, INVALID_ARGUMENT)
                with self.assertRaises(YydbError) as raised:
                    database.read_file("not-a-dict")
                self.assertEqual(raised.exception.code, INVALID_ARGUMENT)
            finally:
                database.close()

    def test_read_schema_file_roundtrips(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            schema_path = Path(directory) / "project.vos"
            schema_path.write_text(SCHEMA, encoding="utf-8")
            self.assertEqual(yydb.read_schema_file(schema_path), SCHEMA)

    def _open_docs(self, directory: str) -> tuple[Any, Path]:
        path = Path(directory) / "docs.yydb"
        database = Database.open(path)
        database.ensure_schema(DOC_SCHEMA)
        return database, path

    @staticmethod
    def _vector(*components: float) -> dict[str, Any]:
        import struct

        return {"$yydb": "vector", "dimension": len(components), "components": struct.pack(f"<{len(components)}f", *components).hex()}

    def test_point_query_and_range_filter_round_trip(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            database, _ = self._open_docs(directory)
            try:
                for index in range(1, 8):
                    database.execute(
                        DOC_INSERT,
                        {"id": index, "name": f"doc-{index}-中文", "score": index + 0.5, "embedding": self._vector(float(index), 1.5, 2.5)},
                    )

                point = database.query("Doc.all().filter(x => x.id == 4).collect()")
                self.assertEqual(len(point), 1)
                self.assertEqual(point[0]["name"], "doc-4-中文")
                self.assertEqual(point[0]["score"], 4.5)
                self.assertEqual(point[0]["embedding"], self._vector(4.0, 1.5, 2.5))

                lower = database.query("Doc.all().filter(x => x.id >= 5).collect()")
                self.assertEqual([row["id"] for row in lower], [5, 6, 7])
                upper = database.query("Doc.all().filter(x => x.id <= 2).collect()")
                self.assertEqual([row["id"] for row in upper], [1, 2])
            finally:
                database.close()

    def test_text_predicates_compare_unicode_code_points(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            database, _ = self._open_docs(directory)
            try:
                for index in range(1, 4):
                    database.execute(
                        DOC_INSERT,
                        {"id": index, "name": f"doc-{index}-中文", "score": index + 0.5, "embedding": self._vector(float(index), 0.0, 0.0)},
                    )

                self.assertEqual([row["id"] for row in database.query('Doc.all().filter(x => x.name.contains("文")).collect()')], [1, 2, 3])
                self.assertEqual([row["id"] for row in database.query('Doc.all().filter(x => x.name.startsWith("doc-2")).collect()')], [2])
                self.assertEqual([row["id"] for row in database.query('Doc.all().filter(x => x.name.endsWith("-中文")).collect()')], [1, 2, 3])
                self.assertEqual(database.query('Doc.all().filter(x => x.name.startsWith("Doc")).collect()'), [], "matching is case sensitive")

                with self.assertRaises(YydbError) as caught:
                    database.query('Doc.all().filter(x => x.score.contains("5")).collect()')
                self.assertEqual(caught.exception.code, NATIVE_ERROR)
                self.assertIn("requires a text field", caught.exception.message)
                self.assertIn("score", caught.exception.message)
            finally:
                database.close()

    def test_upsert_updates_existing_key_and_survives_reopen(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            database, path = self._open_docs(directory)
            try:
                database.execute(DOC_INSERT, {"id": 1, "name": "before", "score": 1.5, "embedding": self._vector(1.0, 2.0, 3.0)})
                database.execute(DOC_INSERT, {"id": 1, "name": "after", "score": 9.5, "embedding": self._vector(4.0, 5.0, 6.0)})
                rows = database.query(DOC_READ)
                self.assertEqual(len(rows), 1, "upsert must not grow the table")
                self.assertEqual(rows[0]["name"], "after")
                self.assertEqual(rows[0]["score"], 9.5)
            finally:
                database.close()

            reopened = Database.open(path)
            try:
                rows = reopened.query(DOC_READ)
                self.assertEqual(len(rows), 1)
                self.assertEqual(rows[0]["name"], "after")
                self.assertEqual(rows[0]["embedding"], self._vector(4.0, 5.0, 6.0))
            finally:
                reopened.close()

    def test_transaction_batch_commits_and_rolls_back(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            database, _ = self._open_docs(directory)
            try:
                database.execute_transaction(
                    [
                        (DOC_INSERT, {"id": 1, "name": "txn-1", "score": 1.5, "embedding": self._vector(1.0, 0.0, 0.0)}),
                        (DOC_INSERT, {"id": 2, "name": "txn-2", "score": 2.5, "embedding": self._vector(2.0, 0.0, 0.0)}),
                    ]
                )
                self.assertEqual([row["id"] for row in database.query(DOC_READ)], [1, 2], "committed batch persists")

                with self.assertRaises(YydbError):
                    database.execute_transaction(
                        [
                            (DOC_INSERT, {"id": 3, "name": "ghost", "score": 3.5, "embedding": self._vector(3.0, 0.0, 0.0)}),
                            ("Doc::insert({ id: 4, broken: true })", {}),
                        ]
                    )
                ids = [row["id"] for row in database.query(DOC_READ)]
                self.assertEqual(ids, [1, 2], "a failing batch rolls back every statement")
            finally:
                database.close()

    def test_wide_integers_and_floats_round_trip(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            database, _ = self._open_docs(directory)
            try:
                edges = [(-9223372036854775808, -0.5), (0, 0.0), (9223372036854775807, 1e300)]
                for index, (rowid, score) in enumerate(edges, start=1):
                    database.execute(DOC_INSERT, {"id": rowid, "name": f"edge-{index}", "score": score, "embedding": self._vector(0.0, 0.0, 0.0)})
                rows = {row["id"]: row for row in database.query(DOC_READ)}
                for rowid, score in edges:
                    self.assertEqual(rows[rowid]["score"], score, f"row {rowid} keeps its exact float")
                self.assertIsInstance(rows[edges[0][0]]["id"], int)
                self.assertEqual(rows[edges[0][0]]["id"], -9223372036854775808)
                self.assertEqual(rows[edges[2][0]]["id"], 9223372036854775807)
            finally:
                database.close()

    def test_vector_dimension_mismatch_is_refused(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            database, _ = self._open_docs(directory)
            try:
                with self.assertRaises(YydbError) as raised:
                    database.execute(DOC_INSERT, {"id": 1, "name": "bad", "score": 1.0, "embedding": self._vector(1.0, 2.0)})
                self.assertEqual(raised.exception.code, NATIVE_ERROR)
                self.assertEqual(database.query(DOC_READ), [])
            finally:
                database.close()

    def test_keyed_update_and_delete_patch_and_remove_one_row(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            database, _ = self._open_docs(directory)
            try:
                database.execute(DOC_INSERT, {"id": 1, "name": "kept", "score": 1.0, "embedding": self._vector(1.0, 0.0, 0.0)})
                database.execute(DOC_INSERT, {"id": 2, "name": "gone", "score": 2.0, "embedding": self._vector(0.0, 1.0, 0.0)})
                database.execute("Doc::update({ id: $id, name: $name })", {"id": 1, "name": "patched"})
                database.execute("Doc::delete({ id: $id })", {"id": 2})
                rows = database.query(DOC_READ)
                self.assertEqual([row["id"] for row in rows], [1], "the keyed delete must remove exactly one row")
                self.assertEqual(rows[0]["name"], "patched", "the patched field must change")
                self.assertEqual(rows[0]["score"], 1.0, "fields outside the patch must keep their stored value")
            finally:
                database.close()

    def test_update_and_delete_pipelines_are_refused(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            database, _ = self._open_docs(directory)
            try:
                database.execute(DOC_INSERT, {"id": 1, "name": "kept", "score": 1.0, "embedding": self._vector(1.0, 0.0, 0.0)})
                with self.assertRaises(YydbError) as update_error:
                    database.execute("Doc.all().filter(x => x.id == 1).update(x => x.{ score: 2.0 })", {})
                self.assertEqual(update_error.exception.code, NATIVE_ERROR)
                with self.assertRaises(YydbError) as delete_error:
                    database.execute("Doc.all().filter(x => x.id == 1).delete()", {})
                self.assertEqual(delete_error.exception.code, NATIVE_ERROR)
                rows = database.query(DOC_READ)
                self.assertEqual(len(rows), 1, "refused pipelines must not mutate rows")
            finally:
                database.close()

    def test_raw_records_crud_scan_and_batch(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-kv-") as directory:
            path = Path(directory) / "kv.yydb"
            database = Database.open(path)
            try:
                database.put("user:1", b"one")
                database.put("user:2", b"two")
                database.put("other:1", b"\x00\xff")
                self.assertEqual(database.get("user:1"), b"one")
                self.assertEqual(database.get("user:2"), b"two")
                self.assertIsNone(database.get("missing"))

                first_page = database.scan("user:", limit=1)
                self.assertEqual(first_page, [("user:1", b"one")])
                second_page = database.scan("user:", after="user:1", limit=10)
                self.assertEqual(second_page, [("user:2", b"two")])

                database.write_batch([("user:3", b"three"), ("user:2", None)])
                self.assertEqual(database.get("user:3"), b"three")
                self.assertIsNone(database.get("user:2"))

                database.delete("user:1")
                self.assertIsNone(database.get("user:1"))
            finally:
                database.close()

    def test_connection_savepoints_roll_back_without_touching_earlier_writes(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-sp-") as directory:
            path = Path(directory) / "session.yydb"
            connection = Connection.open(path)
            try:
                self.assertEqual(connection.journal_mode(), "wal")
                self.assertIsInstance(connection.wal_path(), str)
                connection.put("a", b"kept")
                connection.begin()
                connection.put("b", b"transient")
                connection.savepoint("outer")
                connection.put("c", b"discarded")
                connection.savepoint("inner")
                connection.put("d", b"also-discarded")
                connection.rollback_to("outer")
                self.assertIsNone(connection.get("c"))
                self.assertIsNone(connection.get("d"))
                self.assertEqual(connection.get("b"), b"transient")
                connection.savepoint("probe")
                connection.release("probe")
                connection.commit()
                self.assertEqual(connection.get("a"), b"kept")
                self.assertEqual(connection.get("b"), b"transient")
            finally:
                connection.close()

            reopened = Database.open(path)
            try:
                self.assertEqual(reopened.get("a"), b"kept")
                self.assertEqual(reopened.get("b"), b"transient")
                self.assertIsNone(reopened.get("c"))
            finally:
                reopened.close()

    def test_rollback_discards_the_open_transaction(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-rb-") as directory:
            path = Path(directory) / "rollback.yydb"
            connection = Connection.open(path)
            try:
                connection.begin()
                connection.put("gone", b"value")
                connection.rollback()
                self.assertIsNone(connection.get("gone"))
                self.assertEqual(connection.record_count(), 0)
            finally:
                connection.close()

    def test_backup_restore_and_doctor(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-backup-") as directory:
            path = Path(directory) / "source.yydb"
            backup_path = Path(directory) / "copy.yydb"
            database = Database.open(path)
            try:
                database.put("one", b"1")
                database.put("two", b"2")
                database.backup(backup_path)
                report = json.loads(database.doctor())
                self.assertEqual(
                    [issue for issue in report["issues"] if issue["severity"] == "error"],
                    [],
                    f"doctor must report no errors on a healthy file: {report}",
                )
                self.assertEqual(report["orphan_object_count"], 0)
            finally:
                database.close()

            restored = restore(backup_path)
            try:
                self.assertEqual(restored.get("one"), b"1")
                self.assertEqual(restored.get("two"), b"2")
                self.assertEqual(restored.record_count(), 2)
            finally:
                restored.close()

    def test_compact_is_refused_for_single_file_layouts(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-compact-") as directory:
            path = Path(directory) / "plain.yydb"
            database = Database.open(path)
            try:
                database.put("one", b"1")
                with self.assertRaises(YydbError) as raised:
                    database.compact()
                self.assertEqual(raised.exception.code, NATIVE_ERROR)
            finally:
                database.close()

    def test_closed_connection_refuses_record_operations(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-closed-") as directory:
            connection = Connection.open(Path(directory) / "closed.yydb")
            connection.close()
            with self.assertRaises(YydbError) as raised:
                connection.get("anything")
            self.assertEqual(raised.exception.code, DATABASE_CLOSED)


if __name__ == "__main__":
    unittest.main()
