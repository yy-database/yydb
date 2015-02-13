"""Contract tests for the Python binding.

Native-dependent cases skip when the ``yydb_pyo3`` extension is not built,
the pure import surface still runs everywhere.
"""

from __future__ import annotations

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

    def test_read_schema_file_roundtrips(self) -> None:
        with tempfile.TemporaryDirectory(prefix="yydb-python-") as directory:
            schema_path = Path(directory) / "project.vos"
            schema_path.write_text(SCHEMA, encoding="utf-8")
            self.assertEqual(yydb.read_schema_file(schema_path), SCHEMA)


if __name__ == "__main__":
    unittest.main()
