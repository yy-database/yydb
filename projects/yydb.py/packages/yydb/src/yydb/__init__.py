"""Python bindings for the YYDB VOS database.

The public surface stays thin on purpose: version lookup, an embedded
database handle, and VOS schema file reads. SQL execution is not exposed
here, the YYDB engine owns the VOS authoring surface.
"""

from __future__ import annotations

import os
from typing import Any

from .errors import (
    DATABASE_CLOSED,
    INVALID_ARGUMENT,
    NATIVE_ERROR,
    NATIVE_UNAVAILABLE,
    YydbError,
)

__all__ = [
    "Database",
    "YydbError",
    "is_native_available",
    "native_unavailable_reason",
    "read_schema_file",
    "version",
]

try:
    import yydb_pyo3 as _native
except ImportError as _import_error:  # pragma: no cover - depends on the native build
    _native = None
    _native_reason: str | None = f"{type(_import_error).__name__}: {_import_error}"
else:
    _native_reason = None


def is_native_available() -> bool:
    """Return whether the ``yydb_pyo3`` extension module was imported."""
    return _native is not None


def native_unavailable_reason() -> str | None:
    """Return the import failure behind :func:`is_native_available`, if any."""
    return _native_reason


def _require_native() -> Any:
    if _native is None:
        raise YydbError(
            NATIVE_UNAVAILABLE,
            f"the yydb_pyo3 extension module is not importable ({_native_reason})",
        )
    return _native


def _call(operation: str, function: Any, *args: Any) -> Any:
    try:
        return function(*args)
    except YydbError:
        raise
    except Exception as error:
        raise YydbError(NATIVE_ERROR, f"{operation} failed: {error}") from error


def _resolve_path(value: str | os.PathLike[str], label: str) -> str:
    try:
        resolved = os.fspath(value)
    except TypeError as error:
        raise YydbError(INVALID_ARGUMENT, f"{label} must be a str or os.PathLike") from error
    if isinstance(resolved, bytes):
        resolved = os.fsdecode(resolved)
    if not resolved:
        raise YydbError(INVALID_ARGUMENT, f"{label} must be non-empty")
    return resolved


def version() -> str:
    """Return the embedded YYDB engine version string."""
    native = _require_native()
    return _call("version", native.version)


def read_schema_file(schema_path: str | os.PathLike[str]) -> str:
    """Read a VOS schema document from disk."""
    resolved = _resolve_path(schema_path, "schema_path")
    native = _require_native()
    return _call("read_schema_file", native.read_schema_file, resolved)


class Database:
    """One embedded YYDB database file.

    The native binding opens and drops a Rust connection per operation, so a
    Python handle owns no long-lived native resource. :meth:`close` marks the
    handle unusable and is idempotent, and :meth:`open` on the same path again
    yields a fresh handle.
    """

    def __init__(self, path: str) -> None:
        self._path = path
        self._closed = False

    @classmethod
    def open(cls, path: str | os.PathLike[str]) -> "Database":
        """Open ``path``, creating a blank schema when the file is new."""
        resolved = _resolve_path(path, "path")
        database = cls(resolved)
        native = _require_native()
        _call("open", native.init_db, resolved)
        return database

    def ensure_schema(self, document: str) -> None:
        """Apply a VOS schema ``document`` to this database file."""
        self._ensure_open()
        if not isinstance(document, str):
            raise YydbError(INVALID_ARGUMENT, "schema document must be a str")
        native = _require_native()
        _call("ensure_schema", native.init_db, self._path, document)

    def info(self) -> str:
        """Return multi-line database diagnostics."""
        self._ensure_open()
        native = _require_native()
        return _call("info", native.info_text, self._path)

    def close(self) -> None:
        """Mark this handle closed. Repeated calls are no-ops."""
        self._closed = True

    def _ensure_open(self) -> None:
        if self._closed:
            raise YydbError(DATABASE_CLOSED, f"database {self._path!r} is closed")

    def __repr__(self) -> str:
        return f"Database(path={self._path!r}, closed={self._closed})"
