"""Contract tests for the Python binding.

Native-dependent cases skip when the ``yydb_pyo3`` extension is not built,
the pure import surface still runs everywhere.
"""

from __future__ import annotations

import re
import sys
import tempfile
import unittest
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parents[1]

try:
    import yydb
except ImportError:  # running the suite from a source checkout
    sys.path.insert(0, str(PACKAGE_ROOT / "src"))
    import yydb

from yydb import Database, YydbError
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

    def test_bytes_above_the_inline_boundary_are_refused_in_single_file_mode(self) -> None:
        payload = b"\xff" * (INLINE_BYTES_MAX + 1)
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            path = Path(directory) / "blobs.yydb"
            database = Database.open(path)
            try:
                database.ensure_schema(BYTES_SCHEMA)
                with self.assertRaises(YydbError) as raised:
                    database.execute(BYTES_INSERT, {"id": 1, "label": "big", "body": payload})
                self.assertEqual(raised.exception.code, NATIVE_ERROR)
                self.assertIn("INLINE_BYTES_MAX", raised.exception.message)
                self.assertEqual(database.query(BYTES_READ), [])
            finally:
                database.close()

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

    def test_file_values_are_refused_in_single_file_mode(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-file-") as directory:
            path = Path(directory) / "assets.yydb"
            database = Database.open(path)
            try:
                with self.assertRaises(YydbError) as raised:
                    database.put_file(b"payload", 0)
                self.assertEqual(raised.exception.code, NATIVE_ERROR)
                self.assertIn(".yydx", raised.exception.message)
            finally:
                database.close()

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


if __name__ == "__main__":
    unittest.main()
