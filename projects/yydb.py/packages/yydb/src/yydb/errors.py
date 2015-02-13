"""Stable error surface for the YYDB Python binding.

Every failure that crosses into Python surfaces as :class:`YydbError`, so
callers never have to match bare ``RuntimeError`` messages from the native
extension.
"""

from __future__ import annotations

#: The ``yydb_pyo3`` extension module could not be imported.
NATIVE_UNAVAILABLE = "native_unavailable"
#: The :class:`yydb.Database` handle was already closed.
DATABASE_CLOSED = "database_closed"
#: A Python-side argument did not satisfy the binding contract.
INVALID_ARGUMENT = "invalid_argument"
#: A native YYDB operation failed.
NATIVE_ERROR = "native_error"


class YydbError(Exception):
    """One hierarchical error with a stable ``code`` and a ``message``."""

    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code
        self.message = message

    def __str__(self) -> str:
        return f"{self.code}: {self.message}"

    def __repr__(self) -> str:
        return f"YydbError(code={self.code!r}, message={self.message!r})"
