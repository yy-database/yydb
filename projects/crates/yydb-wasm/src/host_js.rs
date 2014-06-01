//! In-process JS host runtime adapter for browser wasm sessions.

use std::sync::Arc;

use js_sys::{Array, Function, Object, Reflect};
use wasm_bindgen::JsValue;
use yydb::{
    yydb_udf::{Result as UdfResult, UdfError, UdfValue},
    HostFunctionHandle, HostRuntimeAdapter,
};

/// Adapter that forwards scalar host micro calls into a JS invoker function.
pub struct JsHostAdapter {
    invoker: Function,
}

impl JsHostAdapter {
    /// Creates an adapter backed by a JS `(payload) => scalar` callback.
    pub fn new(invoker: Function) -> Self {
        Self { invoker }
    }

    /// Installs the adapter on `conn`.
    pub fn install(conn: &yydb::Connection, invoker: Function) {
        conn.set_host_adapter(Arc::new(Self::new(invoker)));
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
        .map_err(map_js_error)?;
        Reflect::set(
            &payload,
            &JsValue::from_str("handleVersion"),
            &JsValue::from_f64(handle.version as f64),
        )
        .map_err(map_js_error)?;
        Reflect::set(
            &payload,
            &JsValue::from_str("functionId"),
            &JsValue::from_str(&handle.function_id),
        )
        .map_err(map_js_error)?;
        let js_args = udf_values_to_js_array(args)?;
        Reflect::set(
            &payload,
            &JsValue::from_str("args"),
            js_args.as_ref(),
        )
        .map_err(map_js_error)?;

        let result = self
            .invoker
            .call1(&JsValue::NULL, &payload)
            .map_err(map_js_error)?;
        js_scalar_to_udf(result)
    }

    fn invoke_batch(
        &self,
        _handle: &HostFunctionHandle,
        _batches: &[Vec<UdfValue>],
    ) -> UdfResult<Vec<UdfValue>> {
        Err(UdfError::UnsupportedType)
    }
}

fn udf_values_to_js_array(values: &[UdfValue]) -> UdfResult<Array> {
    let array = Array::new();
    for value in values {
        array.push(&udf_value_to_js(value));
    }
    Ok(array)
}

fn udf_value_to_js(value: &UdfValue) -> JsValue {
    match value {
        UdfValue::Null => JsValue::NULL,
        UdfValue::Bool(value) => JsValue::from_bool(*value),
        UdfValue::I64(value) => JsValue::from_f64(*value as f64),
        UdfValue::Text(value) => JsValue::from_str(value),
    }
}

fn js_scalar_to_udf(value: JsValue) -> UdfResult<UdfValue> {
    if value.is_null() || value.is_undefined() {
        return Ok(UdfValue::Null);
    }
    if let Some(value) = value.as_bool() {
        return Ok(UdfValue::Bool(value));
    }
    if let Some(value) = value.as_f64() {
        if !value.is_finite() || value.fract() != 0.0 {
            return Err(UdfError::UnsupportedType);
        }
        return Ok(UdfValue::I64(value as i64));
    }
    if let Some(value) = value.as_string() {
        return Ok(UdfValue::Text(value));
    }
    Err(UdfError::UnsupportedType)
}

fn map_js_error(error: JsValue) -> UdfError {
    UdfError::ExecutionFailed {
        message: error
            .as_string()
            .unwrap_or_else(|| "wasm host micro invoke failed".to_owned()),
    }
}
