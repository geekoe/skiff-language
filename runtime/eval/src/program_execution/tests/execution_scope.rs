use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll, Wake, Waker},
    time::{Duration, Instant},
};

use bytes::Bytes;
use skiff_artifact_model::{
    AssemblyIdentity, DeploymentRevision, InstructionSourceSite, SyntheticInstructionSiteReason,
};
use skiff_runtime_boundary::file::FileCreateOptions;
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorKeyControlMetadata, ActorRemoveControlRequest,
    ActorReplaceControlRequest, CancellationSource, CancellationToken, CapabilityError,
    CapabilityFuture, ConnectionRequestTerminal, ExecutionBudgetFailure, ExecutionBudgetReason,
    ExecutionControl, ExecutionControlApi, ExecutionControlError, ExecutionControlResult,
    ExecutionDeadlineSource, ExecutionScope, ExecutionScopeAccessError, ExecutionScopeDeriveError,
    ExecutionScopeTerminal, FileCapabilityApi, FileCapabilityFuture, FileCapabilitySource,
    FileCapabilitySourceApi, FileChunkSource, FileSourceStreamContext, HttpCapabilityFuture,
    HttpClientCapabilityApi, HttpClientCapabilityContext, OwnedActorCapabilityContext,
    OwnedExecutionControl, OwnedExecutionControlApi, OwnedRequestCapabilityContext,
    RequestCapabilityApi, RequestCapabilityContext, TaskCancelControlRequest,
    TaskCancelControlResponse, TaskStatusControlRequest, TaskStatusControlResponse,
    TaskSubmitControlRequest, TaskSubmitResponseControl, StreamRuntime,
    SupervisedStreamConsumptionLease,
};
use skiff_runtime_linked_program::{
    LinkOverlay, PublicationResourceTable, RuntimeTypeContext, ServiceMeta,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{ActorRef, RuntimeValue},
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};
use skiff_runtime_native::capability::{
    NativeActorCapability, NativeFileCapability, NativeFileCapabilityBundle,
    NativeHttpClientCapability, NativeTimeCapability, NativeWebsocketCapability,
};
use skiff_runtime_native::dispatch::{NativeDispatch, PreparedNativeCall, RuntimeNativeInvocation};
use skiff_runtime_native_contract::{NativeBindingKey, NativeCallPlan, NativeRequiredContext};

use super::super::{
    execution_scope::{
        deadline_after_duration_ms, EvalMonotonicClock, ExecutionCheckpoint,
        ExecutionCheckpointKind, ExecutionClock,
    },
    OwnedProgramExecutionContext, ProgramExecutionContext, ProgramExecutionInput,
};
use crate::{
    actor_executor_test_runtime as test_runtime,
    assembly_execution::RuntimeExecutionProjection,
    capabilities::{
        ActorCapabilityContext, FileCapabilityContext, HttpRuntimeOptions,
        RuntimeNativeInvocationExecutionControl, StreamCapabilityContext, TimeCapabilityContext,
        WebsocketCapabilityContext, WebsocketRequestCapabilityApi,
    },
    error::{BudgetReason, RuntimeError},
    native_capability::{
        project_runtime_execution_native_capability_context_supervised,
        project_runtime_native_capability_context,
    },
};

mod evaluator_checkpoint;
mod evaluator_timeout;
mod tail_call_execution;

fn site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

#[derive(Clone)]
struct ScopeAwareControl {
    scope: Option<ExecutionScope>,
    cancellation: CancellationToken,
    cancelled: Arc<AtomicBool>,
    fail_derive: bool,
    budget_error: Option<ExecutionControlError>,
    instruction_units: Arc<AtomicU64>,
}

impl ScopeAwareControl {
    fn available(scope: ExecutionScope, cancellation: CancellationToken) -> Self {
        Self {
            scope: Some(scope),
            cancelled: cancellation.cancel_flag(),
            cancellation,
            fail_derive: false,
            budget_error: None,
            instruction_units: Arc::new(AtomicU64::new(0)),
        }
    }

    fn unavailable() -> Self {
        let cancellation = CancellationToken::new();
        Self {
            scope: None,
            cancelled: cancellation.cancel_flag(),
            cancellation,
            fail_derive: false,
            budget_error: None,
            instruction_units: Arc::new(AtomicU64::new(0)),
        }
    }

    fn with_derive_failure(mut self) -> Self {
        self.fail_derive = true;
        self
    }

    fn with_budget_error(mut self, error: ExecutionControlError) -> Self {
        self.budget_error = Some(error);
        self
    }
}

impl ExecutionControlApi for ScopeAwareControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        self.scope
            .as_ref()
            .and_then(ExecutionScope::effective_deadline)
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        self.scope
            .clone()
            .ok_or(ExecutionScopeAccessError::Unavailable)
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        if self.fail_derive {
            return Err(ExecutionScopeAccessError::Derive(ExecutionScopeDeriveError));
        }
        let scope = ExecutionControlApi::execution_scope(self)?.derive(local_deadline, site)?;
        Ok(OwnedExecutionControl::new(Self {
            scope: Some(scope),
            cancellation: self.cancellation.clone(),
            cancelled: Arc::clone(&self.cancelled),
            fail_derive: false,
            budget_error: self.budget_error,
            instruction_units: Arc::clone(&self.instruction_units),
        }))
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(ExecutionControlError::Cancelled)
        } else {
            Ok(())
        }
    }

    fn add_instruction_units(&self, units: u64) -> ExecutionControlResult<()> {
        self.instruction_units.fetch_add(units, Ordering::Relaxed);
        Ok(())
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        self.budget_error.map_or(Ok(()), Err)
    }

    fn file_source_stream_context(
        &self,
        stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        test_runtime::file_source_stream_context(stream_runtime)
    }
}

impl OwnedExecutionControlApi for ScopeAwareControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<Instant> {
        ExecutionControlApi::deadline(self)
    }

    fn execution_scope(&self) -> Result<ExecutionScope, ExecutionScopeAccessError> {
        ExecutionControlApi::execution_scope(self)
    }

    fn derive_scope(
        &self,
        local_deadline: Instant,
        site: InstructionSourceSite,
    ) -> Result<OwnedExecutionControl, ExecutionScopeAccessError> {
        ExecutionControlApi::derive_scope(self, local_deadline, site)
    }
}

fn context(control: ScopeAwareControl) -> ProgramExecutionContext<'static> {
    context_with_overrides(control, ContextOverrides::default())
}

#[derive(Default)]
struct ContextOverrides {
    http_client: Option<HttpClientCapabilityContext>,
    file: Option<FileCapabilityContext>,
    file_source_stream: Option<FileSourceStreamContext<'static>>,
    websocket: Option<WebsocketCapabilityContext<'static>>,
    actor: Option<ActorCapabilityContext<'static>>,
    request: Option<RequestCapabilityContext<'static>>,
}

fn context_with_overrides(
    control: ScopeAwareControl,
    overrides: ContextOverrides,
) -> ProgramExecutionContext<'static> {
    let execution = ExecutionControl::new(control);
    let runtime_factory = test_runtime::runtime_factory();
    let stream_runtime = runtime_factory.stream_runtime();
    let test_effect_doubles =
        runtime_factory.reusable_test_effect_doubles(HashMap::new(), &stream_runtime, false);
    let effects = test_runtime::effects_context();
    let actor = overrides.actor.unwrap_or_else(test_runtime::actor_context);
    let request = overrides
        .request
        .unwrap_or_else(test_runtime::request_context);
    ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: test_runtime::config_context(),
        db: skiff_runtime_capability_context::DbCapabilityContext::unavailable(),
        file: overrides.file.unwrap_or_else(test_runtime::file_context),
        file_source_stream: overrides
            .file_source_stream
            .unwrap_or_else(|| test_runtime::file_source_stream_context(stream_runtime.clone())),
        time: TimeCapabilityContext::new(execution),
        websocket: overrides
            .websocket
            .unwrap_or_else(test_runtime::websocket_context),
        effects: effects.clone(),
        http_client: overrides.http_client.unwrap_or_else(|| {
            effects.http_client_context(
                HttpRuntimeOptions::explicit(false),
                stream_runtime,
                test_effect_doubles.clone(),
            )
        }),
        test_effect_doubles,
        actor: actor.clone(),
        request,
        request_heap_limits: RequestHeapLimits::default(),
    })
}

#[derive(Clone)]
struct CarrierReceiptHttp {
    receipts: Arc<Mutex<Vec<OwnedExecutionControl>>>,
}

impl HttpClientCapabilityApi for CarrierReceiptHttp {
    fn with_stream_runtime(&self, _stream_runtime: StreamRuntime) -> HttpClientCapabilityContext {
        HttpClientCapabilityContext::new(self.clone())
    }

    fn dispatch_http_request<'a>(
        &'a self,
        _input: &'a serde_json::Value,
        execution_control: OwnedExecutionControl,
    ) -> HttpCapabilityFuture<'a, serde_json::Value> {
        self.receipts
            .lock()
            .expect("HTTP receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }

    fn dispatch_http_stream<'a>(
        &'a self,
        _input: &'a serde_json::Value,
        _expected_body_item_type: Option<&'a RuntimeTypePlan>,
        execution_control: OwnedExecutionControl,
    ) -> HttpCapabilityFuture<'a, serde_json::Value> {
        self.receipts
            .lock()
            .expect("HTTP receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }

    fn dispatch_http_sse<'a>(
        &'a self,
        _input: &'a serde_json::Value,
        _expected_item_type: Option<&'a RuntimeTypePlan>,
        execution_control: OwnedExecutionControl,
    ) -> HttpCapabilityFuture<'a, serde_json::Value> {
        self.receipts
            .lock()
            .expect("HTTP receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }
}

#[derive(Clone)]
struct CarrierReceiptFile {
    receipts: Arc<Mutex<Vec<OwnedExecutionControl>>>,
}

#[derive(Clone)]
struct CarrierReceiptFileSource {
    receipts: Arc<Mutex<Vec<OwnedExecutionControl>>>,
}

impl FileCapabilitySourceApi for CarrierReceiptFileSource {
    fn context_for_request(
        &self,
        _db_context: skiff_runtime_capability_context::DbCapabilityContext,
    ) -> FileCapabilityContext {
        FileCapabilityContext::new(CarrierReceiptFile {
            receipts: Arc::clone(&self.receipts),
        })
    }
}

impl FileCapabilityApi for CarrierReceiptFile {
    fn source(&self) -> FileCapabilitySource {
        FileCapabilitySource::new(CarrierReceiptFileSource {
            receipts: Arc::clone(&self.receipts),
        })
    }

    fn create_file<'a>(
        &'a self,
        _target: &'a str,
        _input: Bytes,
        _options: FileCreateOptions,
        execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, serde_json::Value> {
        self.receipts
            .lock()
            .expect("file receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }

    fn read_file_wire<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a skiff_runtime_boundary::file::ImmutableFileRef,
        execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, serde_json::Value> {
        self.receipts
            .lock()
            .expect("file receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }

    fn read_text_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a skiff_runtime_boundary::file::ImmutableFileRef,
        execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, serde_json::Value> {
        self.receipts
            .lock()
            .expect("file receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }

    fn file_info<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a skiff_runtime_boundary::file::ImmutableFileRef,
        execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, serde_json::Value> {
        self.receipts
            .lock()
            .expect("file receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }

    fn delete_file<'a>(
        &'a self,
        _target: &'a str,
        _file: &'a skiff_runtime_boundary::file::ImmutableFileRef,
        execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, ()> {
        self.receipts
            .lock()
            .expect("file receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(()) })
    }

    fn create_file_from_chunks<'a>(
        &'a self,
        _target: &'a str,
        _options: FileCreateOptions,
        _next_chunk: FileChunkSource<'a>,
        execution_control: OwnedExecutionControl,
    ) -> FileCapabilityFuture<'a, serde_json::Value> {
        self.receipts
            .lock()
            .expect("file receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(serde_json::Value::Null) })
    }
}

#[derive(Clone)]
struct CarrierReceiptWebsocketShared;

impl skiff_runtime_capability_context::WebsocketCapabilityApi for CarrierReceiptWebsocketShared {
    fn owned(&self) -> skiff_runtime_capability_context::OwnedWebsocketCapabilityContext {
        skiff_runtime_capability_context::WebsocketCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> skiff_runtime_capability_context::WebsocketCapabilityContext<'_> {
        skiff_runtime_capability_context::WebsocketCapabilityContext::new(self.clone())
    }

    fn service_id(&self) -> &str {
        "skiff.run/f445h-i6-receipt"
    }

    fn websocket_entry_id(&self) -> Option<&str> {
        Some("receipt")
    }

    fn send_connection_text_to_business_identity(
        &self,
        _business_identity: String,
        _text: String,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported("send is not under test"))
    }

    fn send_connection_binary_to_business_identity(
        &self,
        _business_identity: String,
        _payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported("send is not under test"))
    }

    fn send_connection_text_to_connection(
        &self,
        _connection_id: String,
        _text: String,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported("send is not under test"))
    }

    fn send_connection_binary_to_connection(
        &self,
        _connection_id: String,
        _payload: Vec<u8>,
    ) -> skiff_runtime_capability_context::CapabilityResult<()> {
        Err(CapabilityError::unsupported("send is not under test"))
    }
}

#[derive(Clone)]
struct CarrierReceiptWebsocketRequest {
    receipts: Arc<Mutex<Vec<OwnedExecutionControl>>>,
}

impl WebsocketRequestCapabilityApi for CarrierReceiptWebsocketRequest {
    fn request_json_to_connection<'a>(
        &'a self,
        _connection_id: String,
        _method: String,
        _payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> crate::capabilities::EvalCapabilityFuture<'a, ConnectionRequestTerminal> {
        self.receipts
            .lock()
            .expect("WebSocket receipt lock")
            .push(execution_control);
        Box::pin(async { Ok(ConnectionRequestTerminal::Success(b"null".to_vec())) })
    }
}

#[derive(Clone)]
struct CarrierReceiptActor {
    receipts: Arc<Mutex<Vec<OwnedExecutionControl>>>,
}

impl CarrierReceiptActor {
    fn record(&self, execution_control: OwnedExecutionControl) {
        self.receipts
            .lock()
            .expect("Actor receipt lock")
            .push(execution_control);
    }
}

impl ActorCapabilityApi for CarrierReceiptActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef> {
        self.record(execution_control);
        Box::pin(async { Err(CapabilityError::unsupported("result is not under test")) })
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef> {
        self.record(execution_control);
        Box::pin(async { Err(CapabilityError::unsupported("result is not under test")) })
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        self.record(execution_control);
        Box::pin(async { Ok(None) })
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, bool> {
        self.record(execution_control);
        Box::pin(async { Ok(false) })
    }

    fn invoke_actor<'a>(
        &'a self,
        _request: skiff_runtime_capability_context::ActorInvocationRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, skiff_runtime_capability_context::ActorInvocationOutcome> {
        self.record(execution_control);
        Box::pin(async {
            Ok(
                skiff_runtime_capability_context::ActorInvocationOutcome::Returned(
                    b"null".to_vec(),
                ),
            )
        })
    }
}

impl RequestCapabilityApi for CarrierReceiptActor {
    fn owned(&self) -> OwnedRequestCapabilityContext {
        RequestCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> RequestCapabilityContext<'_> {
        RequestCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "runtime:f445h-i6-receipt"
    }

    fn service_id(&self) -> &str {
        "skiff.run/f445h-i6-receipt"
    }

    fn service_version(&self) -> &str {
        "1.0.0"
    }

    fn request_id(&self) -> &str {
        "request:f445h-i6-receipt"
    }

    fn request_target(&self) -> &str {
        "receipt"
    }

    fn request_build_id(&self) -> &str {
        "build:f445h-i6-receipt"
    }

    fn task_service_protocol_identity(&self) -> &str {
        "protocol:f445h-i6-receipt"
    }

    fn request_service_protocol_identity(&self) -> &str {
        "protocol:f445h-i6-receipt"
    }

    fn operation_service_protocol_identity(&self) -> Option<&str> {
        Some("protocol:f445h-i6-receipt")
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        None
    }

    fn trace_id(&self) -> Option<&str> {
        Some("trace:f445h-i6-receipt")
    }

    fn submit_task<'a>(
        &'a self,
        _request: TaskSubmitControlRequest,
        _args_payload: Vec<u8>,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskSubmitResponseControl> {
        self.record(execution_control);
        Box::pin(async {
            Ok(TaskSubmitResponseControl {
                task_ref: "skiff-task-v1:b3duZXI.dGFzay0x".to_string(),
                task_id: "task-1".to_string(),
                request_id: "request-1".to_string(),
            })
        })
    }

    fn status_task<'a>(
        &'a self,
        _request: TaskStatusControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskStatusControlResponse> {
        self.record(execution_control);
        Box::pin(async {
            Ok(TaskStatusControlResponse {
                task_ref: "skiff-task-v1:b3duZXI.dGFzay0x".to_string(),
                kind: "scheduled".to_string(),
            })
        })
    }

    fn cancel_task<'a>(
        &'a self,
        _request: TaskCancelControlRequest,
        execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskCancelControlResponse> {
        self.record(execution_control);
        Box::pin(async {
            Ok(TaskCancelControlResponse {
                task_ref: "skiff-task-v1:b3duZXI.dGFzay0x".to_string(),
                kind: "canceled".to_string(),
            })
        })
    }
}

fn root_scope(deadline: Option<Instant>) -> (CancellationSource, ExecutionScope) {
    let cancellation = CancellationSource::new();
    let scope = ExecutionScope::request(cancellation.token(), deadline);
    (cancellation, scope)
}

fn carrier_receipt_context(
    overrides: ContextOverrides,
) -> (ProgramExecutionContext<'static>, ExecutionScope) {
    let base = Instant::now();
    let (request_cancellation, root) = root_scope(None);
    let outer = root
        .derive(base + Duration::from_secs(10), site())
        .expect("outer scope");
    let outer_control = ScopeAwareControl::available(outer, request_cancellation.token());
    let current =
        ExecutionControlApi::derive_scope(&outer_control, base + Duration::from_secs(5), site())
            .expect("inner scope");
    let expected = current.execution_scope().expect("inner execution scope");
    (
        context_with_overrides(outer_control, overrides).with_execution_control(current),
        expected,
    )
}

fn carrier_receipt_program() -> crate::EvalRuntimeProgram {
    crate::EvalRuntimeProgram::new(
        "skiff.run/f445h-i6-carrier-delivery-receipt",
        Vec::new(),
        Vec::new(),
        PublicationResourceTable::default(),
        HashMap::new(),
        LinkOverlay::default(),
        RuntimeTypeContext::default(),
    )
}

fn take_carrier_receipt(
    receipts: &Arc<Mutex<Vec<OwnedExecutionControl>>>,
) -> OwnedExecutionControl {
    let mut receipts = receipts.lock().expect("carrier receipt lock");
    assert_eq!(receipts.len(), 1, "exactly one lower receipt expected");
    receipts.pop().expect("carrier receipt")
}

fn assert_carrier_receipt(receipt: &OwnedExecutionControl, expected: &ExecutionScope) {
    let actual = receipt
        .execution_scope()
        .expect("lower receipt retains the current execution scope");
    assert_eq!(actual.nesting(), expected.nesting());
    assert_eq!(actual.effective_deadline(), expected.effective_deadline());
    assert_eq!(
        actual.lifecycle_snapshot(),
        Default::default(),
        "delivery itself must not acquire a lease, timer, or waiter"
    );
    assert_eq!(
        expected.lifecycle_snapshot(),
        Default::default(),
        "Ready operation completion must leave the invocation owner idle"
    );

    let deadline = expected
        .effective_deadline()
        .expect("inner scope owns an absolute deadline")
        .at();
    assert!(matches!(
        actual.terminal_at(deadline),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert!(
        expected.cancellation_signals().is_cancelled(),
        "the lower receipt and current scope must share the same local signal and deadline owner"
    );
    assert_eq!(expected.lifecycle_snapshot(), Default::default());
}

fn assert_native_invocation_scope(
    invocation: &RuntimeNativeInvocationExecutionControl,
    expected: &ExecutionScope,
) {
    let actual = invocation
        .execution_control()
        .execution_scope()
        .expect("native invocation retains the current execution scope");
    assert_eq!(actual.nesting(), expected.nesting());
    assert_eq!(actual.effective_deadline(), expected.effective_deadline());
    assert_eq!(
        actual.lifecycle_snapshot(),
        expected.lifecycle_snapshot(),
        "carrier retains the existing scope lifecycle"
    );
}

#[test]
fn f445h_i6_native_invocation_scope_projects_current_control_once_for_all_consumers() {
    let base = Instant::now();
    let (request_cancellation, root) = root_scope(None);
    let outer = root
        .derive(base + Duration::from_secs(10), site())
        .expect("outer scope");
    let outer_control = ScopeAwareControl::available(outer.clone(), request_cancellation.token());
    let frozen_context = context(outer_control.clone());
    let current =
        ExecutionControlApi::derive_scope(&outer_control, base + Duration::from_secs(5), site())
            .expect("inner scope");
    let expected = current.execution_scope().expect("inner execution scope");
    let current_context = frozen_context.with_execution_control(current);

    let program = crate::EvalRuntimeProgram::new(
        "skiff.run/f445h-i6-native-invocation-scope",
        Vec::new(),
        Vec::new(),
        PublicationResourceTable::default(),
        HashMap::new(),
        LinkOverlay::default(),
        RuntimeTypeContext::default(),
    );
    let projection = program.projection();
    let stream_context = StreamCapabilityContext::default();

    for required_context in [
        NativeRequiredContext::Actor,
        NativeRequiredContext::Time,
        NativeRequiredContext::HttpClient,
        NativeRequiredContext::HttpResponseStream,
        NativeRequiredContext::Websocket,
    ] {
        let projected = project_runtime_native_capability_context(
            &current_context,
            projection,
            stream_context.clone(),
            required_context,
        );
        let invocation = match &projected {
            skiff_runtime_capability_context::NativeCapabilityContexts::Actor(context) => {
                context.invocation_execution()
            }
            skiff_runtime_capability_context::NativeCapabilityContexts::Time(context) => {
                context.invocation_execution()
            }
            skiff_runtime_capability_context::NativeCapabilityContexts::HttpClient(context) => {
                context.invocation_execution()
            }
            skiff_runtime_capability_context::NativeCapabilityContexts::HttpResponseStream(
                context,
            ) => context.invocation_execution(),
            skiff_runtime_capability_context::NativeCapabilityContexts::Websocket(context) => {
                context.invocation_execution()
            }
            _ => panic!("unexpected native capability projection"),
        };
        assert_native_invocation_scope(invocation, &expected);
    }

    let projected = project_runtime_native_capability_context(
        &current_context,
        projection,
        stream_context,
        NativeRequiredContext::File,
    );
    let skiff_runtime_capability_context::NativeCapabilityContexts::File(context) = projected
    else {
        panic!("file projection expected");
    };
    let (file, source_stream, _) = context.into_native_file_parts();
    assert!(
        file.invocation_execution()
            .is_same_invocation(source_stream.invocation_execution()),
        "file and source-stream consumers share one invocation carrier"
    );
    assert_native_invocation_scope(file.invocation_execution(), &expected);
    assert_native_invocation_scope(source_stream.invocation_execution(), &expected);

    let supervised_stream = serde_json::Value::Null;
    let supervision = SupervisedStreamConsumptionLease::from_cancel(&supervised_stream, |_| {});
    let supervised = project_runtime_execution_native_capability_context_supervised(
        &current_context,
        RuntimeExecutionProjection::Legacy(projection),
        StreamCapabilityContext::default(),
        NativeRequiredContext::File,
        supervision.child(),
    );
    let skiff_runtime_capability_context::NativeCapabilityContexts::File(supervised) = supervised
    else {
        panic!("supervised file projection expected");
    };
    let (supervised_file, supervised_source_stream, _) = supervised.into_native_file_parts();
    assert!(
        supervised_file
            .invocation_execution()
            .is_same_invocation(supervised_source_stream.invocation_execution()),
        "supervised file consumers share one invocation carrier"
    );
    assert_native_invocation_scope(supervised_file.invocation_execution(), &expected);
    assert_native_invocation_scope(supervised_source_stream.invocation_execution(), &expected);
    assert_eq!(
        expected.lifecycle_snapshot(),
        Default::default(),
        "projection and Ready consumers do not manufacture suspension lifecycle state"
    );

    assert!(matches!(
        expected.terminal_at(base + Duration::from_secs(5)),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert!(
        file.invocation_execution()
            .execution_control()
            .execution_scope()
            .expect("file invocation scope")
            .cancellation_signals()
            .is_cancelled(),
        "carrier retains the child-local cancellation signal"
    );
}

#[tokio::test]
async fn f445h_i6_carrier_delivery_receipt_http_unary_reaches_lower_api() {
    let receipts = Arc::new(Mutex::new(Vec::new()));
    let (current_context, expected) = carrier_receipt_context(ContextOverrides {
        http_client: Some(HttpClientCapabilityContext::new(CarrierReceiptHttp {
            receipts: Arc::clone(&receipts),
        })),
        ..ContextOverrides::default()
    });
    let program = carrier_receipt_program();
    let projected = project_runtime_native_capability_context(
        &current_context,
        program.projection(),
        StreamCapabilityContext::default(),
        NativeRequiredContext::HttpClient,
    );
    let skiff_runtime_capability_context::NativeCapabilityContexts::HttpClient(http) = projected
    else {
        panic!("HTTP projection expected");
    };

    NativeHttpClientCapability::dispatch_http_request(&http, &serde_json::Value::Null)
        .await
        .expect("recording HTTP lower API returns Ready");

    assert_carrier_receipt(&take_carrier_receipt(&receipts), &expected);
}

#[tokio::test]
async fn f445h_i6_carrier_delivery_receipt_websocket_request_reaches_lower_api() {
    let receipts = Arc::new(Mutex::new(Vec::new()));
    let (current_context, expected) = carrier_receipt_context(ContextOverrides {
        websocket: Some(WebsocketCapabilityContext::with_request_api(
            CarrierReceiptWebsocketShared,
            CarrierReceiptWebsocketRequest {
                receipts: Arc::clone(&receipts),
            },
        )),
        ..ContextOverrides::default()
    });
    let program = carrier_receipt_program();
    let projected = project_runtime_native_capability_context(
        &current_context,
        program.projection(),
        StreamCapabilityContext::default(),
        NativeRequiredContext::Websocket,
    );
    let skiff_runtime_capability_context::NativeCapabilityContexts::Websocket(websocket) =
        projected
    else {
        panic!("WebSocket projection expected");
    };

    NativeWebsocketCapability::request_json_to_connection(
        &websocket,
        "connection".to_string(),
        "receipt".to_string(),
        b"null".to_vec(),
    )
    .await
    .expect("recording WebSocket lower API returns Ready");

    assert_carrier_receipt(&take_carrier_receipt(&receipts), &expected);
}

#[test]
fn f445h_i6_carrier_delivery_receipt_time_getter_returns_current_control() {
    let (current_context, expected) = carrier_receipt_context(ContextOverrides::default());
    let program = carrier_receipt_program();
    let projected = project_runtime_native_capability_context(
        &current_context,
        program.projection(),
        StreamCapabilityContext::default(),
        NativeRequiredContext::Time,
    );
    let skiff_runtime_capability_context::NativeCapabilityContexts::Time(time) = projected else {
        panic!("time projection expected");
    };

    assert_carrier_receipt(&NativeTimeCapability::execution_control(&time), &expected);
}

struct TimePendingNoopWake;

impl Wake for TimePendingNoopWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_time_pending<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    let waker = Waker::from(Arc::new(TimePendingNoopWake));
    future.poll(&mut Context::from_waker(&waker))
}

fn time_sleep_invocation() -> RuntimeNativeInvocation {
    RuntimeNativeInvocation::new(
        "std.time.sleep".to_string(),
        "std.time.sleep",
        Some(NativeCallPlan::new(
            NativeBindingKey::from_static("std.time.sleep"),
            vec![RuntimeTypePlan::new(
                "number",
                None,
                RuntimeTypeNode::Number,
            )],
            RuntimeTypePlan::new("null", None, RuntimeTypeNode::Null),
            NativeRequiredContext::Time,
        )),
        None,
        None,
    )
}

#[tokio::test]
async fn f445h_i6_time_projection_to_pending_reaches_real_sleep_owner() {
    let base = tokio::time::Instant::now().into_std();
    let current_deadline = base + Duration::from_secs(60);
    let (request_cancellation, root) = root_scope(None);
    let outer = root
        .derive(base + Duration::from_secs(120), site())
        .expect("outer scope");
    let outer_control = ScopeAwareControl::available(outer, request_cancellation.token());
    let current = ExecutionControlApi::derive_scope(&outer_control, current_deadline, site())
        .expect("current scope");
    let expected = current.execution_scope().expect("current execution scope");
    let current_context = context(outer_control).with_execution_control(current);
    let program = carrier_receipt_program();
    let projected = project_runtime_native_capability_context(
        &current_context,
        program.projection(),
        StreamCapabilityContext::default(),
        NativeRequiredContext::Time,
    );
    let mut heap = RequestHeap::default();
    let prepared = NativeDispatch::new()
        .prepare_resolved_native_call(
            projected,
            time_sleep_invocation(),
            vec![RuntimeValue::Number(1_000.0)],
            &mut heap,
        )
        .expect("time sleep prepares through the native projection");
    let PreparedNativeCall::ExternalWait(operation) = prepared else {
        panic!("positive sleep must expose a real external wait");
    };
    let (mut wait, _finalize) = operation.into_parts();

    assert!(matches!(poll_time_pending(wait.as_mut()), Poll::Pending));
    assert_eq!(
        expected.lifecycle_snapshot(),
        skiff_runtime_capability_context::ExecutionScopeLifecycleSnapshot {
            active_leases: 1,
            active_waiters: 1,
            active_timers: 1,
        },
        "the projected current scope must own the real sleep pending lifecycle"
    );

    assert!(matches!(
        expected.terminal_at(current_deadline),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    let Poll::Ready(outcome) = poll_time_pending(wait.as_mut()) else {
        panic!("current scope deadline must wake the real sleep future");
    };
    assert!(matches!(
        outcome,
        Err(skiff_runtime_native::error::RuntimeError::Cancelled)
    ));
    assert!(matches!(
        expected.terminal_at(tokio::time::Instant::now().into_std()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(expected.lifecycle_snapshot(), Default::default());
}

#[tokio::test]
async fn f445h_i6_carrier_delivery_receipt_file_create_reaches_lower_api() {
    let receipts = Arc::new(Mutex::new(Vec::new()));
    let (current_context, expected) = carrier_receipt_context(ContextOverrides {
        file: Some(FileCapabilityContext::new(CarrierReceiptFile {
            receipts: Arc::clone(&receipts),
        })),
        ..ContextOverrides::default()
    });
    let program = carrier_receipt_program();
    let projected = project_runtime_native_capability_context(
        &current_context,
        program.projection(),
        StreamCapabilityContext::default(),
        NativeRequiredContext::File,
    );
    let skiff_runtime_capability_context::NativeCapabilityContexts::File(file) = projected else {
        panic!("file projection expected");
    };
    let (file, _, _) = file.into_native_file_parts();

    NativeFileCapability::create_file(&file, "receipt", Bytes::new(), FileCreateOptions::default())
        .await
        .expect("recording file lower API returns Ready");

    assert_carrier_receipt(&take_carrier_receipt(&receipts), &expected);
}

#[tokio::test]
async fn f445h_i6_carrier_delivery_receipt_actor_find_reaches_lower_api() {
    let receipts = Arc::new(Mutex::new(Vec::new()));
    let (current_context, expected) = carrier_receipt_context(ContextOverrides {
        actor: Some(ActorCapabilityContext::new(CarrierReceiptActor {
            receipts: Arc::clone(&receipts),
        })),
        ..ContextOverrides::default()
    });
    let program = carrier_receipt_program();
    let projected = project_runtime_native_capability_context(
        &current_context,
        program.projection(),
        StreamCapabilityContext::default(),
        NativeRequiredContext::Actor,
    );
    let skiff_runtime_capability_context::NativeCapabilityContexts::Actor(actor) = projected else {
        panic!("Actor projection expected");
    };

    let result = NativeActorCapability::find_actor(
        &actor,
        ActorFindControlRequest {
            rpc_id: "rpc:f445h-i6-receipt".to_string(),
            runtime_id: "runtime:f445h-i6-receipt".to_string(),
            activation_identity: ActivationIdentityControl {
                assembly_identity: AssemblyIdentity::new("assembly:f445h-i6-receipt"),
                generation: 1,
                runtime_replica_id: "replica:f445h-i6-receipt".to_string(),
                deployment_revision: DeploymentRevision::new("revision:f445h-i6-receipt"),
            },
            actor_key: ActorKeyControlMetadata {
                service_id: "skiff.run/f445h-i6-receipt".to_string(),
                actor_type_identity: "actor:f445h-i6-receipt".to_string(),
                actor_id_type_identity: "actor-id:f445h-i6-receipt".to_string(),
                actor_id_encoding_version: "1".to_string(),
                canonical_actor_id_key_bytes_base64: "bnVsbA==".to_string(),
                actor_id_hash: None,
            },
        },
    )
    .await
    .expect("recording Actor lower API returns Ready");
    assert!(result.is_none());

    assert_carrier_receipt(&take_carrier_receipt(&receipts), &expected);
}

#[derive(Clone)]
struct ScriptedClock {
    values: Arc<Mutex<VecDeque<Instant>>>,
    last: Instant,
    calls: Arc<AtomicU64>,
}

impl ScriptedClock {
    fn new(values: Vec<Instant>, calls: Arc<AtomicU64>) -> Self {
        let last = *values.last().expect("scripted clock needs one value");
        Self {
            values: Arc::new(Mutex::new(values.into())),
            last,
            calls,
        }
    }
}

impl EvalMonotonicClock for ScriptedClock {
    fn now(&self) -> Instant {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.values
            .lock()
            .expect("clock mutex poisoned")
            .pop_front()
            .unwrap_or(self.last)
    }
}

#[test]
fn program_execution_scope_duration_uses_exact_or_safely_clamped_deadline() {
    let now = Instant::now();
    assert_eq!(
        deadline_after_duration_ms(now, 25),
        now.checked_add(Duration::from_millis(25))
            .expect("ordinary duration should be representable")
    );

    let clamped = deadline_after_duration_ms(now, u64::MAX);
    assert!(clamped >= now);
    if now.checked_add(Duration::from_millis(u64::MAX)).is_none() {
        assert!(clamped.checked_add(Duration::from_millis(1)).is_none());
    }
}

#[test]
fn program_execution_scope_child_capture_and_owned_round_trip_preserve_current_scope() {
    let (cancellation, scope) = root_scope(None);
    let parent = context(ScopeAwareControl::available(scope, cancellation.token()));
    assert_eq!(parent.execution_scope().expect("parent scope").nesting(), 0);

    let child = parent
        .derive_timeout_child(1_000, site())
        .expect("child scope should derive");
    assert_eq!(child.execution_scope().expect("child scope").nesting(), 1);
    assert_eq!(parent.execution_scope().expect("parent scope").nesting(), 0);

    let owned = OwnedProgramExecutionContext::capture(&child);
    let borrowed = owned.borrow();
    assert_eq!(
        borrowed
            .execution_scope()
            .expect("round-trip child scope")
            .nesting(),
        1
    );
    drop(child);
    assert_eq!(
        parent.execution_scope().expect("restored parent").nesting(),
        0
    );
}

#[test]
fn program_execution_scope_owned_round_trip_preserves_current_scripted_clock_sequence() {
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let scope = root
        .derive(base + Duration::from_millis(3), site())
        .expect("local scope");
    let calls = Arc::new(AtomicU64::new(0));
    let context = context(ScopeAwareControl::available(scope, cancellation.token()))
        .with_execution_clock(ExecutionClock::new(ScriptedClock::new(
            vec![base, base + Duration::from_millis(3)],
            Arc::clone(&calls),
        )));
    let checkpoint = ExecutionCheckpoint::new(ExecutionCheckpointKind::GeneratedChunk, 1);

    context
        .checkpoint(checkpoint)
        .expect("first scripted checkpoint remains before the deadline");
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    let owned = OwnedProgramExecutionContext::capture(&context);
    let borrowed = owned.borrow();
    let error = borrowed
        .checkpoint(checkpoint)
        .expect_err("owned round-trip must continue the same scripted clock");
    assert!(matches!(
        error.scope_terminal().map(|carrier| carrier.terminal()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[test]
fn program_execution_scope_unavailable_and_derive_failure_fail_closed() {
    let unavailable = context(ScopeAwareControl::unavailable());
    assert!(matches!(
        unavailable.execution_scope(),
        Err(RuntimeError::InvalidArtifact(message))
            if message.contains("current execution scope is unavailable")
    ));

    let (cancellation, scope) = root_scope(None);
    let derive_failure =
        context(ScopeAwareControl::available(scope, cancellation.token()).with_derive_failure());
    assert!(matches!(
        derive_failure.derive_timeout_child(1, site()),
        Err(RuntimeError::InvalidArtifact(message))
            if message.contains("execution scope nesting exceeds u32")
    ));
}

#[test]
fn program_execution_scope_nested_deadlines_keep_precise_owner() {
    let base = Instant::now();
    let (_, root) = root_scope(None);

    let outer = root
        .derive(base + Duration::from_millis(20), site())
        .expect("outer scope");
    let inner_earlier = outer
        .derive(base + Duration::from_millis(10), site())
        .expect("inner scope");
    assert!(matches!(
        inner_earlier.terminal_at(base + Duration::from_millis(10)),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));

    let outer_earlier = root
        .derive(base + Duration::from_millis(10), site())
        .expect("outer scope");
    let inner_later = outer_earlier
        .derive(base + Duration::from_millis(20), site())
        .expect("inner scope");
    assert!(matches!(
        inner_later.terminal_at(base + Duration::from_millis(10)),
        Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));

    let equal_inner = outer_earlier
        .derive(base + Duration::from_millis(10), site())
        .expect("equal inner scope");
    assert!(matches!(
        equal_inner.terminal_at(base + Duration::from_millis(10)),
        Some(ExecutionScopeTerminal::InheritedDeadlineExceeded(_))
    ));
}

#[test]
fn program_execution_scope_checkpoint_kinds_account_explicit_units() {
    let (cancellation, scope) = root_scope(None);
    let control = ScopeAwareControl::available(scope, cancellation.token());
    let units = Arc::clone(&control.instruction_units);
    let context = context(control);
    let kinds = [
        ExecutionCheckpointKind::FunctionEntry,
        ExecutionCheckpointKind::LoopCondition,
        ExecutionCheckpointKind::LoopBackedge,
        ExecutionCheckpointKind::LaneStart,
        ExecutionCheckpointKind::LaneEnd,
        ExecutionCheckpointKind::TailStart,
        ExecutionCheckpointKind::GeneratedChunk,
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        let checkpoint = ExecutionCheckpoint::new(kind, index as u64 + 1);
        assert_eq!(checkpoint.kind(), kind);
        context.checkpoint(checkpoint).expect("checkpoint");
    }
    assert_eq!(units.load(Ordering::Relaxed), 28);
}

#[test]
fn program_execution_scope_scripted_clock_crosses_on_bounded_checkpoint() {
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let scope = root
        .derive(base + Duration::from_millis(3), site())
        .expect("local scope");
    let calls = Arc::new(AtomicU64::new(0));
    let clock = ScriptedClock::new(
        vec![
            base,
            base + Duration::from_millis(2),
            base + Duration::from_millis(3),
        ],
        Arc::clone(&calls),
    );
    let context = context(ScopeAwareControl::available(
        scope.clone(),
        cancellation.token(),
    ))
    .with_execution_clock(ExecutionClock::new(clock));

    let checkpoint = ExecutionCheckpoint::new(ExecutionCheckpointKind::GeneratedChunk, 1);
    context.checkpoint(checkpoint).expect("first checkpoint");
    context.checkpoint(checkpoint).expect("second checkpoint");
    let error = context
        .checkpoint(checkpoint)
        .expect_err("third checkpoint crosses deadline");
    assert!(matches!(
        error.scope_terminal().map(|carrier| carrier.terminal()),
        Some(ExecutionScopeTerminal::LocalDeadlineExceeded(_))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 3);
}

#[test]
fn program_execution_scope_checkpoint_normalizes_cancel_and_keeps_instruction_limit() {
    let (cancellation, scope) = root_scope(None);
    let cancelled_context = context(ScopeAwareControl::available(scope, cancellation.token()));
    cancellation.cancel();
    assert!(matches!(
        cancelled_context.checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::FunctionEntry,
            1,
        )),
        Err(RuntimeError::Cancelled)
    ));

    let (active_cancellation, active_scope) = root_scope(None);
    let limit = ExecutionControlError::BudgetExceeded(ExecutionBudgetFailure {
        reason: ExecutionBudgetReason::InstructionLimitExceeded,
        instruction_count: 11,
        limit: Some(10),
        elapsed_ms: 1.5,
    });
    let limit_context = context(
        ScopeAwareControl::available(active_scope, active_cancellation.token())
            .with_budget_error(limit),
    );
    assert!(matches!(
        limit_context.checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::LoopBackedge,
            1,
        )),
        Err(RuntimeError::ExecutionBudgetExceeded {
            reason: BudgetReason::InstructionLimitExceeded,
            instruction_count: 11,
            limit: Some(10),
            ..
        })
    ));
}

#[test]
fn program_execution_scope_generic_deadline_race_recovers_current_owner() {
    let base = Instant::now();
    let (cancellation, root) = root_scope(None);
    let scope = root
        .derive(base + Duration::from_millis(2), site())
        .expect("local scope");
    let deadline_error = ExecutionControlError::BudgetExceeded(ExecutionBudgetFailure {
        reason: ExecutionBudgetReason::DeadlineExceeded,
        instruction_count: 2,
        limit: None,
        elapsed_ms: 2.0,
    });
    let calls = Arc::new(AtomicU64::new(0));
    let context = context(
        ScopeAwareControl::available(scope.clone(), cancellation.token())
            .with_budget_error(deadline_error),
    )
    .with_execution_clock(ExecutionClock::new(ScriptedClock::new(
        vec![base, base + Duration::from_millis(2)],
        Arc::clone(&calls),
    )));

    let error = context
        .checkpoint(ExecutionCheckpoint::new(
            ExecutionCheckpointKind::LoopCondition,
            1,
        ))
        .expect_err("generic deadline must recover scope owner");
    let carrier = error.scope_terminal().expect("internal scope terminal");
    assert!(matches!(
        carrier.terminal(),
        ExecutionScopeTerminal::LocalDeadlineExceeded(_)
    ));
    assert!(carrier.is_owned_by(&scope));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}
