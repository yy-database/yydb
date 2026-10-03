//! TCP-backed [`TypeScriptHostAdapter`] for `yydb serve` wire peers.

use std::net::TcpStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use yydb_udf::{
    Result as UdfResult, TypeScriptFunctionHandle, TypeScriptHostAdapter, UdfError, UdfValue,
};

use crate::wire::{
    self, decode_micro_host_invoke_ok, encode_micro_host_invoke, read_frame, write_frame, Frame,
    MsgType,
};

/// Invokes TypeScript micros on the wire peer that owns the host registry.
pub struct TcpTypeScriptHostAdapter {
    stream: Mutex<TcpStream>,
    next_id: AtomicU32,
}

impl TcpTypeScriptHostAdapter {
    /// Creates an adapter that shares the client TCP connection with the serve loop.
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream: Mutex::new(stream),
            next_id: AtomicU32::new(1_000_000),
        }
    }

    fn roundtrip_invoke(
        &self,
        handle: &TypeScriptFunctionHandle,
        args: &[UdfValue],
    ) -> UdfResult<UdfValue> {
        let request_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = Frame::new(
            MsgType::MicroHostInvoke,
            request_id,
            encode_micro_host_invoke(handle, args),
        );
        let mut stream = self
            .stream
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        write_frame(&mut *stream, &request).map_err(map_wire_error)?;
        let response = read_frame(&mut *stream).map_err(map_wire_error)?;
        if response.msg_type == MsgType::Error {
            return Err(UdfError::ExecutionFailed {
                message: wire::decode_error_message(&response.body),
            });
        }
        if response.request_id != request_id {
            return Err(UdfError::ExecutionFailed {
                message: "wire micro host invoke response id mismatch".into(),
            });
        }
        if response.msg_type != MsgType::MicroHostInvokeOk {
            return Err(UdfError::ExecutionFailed {
                message: format!("unexpected wire response type {}", response.msg_type.as_u16()),
            });
        }
        decode_micro_host_invoke_ok(&response.body).map_err(map_wire_error)
    }
}

impl TypeScriptHostAdapter for TcpTypeScriptHostAdapter {
    fn invoke_scalar(
        &self,
        handle: &TypeScriptFunctionHandle,
        args: &[UdfValue],
    ) -> UdfResult<UdfValue> {
        self.roundtrip_invoke(handle, args)
    }

    fn invoke_batch(
        &self,
        _handle: &TypeScriptFunctionHandle,
        _batches: &[Vec<UdfValue>],
    ) -> UdfResult<Vec<UdfValue>> {
        Err(UdfError::UnsupportedType)
    }
}

fn map_wire_error(error: yydb_types::Error) -> UdfError {
    UdfError::ExecutionFailed {
        message: error.to_string(),
    }
}
