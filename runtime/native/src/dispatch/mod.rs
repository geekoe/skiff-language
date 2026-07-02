//! RuntimeProgram native and builtin dispatch routing.

#![allow(dead_code)]

mod actor;
mod adapter;
mod builtin;
mod bytes;
mod config;
mod core;
mod external;
mod file;
mod http;
mod http_helpers;
mod invocation;
mod json;
mod telemetry;
mod time;
mod websocket;

pub use adapter::NativeDispatch;
use core::{
    ensure_native_capability_context, native_capability_route_mismatch, unsupported_native_target,
};
pub use core::{runtime_shared_native_route, RuntimeNativeRoute};
pub use invocation::{RuntimeActorNativeMetadata, RuntimeNativeInvocation};

#[cfg(test)]
mod tests;
