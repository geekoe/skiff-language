//! Runtime-owned adapters for eval capability contracts.

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
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
    ActivationIdentityControl, ActorFindControlRequest, ActorGetOrCreateControlRequest,
    ActorRemoveControlRequest, ActorReplaceControlRequest, CancellationToken,
    ConnectionRequestRegistry, ConnectionRequestSession, ExecutionControlResult,
    FileCapabilityError, FileCapabilityFuture, RuntimeDeadlineControl, SpawnSubmitControlRequest,
    StreamPoll, StreamPullSource, StreamRuntimeError, StreamRuntimeResult,
};
use skiff_runtime_eval::{
    capabilities as eval_capabilities,
    error::{Result, RuntimeError},
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{ActorRef, RuntimeValue},
};
use skiff_runtime_request::{
    invocation_context_from_request, request_deadline_ms, OutboundRequestRegistry, RequestEnvelope,
    RuntimeOperation,
};

mod activation_execution_rebinder;
mod actor;
mod actor_method_adapter;
mod assembly_execution_context;
mod assembly_request_adapter;
mod config;
mod downcast;
mod effects;
mod error;
mod execution;
mod factory;
mod file_stream;
mod http;
mod request_contexts;
mod websocket;

use crate::capability_context::actor_method_outbound::ActorMethodOutboundRegistry;
#[cfg(test)]
pub(crate) use activation_execution_rebinder::provider_execution_facts_for_test;
pub(crate) use activation_execution_rebinder::{
    activation_execution_context_rebinder, RuntimeActivationExecutionContextRebinderInput,
};
use actor::{actor, RuntimeOwnedActorParts};
pub(crate) use actor_method_adapter::{ActorMethodEvalExecution, ActorMethodEvalExecutionInput};
use config::RuntimeConfigCapabilityContext;
pub(crate) use downcast::concrete_stream_runtime;
use downcast::{
    concrete_actor_context_from_owned, concrete_db_context, concrete_stream_cancel_signals,
    concrete_test_double, concrete_test_effect_double_context, eval_test_double,
};
use effects::{RuntimeEffectDispatchContext, RuntimeTestEffectDoubleContext};
use error::{ordinary_root_error_into_capability, root_result_into_capability, IntoEvalResult};
use execution::RuntimeExecutionControl;
use file_stream::{
    RuntimeFileCapabilitySource, RuntimeOwnedFileSourceStreamContext, RuntimeStreamCancelSignal,
    RuntimeStreamRuntime,
};
use http::{RuntimeHttpClientCapabilityContext, RuntimeTelemetryCapabilityContext};
use websocket::{
    RuntimeConnectionRequestParts, RuntimeWebsocketCapabilityContext,
    RuntimeWebsocketRequestCapabilityContext,
};

pub(crate) use assembly_execution_context::RuntimeAssemblyEvalAdapterContextInput;
pub(crate) use assembly_request_adapter::{
    http_gateway_eval_adapter, spawn_eval_adapter, websocket_connect_eval_adapter,
    websocket_jsonrpc_eval_adapter, RuntimeHttpGatewayEvalAdapterInput,
    RuntimeSpawnEvalAdapterInput, RuntimeWebSocketConnectEvalAdapterInput,
    RuntimeWebSocketJsonRpcEvalAdapterInput,
};
pub(crate) use error::root_error_into_eval;
pub(crate) use factory::actor_from_request;
#[cfg(any(test, feature = "test-support"))]
pub use factory::TestActorCapabilityFactory;
pub use factory::{
    config_context, db_context, effects, execution_control, file_source, runtime_factory,
    websocket, websocket_from_request, websocket_from_runtime_request,
};
pub use request_contexts::effect_dispatch_context_from_request;
pub use websocket::RuntimeOwnedWebsocketParts;

#[cfg(test)]
mod tests;
