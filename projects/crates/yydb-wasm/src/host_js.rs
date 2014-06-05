//! In-process JS host runtime adapter for browser wasm sessions.

use std::sync::Arc;

use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::JsValue;
use yydb::{HostFunctionHandle, HostRuntimeAdapter};
use yydb::yydb_udf::{Result as UdfResult, UdfError, UdfValue};

/// Invokes session-local host micros through a JS callback.
pub struct JsHostAdapter {
    invoker: Function,
}

impl JsHostAdapter {
    /// Creates an adapter backed by `invoker(payload)`.
    pub fn new(invoker: Function) -> Self {
        Self { invoker }
    }

    /// Installs the adapter on an open connection.
    pub fn install(conn: &yydb::Connection, invoker: Function) {
        install_js_host_adapter(conn, invoker);
    }
}

impl HostRuntimeAdapter for JsHostAdapter {
    fn invoke_scalar(
        &self,
        handle: &HostFunctionHandle,
        args: &[UdfValue],
    ) -> UdfResult<UdfValue> {
        let payload = Object::new();
        Reflect::set(
            &payload,
            &JsValue::from_str("hostId"),
            &JsValue::from_f64(handle.host_id as f64),
        )
        .map_err(map_reflect_error)?;
        Reflect::set(
            &payload,
            &JsValue::from_str("handleVersion"),
            &JsValue::from_f64(handle.version as f64),
        )
        .map_err(map_reflect_error)?;
        Reflect::set(
            &payload,
            &JsValue::from_str("functionId"),
            &JsValue::from_str(&handle.function_id),
        )
        .map_err(map_reflect_error)?;

        let js_args = Array::new();
        for arg in args {
            js_args.push(&udf_value_to_js(arg)?);
        }
        Reflect::set(&payload, &JsValue::from_str("args"), &js_args).map_err(map_reflect_error)?;

        let result = self
            .invoker
            .call1(&JsValue::NULL, &payload)
            .map_err(|error| UdfError::ExecutionFailed {
                message: format!("wasm micro host invoke failed: {:?}", error),
            })?;
        js_to_udf_value(&result)
    }

    fn invoke_batch(
        &self,
        _handle: &HostFunctionHandle,
        _batches: &[Vec<UdfValue>],
    ) -> UdfResult<Vec<UdfValue>> {
        Err(UdfError::UnsupportedType)
    }
}

/// Installs the JS host adapter on an open connection.
pub fn install_js_host_adapter(conn: &yydb::Connection, invoker: Function) {
    conn.set_host_adapter(Arc::new(JsHostAdapter::new(invoker)));
}

fn map_reflect_error(error: JsValue) -> UdfError {
    UdfError::ExecutionFailed {
        message: format!("wasm micro host payload build failed: {:?}", error),
    }
}

fn udf_value_to_js(value: &UdfValue) -> UdfResult<JsValue> {
    match value {
        UdfValue::Null => Ok(JsValue::NULL),
        UdfValue::Bool(value) => Ok(JsValue::from_bool(*value)),
        UdfValue::I64(value) => Ok(JsValue::from_f64(*value as f64)),
        UdfValue::Text(value) => Ok(JsValue::from_str(value)),
    }
}

fn js_to_udf_value(value: &JsValue) -> UdfResult<UdfValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(UdfValue::Null);
    }
    if let Some(value) = value.as_bool() {
        return Ok(UdfValue::Bool(value));
    }
    if let Some(value) = value.as_f64() {
        if !value.is_finite() {
            return Err(UdfError::UnsupportedType);
        }
        let as_i64 = value as i64;
        if (as_i64 as f64) != value {
            return Err(UdfError::UnsupportedType);
        }
        return Ok(UdfValue::I64(as_i64));
    }
    if let Some(value) = value.as_string() {
        return Ok(UdfValue::Text(value));
    }
    Err(UdfError::UnsupportedType)
}
