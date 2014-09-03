//! Invocation context and implementation strategies.

mod implementation;
mod invocation;
mod value_execution;

pub use implementation::UdfImplementation;
pub use invocation::{Budget, InvocationMode, UdfContext, UdfInvocation};
pub use value_execution::{
    from_execution, into_execution, type_from_execution, type_to_execution,
};
