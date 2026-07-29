use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::Poll,
};

use serde_json::Value;
use skiff_artifact_model::{
    CanonicalPublicCallableSignature, OperationAbiRef, PublicationAbiUnit, PublicationOperationAbi,
    PublicationOperationKind, TypeRefIr,
};
use skiff_runtime_boundary::{
    binary::encode_payload_plan,
    json::RuntimeBoundaryCodec,
    payload::{PayloadBoundary, PayloadBoundaryKind},
    plan::BoundaryUse,
};
use skiff_runtime_capability_context::{
    CancellationToken, OutboundRequestCancelSendError, OutboundRequestCancelSender,
    OutboundRequestRegistry, OutboundResponse, OutboundResponseReceiver, OutboundStartedRequest,
    RequestEffectDoubleControl, StreamCancelSignal, StreamLifetimeGuard, StreamPoll,
    StreamPullSource, StreamRuntimeApi, StreamRuntimeResult, StreamSink,
};
use skiff_runtime_linked_program::{
    ExternalRefTable, FileDeclarations, FileLinkTargets, LinkOverlay, LinkedFileUnit,
    PublicationResourceTable, RuntimeTypeContext, ServiceDependencyConstraint, SourceMapDto,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};
use tokio::sync::mpsc;

use super::super::*;
use crate::capabilities::{EvalCapabilityFuture, OutboundServiceApi};

pub(super) fn assert_heap_free_wait<F>(_: &F)
where
    F: Future + Send + 'static,
{
}

struct PullSetupRuntime {
    sources: Mutex<Vec<Box<dyn StreamPullSource>>>,
}

impl std::fmt::Debug for PullSetupRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PullSetupRuntime")
            .field(
                "source_count",
                &self.sources.lock().expect("source lock").len(),
            )
            .finish()
    }
}

impl StreamRuntimeApi for PullSetupRuntime {
    fn channel_stream(&self) -> (Value, StreamSink) {
        unreachable!("prepared stream test only exercises pull setup")
    }

    fn channel_stream_with_lifetime(&self, _lifetime: StreamLifetimeGuard) -> (Value, StreamSink) {
        unreachable!("prepared stream test only exercises pull setup")
    }

    fn pull_stream_with_cancellation(
        &self,
        source: Box<dyn StreamPullSource>,
        _cancellation: CancellationToken,
    ) -> Value {
        self.sources.lock().expect("source lock").push(source);
        skiff_runtime_boundary::stream::stream_value("prepared-outbound-stream")
    }

    fn buffered_stream(&self, _items: Vec<Value>) -> Value {
        unreachable!("prepared stream test only exercises pull setup")
    }

    fn next_with_cancel<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_flags: &'a [Arc<AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        unreachable!("prepared stream test only exercises pull setup")
    }

    fn next_with_cancellation<'a>(
        &'a self,
        _value: &'a Value,
        _signals: &'a [StreamCancelSignal],
        _cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        unreachable!("prepared stream test only exercises pull setup")
    }

    fn next<'a>(
        &'a self,
        _value: &'a Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<StreamPoll>> + Send + 'a>> {
        unreachable!("prepared stream test only exercises pull setup")
    }

    fn cancel(&self, _value: &Value) {}
}

pub(super) fn pull_setup_runtime() -> StreamRuntime {
    StreamRuntime::new(PullSetupRuntime {
        sources: Mutex::new(Vec::new()),
    })
}

pub(super) async fn poll_once<F>(mut future: Pin<&mut F>) -> Option<F::Output>
where
    F: Future,
{
    std::future::poll_fn(|context| {
        Poll::Ready(match future.as_mut().poll(context) {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        })
    })
    .await
}

#[derive(Clone)]
pub(super) struct RecordingOutbound {
    pub(super) state: Arc<RecordingOutboundState>,
}

pub(super) struct RecordingOutboundState {
    dependencies: Vec<ServiceDependencyConstraint>,
    pub(super) registry: OutboundRequestRegistry,
    starts: AtomicUsize,
    cancels: Mutex<Vec<(String, String)>>,
    response_sender: Mutex<Option<mpsc::UnboundedSender<OutboundResponse>>>,
    buffered: Mutex<Option<OutboundResponse>>,
}

impl RecordingOutbound {
    pub(super) fn pending() -> Self {
        Self::pending_with_dependencies(Vec::new())
    }

    pub(super) fn pending_with_dependencies(
        dependencies: Vec<ServiceDependencyConstraint>,
    ) -> Self {
        Self {
            state: Arc::new(RecordingOutboundState {
                dependencies,
                registry: OutboundRequestRegistry::default(),
                starts: AtomicUsize::new(0),
                cancels: Mutex::new(Vec::new()),
                response_sender: Mutex::new(None),
                buffered: Mutex::new(None),
            }),
        }
    }

    pub(super) fn buffered(response: OutboundResponse) -> Self {
        let this = Self::pending();
        *this.state.buffered.lock().expect("buffered lock") = Some(response);
        this
    }

    pub(super) fn starts(&self) -> usize {
        self.state.starts.load(Ordering::Acquire)
    }

    pub(super) fn cancels(&self) -> Vec<(String, String)> {
        self.state.cancels.lock().expect("cancel lock").clone()
    }

    pub(super) fn send(&self, response: OutboundResponse) -> bool {
        self.state
            .response_sender
            .lock()
            .expect("response sender lock")
            .as_ref()
            .is_some_and(|sender| sender.send(response).is_ok())
    }
}

impl OutboundServiceApi for RecordingOutbound {
    fn service_dependencies(&self) -> &[ServiceDependencyConstraint] {
        &self.state.dependencies
    }

    fn test_effects_enabled(&self) -> bool {
        false
    }

    fn test_effect_doubles(&self) -> HashMap<String, Vec<RequestEffectDoubleControl>> {
        HashMap::new()
    }

    fn request_heap(&self) -> RequestHeap {
        RequestHeap::default()
    }

    fn effective_timeout_ms(&self, _operation_timeout_ms: Option<u64>) -> Option<u64> {
        None
    }

    fn outbound_deadline_error(&self) -> RuntimeError {
        RuntimeError::Cancelled
    }

    fn start_request(
        &self,
        _start: OutboundServiceRequestStart,
        _payload: Vec<u8>,
    ) -> Result<OutboundStartedRequest> {
        let ordinal = self.state.starts.fetch_add(1, Ordering::AcqRel) + 1;
        let request_id = format!("prepared-outbound-{ordinal}");
        let (sender, response_rx) = mpsc::unbounded_channel();
        let state = self.state.clone();
        let cancel_sender: OutboundRequestCancelSender = Arc::new(move |request_id, reason| {
            state
                .cancels
                .lock()
                .expect("cancel lock")
                .push((request_id.to_string(), reason.to_string()));
            Ok::<(), OutboundRequestCancelSendError>(())
        });
        let lease = self
            .state
            .registry
            .insert_with_lease(
                request_id.clone(),
                sender.clone(),
                Some(cancel_sender),
                "unary_wait_dropped",
            )
            .expect("request lease");
        *self
            .state
            .response_sender
            .lock()
            .expect("response sender lock") = Some(sender.clone());
        if let Some(response) = self.state.buffered.lock().expect("buffered lock").take() {
            sender.send(response).expect("buffered response");
        }
        Ok(OutboundStartedRequest {
            request_id,
            response_rx,
            lease,
        })
    }

    fn receive_response<'a>(
        &'a self,
        _lease: &'a OutboundRequestLease,
        target: &'a str,
        receiver: &'a mut OutboundResponseReceiver,
        _timeout_ms: Option<u64>,
    ) -> EvalCapabilityFuture<'a, OutboundResponse> {
        Box::pin(async move {
            receiver
                .recv()
                .await
                .ok_or_else(|| RuntimeError::ProviderUnavailable {
                    target: target.to_string(),
                    reason: "response channel closed".to_string(),
                })
        })
    }

    fn cancel_signal(&self) -> CancellationToken {
        CancellationToken::new()
    }
}

pub(super) fn string_plan() -> RuntimeTypePlan {
    RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, Vec::new())
}

fn dispatch(mode: &str, response_plan: RuntimeTypePlan) -> OutboundServiceDispatch {
    let return_plan = if mode == "serverStream" {
        RuntimeTypePlan::synthetic_named_builtin(
            "Stream",
            RuntimeTypeNode::Stream(Box::new(response_plan.clone())),
            vec![response_plan.clone()],
        )
    } else {
        response_plan.clone()
    };
    OutboundServiceDispatch {
        service_id: "skiff.test/provider".to_string(),
        version: "1.0.0".to_string(),
        build_id: "build:test".to_string(),
        service_protocol_identity: "protocol:test".to_string(),
        operation_abi_id: "operation:test".to_string(),
        selector: "operation:operation:test".to_string(),
        target: "provider.test".to_string(),
        mode: mode.to_string(),
        timeout_ms: None,
        activation_identity: None,
        params: Vec::new(),
        request_plan: None,
        response_plan,
        return_plan,
    }
}

fn request_start(mode: &str) -> OutboundServiceRequestStart {
    OutboundServiceRequestStart {
        mode: mode.to_string(),
        target: "provider.test".to_string(),
        operation_abi_id: "operation:test".to_string(),
        selector: "operation:operation:test".to_string(),
        service_id: "skiff.test/provider".to_string(),
        version: "1.0.0".to_string(),
        build_id: "build:test".to_string(),
        service_protocol_identity: "protocol:test".to_string(),
        activation_identity: None,
        timeout_ms: None,
        test_effect_doubles: HashMap::new(),
    }
}

pub(super) fn dependency_fixture() -> (ServiceDependencyConstraint, OperationAbiRef) {
    let operation = OperationAbiRef {
        operation_abi_id: "operation:test".to_string(),
        kind: PublicationOperationKind::PublicFunction,
        public_path: "provider.test".to_string(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: "provider.test".to_string(),
    };
    let operation_abi = PublicationOperationAbi {
        operation: operation.clone(),
        public_signature: CanonicalPublicCallableSignature {
            params: Vec::new(),
            return_type: TypeRefIr::builtin("string"),
            may_suspend: true,
        },
        schema_closure: Vec::new(),
        stream_effect_throw_config: Default::default(),
    };
    let mut publication =
        PublicationAbiUnit::empty("skiff.test/provider", "1.0.0", "publication-abi:test");
    publication.operation_exports.push(operation.clone());
    publication.operation_abi.push(operation_abi);
    (
        ServiceDependencyConstraint {
            id: "skiff.test/provider".to_string(),
            version: "1.0.0".to_string(),
            alias: "provider".to_string(),
            build_id: "build:test".to_string(),
            service_protocol_identity: "protocol:test".to_string(),
            publication_abi: publication,
        },
        operation,
    )
}

pub(super) fn outbound_interpreter() -> Interpreter {
    let file = Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:outbound-owner-test".to_string(),
        source_ast_hash: "source:outbound-owner-test".to_string(),
        module_path: "outbound_owner_test".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: SourceMapDto::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        actor_declarations: Vec::new(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: Vec::new(),
        external_refs: ExternalRefTable::default(),
    });
    let program = Arc::new(crate::EvalRuntimeProgram::new(
        "skiff.test/caller",
        vec![file],
        Vec::new(),
        PublicationResourceTable::default(),
        Default::default(),
        LinkOverlay::default(),
        RuntimeTypeContext::default(),
    ));
    Interpreter::with_program(
        program,
        crate::actor_executor_test_runtime::runtime_factory(),
    )
}

pub(super) fn encode_response(plan: &RuntimeTypePlan, wire: &Value) -> Vec<u8> {
    let mut heap = RequestHeap::default();
    let value = RuntimeBoundaryCodec::new(plan, BoundaryUse::NativeReturn, "test response")
        .from_wire_json(wire, &mut heap)
        .expect("test response should materialize");
    encode_payload_plan(
        &value,
        plan,
        &PayloadBoundary::cross_service(
            PayloadBoundaryKind::InboundServiceCall,
            "skiff.test/provider",
        ),
        &mut heap,
    )
    .expect("test response should encode")
}

pub(super) fn prepare(
    api: &RecordingOutbound,
    mode: &str,
    response_plan: RuntimeTypePlan,
    heap: &mut RequestHeap,
    env: &Env,
    stream_runtime: &StreamRuntime,
) -> Result<PreparedOutboundServiceCall> {
    prepare_outbound_service_request(
        &OutboundServiceContext::new(api.clone()),
        stream_runtime,
        heap,
        env,
        dispatch(mode, response_plan),
        Vec::new(),
        request_start(mode),
    )
}
