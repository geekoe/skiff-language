//! Eval capability assembly plus compatibility re-exports.
//!
//! Shared request capability contracts live in `skiff-runtime-capability-context`.
//! Eval keeps its request construction, native bridge, and compatibility exports here.

use std::{any::Any, collections::HashMap, future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_boundary::file::{FileCreateOptions, ImmutableFileRef};
use skiff_runtime_capability_context::{
    CancellationToken, ConnectionRequestTerminal, OutboundRequestLease, RequestEffectDoubleControl,
    WebsocketCapabilityContext as SharedWebsocketCapabilityContext,
};
use skiff_runtime_linked_program::ServiceDependencyConstraint;
use skiff_runtime_model::{
    addr::ExecutableAddr,
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{ActorRef, RuntimeValue},
    type_plan::RuntimeTypePlan,
};
use skiff_runtime_native::{
    capability::{
        NativeActorCapability, NativeCapabilityFuture, NativeConfigCapability,
        NativeFileCapability, NativeFileCapabilityBundle, NativeFileChunkSource,
        NativeFileSourceStreamCapability, NativeHttpClientCapability,
        NativeHttpResponseStreamCapability, NativeTelemetryCapability, NativeTimeCapability,
        NativeWebsocketCapability,
    },
    error::{BudgetReason as NativeBudgetReason, RuntimeError as NativeRuntimeError},
};
use skiff_runtime_native_contract::NativeRequiredContext;

use super::{error::RuntimeError, program_execution::ProgramExecutionInput};
use crate::error::{eval_error_to_native, Result};

pub type EvalCapabilityFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait WebsocketRequestCapabilityApi: Send + Sync {
    fn request_json_to_connection<'a>(
        &'a self,
        connection_id: String,
        method: String,
        payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> EvalCapabilityFuture<'a, ConnectionRequestTerminal>;
}

struct UnsupportedWebsocketRequestCapability;

impl WebsocketRequestCapabilityApi for UnsupportedWebsocketRequestCapability {
    fn request_json_to_connection<'a>(
        &'a self,
        _connection_id: String,
        _method: String,
        _payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> EvalCapabilityFuture<'a, ConnectionRequestTerminal> {
        Box::pin(async {
            Err(RuntimeError::Unsupported(
                "std.websocket.requestJsonToConnection execution is not attached".to_string(),
            ))
        })
    }
}

#[derive(Clone)]
pub struct WebsocketCapabilityContext<'a> {
    shared: SharedWebsocketCapabilityContext<'a>,
    request: Arc<dyn WebsocketRequestCapabilityApi>,
}

impl<'a> WebsocketCapabilityContext<'a> {
    pub fn new<T>(inner: T) -> Self
    where
        T: skiff_runtime_capability_context::WebsocketCapabilityApi + 'a,
    {
        Self {
            shared: SharedWebsocketCapabilityContext::new(inner),
            request: Arc::new(UnsupportedWebsocketRequestCapability),
        }
    }

    pub fn with_request_api<T, R>(inner: T, request: R) -> Self
    where
        T: skiff_runtime_capability_context::WebsocketCapabilityApi + 'a,
        R: WebsocketRequestCapabilityApi + 'static,
    {
        Self {
            shared: SharedWebsocketCapabilityContext::new(inner),
            request: Arc::new(request),
        }
    }

    pub fn owned(&self) -> OwnedWebsocketCapabilityContext {
        WebsocketCapabilityContext {
            shared: self.shared.owned(),
            request: Arc::clone(&self.request),
        }
    }

    pub fn borrow(&self) -> WebsocketCapabilityContext<'_> {
        WebsocketCapabilityContext {
            shared: self.shared.borrow(),
            request: Arc::clone(&self.request),
        }
    }

    pub fn service_id(&self) -> &str {
        self.shared.service_id()
    }

    pub fn websocket_entry_id(&self) -> Option<&str> {
        self.shared.websocket_entry_id()
    }

    pub fn request_json_to_connection<'b>(
        &'b self,
        connection_id: String,
        method: String,
        payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> EvalCapabilityFuture<'b, ConnectionRequestTerminal> {
        self.request
            .request_json_to_connection(connection_id, method, payload, execution_control)
    }

    pub fn send_connection_text_to_business_identity(
        &self,
        business_identity: String,
        text: String,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        self.shared
            .send_connection_text_to_business_identity(business_identity, text)
    }

    pub fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        self.shared
            .send_connection_binary_to_business_identity(business_identity, payload)
    }

    pub fn send_connection_text_to_connection(
        &self,
        connection_id: String,
        text: String,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        self.shared
            .send_connection_text_to_connection(connection_id, text)
    }

    pub fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        self.shared
            .send_connection_binary_to_connection(connection_id, payload)
    }
}

pub type OwnedWebsocketCapabilityContext = WebsocketCapabilityContext<'static>;

type WebsocketCapabilityRebind =
    dyn for<'a> Fn(&'a str, Option<&'a str>) -> OwnedWebsocketCapabilityContext + Send + Sync;

/// Rebuilds the WebSocket native capability when execution crosses into a provider activation.
///
/// The returned context is owned so provider unary and stream execution can retain it for their
/// full lifetime without borrowing caller-local routing data.
#[derive(Clone)]
pub struct WebsocketCapabilityRebinder {
    rebind: Arc<WebsocketCapabilityRebind>,
}

impl WebsocketCapabilityRebinder {
    pub fn new<F>(rebind: F) -> Self
    where
        F: for<'a> Fn(&'a str, Option<&'a str>) -> OwnedWebsocketCapabilityContext
            + Send
            + Sync
            + 'static,
    {
        Self {
            rebind: Arc::new(rebind),
        }
    }

    pub fn for_activation(
        &self,
        service_id: &str,
        websocket_entry_id: Option<&str>,
    ) -> OwnedWebsocketCapabilityContext {
        (self.rebind)(service_id, websocket_entry_id)
    }
}

pub use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorCapabilityContext, ActorClient,
    ActorFindControlRequest, ActorGetOrCreateControlRequest, ActorRemoveControlRequest,
    ActorReplaceControlRequest, CapabilityError, CapabilityFuture, CapabilityResult,
    ConfigCapabilityApi, ConfigCapabilityContext, DbCapabilityContext, DbCapabilityContextApi,
    DbCapabilityError, DbCapabilityFuture, DbCapabilityLeaseHandle, DbCapabilityLeaseHold,
    DbCapabilityLeaseHoldHandle, DbCapabilityResult, DbCapabilityStore, DbCapabilityStoreApi,
    DbRecoverableRuntimeContext, DbRecoverableRuntimeExpectedPlans, DbRuntimeChange,
    DbRuntimeSetOp, ExecutionControl, ExecutionControlApi, FileCapabilityApi,
    FileCapabilityContext, FileCapabilityError, FileCapabilityFuture, FileCapabilityRecord,
    FileCapabilityResult, FileCapabilitySource, FileCapabilitySourceApi, FileChunkSource,
    FileSourceStreamApi, FileSourceStreamContext, HttpCapabilityFuture, HttpClientCapabilityApi,
    HttpClientCapabilityContext, HttpResponseStreamCapabilityContext, HttpRuntimeOptions,
    OutboundServiceRequestStart, OutboundStartedRequest, OwnedActorCapabilityContext,
    OwnedConfigCapabilityContext, OwnedExecutionControl, OwnedExecutionControlApi,
    RestrictedServiceDiagnostic, RestrictedServiceDiagnosticCauseKind,
    RestrictedServiceDiagnosticOwner, RestrictedServiceDiagnosticSink, SpawnSubmitControlRequest,
    StreamCancelSignal, StreamCancelSignalApi, StreamCapabilityContext, StreamConsumerCleanup,
    StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi, StreamRuntimeError,
    StreamRuntimeOwner, StreamSink, StreamSinkApi, SupervisedStreamConsumptionChild,
    TelemetryCapabilityApi, TelemetryCapabilityContext, TimeCapabilityContext, TypedStreamSink,
    WebsocketCapabilityApi, HTTP_REQUEST_ADMIN_OVERRIDE_ENV,
};

pub trait EvalRuntimeFactoryApi: Send + Sync {
    fn stream_runtime(&self) -> StreamRuntime;

    fn reusable_test_effect_doubles(
        &self,
        doubles: HashMap<String, TestEffectDouble>,
        stream_runtime: &StreamRuntime,
        test_effects_enabled: bool,
    ) -> TestEffectDoubleContext;

    fn one_shot_test_effect_double_sequences(
        &self,
        doubles: HashMap<String, Vec<TestEffectDouble>>,
        stream_runtime: &StreamRuntime,
        test_effects_enabled: bool,
    ) -> TestEffectDoubleContext;
}

#[derive(Clone)]
pub struct EvalRuntimeFactory {
    inner: Arc<dyn EvalRuntimeFactoryApi>,
}

impl EvalRuntimeFactory {
    pub fn new<T>(inner: T) -> Self
    where
        T: EvalRuntimeFactoryApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn stream_runtime(&self) -> StreamRuntime {
        self.inner.stream_runtime()
    }

    pub fn reusable_test_effect_doubles(
        &self,
        doubles: HashMap<String, TestEffectDouble>,
        stream_runtime: &StreamRuntime,
        test_effects_enabled: bool,
    ) -> TestEffectDoubleContext {
        self.inner
            .reusable_test_effect_doubles(doubles, stream_runtime, test_effects_enabled)
    }

    pub fn one_shot_test_effect_double_sequences(
        &self,
        doubles: HashMap<String, Vec<TestEffectDouble>>,
        stream_runtime: &StreamRuntime,
        test_effects_enabled: bool,
    ) -> TestEffectDoubleContext {
        self.inner.one_shot_test_effect_double_sequences(
            doubles,
            stream_runtime,
            test_effects_enabled,
        )
    }
}

pub trait EffectDispatchApi: Send + Sync {
    fn telemetry_context(&self) -> TelemetryCapabilityContext;
    fn http_client_context(
        &self,
        http_options: HttpRuntimeOptions,
        stream_runtime: StreamRuntime,
        test_effect_doubles: TestEffectDoubleContext,
    ) -> HttpClientCapabilityContext;
}

#[derive(Clone)]
pub struct EffectDispatchContext {
    inner: Arc<dyn EffectDispatchApi>,
}

impl EffectDispatchContext {
    pub fn new<T>(inner: T) -> Self
    where
        T: EffectDispatchApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn telemetry_context(&self) -> TelemetryCapabilityContext {
        self.inner.telemetry_context()
    }

    pub fn http_client_context(
        &self,
        http_options: HttpRuntimeOptions,
        stream_runtime: StreamRuntime,
        test_effect_doubles: TestEffectDoubleContext,
    ) -> HttpClientCapabilityContext {
        self.inner
            .http_client_context(http_options, stream_runtime, test_effect_doubles)
    }
}

#[derive(Clone, Debug)]
pub struct TestEffectDouble {
    pub expect_request: Option<Value>,
    pub response: Value,
}

pub trait TestEffectDoubleContextApi: Any + Send + Sync {
    fn ensure_fully_consumed(&self) -> Result<()> {
        Ok(())
    }

    fn finalize(&self) -> Result<()> {
        self.ensure_fully_consumed()
    }

    fn next_test_effect_double(&self, target: &str) -> Option<TestEffectDouble>;
    fn dispatch_test_effect_double(
        &self,
        target: &str,
        input: Option<&Value>,
    ) -> Option<Result<Value>>;
    fn dispatch_test_stable_target_double(
        &self,
        target: &str,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>>;
    fn dispatch_test_host_operation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>>;
    fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>>;
}

#[derive(Clone)]
pub struct TestEffectDoubleContext {
    inner: Arc<dyn TestEffectDoubleContextApi>,
}

impl TestEffectDoubleContext {
    pub fn new<T>(inner: T) -> Self
    where
        T: TestEffectDoubleContextApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn next_test_effect_double(&self, target: &str) -> Option<TestEffectDouble> {
        self.inner.next_test_effect_double(target)
    }

    pub fn ensure_fully_consumed(&self) -> Result<()> {
        self.inner.ensure_fully_consumed()
    }

    pub fn finalize(&self) -> Result<()> {
        self.inner.finalize()
    }

    pub fn dispatch_test_effect_double(
        &self,
        target: &str,
        input: Option<&Value>,
    ) -> Option<Result<Value>> {
        self.inner.dispatch_test_effect_double(target, input)
    }

    pub fn dispatch_test_stable_target_double(
        &self,
        target: &str,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.inner
            .dispatch_test_stable_target_double(target, return_plan, heap)
    }

    pub fn dispatch_test_host_operation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.inner
            .dispatch_test_host_operation_double(target, input, arg_plan, return_plan, heap)
    }

    pub fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.inner.dispatch_test_http_effect_invocation_double(
            target,
            input,
            arg_plan,
            return_plan,
            heap,
        )
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        let any = self.inner.as_ref() as &dyn Any;
        any.downcast_ref()
    }
}

pub trait OutboundServiceApi: Send + Sync {
    fn service_dependencies(&self) -> &[ServiceDependencyConstraint];
    fn test_effects_enabled(&self) -> bool;
    fn test_effect_doubles(&self) -> HashMap<String, Vec<RequestEffectDoubleControl>>;
    fn request_heap(&self) -> RequestHeap;
    fn effective_timeout_ms(&self, operation_timeout_ms: Option<u64>) -> Option<u64>;
    fn outbound_deadline_error(&self) -> RuntimeError;
    fn start_request(
        &self,
        start: OutboundServiceRequestStart,
        payload: Vec<u8>,
    ) -> Result<OutboundStartedRequest>;
    #[cfg(any(test, feature = "test-support"))]
    fn request_start_control_for_test(
        &self,
        _start: OutboundServiceRequestStart,
        _request_id: String,
    ) -> skiff_runtime_capability_context::RequestStartControl {
        panic!("request_start_control_for_test is only implemented by test adapters")
    }
    fn receive_response<'a>(
        &'a self,
        lease: &'a OutboundRequestLease,
        target: &'a str,
        receiver: &'a mut skiff_runtime_capability_context::OutboundResponseReceiver,
        timeout_ms: Option<u64>,
    ) -> EvalCapabilityFuture<'a, skiff_runtime_capability_context::OutboundResponse>;
    fn cancel_signal(&self) -> CancellationToken;
}

#[derive(Clone)]
pub struct OutboundServiceContext {
    inner: Arc<dyn OutboundServiceApi>,
}

pub type ServiceDispatchContext = OutboundServiceContext;

impl OutboundServiceContext {
    pub fn new<T>(inner: T) -> Self
    where
        T: OutboundServiceApi + 'static,
    {
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn service_dependencies(&self) -> &[ServiceDependencyConstraint] {
        self.inner.service_dependencies()
    }

    pub fn test_effects_enabled(&self) -> bool {
        self.inner.test_effects_enabled()
    }

    pub fn test_effect_doubles(&self) -> HashMap<String, Vec<RequestEffectDoubleControl>> {
        self.inner.test_effect_doubles()
    }

    pub fn request_heap(&self) -> RequestHeap {
        self.inner.request_heap()
    }

    pub fn effective_timeout_ms(&self, operation_timeout_ms: Option<u64>) -> Option<u64> {
        self.inner.effective_timeout_ms(operation_timeout_ms)
    }

    pub fn outbound_deadline_error(&self) -> RuntimeError {
        self.inner.outbound_deadline_error()
    }

    pub fn start_request(
        &self,
        start: OutboundServiceRequestStart,
        payload: Vec<u8>,
    ) -> Result<OutboundStartedRequest> {
        self.inner.start_request(start, payload)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn request_start_control_for_test(
        &self,
        start: OutboundServiceRequestStart,
        request_id: String,
    ) -> skiff_runtime_capability_context::RequestStartControl {
        self.inner.request_start_control_for_test(start, request_id)
    }

    pub async fn receive_response(
        &self,
        lease: &OutboundRequestLease,
        target: &str,
        receiver: &mut skiff_runtime_capability_context::OutboundResponseReceiver,
        timeout_ms: Option<u64>,
    ) -> Result<skiff_runtime_capability_context::OutboundResponse> {
        self.inner
            .receive_response(lease, target, receiver, timeout_ms)
            .await
    }

    pub fn cancel_signal(&self) -> CancellationToken {
        self.inner.cancel_signal()
    }
}

#[derive(Clone)]
pub struct RuntimeNativeFileCapabilityContext<'execution>(
    skiff_runtime_capability_context::NativeFileCapabilityContext<
        RuntimeNativeFileCapability,
        RuntimeNativeFileSourceStreamCapability<'execution>,
        RequestHeapLimits,
    >,
);

#[derive(Clone)]
pub struct RuntimeNativeFileCapability(
    FileCapabilityContext,
    RuntimeNativeInvocationExecutionControl,
);

#[derive(Clone)]
pub struct RuntimeNativeFileSourceStreamCapability<'execution> {
    context: FileSourceStreamContext<'execution>,
    supervision: Option<SupervisedStreamConsumptionChild>,
    invocation_execution: RuntimeNativeInvocationExecutionControl,
}

#[derive(Clone)]
pub struct RuntimeNativeConfigCapabilityContext<'execution>(ConfigCapabilityContext<'execution>);

#[derive(Clone)]
pub(crate) struct RuntimeNativeInvocationExecutionControl {
    execution: Arc<OwnedExecutionControl>,
}

impl RuntimeNativeInvocationExecutionControl {
    pub(crate) fn new(execution: OwnedExecutionControl) -> Self {
        Self {
            execution: Arc::new(execution),
        }
    }

    pub(crate) fn execution_control(&self) -> &OwnedExecutionControl {
        self.execution.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn is_same_invocation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.execution, &other.execution)
    }
}

impl RuntimeNativeFileCapability {
    pub(crate) fn invocation_execution(&self) -> &RuntimeNativeInvocationExecutionControl {
        &self.1
    }
}

impl RuntimeNativeFileSourceStreamCapability<'_> {
    pub(crate) fn invocation_execution(&self) -> &RuntimeNativeInvocationExecutionControl {
        &self.invocation_execution
    }
}

impl<'execution> RuntimeNativeFileCapabilityContext<'execution> {
    pub(crate) fn new(
        file_context: FileCapabilityContext,
        file_source_stream_context: FileSourceStreamContext<'execution>,
        request_heap_limits: RequestHeapLimits,
        invocation_execution: RuntimeNativeInvocationExecutionControl,
    ) -> Self {
        Self::new_with_stream_supervision(
            file_context,
            file_source_stream_context,
            request_heap_limits,
            None,
            invocation_execution,
        )
    }

    pub(crate) fn new_supervised(
        file_context: FileCapabilityContext,
        file_source_stream_context: FileSourceStreamContext<'execution>,
        request_heap_limits: RequestHeapLimits,
        supervision: SupervisedStreamConsumptionChild,
        invocation_execution: RuntimeNativeInvocationExecutionControl,
    ) -> Self {
        Self::new_with_stream_supervision(
            file_context,
            file_source_stream_context,
            request_heap_limits,
            Some(supervision),
            invocation_execution,
        )
    }

    fn new_with_stream_supervision(
        file_context: FileCapabilityContext,
        file_source_stream_context: FileSourceStreamContext<'execution>,
        request_heap_limits: RequestHeapLimits,
        supervision: Option<SupervisedStreamConsumptionChild>,
        invocation_execution: RuntimeNativeInvocationExecutionControl,
    ) -> Self {
        Self(
            skiff_runtime_capability_context::NativeFileCapabilityContext::new(
                RuntimeNativeFileCapability(file_context, invocation_execution.clone()),
                RuntimeNativeFileSourceStreamCapability {
                    context: file_source_stream_context,
                    supervision,
                    invocation_execution,
                },
                request_heap_limits,
            ),
        )
    }
}

impl<'execution> RuntimeNativeConfigCapabilityContext<'execution> {
    pub fn new(context: ConfigCapabilityContext<'execution>) -> Self {
        Self(context)
    }
}

#[derive(Clone)]
pub struct RuntimeNativeActorCapabilityContext<'execution>(
    ActorCapabilityContext<'execution>,
    RuntimeNativeInvocationExecutionControl,
);

impl<'execution> RuntimeNativeActorCapabilityContext<'execution> {
    pub(crate) fn new(
        context: ActorCapabilityContext<'execution>,
        invocation_execution: RuntimeNativeInvocationExecutionControl,
    ) -> Self {
        Self(context, invocation_execution)
    }

    pub(crate) fn invocation_execution(&self) -> &RuntimeNativeInvocationExecutionControl {
        &self.1
    }
}

#[derive(Clone)]
pub struct RuntimeNativeTimeCapabilityContext<'execution>(
    TimeCapabilityContext<'execution>,
    RuntimeNativeInvocationExecutionControl,
);

impl<'execution> RuntimeNativeTimeCapabilityContext<'execution> {
    pub(crate) fn new(
        context: TimeCapabilityContext<'execution>,
        invocation_execution: RuntimeNativeInvocationExecutionControl,
    ) -> Self {
        Self(context, invocation_execution)
    }

    pub(crate) fn invocation_execution(&self) -> &RuntimeNativeInvocationExecutionControl {
        &self.1
    }
}

#[derive(Clone)]
pub struct RuntimeNativeHttpClientCapabilityContext {
    context: skiff_runtime_capability_context::NativeHttpClientCapabilityContext<
        HttpClientCapabilityContext,
    >,
    test_effect_doubles: TestEffectDoubleContext,
    invocation_execution: RuntimeNativeInvocationExecutionControl,
}

impl RuntimeNativeHttpClientCapabilityContext {
    pub(crate) fn new(
        context: HttpClientCapabilityContext,
        test_effect_doubles: TestEffectDoubleContext,
        invocation_execution: RuntimeNativeInvocationExecutionControl,
    ) -> Self {
        Self {
            context: skiff_runtime_capability_context::NativeHttpClientCapabilityContext::new(
                context,
            ),
            test_effect_doubles,
            invocation_execution,
        }
    }

    pub(crate) fn invocation_execution(&self) -> &RuntimeNativeInvocationExecutionControl {
        &self.invocation_execution
    }
}

#[derive(Clone)]
pub struct RuntimeNativeHttpResponseStreamCapabilityContext<'execution>(
    skiff_runtime_capability_context::NativeHttpResponseStreamCapabilityContext<
        HttpResponseStreamCapabilityContext<'execution>,
    >,
    RuntimeNativeInvocationExecutionControl,
);

impl<'execution> RuntimeNativeHttpResponseStreamCapabilityContext<'execution> {
    pub(crate) fn new(
        context: HttpResponseStreamCapabilityContext<'execution>,
        invocation_execution: RuntimeNativeInvocationExecutionControl,
    ) -> Self {
        Self(
            skiff_runtime_capability_context::NativeHttpResponseStreamCapabilityContext::new(
                context,
            ),
            invocation_execution,
        )
    }

    pub(crate) fn invocation_execution(&self) -> &RuntimeNativeInvocationExecutionControl {
        &self.1
    }
}

#[derive(Clone)]
pub struct RuntimeNativeWebsocketCapabilityContext<'execution>(
    WebsocketCapabilityContext<'execution>,
    RuntimeNativeInvocationExecutionControl,
);

impl<'execution> RuntimeNativeWebsocketCapabilityContext<'execution> {
    pub(crate) fn new(
        context: WebsocketCapabilityContext<'execution>,
        invocation_execution: RuntimeNativeInvocationExecutionControl,
    ) -> Self {
        Self(context, invocation_execution)
    }

    pub(crate) fn invocation_execution(&self) -> &RuntimeNativeInvocationExecutionControl {
        &self.1
    }
}

#[derive(Clone)]
pub struct RuntimeNativeTelemetryCapabilityContext(
    skiff_runtime_capability_context::NativeTelemetryCapabilityContext<TelemetryCapabilityContext>,
);

impl RuntimeNativeTelemetryCapabilityContext {
    pub fn new(context: TelemetryCapabilityContext) -> Self {
        Self(skiff_runtime_capability_context::NativeTelemetryCapabilityContext::new(context))
    }
}

pub type RuntimeNativeCapabilityContexts<'execution> =
    skiff_runtime_capability_context::NativeCapabilityContexts<
        RuntimeNativeActorCapabilityContext<'execution>,
        RuntimeNativeFileCapabilityContext<'execution>,
        RuntimeNativeTimeCapabilityContext<'execution>,
        RuntimeNativeHttpClientCapabilityContext,
        RuntimeNativeHttpResponseStreamCapabilityContext<'execution>,
        RuntimeNativeWebsocketCapabilityContext<'execution>,
        RuntimeNativeTelemetryCapabilityContext,
        (),
    >;

pub struct EvalRequestExecutionCapabilities<'a> {
    execution: ExecutionControl<'a>,
    config: ConfigCapabilityContext<'a>,
    db: DbCapabilityContext,
    file_source: FileCapabilitySource,
    websocket: WebsocketCapabilityContext<'a>,
    effects: EffectDispatchContext,
    runtime_activation: Arc<RuntimeActivation>,
    actor: ActorCapabilityContext<'a>,
    spawn: ActorCapabilityContext<'a>,
    outbound: OutboundServiceContext,
}

pub struct EvalRequestProgramExecutionInput {
    pub stream_runtime: StreamRuntime,
    pub http_options: HttpRuntimeOptions,
    pub test_effect_doubles: TestEffectDoubleContext,
    pub request_heap_limits: RequestHeapLimits,
}

impl<'a> EvalRequestExecutionCapabilities<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        execution: ExecutionControl<'a>,
        config: ConfigCapabilityContext<'a>,
        db: DbCapabilityContext,
        file_source: FileCapabilitySource,
        websocket: WebsocketCapabilityContext<'a>,
        effects: EffectDispatchContext,
        runtime_activation: Arc<RuntimeActivation>,
        actor: ActorCapabilityContext<'a>,
        spawn: ActorCapabilityContext<'a>,
        outbound: OutboundServiceContext,
    ) -> Self {
        Self {
            execution,
            config,
            db,
            file_source,
            websocket,
            effects,
            runtime_activation,
            actor,
            spawn,
            outbound,
        }
    }

    pub fn into_program_execution_input(
        self,
        input: EvalRequestProgramExecutionInput,
    ) -> ProgramExecutionInput<'a> {
        let Self {
            execution,
            config,
            db,
            file_source,
            websocket,
            effects,
            runtime_activation,
            actor,
            spawn,
            outbound,
        } = self;
        let EvalRequestProgramExecutionInput {
            stream_runtime,
            http_options,
            test_effect_doubles,
            request_heap_limits,
        } = input;
        let file = file_source.context_for_request(db.clone());
        let file_source_stream =
            FileSourceStreamContext::new(stream_runtime.clone(), execution.clone());
        let http_client =
            effects.http_client_context(http_options, stream_runtime, test_effect_doubles.clone());
        ProgramExecutionInput {
            execution: execution.clone(),
            config,
            db,
            file,
            file_source_stream,
            time: TimeCapabilityContext::new(execution),
            websocket,
            effects: effects.clone(),
            http_client,
            test_effect_doubles,
            runtime_activation,
            actor,
            spawn,
            outbound,
            request_heap_limits,
        }
    }
}

impl NativeConfigCapability for RuntimeNativeConfigCapabilityContext<'_> {
    fn read_config_target(
        &self,
        current_addr: &ExecutableAddr,
        target: &str,
        args: &[Value],
        type_arg: Option<&RuntimeTypePlan>,
    ) -> skiff_runtime_native::error::Result<Value> {
        ConfigCapabilityContext::read_config_target(&self.0, current_addr, target, args, type_arg)
            .map_err(NativeRuntimeError::from)
    }
}

impl NativeActorCapability for RuntimeNativeActorCapabilityContext<'_> {
    fn service_id(&self) -> &str {
        self.0.service_id()
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        self.0.activation_identity()
    }

    fn get_or_create_actor<'a>(
        &'a self,
        request: ActorGetOrCreateControlRequest,
        bootstrap_payload: Vec<u8>,
    ) -> NativeCapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            ActorClient::new(self.0.clone())
                .get_or_create_actor(
                    request,
                    bootstrap_payload,
                    self.1.execution_control().clone(),
                )
                .await
                .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
        })
    }

    fn replace_actor<'a>(
        &'a self,
        request: ActorReplaceControlRequest,
        bootstrap_payload: Vec<u8>,
    ) -> NativeCapabilityFuture<'a, ActorRef> {
        Box::pin(async move {
            ActorClient::new(self.0.clone())
                .replace_actor(
                    request,
                    bootstrap_payload,
                    self.1.execution_control().clone(),
                )
                .await
                .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
        })
    }

    fn find_actor<'a>(
        &'a self,
        request: ActorFindControlRequest,
    ) -> NativeCapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async move {
            ActorClient::new(self.0.clone())
                .find_actor(request, self.1.execution_control().clone())
                .await
                .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
        })
    }

    fn remove_actor<'a>(
        &'a self,
        request: ActorRemoveControlRequest,
    ) -> NativeCapabilityFuture<'a, bool> {
        Box::pin(async move {
            ActorClient::new(self.0.clone())
                .remove_actor(request, self.1.execution_control().clone())
                .await
                .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
        })
    }
}

impl<'execution> NativeFileCapabilityBundle for RuntimeNativeFileCapabilityContext<'execution> {
    type File = RuntimeNativeFileCapability;
    type FileSourceStream = RuntimeNativeFileSourceStreamCapability<'execution>;

    fn into_native_file_parts(self) -> (Self::File, Self::FileSourceStream, RequestHeapLimits) {
        self.0.into_parts()
    }
}

impl NativeFileCapability for RuntimeNativeFileCapability {
    fn create_file<'a>(
        &'a self,
        target: &'a str,
        input: Bytes,
        options: FileCreateOptions,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::create_file(
                &self.0,
                target,
                input,
                options,
                self.1.execution_control().clone(),
            )
            .await
            .map_err(NativeRuntimeError::from)
        })
    }

    fn read_file_wire<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::read_file_wire(
                &self.0,
                target,
                file,
                self.1.execution_control().clone(),
            )
            .await
            .map_err(NativeRuntimeError::from)
        })
    }

    fn read_text_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::read_text_file(
                &self.0,
                target,
                file,
                self.1.execution_control().clone(),
            )
            .await
            .map_err(NativeRuntimeError::from)
        })
    }

    fn file_info<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::file_info(
                &self.0,
                target,
                file,
                self.1.execution_control().clone(),
            )
            .await
            .map_err(NativeRuntimeError::from)
        })
    }

    fn delete_file<'a>(
        &'a self,
        target: &'a str,
        file: &'a ImmutableFileRef,
    ) -> NativeCapabilityFuture<'a, ()> {
        Box::pin(async move {
            FileCapabilityContext::delete_file(
                &self.0,
                target,
                file,
                self.1.execution_control().clone(),
            )
            .await
            .map_err(NativeRuntimeError::from)
        })
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        target: &'a str,
        options: FileCreateOptions,
        next_chunk: NativeFileChunkSource<'a>,
    ) -> NativeCapabilityFuture<'a, Value> {
        Box::pin(async move {
            FileCapabilityContext::create_file_from_chunks(
                &self.0,
                target,
                options,
                next_chunk,
                self.1.execution_control().clone(),
            )
            .await
            .map_err(NativeRuntimeError::from)
        })
    }
}

impl NativeFileSourceStreamCapability for RuntimeNativeFileSourceStreamCapability<'_> {
    fn stream_consumer_cleanup(&self, stream: &Value) -> StreamConsumerCleanup {
        match &self.supervision {
            Some(supervision) => supervision.consumer_cleanup(stream),
            None => StreamConsumerCleanup::new(self.context.stream_runtime_handle(), stream),
        }
    }

    fn next_file_source_stream_item<'a>(
        &'a self,
        stream: &'a Value,
    ) -> FileCapabilityFuture<'a, Option<Value>> {
        Box::pin(async move {
            let result = self
                .context
                .next_file_source_stream_item(
                    stream,
                    self.invocation_execution.execution_control().clone(),
                )
                .await;
            if let Some(supervision) = &self.supervision {
                match &result {
                    Ok(None) => {
                        supervision.observe_end(stream);
                    }
                    Err(FileCapabilityError::Stream(StreamRuntimeError::Producer(_))) => {
                        supervision.observe_producer_error(stream);
                    }
                    Ok(Some(_)) | Err(_) => {}
                }
            }
            result
        })
    }
}

impl NativeTimeCapability for RuntimeNativeTimeCapabilityContext<'_> {
    fn execution_control(&self) -> OwnedExecutionControl {
        self.1.execution_control().clone()
    }

    fn poll_execution_budget(&self) -> skiff_runtime_native::error::Result<()> {
        self.0
            .execution_control()
            .poll_execution_budget()
            .map_err(execution_control_error_to_native)
    }
}

impl NativeHttpClientCapability for RuntimeNativeHttpClientCapabilityContext {
    fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<skiff_runtime_native::error::Result<RuntimeValue>> {
        self.test_effect_doubles
            .dispatch_test_http_effect_invocation_double(target, input, arg_plan, return_plan, heap)
            .map(|result| result.map_err(eval_error_to_native))
    }

    fn dispatch_http_request<'a>(&'a self, input: &'a Value) -> NativeCapabilityFuture<'a, Value> {
        let context = self.context.clone().into_effect_context();
        let execution_control = self.invocation_execution.execution_control().clone();
        Box::pin(async move {
            context
                .dispatch_http_request(input, execution_control)
                .await
                .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
        })
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        input: &'a Value,
        expected_body_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value> {
        let context = self.context.clone().into_effect_context();
        let execution_control = self.invocation_execution.execution_control().clone();
        Box::pin(async move {
            context
                .dispatch_http_stream(input, expected_body_item_type, execution_control)
                .await
                .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
        })
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        input: &'a Value,
        expected_item_type: Option<&'a RuntimeTypePlan>,
    ) -> NativeCapabilityFuture<'a, Value> {
        let context = self.context.clone().into_effect_context();
        let execution_control = self.invocation_execution.execution_control().clone();
        Box::pin(async move {
            context
                .dispatch_http_sse(input, expected_item_type, execution_control)
                .await
                .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
        })
    }
}

impl NativeHttpResponseStreamCapability for RuntimeNativeHttpResponseStreamCapabilityContext<'_> {
    fn response_item_type(
        &self,
        target: &str,
    ) -> skiff_runtime_native::error::Result<RuntimeTypePlan> {
        self.0
            .clone()
            .into_execution_context()
            .response_item_type(target)
            .cloned()
            .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
    }

    fn send_response_event<'a>(
        &'a self,
        target: &'a str,
        event: Value,
    ) -> NativeCapabilityFuture<'a, ()> {
        let context = self.0.clone().into_execution_context();
        Box::pin(async move {
            context
                .send_response_event(target, event)
                .await
                .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
        })
    }
}

impl NativeWebsocketCapability for RuntimeNativeWebsocketCapabilityContext<'_> {
    fn request_json_to_connection<'a>(
        &'a self,
        connection_id: String,
        method: String,
        payload: Vec<u8>,
    ) -> NativeCapabilityFuture<'a, skiff_runtime_capability_context::ConnectionRequestTerminal>
    {
        Box::pin(async move {
            self.0
                .request_json_to_connection(
                    connection_id,
                    method,
                    payload,
                    self.1.execution_control().clone(),
                )
                .await
                .map_err(eval_error_to_native)
        })
    }

    fn send_connection_text_to_business_identity(
        &self,
        business_identity: String,
        text: String,
    ) -> skiff_runtime_native::error::Result<()> {
        self.0
            .send_connection_text_to_business_identity(business_identity, text)
            .map_err(NativeRuntimeError::from)
    }

    fn send_connection_binary_to_business_identity(
        &self,
        business_identity: String,
        payload: Vec<u8>,
    ) -> skiff_runtime_native::error::Result<()> {
        self.0
            .send_connection_binary_to_business_identity(business_identity, payload)
            .map_err(NativeRuntimeError::from)
    }

    fn send_connection_text_to_connection(
        &self,
        connection_id: String,
        text: String,
    ) -> skiff_runtime_native::error::Result<()> {
        self.0
            .send_connection_text_to_connection(connection_id, text)
            .map_err(NativeRuntimeError::from)
    }

    fn send_connection_binary_to_connection(
        &self,
        connection_id: String,
        payload: Vec<u8>,
    ) -> skiff_runtime_native::error::Result<()> {
        self.0
            .send_connection_binary_to_connection(connection_id, payload)
            .map_err(NativeRuntimeError::from)
    }
}

impl NativeTelemetryCapability for RuntimeNativeTelemetryCapabilityContext {
    fn emit_native(
        &self,
        target: &str,
        args: &[Value],
    ) -> skiff_runtime_native::error::Result<Value> {
        self.0
            .clone()
            .into_effect_context()
            .emit_native(target, args)
            .map_err(|error| eval_error_to_native(RuntimeError::from(error)))
    }
}

pub fn native_required_context_none() -> NativeRequiredContext {
    NativeRequiredContext::None
}

fn execution_control_error_to_native(
    error: skiff_runtime_capability_context::ExecutionControlError,
) -> NativeRuntimeError {
    match error {
        skiff_runtime_capability_context::ExecutionControlError::Cancelled => {
            NativeRuntimeError::Cancelled
        }
        skiff_runtime_capability_context::ExecutionControlError::BudgetExceeded(failure) => {
            if failure.reason == skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled
            {
                NativeRuntimeError::Cancelled
            } else {
                NativeRuntimeError::ExecutionBudgetExceeded {
                    reason: native_capability_budget_reason(failure.reason),
                    instruction_count: failure.instruction_count,
                    limit: failure.limit,
                    elapsed_ms: failure.elapsed_ms,
                }
            }
        }
    }
}

fn native_capability_budget_reason(
    reason: skiff_runtime_capability_context::ExecutionBudgetReason,
) -> NativeBudgetReason {
    match reason {
        skiff_runtime_capability_context::ExecutionBudgetReason::Cancelled => {
            NativeBudgetReason::Cancelled
        }
        skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded => {
            NativeBudgetReason::DeadlineExceeded
        }
        skiff_runtime_capability_context::ExecutionBudgetReason::InstructionLimitExceeded => {
            NativeBudgetReason::InstructionLimitExceeded
        }
    }
}
