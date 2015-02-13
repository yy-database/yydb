"""Open an embedded YYDB database and print its diagnostics.

Run from a source checkout after building the native wheel, or from an
environment where ``yydb`` is installed.
"""

from __future__ import annotations

import sys
import tempfile
from pathlib import Path

try:
    import yydb
except ImportError:  # running the example straight from a source checkout
    sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "src"))
    import yydb

SCHEMA = "table Project { @@id: uuid, title: utf8 }"


def main() -> int:
    if not yydb.is_native_available():
        print(
            f"yydb native module is not available: {yydb.native_unavailable_reason()}",
            file=sys.stderr,
        )
        return 1

    with tempfile.TemporaryDirectory(prefix="yydb-embedded-") as directory:
        database = yydb.Database.open(Path(directory) / "example.yydb")
        try:
            database.ensure_schema(SCHEMA)
            print(f"yydb version: {yydb.version()}")
            print(database.info())
        finally:
            database.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
