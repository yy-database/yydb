use std::sync::Arc;

use crate::execution;
use crate::udf::{ClosureUdf, ScalarFn, ScalarUdf};
use crate::{HostMicroDefinition, HostRuntimeAdapter, Result, Value};

use super::Connection;

impl Connection {
    /// Register a Rust scalar UDF with a fixed arity.
    ///
    /// Replaces any previous registration with the same `name`. UDFs live only
    /// in this process; reopen the file and they are gone until re-registered.
    pub fn create_scalar<F>(&self, name: &str, n_args: usize, func: F) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        self.create_scalar_versioned(name, 1, n_args, func)
    }

    /// Register a versioned Rust scalar UDF with a fixed arity.
    pub fn create_scalar_versioned<F>(
        &self,
        name: &str,
        version: u32,
        n_args: usize,
        func: F,
    ) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        self.create_scalar_variadic_versioned(name, version, Some(n_args), func)
    }

    /// Register a Rust scalar UDF; `None` arity accepts any argument count.
    pub fn create_scalar_variadic<F>(&self, name: &str, arity: Option<usize>, func: F) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        self.create_scalar_variadic_versioned(name, 1, arity, func)
    }

    /// Register a versioned Rust scalar UDF with optional fixed arity.
    pub fn create_scalar_variadic_versioned<F>(
        &self,
        name: &str,
        version: u32,
        arity: Option<usize>,
        func: F,
    ) -> Result<()>
    where
        F: Fn(&[Value]) -> Result<Value> + Send + Sync + 'static,
    {
        let boxed: Arc<ScalarFn> = Arc::new(func);
        if matches!(arity, Some(1) | Some(2)) {
            return self
                .udfs
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .register_closure_scalar(name, version, arity, boxed);
        }
        let udf: Arc<dyn ScalarUdf> = Arc::new(ClosureUdf::new(arity, boxed));
        self.register_scalar_versioned(name, version, udf)
    }

    /// Register an object-safe [`ScalarUdf`] at version `1`.
    pub fn register_scalar(&self, name: &str, udf: Arc<dyn ScalarUdf>) -> Result<()> {
        self.register_scalar_versioned(name, 1, udf)
    }

    /// Register a validated local read-only execution body with its identity and version.
    pub fn register_execution_udf(&self, body: execution::ValidatedUdf) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_execution_udf(body)
    }

    /// Parse and register a VOS-authored local scalar UDF.
    ///
    /// The source is parsed through the VOS facade, which uses Oak for the
    /// language surface. Only the validated local scalar subset is lowered by
    /// this embedded binder. The body is not persisted in the `.yydb` file.
    pub fn register_vos_scalar(&self, source: &str) -> Result<()> {
        self.register_vos_scalar_versioned(source, 1)
    }

    /// Parse and register a versioned VOS-authored local scalar UDF.
    pub fn register_vos_scalar_versioned(&self, source: &str, version: u32) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_vos_scalar(source, version)
    }

    /// Parse and register a VOS-authored catalog macro UDF.
    ///
    /// The lowered body is registered in the connection catalog layer. Persistence
    /// into the `.yydb` file is not implemented yet.
    pub fn register_vos_macro(&self, source: &str) -> Result<()> {
        self.register_vos_macro_versioned(source, 1)
    }

    /// Parse and register a versioned VOS-authored catalog macro UDF.
    pub fn register_vos_macro_versioned(&self, source: &str, version: u32) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_vos_macro(source, version)
    }

    /// Install the process-local host runtime adapter used by [`register_host_micro`].
    pub fn set_host_adapter(&self, adapter: Arc<dyn HostRuntimeAdapter>) {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_host_adapter(adapter);
    }

    /// Register a session-local host micro through the host runtime adapter.
    pub fn register_host_micro(&self, definition: HostMicroDefinition) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_host_micro(definition)
    }

    /// Register an object-safe [`ScalarUdf`] at an explicit version.
    pub fn register_scalar_versioned(
        &self,
        name: &str,
        version: u32,
        udf: Arc<dyn ScalarUdf>,
    ) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .register_scalar_versioned(name, version, udf)
    }

    /// Remove a previously registered scalar UDF.
    pub fn remove_scalar(&self, name: &str) -> Result<()> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove_scalar(name)
    }

    /// Names of scalar UDFs registered on this connection.
    pub fn list_scalars(&self) -> Vec<String> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .list_scalars()
    }

    /// Invoke a registered scalar UDF at version `1`.
    pub fn call_scalar(&self, name: &str, args: &[Value]) -> Result<Value> {
        self.call_scalar_version(name, 1, args)
    }

    /// Invoke a registered scalar UDF at an explicit version.
    pub fn call_scalar_version(&self, name: &str, version: u32, args: &[Value]) -> Result<Value> {
        self.udfs
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .call_scalar_version(name, version, args)
    }
}
