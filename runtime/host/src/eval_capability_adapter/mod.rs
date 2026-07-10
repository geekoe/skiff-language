//! Runtime-owned adapters for eval capability contracts.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
};

use bytes::Bytes;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::{
    capability_context as concrete, error as root_error, telemetry::RequestTelemetryContext,
};

use skiff_runtime_boundary::file::{FileCreateOptions, ImmutableFileRef};
use skiff_runtime_capability_context as capability_contract;
use skiff_runtime_capability_context::{
    ActorFindControlRequest, ActorPutControlRequest, ActorRemoveControlRequest, CancellationToken,
    ExecutionControlResult, FileCapabilityError, FileCapabilityFuture, RequestEffectDoubleControl,
    SpawnSubmitControlRequest, StreamPoll, StreamPullSource, StreamRuntimeError,
    StreamRuntimeResult,
};
use skiff_runtime_eval::{
    capabilities as eval_capabilities,
    error::{Result, RuntimeError},
};
use skiff_runtime_linked_program::{
    ExecutableAddr, ServiceDependencyConstraint, ServiceTimeoutConfig,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{ActorRef, RuntimeValue},
};
use skiff_runtime_request::{
    invocation_context_from_request, request_deadline_ms, OutboundRequestRegistry,
    RequestEffectDouble, RequestEnvelope, RequestEvalAdapter, RequestEvalExecutionInputParts,
    RuntimeOperation,
};

mod actor;
mod config;
mod downcast;
mod effects;
mod error;
mod execution;
mod factory;
mod file_stream;
mod http;
mod outbound;
mod request_adapter;
mod request_contexts;
mod websocket;

use actor::{actor, RuntimeOwnedActorParts};
use config::RuntimeConfigCapabilityContext;
use downcast::*;
use effects::{RuntimeEffectDispatchContext, RuntimeTestEffectDoubleContext};
use error::IntoEvalResult;
use execution::RuntimeExecutionControl;
use file_stream::{
    RuntimeFileCapabilitySource, RuntimeOwnedFileSourceStreamContext, RuntimeStreamCancelSignal,
    RuntimeStreamRuntime,
};
use http::{RuntimeHttpClientCapabilityContext, RuntimeTelemetryCapabilityContext};
use outbound::RuntimeOutboundServiceContext;
use websocket::RuntimeWebsocketCapabilityContext;

pub(crate) use error::root_error_into_eval;
pub use factory::{
    actor_from_request, config_context, db_context, effects, execution_control, file_source,
    outbound, runtime_factory, websocket, websocket_from_request,
};
pub(crate) use request_adapter::{
    attach_request_error_diagnostic_frame, request_eval_adapter, RuntimeRequestEvalAdapterInput,
};
pub use request_contexts::{
    effect_dispatch_context_from_request, outbound_service_context_from_request,
};
pub use websocket::RuntimeOwnedWebsocketParts;
