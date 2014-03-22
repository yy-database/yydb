//! Phase 1 VOS query and DML — re-exported from the [`yydb_query`] engine crate.
//!
//! Prefer [`crate::Connection::query`] and [`crate::Connection::execute`] for embed hosts. Call
//! [`execute`](yydb_query::execute) directly only when driving a custom record map.

pub use yydb_query::*;