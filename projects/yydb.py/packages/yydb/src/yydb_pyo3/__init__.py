"""Import shim for the compiled ``yydb_pyo3`` extension module.

Maturin places the extension module inside the Python source directory, so
this package exposes the same functions on the ``yydb_pyo3`` import name that
the ``yydb`` package calls.
"""

from __future__ import annotations

try:
    from .yydb_pyo3 import (
        Connection,
        backup_db,
        compact_db,
        doctor_text,
        execute_vos_transaction,
        execute_vos_write,
        info_text,
        init_db,
        kv_delete,
        kv_get,
        kv_put,
        kv_scan,
        kv_write_batch,
        put_file,
        query_vos,
        read_file,
        read_file_window,
        read_schema_file,
        restore,
        serve,
        version,
    )
except ImportError as error:  # running from a source checkout without a wheel
    raise ImportError(f"the yydb_pyo3 extension is not built ({error})") from error

__all__ = [
    "Connection",
    "backup_db",
    "compact_db",
    "doctor_text",
    "execute_vos_transaction",
    "execute_vos_write",
    "info_text",
    "init_db",
    "kv_delete",
    "kv_get",
    "kv_put",
    "kv_scan",
    "kv_write_batch",
    "put_file",
    "query_vos",
    "read_file",
    "read_file_window",
    "read_schema_file",
    "restore",
    "serve",
    "version",
]
