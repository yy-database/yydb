"""Python bindings for the YYDB VOS database.

The public surface stays thin on purpose: version lookup, an embedded
database handle with one VOS read pipeline and one parameterized VOS write,
and VOS schema file reads. SQL execution is not exposed here, the YYDB
engine owns the VOS authoring surface.
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
        """Open ``path`` without writing a schema document."""
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

    def execute(self, source: str, params: dict[str, Any] | None = None) -> None:
        """Run one VOS write statement with named parameters.

        A :class:`bytes` parameter is handed to the engine as raw octets and
        never spelled as text, so NUL bytes and invalid UTF-8 survive.
        """
        self._ensure_open()
        if not isinstance(source, str):
            raise YydbError(INVALID_ARGUMENT, "source must be a str")
        native = _require_native()
        _call("execute", native.execute_vos_write, self._path, source, dict(params or {}))

    def query(self, source: str) -> list[dict[str, Any]]:
        """Run one VOS read pipeline and return one dict per row.

        A ``bytes`` column is returned as :class:`bytes` holding the stored
        octets, not as text.
        """
        self._ensure_open()
        if not isinstance(source, str):
            raise YydbError(INVALID_ARGUMENT, "source must be a str")
        native = _require_native()
        return _call("query", native.query_vos, self._path, source)

    def put_file(
        self,
        payload: bytes,
        chunk_size: int = 0,
        name: str | None = None,
        media_type: str | None = None,
    ) -> dict[str, Any]:
        """Publish a logical ``file`` value: chunks first, then the manifest.

        Only ``.yydx`` layouts keep the object tree a descriptor depends on, so
        a single-file ``.yydb`` handle refuses the call. The returned tagged
        dict ``{"$yydb": "file", ...}`` can be bound as a VOS write parameter
        for a ``file`` column; renaming never changes content identity because
        the digest covers the complete octets.
        """
        self._ensure_open()
        if not isinstance(payload, (bytes, bytearray, memoryview)):
            raise YydbError(INVALID_ARGUMENT, "payload must be bytes-like")
        if isinstance(chunk_size, bool) or not isinstance(chunk_size, int):
            raise YydbError(INVALID_ARGUMENT, "chunk_size must be an int")
        native = _require_native()
        return _call(
            "put_file",
            native.put_file,
            self._path,
            bytes(payload),
            chunk_size,
            name,
            media_type,
        )

    def read_file(self, descriptor: dict[str, Any]) -> bytes:
        """Read the complete octets of a ``file`` value and verify its digest.

        A descriptor whose chunks were tampered with fails instead of returning
        plausible-looking bytes. Accepts the dict returned by :meth:`put_file`
        or one read back from a ``file`` column.
        """
        self._ensure_open()
        if not isinstance(descriptor, dict):
            raise YydbError(INVALID_ARGUMENT, "descriptor must be a dict")
        native = _require_native()
        return _call("read_file", native.read_file, self._path, descriptor)

    def read_file_window(self, descriptor: dict[str, Any], offset: int, length: int) -> bytes:
        """Read one VOS window of a ``file`` value.

        An offset at or past end of file yields empty bytes; a request starting
        before the end is truncated to the remaining octets. Every touched
        chunk is verified against the manifest.
        """
        self._ensure_open()
        if not isinstance(descriptor, dict):
            raise YydbError(INVALID_ARGUMENT, "descriptor must be a dict")
        if isinstance(offset, bool) or not isinstance(offset, int) or offset < 0:
            raise YydbError(INVALID_ARGUMENT, "offset must be a non-negative int")
        if isinstance(length, bool) or not isinstance(length, int) or length < 0:
            raise YydbError(INVALID_ARGUMENT, "length must be a non-negative int")
        native = _require_native()
        return _call("read_file_window", native.read_file_window, self._path, descriptor, offset, length)

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
