//! YYDB wire server — loopback-first TCP/WebSocket serve for out-of-process clients.
//!
//! Embedded Rust hosts use the [`yydb`] embed facade directly. Browser and lightweight
//! TypeScript hosts use [`@yydb/yydb-client`](../../packages/yydb-client) against a
//! process running this server (typically started by [`@yydb/yydb`](../../packages/yydb)
//! `yydb serve` or `Database.open()`).

#![deny(missing_docs)]

pub mod host_wire;

mod serve;

pub use serve::{assert_bind_allowed, run_serve};
