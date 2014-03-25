//! YYDB wire server — loopback-first TCP/WebSocket serve for out-of-process clients.
//!
//! Depends on the [`yydb`] facade only. Embedded Rust hosts use `yydb::Connection`
//! directly. Remote hosts connect through `yydb-client` / `@yydb/yydb-client`.
//! Language bindings (`yydb-napi`, `yydb-pyo3`) start this loop for `serve` /
//! `Database.open()`-style products.

#![deny(missing_docs)]

pub mod host_wire;

mod serve;

pub use serve::{assert_bind_allowed, run_serve};
