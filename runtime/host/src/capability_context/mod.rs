//! Runtime host capability implementations.
//!
//! Pure request/control/stream/db contracts live in
//! `skiff_runtime_capability_context`; import them from that crate directly.

mod actor;
mod effect_context;
mod http;
mod native_projection;
mod outbound_service;
mod request_payload;
mod response;
mod store;
mod stream;
mod stream_runtime;
mod telemetry;
mod test_effect_double;
mod time;
mod websocket;

pub(crate) use skiff_runtime_capability_context::RouterWriterMessage;
pub use skiff_runtime_capability_context::{
    DbCapabilityContext, DbCapabilityContextApi, DbCapabilityError, DbCapabilityFactory,
    DbCapabilityFuture, DbCapabilityLeaseHandle, DbCapabilityLeaseHold,
    DbCapabilityLeaseHoldHandle, DbCapabilityResult, DbCapabilitySource, DbCapabilityStore,
    DbCapabilityStoreApi, DbProviderBuildInput, DbProviderConfig, DbProviderFactory,
    DbProviderSource, DbRecoverableRuntimeContext, DbRecoverableRuntimeExpectedPlans,
    DbRuntimeChange, DbRuntimeSetOp, FileCapabilityRecord,
};
pub(crate) use skiff_runtime_native_contract::{
    TARGET_STD_HTTP_REQUEST, TARGET_STD_HTTP_SSE, TARGET_STD_HTTP_STREAM,
};

pub use actor::{ActorCapabilityContext, ActorClient, ActorClientContext};
pub use effect_context::{EffectDispatchContext, HttpEffectContext};
pub use http::HttpClientCapabilityContext;
pub use native_projection::{
    RuntimeNativeFileCapabilityContext, RuntimeNativeHttpClientCapabilityContext,
    RuntimeNativeHttpResponseStreamCapabilityContext, RuntimeNativeTelemetryCapabilityContext,
    RuntimeNativeTimeCapabilityContext,
};
#[allow(unused_imports)]
pub use outbound_service::{
    OutboundCallerDeadline, OutboundServiceContext, OutboundServiceContextInput,
    OutboundServiceRequestStart, OutboundTraceMetadata, ServiceDispatchContext,
};
pub use request_payload::ConfigCapabilityContext;
pub use response::response_error_from_runtime_error;
pub use skiff_runtime_capability_context::HttpRuntimeOptions;
#[cfg(any(test, feature = "test-support"))]
pub use skiff_runtime_capability_context::HTTP_REQUEST_ADMIN_OVERRIDE_ENV;
pub use store::{
    FileCapabilityContext, FileCapabilityRuntime, FileCapabilitySource, FileSourceStreamContext,
    HostCapabilityFuture,
};
pub use stream::{HttpResponseStreamCapabilityContext, StreamCapabilityContext, TypedStreamSink};
pub(crate) use stream_runtime::stream_runtime_streams_active;
pub use stream_runtime::{StreamCancelSignal, StreamRuntime, StreamSink};
pub use telemetry::TelemetryCapabilityContext;
pub use test_effect_double::TestEffectDouble;
pub use test_effect_double::TestEffectDoubleContext;
pub use time::TimeCapabilityContext;
pub use websocket::WebsocketCapabilityContext;
