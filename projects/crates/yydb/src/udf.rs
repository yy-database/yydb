//! Rust scalar UDF registration for the embedded YYDB host.
//!
//! YYDB’s supported embedded host is **Rust**. Register native closures or
//! [`ScalarUdf`] implementations on a [`crate::Connection`]. Other languages
//! should use [`yydb-client`](https://github.com/yy-database/yydb) against
//! `yydb serve` rather than embedding this crate.

use std::sync::Arc;

use yydb_types::{Result, Value};

/// Object-safe scalar UDF invoked with [`Value`] arguments.
pub trait ScalarUdf: Send + Sync {
    /// Fixed arity, or `None` for variadic.
    fn arity(&self) -> Option<usize>;

    /// Execute the function.
    fn call(&self, args: &[Value]) -> Result<Value>;
}

/// Type alias for closure-style scalar UDFs.
pub type ScalarFn = dyn Fn(&[Value]) -> Result<Value> + Send + Sync;

pub(crate) struct ClosureUdf {
    arity: Option<usize>,
    func: Arc<ScalarFn>,
}

impl ClosureUdf {
    pub(crate) fn new(arity: Option<usize>, func: Arc<ScalarFn>) -> Self {
        Self { arity, func }
    }
}

impl ScalarUdf for ClosureUdf {
    fn arity(&self) -> Option<usize> {
        self.arity
    }

    fn call(&self, args: &[Value]) -> Result<Value> {
        (self.func)(args)
    }
}

pub(crate) struct RegisteredUdf {
    pub(crate) udf: Arc<dyn ScalarUdf>,
}
