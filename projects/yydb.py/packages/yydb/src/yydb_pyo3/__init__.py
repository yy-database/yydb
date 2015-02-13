"""Import shim for the compiled ``yydb_pyo3`` extension module.

Maturin places the extension module inside the Python source directory, so
this package exposes the same functions on the ``yydb_pyo3`` import name that
the ``yydb`` package calls.
"""

from __future__ import annotations

try:
    from .yydb_pyo3 import (
        info_text,
        init_db,
        read_schema_file,
        serve,
        version,
    )
except ImportError as error:  # running from a source checkout without a wheel
    raise ImportError(f"the yydb_pyo3 extension is not built ({error})") from error

__all__ = ["info_text", "init_db", "read_schema_file", "serve", "version"]
