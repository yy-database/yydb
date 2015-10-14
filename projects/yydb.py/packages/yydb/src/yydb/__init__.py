"""Python bindings for the YYDB VOS database.

The public surface stays thin on purpose: version lookup, an embedded
database handle with VOS read/write pipelines, raw record operations and a
session handle for explicit transactions and savepoints. SQL execution is not
exposed here, the YYDB engine owns the VOS authoring surface.

Two handles exist for two lifetimes. :class:`Database` opens and drops a native
connection per call, so it cannot span a transaction. :class:`Connection`
keeps one native handle open, which is what `begin`, `savepoint`, `rollback_to`
and `release` require.
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
    "Connection",
    "Database",
    "YydbError",
    "is_native_available",
    "native_unavailable_reason",
    "read_schema_file",
    "restore",
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


def restore(backup_path: str | os.PathLike[str]) -> "Connection":
    """Open a verified backup copy and return a session handle.

    ``.yydb`` backups get every page CRC32C check, ``.yydx`` backups get the
    object-tree closure and file digest check. Restoring opens the copy in
    place, it does not overwrite a live database: copy the result to the
    destination yourself, then open that path.
    """
    resolved = _resolve_path(backup_path, "backup_path")
    native = _require_native()
    return Connection(_call("restore", native.restore, resolved))


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

    def execute_transaction(self, statements: list[tuple[str, dict[str, Any]]]) -> None:
        """Run a batch of VOS write statements inside one transaction.

        A failure in any statement rolls the whole batch back, otherwise the
        batch commits. The Python handle is stateless by design, so one
        transaction spans exactly one native call.
        """
        self._ensure_open()
        if not isinstance(statements, list) or not statements:
            raise YydbError(INVALID_ARGUMENT, "statements must be a non-empty list")
        batch = []
        for statement in statements:
            if not isinstance(statement, tuple) or len(statement) != 2:
                raise YydbError(INVALID_ARGUMENT, "each statement must be a (source, params) tuple")
            source, params = statement
            if not isinstance(source, str):
                raise YydbError(INVALID_ARGUMENT, "source must be a str")
            batch.append((source, dict(params or {})))
        native = _require_native()
        _call("execute_transaction", native.execute_vos_transaction, self._path, batch)

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

        A ``.yydx`` layout keeps the object tree in ``<stem>-objects/``; a
        single-file ``.yydb`` embeds the same chunk and manifest objects as
        records in its main image. Both are supported, but embedding large
        payloads inflates the file and slows down every snapshot and backup.
        The returned tagged dict ``{"$yydb": "file", ...}`` can be bound as a VOS
        write parameter for a ``file`` column; renaming never changes content
        identity because the digest covers the complete octets.
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

    def put(self, key: str, value: bytes) -> None:
        """Write one record on the raw record tree."""
        self._ensure_open()
        if not isinstance(key, str) or not key:
            raise YydbError(INVALID_ARGUMENT, "key must be a non-empty str")
        if not isinstance(value, (bytes, bytearray, memoryview)):
            raise YydbError(INVALID_ARGUMENT, "value must be bytes-like")
        native = _require_native()
        _call("put", native.kv_put, self._path, key, bytes(value))

    def get(self, key: str) -> bytes | None:
        """Read one record, or ``None`` when the key is absent."""
        self._ensure_open()
        if not isinstance(key, str) or not key:
            raise YydbError(INVALID_ARGUMENT, "key must be a non-empty str")
        native = _require_native()
        return _call("get", native.kv_get, self._path, key)

    def delete(self, key: str) -> None:
        """Delete one record. Absent keys succeed silently."""
        self._ensure_open()
        if not isinstance(key, str) or not key:
            raise YydbError(INVALID_ARGUMENT, "key must be a non-empty str")
        native = _require_native()
        _call("delete", native.kv_delete, self._path, key)

    def scan(
        self,
        prefix: str = "",
        after: str | None = None,
        limit: int = 1000,
    ) -> list[tuple[str, bytes]]:
        """Scan records in lexical key order.

        ``after`` is an exclusive continuation cursor: pass the last key of the
        previous page to continue. An empty ``prefix`` scans every key.
        """
        self._ensure_open()
        if not isinstance(prefix, str):
            raise YydbError(INVALID_ARGUMENT, "prefix must be a str")
        if after is not None and not isinstance(after, str):
            raise YydbError(INVALID_ARGUMENT, "after must be a str or None")
        if isinstance(limit, bool) or not isinstance(limit, int) or limit <= 0:
            raise YydbError(INVALID_ARGUMENT, "limit must be a positive int")
        native = _require_native()
        return _call("scan", native.kv_scan, self._path, prefix, after, limit)

    def write_batch(self, changes: list[tuple[str, bytes | None]]) -> None:
        """Write or delete many records in one commit.

        Each entry is ``(key, value)``, and ``value=None`` deletes the key.
        """
        self._ensure_open()
        native = _require_native()
        _call("write_batch", native.kv_write_batch, self._path, _changes(changes))

    def doctor(self) -> str:
        """Run read-only consistency probes and return the report as JSON.

        This reports damage, it never repairs it. A data page that fails CRC32C
        stays failed, use :meth:`backup` artifacts and ``restore()`` instead.
        """
        self._ensure_open()
        native = _require_native()
        return _call("doctor", native.doctor_text, self._path)

    def backup(self, dest: str | os.PathLike[str]) -> None:
        """Checkpoint, then copy a self-contained main file to ``dest``."""
        self._ensure_open()
        resolved = _resolve_path(dest, "dest")
        native = _require_native()
        _call("backup", native.backup_db, self._path, resolved)

    def compact(self) -> str:
        """Fold the WAL and reclaim safe orphan objects, returning JSON.

        Applies to ``.yydx`` layouts. A single-file ``.yydb`` handle raises a
        native error because it has no object tree to compact.
        """
        self._ensure_open()
        native = _require_native()
        return _call("compact", native.compact_db, self._path)

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


def _changes(changes: list[tuple[str, bytes | None]]) -> list[tuple[str, bytes | None]]:
    if not isinstance(changes, list) or not changes:
        raise YydbError(INVALID_ARGUMENT, "changes must be a non-empty list")
    prepared: list[tuple[str, bytes | None]] = []
    for change in changes:
        if not isinstance(change, tuple) or len(change) != 2:
            raise YydbError(INVALID_ARGUMENT, "each change must be a (key, value) tuple")
        key, value = change
        if not isinstance(key, str) or not key:
            raise YydbError(INVALID_ARGUMENT, "change keys must be non-empty str")
        if value is None:
            prepared.append((key, None))
            continue
        if not isinstance(value, (bytes, bytearray, memoryview)):
            raise YydbError(INVALID_ARGUMENT, "change values must be bytes-like or None")
        prepared.append((key, bytes(value)))
    return prepared


class Connection:
    """One native handle that stays open across calls.

    The stateless :class:`Database` handle serves single-call operations. A
    transaction, a savepoint or a rollback spans several calls and therefore
    needs this handle. Instances are not thread-safe, matching the underlying
    writer handle.

    ``wal_path()`` and ``shm_path()`` return *reserved* companion paths, not
    evidence of existing files: the sidecar appears only in WAL mode after a
    commit appended frames. Use :meth:`journal_mode` and
    :meth:`wal_frame_count` for durability state instead of testing paths.
    """

    def __init__(self, handle: Any) -> None:
        self._handle = handle
        self._closed = False

    @classmethod
    def open(cls, path: str | os.PathLike[str], *, wal: bool = True) -> "Connection":
        """Open ``path``; ``wal=False`` selects the delete journal."""
        resolved = _resolve_path(path, "path")
        native = _require_native()
        return cls(_call("open", native.Connection, resolved, wal))

    @classmethod
    def open_yydx(cls, path: str | os.PathLike[str]) -> "Connection":
        """Open a ``.yydx`` database with its sibling object tree."""
        resolved = _resolve_path(path, "path")
        native = _require_native()
        return cls(_call("open_yydx", native.Connection.open_yydx, resolved))

    @classmethod
    def open_in_memory(cls) -> "Connection":
        """Open an in-memory database."""
        native = _require_native()
        return cls(_call("open_in_memory", native.Connection.open_in_memory))

    def close(self) -> None:
        """Close the handle and release its writer lock. Repeated calls are no-ops."""
        if not self._closed:
            self._handle.close()
            # Dropping the native object releases the writer lock, `close()` alone
            # only marks the handle closed inside the engine.
            self._handle = None
            self._closed = True

    def is_closed(self) -> bool:
        """Whether this handle has been closed."""
        return self._closed or self._handle.is_closed()

    def path(self) -> str | None:
        """Main database path, or ``None`` for an in-memory handle."""
        return self._native_call("path", "path")

    def wal_path(self) -> str | None:
        """Reserved ``-wal`` path; the sidecar is not guaranteed to exist."""
        return self._native_call("wal_path", "wal_path")

    def shm_path(self) -> str | None:
        """Reserved ``-shm`` path; the sidecar is not guaranteed to exist."""
        return self._native_call("shm_path", "shm_path")

    def journal_mode(self) -> str:
        """Active journal mode, ``delete`` or ``wal``."""
        return self._native_call("journal_mode", "journal_mode")

    def wal_frame_count(self) -> int:
        """Frames currently present in the WAL sidecar."""
        return self._native_call("wal_frame_count", "wal_frame_count")

    def checkpoint(self) -> None:
        """Fold the WAL into the main file."""
        self._native_call("checkpoint", "checkpoint")

    def schema_version(self) -> int | None:
        """Published schema revision, ``None`` before the first schema."""
        return self._native_call("schema_version", "schema_version")

    def record_count(self) -> int:
        """Number of committed records."""
        return self._native_call("record_count", "record_count")

    def get(self, key: str) -> bytes | None:
        """Read one record, or ``None`` when the key is absent."""
        return self._native_call("get", "get", key)

    def put(self, key: str, value: bytes) -> None:
        """Write one record."""
        if not isinstance(value, (bytes, bytearray, memoryview)):
            raise YydbError(INVALID_ARGUMENT, "value must be bytes-like")
        self._native_call("put", "put", key, bytes(value))

    def delete(self, key: str) -> None:
        """Delete one record. Absent keys succeed silently."""
        self._native_call("delete", "delete", key)

    def scan(
        self,
        prefix: str = "",
        after: str | None = None,
        limit: int = 1000,
    ) -> list[tuple[str, bytes]]:
        """Scan records in lexical key order, ``after`` is exclusive."""
        if isinstance(limit, bool) or not isinstance(limit, int) or limit <= 0:
            raise YydbError(INVALID_ARGUMENT, "limit must be a positive int")
        return self._native_call("scan", "scan", prefix, after, limit)

    def write_batch(self, changes: list[tuple[str, bytes | None]]) -> None:
        """Write or delete many records in one commit."""
        self._native_call("write_batch", "write_batch", _changes(changes))

    def begin(self) -> None:
        """Begin an explicit transaction."""
        self._native_call("begin", "begin")

    def commit(self) -> None:
        """Commit the open transaction."""
        self._native_call("commit", "commit")

    def rollback(self) -> None:
        """Roll the open transaction back."""
        self._native_call("rollback", "rollback")

    def savepoint(self, name: str) -> None:
        """Create a named savepoint inside the open transaction."""
        self._native_call("savepoint", "savepoint", name)

    def rollback_to(self, name: str) -> None:
        """Roll back to a savepoint, discarding writes made after it."""
        self._native_call("rollback_to", "rollback_to", name)

    def release(self, name: str) -> None:
        """Release a savepoint so it can no longer be rolled back to."""
        self._native_call("release", "release", name)

    def doctor(self) -> str:
        """Read-only consistency report as JSON. Does not repair damage."""
        return self._native_call("doctor", "doctor")

    def backup(self, dest: str | os.PathLike[str]) -> None:
        """Checkpoint, then copy a self-contained main file to ``dest``."""
        resolved = _resolve_path(dest, "dest")
        self._native_call("backup", "backup", resolved)

    def compact(self) -> str:
        """Fold the WAL and reclaim safe orphan objects (``.yydx`` only)."""
        return self._native_call("compact", "compact")

    def _native_call(self, operation: str, name: str, *args: Any) -> Any:
        if self.is_closed():
            raise YydbError(DATABASE_CLOSED, "connection is closed")
        return _call(operation, getattr(self._handle, name), *args)

    def __repr__(self) -> str:
        return f"Connection(path={self.path()!r}, closed={self.is_closed()})"
