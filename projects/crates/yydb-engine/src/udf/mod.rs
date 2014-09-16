//! UDF surface for the embedded YYDB host.
//!
//! - **Simple path:** [`ScalarUdf`] + [`crate::Connection::create_scalar`].
//! - **Advanced path:** [`UdfRegistry`] and related types re-exported from
//!   `yydb-udf` below (`use yydb::udf::UdfRegistry`).
//!
//! Other languages should use `yydb-client` / `@yydb/yydb-client` against
//! `yydb serve` rather than embedding this crate.

mod api;
mod session;

pub use api::*;
pub(crate) use session::UdfSubsystem;
