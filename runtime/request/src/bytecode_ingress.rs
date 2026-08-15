use std::{
    future::Future,
    num::{NonZeroU32, NonZeroUsize},
    pin::Pin,
    sync::{mpsc, Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
};

use skiff_artifact_model::{
    http_boundary::{HTTP_BOUNDARY_PACKAGE_ID, HTTP_REQUEST_TYPE},
    HostEffectExecutorIdentity, Opcode, PackageRefIr, PrivilegedAffineCompositeIdentity, TypeRefIr,
};
use skiff_runtime_boundary::http::HttpBoundaryNameValue;
use skiff_runtime_capability_context::{CancellationToken, ExecutionBudgetReason};
use skiff_runtime_linked_bytecode::{
    LinkedDbOperation, LinkedNativeCallableSignature, LinkedRepresentationCarrier,
    LinkedResumeResultMaterialization, LinkedShapeEntry, LinkedValueDropPlan,
    LinkedValueTransferPlan, TypeIndex,
};
use skiff_runtime_linker::{DeploymentExecutionEntry, DeploymentExecutionImage};
use skiff_runtime_model::{
    bytecode_execution_observation::{
        BytecodeExecutionObserver, RequestExecutionOwnerInventorySnapshot,
    },
    error::RuntimeErrorPayload,
    request_heap::RequestHeapLimits,
    runtime_value::RuntimeValue,
    service_error::ErrorCorrelation,
    vm_heap::{VmContainerShape, VmHeap, VmHeapError, VmHeapOperation, VmRecordField},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};

const HTTP_HEADER_TYPE: &str = "std.http.HttpHeader";
const HTTP_QUERY_PARAM_TYPE: &str = "std.http.HttpQueryParam";
const HTTP_CLIENT_REQUEST_TYPE: &str = "std.http.HttpClientRequest";
const HTTP_CLIENT_RESPONSE_TYPE: &str = "std.http.HttpClientResponse";
const HTTP_CLIENT_STREAM_HANDLE_TYPE: &str = "std.http.HttpClientStreamHandle";
use skiff_runtime_scheduler::{
    BytecodeAdapterHandoff, BytecodeChildExecutor, BytecodeChildHandoff, BytecodeHandoff,
    BytecodeParkFailure, BytecodeParkRequest, BytecodePortFailure, BytecodeScheduler,
    BytecodeSchedulerError, BytecodeSchedulerFailure, BytecodeSchedulerFailureOwner,
    BytecodeSchedulerOutcome, BytecodeSchedulerPorts, BytecodeStreamHandoff,
    BytecodeStreamSupervisor, ChildHeapCarrier, ClaimedPendingWakeGuard, CompletionHandle,
    PendingRegistry, PendingWake, PendingWakeQueue, RequestByteStreamFailure,
    RequestExecutionContext, RequestResourceFinishReason, RequestResourceHandle,
    RequestResourceTable, RequestResourceTermination, RequestServerStreamReservation, RootEscrow,
    SuspendedTrampoline,
};
use skiff_runtime_vm::{
    AdapterInvocation, ChildInvocation, ChildTarget, PendingOperation, ResumeOutcome, Vm, VmBudget,
    VmBudgetClosed, VmBudgetTerminal, VmCompletion, VmError, VmFiber, VmHostEffectArguments,
    VmHostEffectArgumentsReleaseError, VmInternalTerminal, VmLimits, VmOwnedValues, VmResumeToken,
    VmTerminalCause, VmTerminalEscrow,
};

use crate::{
    bytecode_children::{
        execute_actor_child, execute_interface_child, execute_service_child, execute_task_child,
        is_task_request, linked_db_target, materialize_db_result_to_vm, require_db_operation,
        task_arguments, BytecodeChildHeapFactory, BytecodeChildLane,
        BytecodeRequestChildComposition, RequestChildHeapFactory,
    },
    bytecode_host_effects::{
        BytecodeHttpFailure, BytecodeHttpRequest, BytecodeHttpResponse,
        BytecodeHttpStreamRegistrar, BytecodeHttpStreamResponse, BytecodeServerStreamWriteFailure,
        BytecodeServerStreamWriteFuture, SharedBytecodeHttpClientPort,
        SharedBytecodeServerStreamWriterPort,
    },
    bytecode_server_stream::{
        materialize_server_stream_flush_outcome, BytecodeServerStreamSupervisor,
    },
    execution_budget::{ExecutionWinner, RequestPendingSink},
    vm_heap::RequestVmHeap,
    BinaryHttpRequest, BoundaryResponse, ExecutionBudget, ExecutionControl, GatewayAdapterSource,
    HttpAdapterKind, HttpNameValue, HttpResponseMetadata, RequestEnvelope, RequestError,
    RequestResult,
};

pub struct BytecodeRequestExecutionInput {
    pub target: DeploymentExecutionEntry,
    pub request: RequestEnvelope,
    pub observer: BytecodeExecutionObserver,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: BytecodeRequestExecutionHandles,
    /// Exact typed HTTP provider for this request. The two admitted HTTP
    /// executor identities fail closed when it is absent; sleep does not
    /// require this port, and no legacy context or binding-string fallback is
    /// consulted.
    pub http_client: Option<Arc<dyn crate::BytecodeHttpClientPort>>,
    /// Transport-only writer installed only for an exact linked raw-HTTP
    /// server-stream entry. Capacity, sequence and terminal state stay in the
    /// scheduler-owned resource table.
    pub server_stream_writer: Option<Arc<dyn crate::BytecodeServerStreamWriterPort>>,
    /// Child composition injected by the host/request seam. Service children
    /// fail closed when the resolver/factory is absent.
    pub child_composition: BytecodeRequestChildComposition,
    /// Optional injected VM heap (production composition or a recording heap
    /// spy). When `None`, the driver constructs the production
    /// [`RequestVmHeap`] from `handles.request_heap_limits`. The injected heap
    /// is exactly the heap driven into the VM and retained for the boundary
    /// result lifetime.
    pub heap: Option<Box<dyn VmHeap + Send>>,
}

pub struct BytecodeRequestExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
    pub max_response_bytes: NonZeroUsize,
}

/// Opaque retention carrier for one driven bytecode request.
///
/// The carrier keeps the VM heap and budget alive for the lifetime of the
/// boundary result; dropping it releases all remaining heap owners and
/// detaches the VM budget. The fields are intentionally never read: the
/// carrier's entire contract is its Drop lifetime.
#[allow(dead_code)]
pub struct BytecodeRequestRetention {
    scheduler_failure_owner: Option<BytecodeSchedulerFailureOwner<VmFiber>>,
    terminal_cause: Option<VmTerminalCause>,
    terminal_escrow: Option<VmTerminalEscrow>,
    materialization_escrows: Vec<VmTerminalEscrow>,
    budget: Option<Box<dyn VmBudget + Send>>,
    cleanup_roots: Vec<ValueSlot>,
    // The concrete request heap is deliberately last. Every carrier above is
    // released or dropped before heap teardown, including on retryable
    // lifecycle failure.
    heap: Option<Box<dyn VmHeap + Send>>,
}

impl Drop for BytecodeRequestRetention {
    fn drop(&mut self) {
        let Some(heap) = self.heap.as_deref_mut() else {
            return;
        };
        if let Some(owner) = self.scheduler_failure_owner.as_mut() {
            let _ = owner.release_terminal_escrow(heap);
        }
        if let Some(cause) = self.terminal_cause.as_mut() {
            let _ = cause.release_all(heap);
        }
        if let Some(escrow) = self.terminal_escrow.as_mut() {
            let _ = escrow.release_all(heap);
        }
        for escrow in &mut self.materialization_escrows {
            let _ = escrow.release_all(heap);
        }
        while let Some(root) = self.cleanup_roots.last().copied() {
            let released = if root.kind() == Some(ValueKind::ResourceRef) {
                heap.release_resource(&root)
            } else {
                heap.release_snapshot(&root)
            };
            if released.is_err() {
                // The concrete heap remains the final owner and is dropped
                // immediately after this retention carrier. Never discard the
                // slot before a successful explicit release.
                break;
            }
            self.cleanup_roots.pop();
        }
    }
}

impl VmRootSource for BytecodeRequestRetention {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        if let Some(owner) = &self.scheduler_failure_owner {
            owner.visit_roots(visitor)?;
        }
        if let Some(cause) = &self.terminal_cause {
            cause.visit_roots(visitor)?;
        }
        if let Some(escrow) = &self.terminal_escrow {
            escrow.visit_roots(visitor)?;
        }
        for escrow in &self.materialization_escrows {
            escrow.visit_roots(visitor)?;
        }
        for root in &self.cleanup_roots {
            visitor.visit_root(root)?;
        }
        Ok(())
    }
}

/// The result of the sole synchronous Phase 1 bytecode request drive.
///
/// `retention` is opaque and holds the heap/budget carriers; `owner_inventory`
/// reports the actual frozen owner inventory, `NotStarted` when the start
/// phase failed before any drive and `Started` once the single drive ran.
#[must_use]
pub struct DrivenBytecodeRequest {
    pub result: RequestResult<BoundaryResponse>,
    pub retention: BytecodeRequestRetention,
    pub owner_inventory: DrivenBytecodeRequestOwnerInventory,
}

pub enum DrivenBytecodeRequestOwnerInventory {
    NotStarted(RequestExecutionOwnerInventorySnapshot),
    Started(RequestExecutionOwnerInventorySnapshot),
}

impl DrivenBytecodeRequestOwnerInventory {
    pub fn into_snapshot(self) -> RequestExecutionOwnerInventorySnapshot {
        match self {
            Self::NotStarted(snapshot) | Self::Started(snapshot) => snapshot,
        }
    }
}

/// Suspended invocation chain of one parked bytecode request.
pub(super) type VmSuspended = SuspendedTrampoline<VmFiber, VmResumeToken>;

struct ServerStreamStart {
    handle: RequestResourceHandle,
    writer: SharedBytecodeServerStreamWriterPort,
}

/// Everything needed to project and retain one started request across any
/// number of park/resume cycles.
struct BytecodeStart {
    fiber: VmFiber,
    heap: Box<dyn VmHeap + Send>,
    budget: Box<dyn VmBudget + Send>,
    execution_budget: Arc<ExecutionBudget>,
    mode: String,
    raw_http_adapter: bool,
    http_client: Option<SharedBytecodeHttpClientPort>,
    server_stream: Option<ServerStreamStart>,
    execution_control: crate::OwnedExecutionControl,
}

fn start_bytecode_request(
    input: BytecodeRequestExecutionInput,
    resources: RequestResourceTable,
) -> RequestResult<BytecodeStart> {
    let BytecodeRequestExecutionInput {
        target,
        request,
        observer,
        cancellation,
        execution_budget,
        handles,
        http_client,
        server_stream_writer,
        child_composition: _,
        heap: injected_heap,
    } = input;

    let mode = request.mode.clone();
    let raw_http_adapter = request
        .http_adapter
        .as_ref()
        .is_some_and(|adapter| adapter.kind == HttpAdapterKind::RawHttp);
    let server_stream = validate_bytecode_request(
        &request,
        &target,
        server_stream_writer,
        &resources,
        handles.max_response_bytes,
    )?;
    let execution_control = ExecutionControl::new(cancellation, &execution_budget);
    execution_control
        .check_cancelled()
        .map_err(RequestError::from)?;
    let execution_control = execution_control.owned();
    let mut heap: Box<dyn VmHeap + Send> = match injected_heap {
        Some(heap) => heap,
        None => Box::new(RequestVmHeap::for_execution(
            resources,
            handles.request_heap_limits,
        )),
    };
    let arguments = gateway_entry_arguments(&request, &target, &mut *heap)?;
    let mut fiber = Vm::start(target, arguments.into_boxed_slice(), vm_limits(), observer)
        .map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
    fiber.set_error_correlation(bytecode_error_correlation(&request));
    let budget = execution_budget.attach_vm().map_err(|error| {
        RequestError::Decode(format!("bytecode VM budget attachment failed: {error}"))
    })?;
    Ok(BytecodeStart {
        fiber,
        heap,
        budget: Box::new(budget),
        execution_budget,
        mode,
        raw_http_adapter,
        http_client,
        server_stream,
        execution_control,
    })
}

/// The only public composition of one bytecode request on the production
/// seam: start the fiber exactly once, then drive with the deterministic
/// controlled completion.
///
/// A pinned `std.time.sleep` still executes the real park/publish/wake/claim/
/// resume chain; its deterministic completion is injected at the production
/// boundary instead of a real clock, so the drive never blocks or fakes a
/// synchronous `Ready`. A failure during the start phase returns an empty
/// retention carrier and a frozen `NotStarted` owner inventory.
pub fn drive_runtime_bytecode_request(
    input: BytecodeRequestExecutionInput,
) -> DrivenBytecodeRequest {
    let mut drive = drive_runtime_bytecode_request_controlled(input);
    loop {
        match drive {
            ControlledBytecodeDrive::Complete(driven) => return driven,
            ControlledBytecodeDrive::Parked(parked) => drive = parked.complete_and_resume(),
        }
    }
}

/// Async production driver for the Phase 4 pending lane.
///
/// The controlled driver still owns park/publish/wake/claim/resume. This
/// wrapper waits only for the request's shared runnable queue. Each typed
/// executor owns and settles its real future; the outer driver never guesses
/// that an arbitrary `Parked` operation is a sleep or injects an empty result.
pub async fn drive_runtime_bytecode_request_async(
    input: BytecodeRequestExecutionInput,
) -> DrivenBytecodeRequest {
    let mut drive = drive_runtime_bytecode_request_controlled(input);
    loop {
        match drive {
            ControlledBytecodeDrive::Complete(driven) => return driven,
            ControlledBytecodeDrive::Parked(parked) => {
                wait_for_pending_wake(Arc::clone(&parked.runtime)).await;
                drive = parked.resume_with_claimed_signal();
            }
        }
    }
}

/// Waits only on the shared pending runtime. Keeping the parked carrier out of
/// this future is essential: its unique VM heap, budget adapter and sync wake
/// receiver are `Send` owners but intentionally not `Sync` shared state.
async fn wait_for_pending_wake(runtime: Arc<RequestPendingRuntime>) {
    let cancellation = runtime.execution_control.cancellation_token();
    let deadline = runtime.execution_control.deadline();
    let deadline_wait = async move {
        match deadline {
            Some(deadline) => {
                tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            }
            None => std::future::pending::<()>().await,
        }
    };
    tokio::pin!(deadline_wait);
    tokio::select! {
        _ = runtime.wake_queue.wait() => return,
        _ = cancellation.wait_cancelled() => {
            let _ = runtime.budget.request_cancel();
        }
        _ = &mut deadline_wait => {
            let _ = runtime.budget.pending_terminal_winner();
        }
    }
    runtime.wake_queue.wait().await;
}

/// One controlled-drive step.
///
/// `Parked` carries the live request and its completion authority so a host
/// boundary can finish the pending effect before resuming; `Complete` is the
/// frozen terminal carrier shared with the synchronous lane.
#[must_use = "a controlled bytecode drive must be completed or resumed"]
pub enum ControlledBytecodeDrive {
    Complete(DrivenBytecodeRequest),
    Parked(ParkedBytecodeRequest),
}

/// Drives a bytecode request until its first actual-Pending park, a root
/// completion, or a terminal error, without injecting any completion.
///
/// This is the seam the Phase 4 proof gate uses to inject a fake host
/// completion at the production boundary: [`ParkedBytecodeRequest`] exposes
/// [`RequestPendingCompletion`], and [`ParkedBytecodeRequest::resume`] drains
/// exactly one wake and restores the original VM site.
pub fn drive_runtime_bytecode_request_controlled(
    input: BytecodeRequestExecutionInput,
) -> ControlledBytecodeDrive {
    let mut context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
    let input_child_composition = input.child_composition.clone();
    let observer = input.observer.clone();
    let start = match start_bytecode_request(input, context.resource_table()) {
        Ok(start) => start,
        Err(error) => {
            return ControlledBytecodeDrive::Complete(DrivenBytecodeRequest {
                result: Err(error),
                retention: BytecodeRequestRetention {
                    scheduler_failure_owner: None,
                    terminal_cause: None,
                    terminal_escrow: None,
                    materialization_escrows: Vec::new(),
                    budget: None,
                    cleanup_roots: Vec::new(),
                    heap: None,
                },
                owner_inventory: DrivenBytecodeRequestOwnerInventory::NotStarted(
                    context.into_not_started(),
                ),
            });
        }
    };

    let (wake_queue, wake_receiver) = RequestPendingWakeQueue::new();
    let resources = context.resource_table();
    let stream_registrar = BytecodeHttpStreamRegistrar::new(resources.clone());
    let runtime = Arc::new(RequestPendingRuntime {
        registry: Arc::new(RequestPendingRegistry::new(
            context
                .take_pending_registration()
                .expect("fresh request context"),
        )),
        wake_queue,
        budget: Arc::clone(&start.execution_budget),
        resources,
        http_client: start.http_client.clone(),
        execution_control: start.execution_control.clone(),
        stream_registrar,
        child_composition: input_child_composition.clone(),
        cleanup_roots: Mutex::new(Vec::new()),
        materialization_escrows: Mutex::new(Vec::new()),
        manual_sleep_completion: Mutex::new(None),
    });
    let child_heap_factory: Arc<dyn BytecodeChildHeapFactory> = input_child_composition
        .child_heap_factory
        .clone()
        .unwrap_or_else(|| {
            Arc::new(RequestChildHeapFactory::new(
                context.child_heap_registration(),
            ))
        });
    let stream_supervisor: Option<Arc<dyn BytecodeStreamSupervisor<VmFiber>>> =
        start.server_stream.as_ref().map(|stream| {
            Arc::new(BytecodeServerStreamSupervisor::new(
                Arc::clone(&runtime),
                stream.handle,
                Arc::clone(&stream.writer),
            )) as Arc<dyn BytecodeStreamSupervisor<VmFiber>>
        });
    let mut context = context.with_ports(BytecodeSchedulerPorts {
        child_executor: Some(Arc::new(BytecodeHostExecutor {
            runtime: Arc::clone(&runtime),
            child_composition: input_child_composition,
            child_heap_factory,
            observer,
        })),
        stream_supervisor,
    });

    let BytecodeStart {
        fiber,
        mut heap,
        mut budget,
        execution_budget,
        mode,
        raw_http_adapter,
        http_client: _,
        server_stream: _,
        execution_control: _,
    } = start;
    context.install_root(fiber);
    let outcome = context.start_drive(&mut *heap, &mut *budget);
    ParkedBytecodeRequest {
        context,
        heap,
        budget,
        execution_budget,
        runtime,
        wake_receiver,
        mode,
        raw_http_adapter,
    }
    .finish_drive(outcome)
}

#[derive(Clone, Copy, Debug)]
pub(super) struct HttpResultLayout {
    root_tag: CompactTypeTag,
    header_tag: CompactTypeTag,
    body_tag: CompactTypeTag,
    string_tag: CompactTypeTag,
}

pub(super) enum RequestPendingOutcome {
    Vm(ResumeOutcome),
    HttpRequest {
        layout: HttpResultLayout,
        result: Result<BytecodeHttpResponse, BytecodeHttpFailure>,
    },
    HttpStream {
        layout: HttpResultLayout,
        result: Result<BytecodeHttpStreamResponse, BytecodeHttpFailure>,
    },
    StreamNext {
        handle: RequestResourceHandle,
        item_type: TypeIndex,
        result: Result<Option<Vec<u8>>, RequestByteStreamFailure>,
    },
    ServerStreamFlush {
        reservation: RequestServerStreamReservation,
        result: Result<(), BytecodeServerStreamWriteFailure>,
    },
    Db {
        operation: LinkedDbOperation,
        child_heap: ChildHeapCarrier,
        result: Result<RuntimeValue, String>,
    },
}

impl VmRootSource for RequestPendingOutcome {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::Vm(outcome) => outcome.visit_roots(visitor),
            Self::HttpRequest { .. }
            | Self::HttpStream { .. }
            | Self::StreamNext { .. }
            | Self::ServerStreamFlush { .. } => Ok(()),
            Self::Db { child_heap, .. } => child_heap.visit_roots(visitor),
        }
    }
}

pub(super) type RequestPendingRegistry =
    PendingRegistry<VmResumeToken, VmSuspended, RequestPendingOutcome>;
type RequestCompletionHandle = CompletionHandle<VmResumeToken, VmSuspended, RequestPendingOutcome>;
type ClaimedRequestPendingWake =
    ClaimedPendingWakeGuard<VmResumeToken, VmSuspended, RequestPendingOutcome>;
pub(super) type RequestPendingWakeQueue =
    PendingWakeSignalQueue<VmResumeToken, VmSuspended, RequestPendingOutcome>;

/// Runtime-neutral runnable queue for claimed pending wakes.
///
/// Every wake stays root-enumerable while queued; `enqueue` also signals the
/// single parked-request receiver so a resume can drain exactly one wake.
pub(super) struct PendingWakeSignalQueue<R, S, O> {
    wakes: Mutex<Vec<PendingWake<R, S, O>>>,
    signal: mpsc::Sender<()>,
    async_signal: tokio::sync::Semaphore,
}

impl<R, S, O> PendingWakeSignalQueue<R, S, O> {
    fn new() -> (Arc<Self>, mpsc::Receiver<()>) {
        let (signal, receiver) = mpsc::channel();
        (
            Arc::new(Self {
                wakes: Mutex::new(Vec::new()),
                signal,
                async_signal: tokio::sync::Semaphore::new(0),
            }),
            receiver,
        )
    }

    fn claim(&self) -> Option<ClaimedPendingWakeGuard<R, S, O>> {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
            .map(PendingWake::claim)
    }

    async fn wait(&self) {
        self.async_signal
            .acquire()
            .await
            .expect("the request wake queue semaphore is never closed")
            .forget();
    }

    fn consume_async_signal_if_present(&self) {
        if let Ok(permit) = self.async_signal.try_acquire() {
            permit.forget();
        }
    }
}

impl<R, S, O> PendingWakeQueue<R, S, O> for PendingWakeSignalQueue<R, S, O>
where
    R: Send + 'static,
    S: Send + 'static,
    O: Send + 'static,
{
    fn enqueue(&self, wake: PendingWake<R, S, O>) {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(wake);
        let _ = self.signal.send(());
        self.async_signal.add_permits(1);
    }
}

impl<R, S, O> VmRootSource for PendingWakeSignalQueue<R, S, O>
where
    S: VmRootSource,
    O: VmRootSource,
{
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        for wake in self
            .wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
        {
            wake.visit_roots(visitor)?;
        }
        Ok(())
    }
}

/// Shared pending state of one request: the registry, the wake queue, the
/// authoritative budget and the most recently parked completion cell.
pub(super) struct RequestPendingRuntime {
    pub(super) registry: Arc<RequestPendingRegistry>,
    pub(super) wake_queue: Arc<RequestPendingWakeQueue>,
    pub(super) budget: Arc<ExecutionBudget>,
    pub(super) resources: RequestResourceTable,
    http_client: Option<SharedBytecodeHttpClientPort>,
    #[allow(dead_code)]
    pub(super) execution_control: crate::OwnedExecutionControl,
    #[allow(dead_code)]
    stream_registrar: BytecodeHttpStreamRegistrar,
    #[allow(dead_code)]
    child_composition: BytecodeRequestChildComposition,
    /// Owners whose explicit release failed during synchronous host-result
    /// materialization. No pending or GC safepoint intervenes before terminal
    /// request retention takes this escrow.
    cleanup_roots: Mutex<Vec<ValueSlot>>,
    /// Exact or explicitly damaged owners rejected by the VM resume-token
    /// binder. These carriers never fall back to runtime-kind cleanup and are
    /// moved into request retention before the request heap can be destroyed.
    materialization_escrows: Mutex<Vec<VmTerminalEscrow>>,
    /// Deterministic Phase 4 regression authority. Only typed Sleep may
    /// install a handle here; HTTP and StreamNext are host-owned futures and
    /// can never accept an injected empty result.
    manual_sleep_completion: Mutex<Option<RequestCompletionHandle>>,
}

/// Converts the budget's single authoritative winner into the exact pending
/// cell settlement. The cell arbiter drops every duplicate, so this path can
/// never produce a second terminal.
fn complete_cell_from_winner(
    completion: &RequestCompletionHandle,
    winner: ExecutionWinner,
) -> skiff_runtime_scheduler::SettleDisposition<RequestPendingOutcome> {
    let outcome = RequestPendingOutcome::Vm(resume_outcome_from_winner(winner));
    match winner {
        ExecutionWinner::DeadlineExceeded => completion.deadline(outcome),
        ExecutionWinner::Cancelled => completion.cancel(outcome),
        _ => completion.internal_stop(outcome),
    }
}

fn resume_outcome_from_winner(winner: ExecutionWinner) -> ResumeOutcome {
    ResumeOutcome::InternalTerminal(match winner {
        ExecutionWinner::DeadlineExceeded => {
            VmInternalTerminal::Budget(VmBudgetClosed::DeadlineExceeded)
        }
        ExecutionWinner::InstructionLimitExceeded => {
            VmInternalTerminal::Budget(VmBudgetClosed::InstructionLimitExceeded)
        }
        ExecutionWinner::AccountingFailure => {
            VmInternalTerminal::Budget(VmBudgetClosed::AccountingFailure)
        }
        ExecutionWinner::Cancelled
        | ExecutionWinner::InternalStop
        | ExecutionWinner::Succeeded
        | ExecutionWinner::Failed => VmInternalTerminal::OwnerStopped,
    })
}

struct PendingCellSink {
    completion: RequestCompletionHandle,
}

impl RequestPendingSink for PendingCellSink {
    fn on_terminal(&self, winner: ExecutionWinner) {
        let _ = complete_cell_from_winner(&self.completion, winner);
    }
}

#[derive(Debug)]
struct FirstPollWake;

impl Wake for FirstPollWake {
    fn wake(self: Arc<Self>) {}
}

/// Owner-returning failure to create an actual-pending operation.
///
/// Construction is sealed in this module so every pre-publication error must
/// return the exact continuation it did not consume. Scheduler ports convert
/// this carrier to `BytecodePortFailure::Continuation` instead of flattening
/// the reason and losing the resume token.
#[must_use = "a failed pending begin still owns its exact resume continuation"]
pub(super) struct BeginPendingFailure {
    reason: BytecodeSchedulerError,
    resume: VmResumeToken,
}

impl BeginPendingFailure {
    fn new(reason: BytecodeSchedulerError, resume: VmResumeToken) -> Self {
        Self { reason, resume }
    }

    pub(super) fn into_parts(self) -> (BytecodeSchedulerError, VmResumeToken) {
        (self.reason, self.resume)
    }
}

impl std::fmt::Debug for BeginPendingFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BeginPendingFailure")
            .field("reason", &self.reason)
            .finish_non_exhaustive()
    }
}

impl VmRootSource for BeginPendingFailure {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        self.reason.visit_roots(visitor)
    }
}

pub(super) fn poll_future_once<F>(future: Pin<&mut F>) -> Poll<F::Output>
where
    F: Future + ?Sized,
{
    let waker = Waker::from(Arc::new(FirstPollWake));
    let mut context = Context::from_waker(&waker);
    future.poll(&mut context)
}

/// Exhaustive executor over the closed linked host-effect identity set.
struct BytecodeHostExecutor {
    runtime: Arc<RequestPendingRuntime>,
    child_composition: BytecodeRequestChildComposition,
    child_heap_factory: Arc<dyn BytecodeChildHeapFactory>,
    observer: BytecodeExecutionObserver,
}

enum HostArgumentUseFailure {
    Prepared(BytecodeSchedulerError),
    Release {
        primary: Option<BytecodeSchedulerError>,
        failure: VmHostEffectArgumentsReleaseError,
    },
}

fn finish_host_argument_use<T>(
    prepared: Result<T, BytecodeSchedulerError>,
    arguments: VmHostEffectArguments,
    heap: &mut dyn VmHeap,
) -> Result<T, HostArgumentUseFailure> {
    let released = arguments.release(heap);
    match (prepared, released) {
        (Ok(prepared), Ok(())) => Ok(prepared),
        (Err(error), Ok(())) => Err(HostArgumentUseFailure::Prepared(error)),
        (prepared, Err(failure)) => Err(HostArgumentUseFailure::Release {
            primary: prepared.err(),
            failure,
        }),
    }
}

impl RequestPendingRuntime {
    fn take_cleanup_roots(&self) -> Vec<ValueSlot> {
        std::mem::take(
            &mut *self
                .cleanup_roots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    fn take_materialization_escrows(&self) -> Vec<VmTerminalEscrow> {
        std::mem::take(
            &mut *self
                .materialization_escrows
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    pub(super) fn begin_pending<T, F, M>(
        &self,
        resume: VmResumeToken,
        future: Pin<Box<F>>,
        allow_manual_sleep_completion: bool,
        map: M,
    ) -> Result<PendingOperation, BeginPendingFailure>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static + ?Sized,
        M: FnOnce(T) -> RequestPendingOutcome + Send + 'static,
    {
        self.begin_pending_with_policy(
            resume,
            future,
            allow_manual_sleep_completion,
            PendingFutureTerminalPolicy::DropFuture,
            map,
        )
    }

    /// Begins the sole pending cell for a server-response flush whose first
    /// poll already enqueued an irrevocable Router frame.
    ///
    /// Cancellation or deadline still settles this same cell immediately,
    /// but the one task retains the transport future until its real ACK. The
    /// late heap-free outcome can then only lose as a duplicate; it cannot
    /// install another wake or mutate the already-terminal resource table.
    pub(super) fn begin_server_stream_pending(
        &self,
        resume: VmResumeToken,
        future: BytecodeServerStreamWriteFuture,
        reservation: RequestServerStreamReservation,
    ) -> Result<PendingOperation, BeginPendingFailure> {
        self.begin_pending_with_policy(
            resume,
            future,
            false,
            PendingFutureTerminalPolicy::AwaitAckAfterTerminal,
            move |result| RequestPendingOutcome::ServerStreamFlush {
                reservation,
                result,
            },
        )
    }

    fn begin_pending_with_policy<T, F, M>(
        &self,
        resume: VmResumeToken,
        future: Pin<Box<F>>,
        allow_manual_sleep_completion: bool,
        terminal_policy: PendingFutureTerminalPolicy,
        map: M,
    ) -> Result<PendingOperation, BeginPendingFailure>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static + ?Sized,
        M: FnOnce(T) -> RequestPendingOutcome + Send + 'static,
    {
        let spawner = match tokio::runtime::Handle::try_current() {
            Ok(spawner) => spawner,
            Err(_) => {
                return Err(BeginPendingFailure::new(
                    BytecodeSchedulerError::Port(
                        "actual-Pending host effect requires the current request Tokio runtime"
                            .to_string(),
                    ),
                    resume,
                ));
            }
        };
        let completion = match self
            .registry
            .begin_with_resource_roots(RootEscrow::empty(), self.resources.root_pin())
        {
            Ok(completion) => completion,
            Err(error) => {
                return Err(BeginPendingFailure::new(
                    BytecodeSchedulerError::Port(error.to_string()),
                    resume,
                ));
            }
        };
        let sink: Arc<dyn RequestPendingSink> = Arc::new(PendingCellSink {
            completion: completion.clone(),
        });
        let winner = self.budget.register_pending_sink(sink);
        *self
            .manual_sleep_completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            allow_manual_sleep_completion.then(|| completion.clone());
        let terminal_already_won = winner.is_some();
        if let Some(winner) = winner {
            let _ = complete_cell_from_winner(&completion, winner);
        }
        if terminal_already_won && terminal_policy == PendingFutureTerminalPolicy::DropFuture {
            drop(future);
        } else {
            let budget = Arc::clone(&self.budget);
            let execution_control = self.execution_control.clone();
            let completion_for_task = completion.clone();
            spawner.spawn(async move {
                let mut future = future;
                let cancellation = execution_control.cancellation_token();
                let deadline = execution_control.deadline();
                let deadline_wait = async move {
                    match deadline {
                        Some(deadline) => {
                            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline))
                                .await;
                        }
                        None => std::future::pending::<()>().await,
                    }
                };
                tokio::pin!(deadline_wait);
                let output = if terminal_already_won {
                    Some(future.as_mut().await)
                } else {
                    match terminal_policy {
                        PendingFutureTerminalPolicy::DropFuture => tokio::select! {
                            output = future.as_mut() => Some(output),
                            _ = cancellation.wait_cancelled() => {
                                let _ = budget.request_cancel();
                                None
                            }
                            _ = &mut deadline_wait => {
                                let _ = budget.pending_terminal_winner();
                                None
                            }
                        },
                        PendingFutureTerminalPolicy::AwaitAckAfterTerminal => {
                            Some(tokio::select! {
                                output = future.as_mut() => output,
                                _ = cancellation.wait_cancelled() => {
                                    let _ = budget.request_cancel();
                                    future.as_mut().await
                                }
                                _ = &mut deadline_wait => {
                                    let _ = budget.pending_terminal_winner();
                                    future.as_mut().await
                                }
                            })
                        }
                    }
                };
                let Some(output) = output else {
                    return;
                };
                let outcome = map(output);
                if request_pending_outcome_is_cancelled(&outcome) {
                    let _ = budget.request_cancel();
                } else if execution_control
                    .cancelled()
                    .load(std::sync::atomic::Ordering::Acquire)
                {
                    let _ = budget.request_cancel();
                }
                if budget.pending_terminal_winner().is_some() {
                    let disposition = completion_for_task.complete(outcome);
                    debug_assert!(matches!(
                        disposition,
                        skiff_runtime_scheduler::SettleDisposition::Duplicate(_)
                    ));
                } else {
                    let _ = completion_for_task.complete(outcome);
                }
            });
        }
        Ok(resume.into_pending(completion.ticket()))
    }

    pub(super) fn ready_terminal(&self) -> Option<ResumeOutcome> {
        if self
            .execution_control
            .cancelled()
            .load(std::sync::atomic::Ordering::Acquire)
        {
            let _ = self.budget.request_cancel();
        }
        self.budget
            .pending_terminal_winner()
            .map(resume_outcome_from_winner)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingFutureTerminalPolicy {
    DropFuture,
    AwaitAckAfterTerminal,
}

impl BytecodeHostExecutor {
    fn ready_adapter(
        resume: VmResumeToken,
        outcome: ResumeOutcome,
    ) -> BytecodeAdapterHandoff<VmFiber> {
        BytecodeAdapterHandoff::Ready(BytecodeHandoff { resume, outcome })
    }

    fn execute_db_child(
        &self,
        invocation: ChildInvocation,
        heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeChildHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>>
    {
        let ChildTarget::Db(index) = invocation.target() else {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::UnsupportedChild,
                invocation,
            ));
        };
        let image = Arc::clone(invocation.resume().image());
        let operation = match image
            .intrinsics()
            .get(usize::try_from(index.get()).unwrap_or(usize::MAX))
            .filter(|row| row.index() == index)
            .and_then(|row| row.db_operation().cloned())
        {
            Some(operation) => operation,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "DB intrinsic table row is absent or has no linked operation".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        if let Err(message) = require_db_operation(&operation) {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(message),
                invocation,
            ));
        }
        if invocation.arguments().values().len() != 1 || invocation.stream_endpoint().is_some() {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "DB intrinsic child must carry exactly one argument and no stream endpoint"
                        .to_string(),
                ),
                invocation,
            ));
        }
        let mut db_composition = self.child_composition.db_child.clone();
        if db_composition.exact_target.is_none() {
            db_composition.exact_target = Some(linked_db_target(&operation));
        }
        if !db_composition.is_available() {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "DB intrinsic child composition is not available; exact target, capability or recoverable context is missing"
                        .to_string(),
                ),
                invocation,
            ));
        }
        let caller_vm = match heap
            .as_any()
            .and_then(|heap| heap.downcast_ref::<RequestVmHeap>())
        {
            Some(heap) => heap,
            None => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(
                        "DB intrinsic caller heap is not a request VM heap".to_string(),
                    ),
                    invocation,
                ));
            }
        };
        let argument = invocation.arguments().values()[0];
        let argument_runtime = match caller_vm.runtime_value_for_slot(&argument) {
            Ok(value) => value,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Vm(VmError::Heap(error)),
                    invocation,
                ));
            }
        };
        let mut child_heap = match self.child_heap_factory.create_child_heap(
            image.owner(),
            self.child_composition.heap_limits.clone(),
            self.runtime.resources.clone(),
            Arc::clone(&self.child_composition.memory_ledger),
        ) {
            Ok(heap) => heap,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(format!("DB child heap creation failed: {error}")),
                    invocation,
                ));
            }
        };
        let child_runtime = {
            let child_vm = match child_heap
                .heap_mut()
                .as_any_mut()
                .and_then(|heap| heap.downcast_mut::<RequestVmHeap>())
            {
                Some(heap) => heap,
                None => {
                    return Err(BytecodePortFailure::input(
                        BytecodeSchedulerError::Port(
                            "DB child heap is not a request VM heap".to_string(),
                        ),
                        invocation,
                    ));
                }
            };
            match skiff_runtime_model::request_heap::deep_clone_runtime_value_between_heaps(
                caller_vm.request_heap(),
                child_vm.request_heap_mut(),
                &argument_runtime,
            ) {
                Ok(value) => value,
                Err(error) => {
                    return Err(BytecodePortFailure::input(
                        BytecodeSchedulerError::Port(format!(
                            "DB argument materialization failed: {error}"
                        )),
                        invocation,
                    ));
                }
            }
        };

        let (_target, arguments, _endpoint, resume) = invocation.into_parts();
        let mut argument_escrow = arguments.into_terminal_escrow();
        if let Err(error) = argument_escrow.release_all(heap) {
            return Err(BytecodePortFailure::continuation(
                BytecodeSchedulerError::Vm(error),
                resume,
            ));
        }

        let ledger = Arc::clone(&self.child_composition.memory_ledger);
        let future = async move {
            let result = async {
                let mut session = match db_composition.begin_transaction(ledger.as_ref()).await {
                    Ok(session) => session,
                    Err(error) => return Err(format!("DB transaction begin failed: {error}")),
                };
                let prepared = {
                    let child_vm = match child_heap
                        .heap_mut()
                        .as_any_mut()
                        .and_then(|heap| heap.downcast_mut::<RequestVmHeap>())
                    {
                        Some(heap) => heap,
                        None => {
                            return Err("DB child heap is not a request VM heap".to_string());
                        }
                    };
                    match session.prepared_create(child_vm.request_heap_mut(), &child_runtime) {
                        Ok(prepared) => prepared,
                        Err(error) => {
                            session.abort().await;
                            return Err(format!("DB create preparation failed: {error}"));
                        }
                    }
                };
                let finalizer = match prepared.into_wait().await {
                    Ok(finalizer) => finalizer,
                    Err(error) => {
                        session.abort().await;
                        return Err(format!("DB create wait failed: {error}"));
                    }
                };
                let created = {
                    let child_vm = match child_heap
                        .heap_mut()
                        .as_any_mut()
                        .and_then(|heap| heap.downcast_mut::<RequestVmHeap>())
                    {
                        Some(heap) => heap,
                        None => {
                            return Err("DB child heap is not a request VM heap".to_string());
                        }
                    };
                    match finalizer.finalize(child_vm.request_heap_mut()) {
                        Ok(value) => value,
                        Err(error) => {
                            session.abort().await;
                            return Err(format!("DB create finalization failed: {error}"));
                        }
                    }
                };
                if let Err(error) = session.commit().await {
                    session.abort().await;
                    return Err(format!("DB commit failed: {error}"));
                }
                Ok(created)
            }
            .await;
            (result, child_heap)
        };

        self.runtime
            .begin_pending(
                resume,
                Box::pin(future),
                false,
                move |(result, child_heap)| RequestPendingOutcome::Db {
                    operation,
                    child_heap,
                    result,
                },
            )
            .map(BytecodeChildHandoff::Pending)
            .map_err(|failure| {
                let (reason, resume) = failure.into_parts();
                BytecodePortFailure::continuation(reason, resume)
            })
    }
}

impl BytecodeChildExecutor<VmFiber> for BytecodeHostExecutor {
    fn execute_child(
        &self,
        invocation: skiff_runtime_vm::ChildInvocation,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<
        BytecodeChildHandoff<VmFiber>,
        BytecodePortFailure<skiff_runtime_vm::ChildInvocation, VmResumeToken>,
    > {
        match BytecodeChildLane::for_target(invocation.target()) {
            BytecodeChildLane::Service => execute_service_child(
                invocation,
                heap,
                budget,
                &self.child_composition,
                Arc::clone(&self.child_heap_factory),
                self.runtime.resources.clone(),
                self.observer.clone(),
                vm_limits(),
            ),
            BytecodeChildLane::Interface => execute_interface_child(
                invocation,
                heap,
                budget,
                &self.child_composition,
                Arc::clone(&self.child_heap_factory),
                self.runtime.resources.clone(),
                self.observer.clone(),
                vm_limits(),
            ),
            BytecodeChildLane::Actor => execute_actor_child(
                invocation,
                heap,
                budget,
                &self.child_composition.actor_child,
                Arc::clone(&self.child_heap_factory),
                self.runtime.resources.clone(),
                self.observer.clone(),
                vm_limits(),
            ),
            BytecodeChildLane::Task => {
                execute_task_child(invocation, heap, budget, &self.child_composition.task_child)
            }
            BytecodeChildLane::Db => self.execute_db_child(invocation, heap, budget),
            BytecodeChildLane::Disabled => Err(BytecodePortFailure::input(
                BytecodeSchedulerError::UnsupportedChild,
                invocation,
            )),
        }
    }

    fn execute_adapter(
        &self,
        invocation: AdapterInvocation,
        heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<
        BytecodeAdapterHandoff<VmFiber>,
        BytecodePortFailure<AdapterInvocation, VmResumeToken>,
    > {
        let adapter_index = invocation.adapter();
        let image = Arc::clone(invocation.resume().image());
        let target = image
            .host_effect_target(adapter_index)
            .map(|target| (target.executor_identity(), target.signature().clone()));
        let Some((identity, signature)) = target else {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "pending host effect adapter row is absent from the pinned image".to_string(),
                ),
                invocation,
            ));
        };
        let (_adapter, arguments, resume) = invocation.into_parts();
        match identity {
            HostEffectExecutorIdentity::Sleep => {
                let prepared: Result<_, BytecodeSchedulerError> = (|| {
                    validate_native_arity(&signature, 1, 0)?;
                    let carrier = sleep_duration_carrier(&image, &signature)?;
                    let argument = arguments.values().first().ok_or_else(|| {
                        BytecodeSchedulerError::Port(
                            "typed sleep invocation is missing its duration".to_string(),
                        )
                    })?;
                    let millis = sleep_millis_from_vm_value(argument, carrier)?;
                    tokio::runtime::Handle::try_current().map_err(|_| {
                        BytecodeSchedulerError::Port(
                            "typed sleep requires the current request Tokio runtime".to_string(),
                        )
                    })?;
                    let mut future: Pin<Box<dyn Future<Output = ()> + Send>> =
                        Box::pin(async move {
                            tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
                        });
                    let first_poll = poll_future_once(future.as_mut());
                    Ok((future, first_poll))
                })();
                let (future, first_poll) = match finish_host_argument_use(prepared, arguments, heap)
                {
                    Ok(prepared) => prepared,
                    Err(HostArgumentUseFailure::Prepared(reason)) => {
                        return Err(BytecodePortFailure::continuation(reason, resume));
                    }
                    Err(HostArgumentUseFailure::Release { primary, failure }) => {
                        return Err(match primary {
                            Some(primary) => {
                                BytecodePortFailure::terminal_host_arguments_release_with_primary(
                                    primary, failure,
                                )
                            }
                            None => BytecodePortFailure::terminal_host_arguments_release(failure),
                        });
                    }
                };
                match first_poll {
                    Poll::Ready(()) => Ok(Self::ready_adapter(
                        resume,
                        self.runtime
                            .ready_terminal()
                            .unwrap_or(ResumeOutcome::Empty),
                    )),
                    Poll::Pending => self
                        .runtime
                        .begin_pending(resume, future, true, |_| {
                            RequestPendingOutcome::Vm(ResumeOutcome::Empty)
                        })
                        .map(BytecodeAdapterHandoff::Pending)
                        .map_err(|failure| {
                            let (reason, resume) = failure.into_parts();
                            BytecodePortFailure::continuation(reason, resume)
                        }),
                }
            }
            HostEffectExecutorIdentity::HttpClientRequest => {
                let prepared: Result<_, BytecodeSchedulerError> = (|| {
                    let provider = self.runtime.http_client.clone().ok_or_else(|| {
                        BytecodeSchedulerError::Port(
                            "typed bytecode HTTP provider is unavailable".to_string(),
                        )
                    })?;
                    let (request, string_type) =
                        decode_http_request(&image, &signature, arguments.values(), heap)?;
                    let layout =
                        http_result_layout(&image, &signature, &resume, string_type, false)?;
                    let mut future =
                        provider.request(request, self.runtime.execution_control.clone());
                    let first_poll = poll_future_once(future.as_mut());
                    Ok((layout, future, first_poll))
                })();
                let (layout, future, first_poll) =
                    match finish_host_argument_use(prepared, arguments, heap) {
                        Ok(prepared) => prepared,
                        Err(HostArgumentUseFailure::Prepared(reason)) => {
                            return Err(BytecodePortFailure::continuation(reason, resume));
                        }
                        Err(HostArgumentUseFailure::Release { primary, failure }) => {
                            return Err(match primary {
                            Some(primary) => {
                                BytecodePortFailure::terminal_host_arguments_release_with_primary(
                                    primary, failure,
                                )
                            }
                            None => BytecodePortFailure::terminal_host_arguments_release(failure),
                        });
                        }
                    };
                match first_poll {
                    Poll::Ready(result) => {
                        if matches!(&result, Err(BytecodeHttpFailure::Cancelled)) {
                            let _ = self.runtime.budget.request_cancel();
                        }
                        let outcome = self.runtime.ready_terminal().unwrap_or_else(|| {
                            materialize_http_request_outcome(
                                &resume,
                                layout,
                                result,
                                &self.runtime.cleanup_roots,
                                &self.runtime.materialization_escrows,
                                heap,
                            )
                        });
                        Ok(Self::ready_adapter(resume, outcome))
                    }
                    Poll::Pending => self
                        .runtime
                        .begin_pending(resume, future, false, move |result| {
                            RequestPendingOutcome::HttpRequest { layout, result }
                        })
                        .map(BytecodeAdapterHandoff::Pending)
                        .map_err(|failure| {
                            let (reason, resume) = failure.into_parts();
                            BytecodePortFailure::continuation(reason, resume)
                        }),
                }
            }
            HostEffectExecutorIdentity::HttpClientStream => {
                let prepared: Result<_, BytecodeSchedulerError> = (|| {
                    let provider = self.runtime.http_client.clone().ok_or_else(|| {
                        BytecodeSchedulerError::Port(
                            "typed bytecode HTTP provider is unavailable".to_string(),
                        )
                    })?;
                    let (request, string_type) =
                        decode_http_request(&image, &signature, arguments.values(), heap)?;
                    let layout =
                        http_result_layout(&image, &signature, &resume, string_type, true)?;
                    let mut future = provider.stream(
                        request,
                        self.runtime.execution_control.clone(),
                        self.runtime.stream_registrar.clone(),
                    );
                    let first_poll = poll_future_once(future.as_mut());
                    Ok((layout, future, first_poll))
                })();
                let (layout, future, first_poll) =
                    match finish_host_argument_use(prepared, arguments, heap) {
                        Ok(prepared) => prepared,
                        Err(HostArgumentUseFailure::Prepared(reason)) => {
                            return Err(BytecodePortFailure::continuation(reason, resume));
                        }
                        Err(HostArgumentUseFailure::Release { primary, failure }) => {
                            return Err(match primary {
                            Some(primary) => {
                                BytecodePortFailure::terminal_host_arguments_release_with_primary(
                                    primary, failure,
                                )
                            }
                            None => BytecodePortFailure::terminal_host_arguments_release(failure),
                        });
                        }
                    };
                match first_poll {
                    Poll::Ready(result) => {
                        if matches!(&result, Err(BytecodeHttpFailure::Cancelled)) {
                            let _ = self.runtime.budget.request_cancel();
                        }
                        let outcome = self.runtime.ready_terminal().unwrap_or_else(|| {
                            materialize_http_stream_outcome(
                                &resume,
                                &self.runtime.resources,
                                layout,
                                result,
                                &self.runtime.cleanup_roots,
                                &self.runtime.materialization_escrows,
                                heap,
                            )
                        });
                        Ok(Self::ready_adapter(resume, outcome))
                    }
                    Poll::Pending => self
                        .runtime
                        .begin_pending(resume, future, false, move |result| {
                            RequestPendingOutcome::HttpStream { layout, result }
                        })
                        .map(BytecodeAdapterHandoff::Pending)
                        .map_err(|failure| {
                            let (reason, resume) = failure.into_parts();
                            BytecodePortFailure::continuation(reason, resume)
                        }),
                }
            }
        }
    }

    fn park_child(
        &self,
        request: BytecodeParkRequest<VmFiber>,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeParkFailure<VmFiber>> {
        // Service children publish actual-Pending operations through the same
        // request pending registry as host adapters. The async provider-load
        // extension can return a Pending child handoff here without changing
        // the scheduler's request authority.
        self.park_adapter(request, heap, budget)
    }

    fn park_adapter(
        &self,
        request: BytecodeParkRequest<VmFiber>,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeParkFailure<VmFiber>> {
        let queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, RequestPendingOutcome>> =
            self.runtime.wake_queue.clone();
        let (operation, suspended) = request.into_parts();
        match self
            .runtime
            .registry
            .publish_operation_or_abandon(operation, suspended, queue)
        {
            Ok(_) => Ok(()),
            Err(error) => {
                let reason = BytecodeSchedulerError::Port(error.reason().to_string());
                Err(BytecodeParkFailure::pending_draft(
                    reason,
                    error.into_draft(),
                ))
            }
        }
    }

    fn execute_stream_next(
        &self,
        invocation: ChildInvocation,
        heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<VmFiber>, BytecodePortFailure<ChildInvocation, VmResumeToken>>
    {
        if !invocation.arguments().is_empty() {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "verified StreamNext invocation carried unexpected owned arguments".to_string(),
                ),
                invocation,
            ));
        }
        let Some(endpoint) = invocation.stream_endpoint() else {
            return Err(BytecodePortFailure::input(
                BytecodeSchedulerError::Port(
                    "StreamNext invocation is missing its exact endpoint route".to_string(),
                ),
                invocation,
            ));
        };
        let handle = match self.runtime.resources.validate_vm_route(endpoint.route()) {
            Ok(handle) => handle,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(error.to_string()),
                    invocation,
                ));
            }
        };
        let item_type = match stream_next_item_type(invocation.resume()) {
            Ok(item_type) => item_type,
            Err(reason) => return Err(BytecodePortFailure::input(reason, invocation)),
        };
        let mut future = match self.runtime.resources.start_byte_stream_pull(&handle) {
            Ok(future) => future,
            Err(error) => {
                return Err(BytecodePortFailure::input(
                    BytecodeSchedulerError::Port(error.to_string()),
                    invocation,
                ));
            }
        };
        let (_target, arguments, _endpoint, resume) = invocation.into_parts();
        debug_assert!(arguments.is_empty());
        match poll_future_once(future.as_mut()) {
            Poll::Ready(result) => {
                if matches!(&result, Err(RequestByteStreamFailure::Cancelled)) {
                    let _ = self.runtime.budget.request_cancel();
                }
                let outcome = self.runtime.ready_terminal().unwrap_or_else(|| {
                    materialize_stream_next_outcome(
                        &resume,
                        &self.runtime.resources,
                        handle,
                        item_type,
                        result,
                        &self.runtime.materialization_escrows,
                        heap,
                    )
                });
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    outcome,
                    resume,
                }))
            }
            Poll::Pending => self
                .runtime
                .begin_pending(resume, future, false, move |result| {
                    RequestPendingOutcome::StreamNext {
                        handle,
                        item_type,
                        result,
                    }
                })
                .map(BytecodeStreamHandoff::Pending)
                .map_err(|failure| {
                    let (reason, resume) = failure.into_parts();
                    BytecodePortFailure::continuation(reason, resume)
                }),
        }
    }

    fn park_stream_next(
        &self,
        request: BytecodeParkRequest<VmFiber>,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeParkFailure<VmFiber>> {
        let queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, RequestPendingOutcome>> =
            self.runtime.wake_queue.clone();
        let (operation, suspended) = request.into_parts();
        match self
            .runtime
            .registry
            .publish_operation_or_abandon(operation, suspended, queue)
        {
            Ok(_) => Ok(()),
            Err(error) => {
                let reason = BytecodeSchedulerError::Port(error.reason().to_string());
                Err(BytecodeParkFailure::pending_draft(
                    reason,
                    error.into_draft(),
                ))
            }
        }
    }
}

fn request_pending_outcome_is_cancelled(outcome: &RequestPendingOutcome) -> bool {
    matches!(
        outcome,
        RequestPendingOutcome::HttpRequest {
            result: Err(BytecodeHttpFailure::Cancelled),
            ..
        } | RequestPendingOutcome::HttpStream {
            result: Err(BytecodeHttpFailure::Cancelled),
            ..
        } | RequestPendingOutcome::StreamNext {
            result: Err(RequestByteStreamFailure::Cancelled),
            ..
        } | RequestPendingOutcome::ServerStreamFlush {
            result: Err(BytecodeServerStreamWriteFailure::Cancelled),
            ..
        }
    )
}

fn validate_native_arity(
    signature: &LinkedNativeCallableSignature,
    parameters: usize,
    results: usize,
) -> Result<(), BytecodeSchedulerError> {
    if signature.parameter_types().len() != parameters || signature.result_types().len() != results
    {
        return Err(BytecodeSchedulerError::Port(format!(
            "typed host signature has {} parameters and {} results; expected {parameters} and {results}",
            signature.parameter_types().len(),
            signature.result_types().len(),
        )));
    }
    Ok(())
}

fn sleep_duration_carrier(
    image: &DeploymentExecutionImage,
    signature: &LinkedNativeCallableSignature,
) -> Result<LinkedRepresentationCarrier, BytecodeSchedulerError> {
    let [parameter] = signature.parameter_types() else {
        return Err(BytecodeSchedulerError::Port(
            "typed sleep target does not retain exactly one duration parameter".to_string(),
        ));
    };
    let entry = image
        .types()
        .get(parameter.get() as usize)
        .filter(|entry| entry.index() == *parameter)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "typed sleep duration type {} is absent from the verified image",
                parameter.get()
            ))
        })?;
    let carrier = entry.representation_carrier().copied().ok_or_else(|| {
        BytecodeSchedulerError::Port(
            "typed sleep duration lacks its compiler-owned representation carrier fact".to_string(),
        )
    })?;
    validate_builtin_type(image, carrier.representation_type(), "integer")?;
    validate_builtin_type(image, carrier.physical_carrier_type(), "number")?;
    Ok(carrier)
}

fn sleep_millis_from_vm_value(
    value: &ValueSlot,
    _carrier: LinkedRepresentationCarrier,
) -> Result<u64, BytecodeSchedulerError> {
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
    const MAX_SLEEP_MILLIS: u64 = 60_000;

    let value = value.as_number().ok_or_else(|| {
        BytecodeSchedulerError::Port(
            "typed sleep duration is not the exact number payload".to_string(),
        )
    })?;
    if !value.is_finite() || value.fract() != 0.0 {
        return Err(BytecodeSchedulerError::Port(
            "typed sleep duration is not an integer millisecond payload".to_string(),
        ));
    }
    if value.abs() > MAX_SAFE_INTEGER {
        return Err(BytecodeSchedulerError::Port(
            "typed sleep duration is not a safe integer millisecond payload".to_string(),
        ));
    }
    if value <= 0.0 {
        return Ok(0);
    }
    Ok((value as u64).min(MAX_SLEEP_MILLIS))
}

fn linked_type_ref<'a>(
    image: &'a DeploymentExecutionImage,
    ty: TypeIndex,
    label: &str,
) -> Result<&'a TypeRefIr, BytecodeSchedulerError> {
    image
        .types()
        .get(ty.get() as usize)
        .filter(|entry| entry.index() == ty)
        .map(|entry| entry.type_ref())
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "{label} references missing linked type {}",
                ty.get()
            ))
        })
}

pub(super) fn require_exact_slot_type_ref(
    image: &DeploymentExecutionImage,
    slot: &ValueSlot,
    expected: TypeIndex,
    label: &str,
) -> Result<TypeIndex, BytecodeSchedulerError> {
    let actual = required_slot_type(slot, label)?;
    let expected_ref = linked_type_ref(image, expected, label)?;
    let actual_ref = linked_type_ref(image, actual, label)?;
    if actual_ref != expected_ref {
        return Err(BytecodeSchedulerError::Port(format!(
            "{label} linked type/ABI differs from the exact compiler-owned carrier"
        )));
    }
    Ok(actual)
}

pub(super) fn required_slot_type(
    slot: &ValueSlot,
    label: &str,
) -> Result<TypeIndex, BytecodeSchedulerError> {
    slot.compact_type_tag()
        .map(CompactTypeTag::type_index)
        .map(TypeIndex::new)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!("{label} has no linked concrete type/ABI tag"))
        })
}

fn scheduler_compact_type_tag(
    ty: TypeIndex,
    label: &str,
) -> Result<CompactTypeTag, BytecodeSchedulerError> {
    CompactTypeTag::try_from_type_index(ty.get()).ok_or_else(|| {
        BytecodeSchedulerError::Port(format!(
            "{label} linked type {} cannot be represented by a VM type tag",
            ty.get()
        ))
    })
}

fn request_compact_type_tag(ty: TypeIndex, label: &str) -> RequestResult<CompactTypeTag> {
    CompactTypeTag::try_from_type_index(ty.get()).ok_or_else(|| {
        RequestError::Decode(format!(
            "{label} linked type {} cannot be represented by a VM type tag",
            ty.get()
        ))
    })
}

fn heap_compact_type_tag(ty: TypeIndex) -> Result<CompactTypeTag, VmHeapError> {
    CompactTypeTag::try_from_type_index(ty.get()).ok_or(VmHeapError::InvalidValueMetadata)
}

pub(super) fn validate_record_carrier_fields(
    heap: &dyn VmHeap,
    record: &ValueSlot,
    expected: &[&str],
    label: &str,
) -> Result<(), BytecodeSchedulerError> {
    let container = heap
        .container_elements(record)
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    if container.shape != VmContainerShape::Record
        || container.elements.len() != expected.len()
        || !container
            .elements
            .iter()
            .zip(expected)
            .all(|(element, expected)| element.field.as_deref() == Some(*expected))
    {
        return Err(BytecodeSchedulerError::Port(format!(
            "{label} does not carry its exact compiler-owned record fields"
        )));
    }
    Ok(())
}

fn exact_std_http_symbol_abi<'a>(
    ty: &'a TypeRefIr,
    expected_path: &str,
    label: &str,
) -> Result<&'a str, BytecodeSchedulerError> {
    let TypeRefIr::PackageSymbol { symbol } = ty else {
        return Err(BytecodeSchedulerError::Port(format!(
            "{label} is not exact canonical {expected_path}"
        )));
    };
    if !matches!(
        &symbol.package,
        PackageRefIr::PackageId { package_id } if package_id == HTTP_BOUNDARY_PACKAGE_ID
    ) || symbol.symbol_path != expected_path
    {
        return Err(BytecodeSchedulerError::Port(format!(
            "{label} is not exact canonical {expected_path}"
        )));
    }
    symbol
        .abi_expectation
        .as_deref()
        .filter(|abi| !abi.is_empty())
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "{label} canonical {expected_path} type has no exact ABI"
            ))
        })
}

fn require_std_http_symbol_abi<'a>(
    image: &'a DeploymentExecutionImage,
    ty: TypeIndex,
    expected_path: &str,
    label: &str,
) -> Result<&'a str, BytecodeSchedulerError> {
    exact_std_http_symbol_abi(linked_type_ref(image, ty, label)?, expected_path, label)
}

fn require_same_http_abi(
    actual: &str,
    expected: &str,
    label: &str,
) -> Result<(), BytecodeSchedulerError> {
    if actual != expected {
        return Err(BytecodeSchedulerError::Port(format!(
            "{label} ABI differs from its canonical HTTP root ABI"
        )));
    }
    Ok(())
}

fn exact_http_header_element_type(
    image: &DeploymentExecutionImage,
    array_type: TypeIndex,
    expected_abi: &str,
    label: &str,
) -> Result<TypeIndex, BytecodeSchedulerError> {
    exact_std_http_array_element_type(image, array_type, expected_abi, HTTP_HEADER_TYPE, label)
}

fn exact_std_http_array_element_type(
    image: &DeploymentExecutionImage,
    array_type: TypeIndex,
    expected_abi: &str,
    expected_element: &str,
    label: &str,
) -> Result<TypeIndex, BytecodeSchedulerError> {
    let header_type = array_element_type(image, array_type)?;
    let array_ref = linked_type_ref(image, array_type, label)?;
    let header_ref = linked_type_ref(image, header_type, label)?;
    if !matches!(
        array_ref,
        TypeRefIr::Builtin { name, args }
            if name == "Array"
                && matches!(args.as_slice(), [argument] if argument == header_ref)
    ) {
        return Err(BytecodeSchedulerError::Port(format!(
            "{label} is not an exact linked Array carrier"
        )));
    }
    let header_abi = require_std_http_symbol_abi(image, header_type, expected_element, label)?;
    require_same_http_abi(header_abi, expected_abi, label)?;
    Ok(header_type)
}

pub(super) fn validate_shape_fields(
    shape: &LinkedShapeEntry,
    expected: &[&str],
) -> Result<(), BytecodeSchedulerError> {
    if shape.fields().len() != expected.len()
        || !shape
            .fields()
            .iter()
            .zip(expected)
            .all(|(field, expected)| field.name() == *expected)
    {
        return Err(BytecodeSchedulerError::Port(
            "typed host value does not match its exact verified dense field layout".to_string(),
        ));
    }
    Ok(())
}

pub(super) fn shape_field_type(
    shape: &LinkedShapeEntry,
    name: &str,
) -> Result<TypeIndex, BytecodeSchedulerError> {
    shape
        .fields()
        .iter()
        .find(|field| field.name() == name)
        .map(|field| field.ty())
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "typed host value is missing verified field {name:?}"
            ))
        })
}

pub(super) fn array_element_type(
    image: &DeploymentExecutionImage,
    array_type: TypeIndex,
) -> Result<TypeIndex, BytecodeSchedulerError> {
    image
        .types()
        .get(array_type.get() as usize)
        .filter(|entry| entry.index() == array_type)
        .and_then(|entry| entry.container_layout())
        .and_then(|layout| layout.element())
        .map(|element| element.ty())
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "typed host array type {} has no verified element layout",
                array_type.get()
            ))
        })
}

pub(super) fn validate_builtin_type(
    image: &DeploymentExecutionImage,
    ty: TypeIndex,
    expected: &str,
) -> Result<(), BytecodeSchedulerError> {
    let entry = image
        .types()
        .get(ty.get() as usize)
        .filter(|entry| entry.index() == ty)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "typed host value type {} is absent from the verified image",
                ty.get()
            ))
        })?;
    if !matches!(
        entry.type_ref(),
        TypeRefIr::Builtin { name, args } if name == expected && args.is_empty()
    ) {
        return Err(BytecodeSchedulerError::Port(format!(
            "typed host value type {} is not exact builtin {expected:?}",
            ty.get()
        )));
    }
    Ok(())
}

fn validate_byte_stream_type(
    image: &DeploymentExecutionImage,
    ty: TypeIndex,
) -> Result<(), BytecodeSchedulerError> {
    let entry = image
        .types()
        .get(ty.get() as usize)
        .filter(|entry| entry.index() == ty)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(format!(
                "typed host stream type {} is absent from the verified image",
                ty.get()
            ))
        })?;
    if !matches!(
        entry.type_ref(),
        TypeRefIr::Builtin { name, args }
            if name == "Stream"
                && matches!(
                    args.as_slice(),
                    [TypeRefIr::Builtin { name, args }]
                        if name == "bytes" && args.is_empty()
                )
    ) {
        return Err(BytecodeSchedulerError::Port(format!(
            "typed host stream type {} is not exact Stream<bytes>",
            ty.get()
        )));
    }
    Ok(())
}

fn stream_next_item_type(resume: &VmResumeToken) -> Result<TypeIndex, BytecodeSchedulerError> {
    let resume_site = resume
        .image()
        .resume_sites()
        .get(resume.resume_site())
        .filter(|resume_site| {
            resume_site.function() == resume.function()
                && resume_site.site() == resume.instruction()
        })
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "StreamNext resume token has no matching linked resume site".to_string(),
            )
        })?;
    let [item_type] = resume_site.result_types() else {
        return Err(BytecodeSchedulerError::Port(
            "StreamNext linked resume site does not carry exactly one item type".to_string(),
        ));
    };
    validate_builtin_type(resume.image(), *item_type, "bytes")?;
    Ok(*item_type)
}

fn decode_optional_http_body(
    heap: &mut dyn VmHeap,
    body: &ValueSlot,
) -> Result<Option<Vec<u8>>, BytecodeSchedulerError> {
    if body.is_null() {
        Ok(None)
    } else {
        heap.bytes_value(body)
            .map(Some)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
    }
}

fn decode_http_request(
    image: &DeploymentExecutionImage,
    signature: &LinkedNativeCallableSignature,
    arguments: &[ValueSlot],
    heap: &mut dyn VmHeap,
) -> Result<(BytecodeHttpRequest, TypeIndex), BytecodeSchedulerError> {
    validate_native_arity(signature, 1, 1)?;
    let request_type = signature.parameter_types()[0];
    let request_abi = require_std_http_symbol_abi(
        image,
        request_type,
        HTTP_CLIENT_REQUEST_TYPE,
        "typed HTTP request signature",
    )?;
    if !matches!(
        signature.parameter_plans(),
        [LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease,
        }]
    ) {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP request signature has no exact snapshot lifecycle plan".to_string(),
        ));
    }
    let request = arguments.first().ok_or_else(|| {
        BytecodeSchedulerError::Port("typed HTTP invocation is missing its request".to_string())
    })?;
    require_exact_slot_type_ref(image, request, request_type, "typed HTTP request")?;
    validate_record_carrier_fields(
        heap,
        request,
        &["body", "headers", "method", "timeoutMs", "url"],
        "typed HTTP request",
    )?;
    let method_slot = heap
        .record_field(request, "method")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let string_type = required_slot_type(&method_slot, "typed HTTP request method")?;
    validate_builtin_type(image, string_type, "string")?;
    let method = heap
        .string_value(&method_slot)
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let url_slot = heap
        .record_field(request, "url")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    require_exact_slot_type_ref(image, &url_slot, string_type, "typed HTTP request URL")?;
    let url = heap
        .string_value(&url_slot)
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let headers_value = heap
        .record_field(request, "headers")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let header_type = headers_value
        .compact_type_tag()
        .map(CompactTypeTag::type_index)
        .map(TypeIndex::new)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "typed HTTP headers have no linked element type/ABI tag".to_string(),
            )
        })?;
    let header_abi =
        require_std_http_symbol_abi(image, header_type, HTTP_HEADER_TYPE, "typed HTTP headers")?;
    require_same_http_abi(header_abi, request_abi, "typed HTTP headers")?;
    let header_count = heap
        .array_len(&headers_value)
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let mut headers = Vec::with_capacity(header_count);
    for index in 0..header_count {
        let header = heap
            .array_get(&headers_value, index)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        require_exact_slot_type_ref(image, &header, header_type, "typed HTTP header")?;
        validate_record_carrier_fields(heap, &header, &["name", "value"], "typed HTTP header")?;
        let name = heap
            .record_field(&header, "name")
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        require_exact_slot_type_ref(image, &name, string_type, "typed HTTP header name")?;
        let name = heap
            .string_value(&name)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let value = heap
            .record_field(&header, "value")
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        require_exact_slot_type_ref(image, &value, string_type, "typed HTTP header value")?;
        let value = heap
            .string_value(&value)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        headers.push(HttpNameValue { name, value });
    }
    let body_value = heap
        .record_field(request, "body")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let body = decode_optional_http_body(heap, &body_value)?;
    let timeout_value = heap
        .record_field(request, "timeoutMs")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let timeout_ms = if timeout_value.is_null() {
        None
    } else {
        Some(
            timeout_value
                .as_integer()
                .and_then(|value| u64::try_from(value).ok())
                .or_else(|| {
                    timeout_value.as_number().and_then(|value| {
                        const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
                        (value.is_finite()
                            && value.fract() == 0.0
                            && (0.0..=MAX_SAFE_INTEGER).contains(&value))
                        .then_some(value as u64)
                    })
                })
                .ok_or_else(|| {
                    BytecodeSchedulerError::Port(
                        "typed HTTP timeoutMs is not a non-negative integer".to_string(),
                    )
                })?,
        )
    };
    Ok((
        BytecodeHttpRequest {
            method,
            url,
            headers,
            body,
            timeout_ms,
        },
        string_type,
    ))
}

fn exact_http_result_shape<'a>(
    image: &'a DeploymentExecutionImage,
    signature: &LinkedNativeCallableSignature,
    resume: &VmResumeToken,
    stream: bool,
) -> Result<(&'a LinkedShapeEntry, TypeIndex), BytecodeSchedulerError> {
    let [root] = signature.result_types() else {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP result has no exact linked type".to_string(),
        ));
    };
    let [plan] = signature.result_plans() else {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP result has no exact lifecycle plan".to_string(),
        ));
    };
    let resume_site = image
        .resume_sites()
        .get(resume.resume_site())
        .filter(|resume_site| {
            resume_site.function() == resume.function()
                && resume_site.site() == resume.instruction()
        })
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "typed HTTP invocation has no matching linked resume site".to_string(),
            )
        })?;
    let [resume_type] = resume_site.result_types() else {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP resume site has no exact linked result type".to_string(),
        ));
    };
    let [resume_plan] = resume_site.result_plans() else {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP resume site has no exact linked result lifecycle plan".to_string(),
        ));
    };
    if linked_type_ref(image, *resume_type, "typed HTTP resume result")?
        != linked_type_ref(image, *root, "typed HTTP result")?
        || resume_plan != plan
    {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP resume result differs from its exact host signature".to_string(),
        ));
    }
    if stream {
        let LinkedValueTransferPlan::MoveOnly {
            drop: LinkedValueDropPlan::RecursiveShape { shape },
        } = plan
        else {
            return Err(BytecodeSchedulerError::Port(
                "typed HTTP stream result has no exact recursive lifecycle shape".to_string(),
            ));
        };
        let linked_shape = image
            .shapes()
            .get(shape.get() as usize)
            .filter(|entry| entry.index() == *shape)
            .ok_or_else(|| {
                BytecodeSchedulerError::Port(
                    "typed HTTP stream result references a missing lifecycle shape".to_string(),
                )
            })?;
        if linked_shape.plan() != plan
            || linked_shape.privileged_affine_composite()
                != Some(PrivilegedAffineCompositeIdentity::HttpClientStreamHandle)
            || linked_type_ref(
                image,
                linked_shape.nominal_type(),
                "typed HTTP stream result",
            )? != linked_type_ref(image, *root, "typed HTTP stream result")?
        {
            return Err(BytecodeSchedulerError::Port(
                "typed HTTP stream result lifecycle shape differs from its exact linked signature"
                    .to_string(),
            ));
        }
        return Ok((linked_shape, *resume_type));
    }

    if !matches!(
        plan,
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease,
        }
    ) {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP response result has no exact snapshot lifecycle plan".to_string(),
        ));
    }
    let [Some(LinkedResumeResultMaterialization::DenseRecord { shape })] =
        resume_site.result_materializations()
    else {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP response resume site has no exact dense materialization shape".to_string(),
        ));
    };
    let linked_shape = image
        .shapes()
        .get(shape.get() as usize)
        .filter(|entry| entry.index() == *shape)
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "typed HTTP response resume site references a missing materialization shape"
                    .to_string(),
            )
        })?;
    if linked_shape.privileged_affine_composite().is_some()
        || linked_shape.plan() != plan
        || linked_type_ref(
            image,
            linked_shape.nominal_type(),
            "typed HTTP response result",
        )? != linked_type_ref(image, *root, "typed HTTP response result")?
    {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP response materialization shape differs from its exact linked signature"
                .to_string(),
        ));
    }
    Ok((linked_shape, *resume_type))
}

fn http_result_layout(
    image: &DeploymentExecutionImage,
    signature: &LinkedNativeCallableSignature,
    resume: &VmResumeToken,
    string_type: TypeIndex,
    stream: bool,
) -> Result<HttpResultLayout, BytecodeSchedulerError> {
    validate_native_arity(signature, 1, 1)?;
    let request_abi = require_std_http_symbol_abi(
        image,
        signature.parameter_types()[0],
        HTTP_CLIENT_REQUEST_TYPE,
        "typed HTTP request signature",
    )?;
    let result_type = signature.result_types()[0];
    let result_abi = require_std_http_symbol_abi(
        image,
        result_type,
        if stream {
            HTTP_CLIENT_STREAM_HANDLE_TYPE
        } else {
            HTTP_CLIENT_RESPONSE_TYPE
        },
        "typed HTTP result signature",
    )?;
    require_same_http_abi(result_abi, request_abi, "typed HTTP request/result")?;
    let (shape, resume_type) = exact_http_result_shape(image, signature, resume, stream)?;
    validate_builtin_type(image, string_type, "string")?;
    validate_shape_fields(shape, &["body", "headers", "status"])?;
    let headers = shape_field_type(shape, "headers")?;
    let header =
        exact_http_header_element_type(image, headers, result_abi, "typed HTTP result headers")?;
    let status = shape_field_type(shape, "status")?;
    validate_builtin_type(image, status, "integer")?;
    let body = shape_field_type(shape, "body")?;
    if stream {
        validate_byte_stream_type(image, body)?;
    } else {
        validate_builtin_type(image, body, "bytes")?;
    }
    Ok(HttpResultLayout {
        // The materialization shape may legitimately retain a distinct
        // duplicate TypeRef row with the same exact ABI and lifecycle plan.
        // The value crosses this particular continuation, so its physical tag
        // must be the resume site's exact result TypeIndex, not the shape or
        // host-signature row.
        root_tag: scheduler_compact_type_tag(resume_type, "typed HTTP resume result root")?,
        header_tag: scheduler_compact_type_tag(header, "typed HTTP result header")?,
        body_tag: scheduler_compact_type_tag(body, "typed HTTP result body")?,
        string_tag: scheduler_compact_type_tag(string_type, "typed HTTP result string")?,
    })
}

fn allocate_http_headers(
    heap: &mut dyn VmHeap,
    layout: HttpResultLayout,
    headers: Vec<HttpNameValue>,
    cleanup_escrow: &Mutex<Vec<ValueSlot>>,
) -> Result<ValueSlot, VmHeapError> {
    let mut owned = Vec::with_capacity(headers.len());
    for header in headers {
        let checkpoint = owned.len();
        let name = retain_http_materialized_root(
            heap.alloc_typed_string(header.name, layout.string_tag, ValueFlags::new(0)),
            heap,
            &mut owned,
            cleanup_escrow,
        )?;
        let value = retain_http_materialized_root(
            heap.alloc_typed_string(header.value, layout.string_tag, ValueFlags::new(0)),
            heap,
            &mut owned,
            cleanup_escrow,
        )?;
        let record = heap.allocate_record(
            &[
                VmRecordField {
                    name: "name".to_string(),
                    value: name,
                },
                VmRecordField {
                    name: "value".to_string(),
                    value,
                },
            ],
            layout.header_tag,
            ValueFlags::new(0),
        );
        let record = match record {
            Ok(record) => record,
            Err(error) => {
                return Err(cleanup_http_materialized_roots(
                    heap,
                    &mut owned,
                    error,
                    cleanup_escrow,
                ));
            }
        };
        owned.truncate(checkpoint);
        owned.push(record);
    }
    let array = heap.allocate_array(
        &owned,
        // VM array carriers store their compiler-emitted element TypeIndex,
        // not the enclosing `Array<T>` row. This matches NewArrayBuilder and
        // lets duplicate exact array rows remain non-authoritative at runtime.
        layout.header_tag,
        ValueFlags::new(0),
    );
    match array {
        Ok(array) => {
            owned.clear();
            Ok(array)
        }
        Err(error) => Err(cleanup_http_materialized_roots(
            heap,
            &mut owned,
            error,
            cleanup_escrow,
        )),
    }
}

fn materialize_http_response_value(
    layout: HttpResultLayout,
    result: BytecodeHttpResponse,
    cleanup_escrow: &Mutex<Vec<ValueSlot>>,
    heap: &mut dyn VmHeap,
) -> Result<ValueSlot, VmHeapError> {
    let mut owned = Vec::with_capacity(2);
    let headers = match allocate_http_headers(heap, layout, result.headers, cleanup_escrow) {
        Ok(headers) => {
            owned.push(headers);
            headers
        }
        Err(error) => return Err(error),
    };
    let body = retain_http_materialized_root(
        heap.alloc_typed_bytes(result.body, layout.body_tag, ValueFlags::new(0)),
        heap,
        &mut owned,
        cleanup_escrow,
    )?;
    let record = heap.allocate_record(
        &[
            VmRecordField {
                name: "body".to_string(),
                value: body,
            },
            VmRecordField {
                name: "headers".to_string(),
                value: headers,
            },
            VmRecordField {
                name: "status".to_string(),
                value: ValueSlot::integer(i64::from(result.status)),
            },
        ],
        layout.root_tag,
        ValueFlags::new(0),
    );
    match record {
        Ok(record) => {
            owned.clear();
            Ok(record)
        }
        Err(error) => Err(cleanup_http_materialized_roots(
            heap,
            &mut owned,
            error,
            cleanup_escrow,
        )),
    }
}

fn materialize_http_stream_value(
    layout: HttpResultLayout,
    result: BytecodeHttpStreamResponse,
    resources: &RequestResourceTable,
    cleanup_escrow: &Mutex<Vec<ValueSlot>>,
    heap: &mut dyn VmHeap,
) -> Result<ValueSlot, VmHeapError> {
    let mut owned = Vec::with_capacity(2);
    let headers = match allocate_http_headers(heap, layout, result.headers, cleanup_escrow) {
        Ok(headers) => {
            owned.push(headers);
            headers
        }
        Err(error) => {
            return Err(release_unadmitted_http_route(
                resources,
                &result.body,
                error,
            ));
        }
    };
    let body =
        match heap.admit_resource_ref(result.body.vm_handle(), layout.body_tag, ValueFlags::new(0))
        {
            Ok(body) => {
                owned.push(body);
                body
            }
            Err(error) => {
                let error =
                    cleanup_http_materialized_roots(heap, &mut owned, error, cleanup_escrow);
                return Err(release_unadmitted_http_route(
                    resources,
                    &result.body,
                    error,
                ));
            }
        };
    let record = heap.allocate_record(
        &[
            VmRecordField {
                name: "body".to_string(),
                value: body,
            },
            VmRecordField {
                name: "headers".to_string(),
                value: headers,
            },
            VmRecordField {
                name: "status".to_string(),
                value: ValueSlot::integer(i64::from(result.status)),
            },
        ],
        layout.root_tag,
        ValueFlags::new(0),
    );
    match record {
        Ok(record) => {
            owned.clear();
            Ok(record)
        }
        Err(error) => Err(cleanup_http_materialized_roots(
            heap,
            &mut owned,
            error,
            cleanup_escrow,
        )),
    }
}

fn release_unadmitted_http_route(
    resources: &RequestResourceTable,
    handle: &RequestResourceHandle,
    primary: VmHeapError,
) -> VmHeapError {
    match resources.release(handle) {
        Ok(_) => primary,
        Err(error) => VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseResource,
            message: format!(
                "typed HTTP stream materialization failed: {primary}; route cleanup failed: {error}"
            ),
        },
    }
}

fn retain_http_materialized_root(
    result: Result<ValueSlot, VmHeapError>,
    heap: &mut dyn VmHeap,
    owned: &mut Vec<ValueSlot>,
    cleanup_escrow: &Mutex<Vec<ValueSlot>>,
) -> Result<ValueSlot, VmHeapError> {
    match result {
        Ok(value) => {
            owned.push(value);
            Ok(value)
        }
        Err(error) => Err(cleanup_http_materialized_roots(
            heap,
            owned,
            error,
            cleanup_escrow,
        )),
    }
}

fn cleanup_http_materialized_roots(
    heap: &mut dyn VmHeap,
    owned: &mut Vec<ValueSlot>,
    primary: VmHeapError,
    cleanup_escrow: &Mutex<Vec<ValueSlot>>,
) -> VmHeapError {
    let mut result = primary.clone();
    while let Some(root) = owned.last().copied() {
        let released = if root.kind() == Some(ValueKind::ResourceRef) {
            heap.release_resource(&root)
        } else {
            heap.release_snapshot(&root)
        };
        match released {
            Ok(()) => {
                owned.pop();
            }
            Err(error) => {
                result = VmHeapError::HeapOperationFailed {
                    operation: if root.kind() == Some(ValueKind::ResourceRef) {
                        VmHeapOperation::ReleaseResource
                    } else {
                        VmHeapOperation::ReleaseSnapshot
                    },
                    message: format!(
                        "typed HTTP value materialization failed: {primary}; owner cleanup failed: {error}; the failing owner and earlier roots remain escrowed"
                    ),
                };
                break;
            }
        }
    }
    if !owned.is_empty() {
        cleanup_escrow
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .append(owned);
    }
    result
}

fn materialize_http_request_outcome(
    resume: &VmResumeToken,
    layout: HttpResultLayout,
    result: Result<BytecodeHttpResponse, BytecodeHttpFailure>,
    cleanup_escrow: &Mutex<Vec<ValueSlot>>,
    terminal_escrows: &Mutex<Vec<VmTerminalEscrow>>,
    heap: &mut dyn VmHeap,
) -> ResumeOutcome {
    let result = match result {
        Ok(result) => result,
        Err(error) => return http_failure_outcome(error),
    };
    let materialized = materialize_http_response_value(layout, result, cleanup_escrow, heap);
    match materialized {
        Ok(value) => {
            materialize_resume_values(resume, vec![value].into_boxed_slice(), terminal_escrows)
        }
        Err(error) => ResumeOutcome::Failure(VmError::Heap(error)),
    }
}

fn materialize_http_stream_outcome(
    resume: &VmResumeToken,
    resources: &RequestResourceTable,
    layout: HttpResultLayout,
    result: Result<BytecodeHttpStreamResponse, BytecodeHttpFailure>,
    cleanup_escrow: &Mutex<Vec<ValueSlot>>,
    terminal_escrows: &Mutex<Vec<VmTerminalEscrow>>,
    heap: &mut dyn VmHeap,
) -> ResumeOutcome {
    let result = match result {
        Ok(result) => result,
        Err(error) => return http_failure_outcome(error),
    };
    let materialized =
        materialize_http_stream_value(layout, result, resources, cleanup_escrow, heap);
    match materialized {
        Ok(value) => {
            materialize_resume_values(resume, vec![value].into_boxed_slice(), terminal_escrows)
        }
        Err(error) => ResumeOutcome::Failure(VmError::Heap(error)),
    }
}

fn http_failure_outcome(error: BytecodeHttpFailure) -> ResumeOutcome {
    match error {
        BytecodeHttpFailure::Cancelled => {
            ResumeOutcome::InternalTerminal(VmInternalTerminal::OwnerStopped)
        }
        BytecodeHttpFailure::DeadlineExceeded => {
            ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
                code: "TimeoutError".to_string(),
                message: "HTTP request deadline exceeded".to_string(),
                status: None,
                details: None,
            }))
        }
        BytecodeHttpFailure::ResponseLimitExceeded {
            limit_bytes,
            received_bytes,
        } => ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
            code: "ResponseLimitExceeded".to_string(),
            message: "HTTP response exceeded the request response limit".to_string(),
            status: None,
            details: Some(serde_json::json!({
                "limitBytes": limit_bytes,
                "receivedBytes": received_bytes,
            })),
        })),
        BytecodeHttpFailure::Transport(error) | BytecodeHttpFailure::InvalidInput(error) => {
            ResumeOutcome::Failure(VmError::HostEffectFailure(error.payload()))
        }
        BytecodeHttpFailure::InvalidProviderContract(message) => {
            ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
                code: "InternalError".to_string(),
                message,
                status: None,
                details: None,
            }))
        }
    }
}

fn materialize_stream_next_outcome(
    resume: &VmResumeToken,
    resources: &RequestResourceTable,
    handle: RequestResourceHandle,
    item_type: TypeIndex,
    result: Result<Option<Vec<u8>>, RequestByteStreamFailure>,
    terminal_escrows: &Mutex<Vec<VmTerminalEscrow>>,
    heap: &mut dyn VmHeap,
) -> ResumeOutcome {
    match result {
        Ok(Some(bytes)) => match heap_compact_type_tag(item_type)
            .and_then(|tag| heap.alloc_typed_bytes(bytes, tag, ValueFlags::new(0)))
        {
            Ok(value) => {
                materialize_resume_values(resume, vec![value].into_boxed_slice(), terminal_escrows)
            }
            Err(error) => ResumeOutcome::Failure(VmError::Heap(error)),
        },
        Ok(None) => match resources.finish(&handle, RequestResourceFinishReason::Exhausted) {
            Ok(_) => ResumeOutcome::StreamEnd,
            Err(error) => resource_failure_outcome(error.to_string()),
        },
        Err(RequestByteStreamFailure::Cancelled) => {
            match resources.terminate(&handle, RequestResourceTermination::Cancelled) {
                Ok(_) => ResumeOutcome::InternalTerminal(VmInternalTerminal::OwnerStopped),
                Err(error) => resource_failure_outcome(error.to_string()),
            }
        }
        Err(RequestByteStreamFailure::Ordinary(error)) => {
            let outcome = ResumeOutcome::Failure(VmError::HostEffectFailure(error.payload()));
            match resources.finish(&handle, RequestResourceFinishReason::HostError) {
                Ok(_) => outcome,
                Err(error) => resource_failure_outcome(error.to_string()),
            }
        }
        Err(RequestByteStreamFailure::InvalidProviderContract(message)) => {
            let outcome = resource_failure_outcome(message);
            match resources.finish(&handle, RequestResourceFinishReason::HostError) {
                Ok(_) => outcome,
                Err(error) => resource_failure_outcome(error.to_string()),
            }
        }
    }
}

fn materialize_resume_values(
    resume: &VmResumeToken,
    values: Box<[ValueSlot]>,
    terminal_escrows: &Mutex<Vec<VmTerminalEscrow>>,
) -> ResumeOutcome {
    match VmOwnedValues::try_from_resume(resume, values) {
        Ok(values) => ResumeOutcome::Values(values),
        Err(rejected) => {
            let (error, escrow) = rejected.into_terminal_escrow();
            terminal_escrows
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(escrow);
            ResumeOutcome::Failure(error)
        }
    }
}

fn resource_failure_outcome(message: String) -> ResumeOutcome {
    ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
        code: "InternalError".to_string(),
        message,
        status: None,
        details: None,
    }))
}

fn materialize_request_pending_outcome(
    resume: &VmResumeToken,
    resources: &RequestResourceTable,
    outcome: RequestPendingOutcome,
    cleanup_escrow: &Mutex<Vec<ValueSlot>>,
    terminal_escrows: &Mutex<Vec<VmTerminalEscrow>>,
    heap: &mut dyn VmHeap,
) -> ResumeOutcome {
    match outcome {
        RequestPendingOutcome::Vm(outcome) => outcome,
        RequestPendingOutcome::HttpRequest { layout, result } => materialize_http_request_outcome(
            resume,
            layout,
            result,
            cleanup_escrow,
            terminal_escrows,
            heap,
        ),
        RequestPendingOutcome::HttpStream { layout, result } => materialize_http_stream_outcome(
            resume,
            resources,
            layout,
            result,
            cleanup_escrow,
            terminal_escrows,
            heap,
        ),
        RequestPendingOutcome::StreamNext {
            handle,
            item_type,
            result,
        } => materialize_stream_next_outcome(
            resume,
            resources,
            handle,
            item_type,
            result,
            terminal_escrows,
            heap,
        ),
        RequestPendingOutcome::ServerStreamFlush {
            reservation,
            result,
        } => materialize_server_stream_flush_outcome(resources, reservation, result),
        RequestPendingOutcome::Db {
            operation,
            child_heap,
            result,
        } => materialize_db_pending_outcome(
            resume,
            operation,
            child_heap,
            result,
            terminal_escrows,
            heap,
        ),
    }
}

fn materialize_db_pending_outcome(
    resume: &VmResumeToken,
    operation: LinkedDbOperation,
    mut child_heap: ChildHeapCarrier,
    result: Result<RuntimeValue, String>,
    terminal_escrows: &Mutex<Vec<VmTerminalEscrow>>,
    heap: &mut dyn VmHeap,
) -> ResumeOutcome {
    let parent = match heap
        .as_any_mut()
        .and_then(|heap| heap.downcast_mut::<RequestVmHeap>())
    {
        Some(parent) => parent,
        None => {
            return ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
                code: "DbOperationFailed".to_string(),
                message: "DB intrinsic result requires a request VM heap".to_string(),
                status: None,
                details: None,
            }));
        }
    };
    let value = match result {
        Ok(value) => value,
        Err(message) => {
            return ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
                code: "DbOperationFailed".to_string(),
                message,
                status: None,
                details: None,
            }));
        }
    };
    let child_vm = match child_heap
        .heap_mut()
        .as_any_mut()
        .and_then(|heap| heap.downcast_mut::<RequestVmHeap>())
    {
        Some(child_vm) => child_vm,
        None => {
            return ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
                code: "DbOperationFailed".to_string(),
                message: "DB intrinsic child heap is not a request VM heap".to_string(),
                status: None,
                details: None,
            }));
        }
    };
    let slot = match materialize_db_result_to_vm(
        parent,
        child_vm.request_heap(),
        resume.image(),
        &value,
        &operation,
    ) {
        Ok(slot) => slot,
        Err(message) => {
            return ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
                code: "DbOperationFailed".to_string(),
                message,
                status: None,
                details: None,
            }));
        }
    };
    match VmOwnedValues::try_from_db_intrinsic_resume(resume, Box::new([slot]), &operation) {
        Ok(values) => ResumeOutcome::Values(values),
        Err(rejected) => {
            let (error, escrow) = rejected.into_terminal_escrow();
            terminal_escrows
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(escrow);
            ResumeOutcome::Failure(error)
        }
    }
}

/// Cloneable authority to complete the most recently parked pending host
/// effect of one request.
#[derive(Clone)]
pub struct RequestPendingCompletion {
    runtime: Arc<RequestPendingRuntime>,
}

impl RequestPendingCompletion {
    /// Delivers the deterministic zero-result host completion exactly once.
    ///
    /// The request budget arbitrates first: a cancelled, stopped or
    /// deadline-exceeded request converts the completion into its single
    /// terminal instead of a host value. Returns `true` only when this call
    /// won the pending cell; a duplicate completion is dropped and returns
    /// `false`.
    pub fn complete(&self) -> bool {
        let Some(completion) = self
            .runtime
            .manual_sleep_completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        else {
            return false;
        };
        let disposition = match self.runtime.budget.pending_terminal_winner() {
            None => completion.complete(RequestPendingOutcome::Vm(ResumeOutcome::Empty)),
            Some(winner) => complete_cell_from_winner(&completion, winner),
        };
        matches!(
            disposition,
            skiff_runtime_scheduler::SettleDisposition::StoredBeforePublication
                | skiff_runtime_scheduler::SettleDisposition::Enqueued
        )
    }
}

/// A bytecode request parked on its first (or a later) actual pending effect.
///
/// The carrier retains the heap, budget, owner inventory and scheduler ports
/// until the request reaches its single terminal, so a cancel, deadline or
/// session stop can still settle the parked cell through the budget.
#[must_use = "a parked bytecode request must be completed and resumed"]
pub struct ParkedBytecodeRequest {
    context: RequestExecutionContext<VmFiber>,
    heap: Box<dyn VmHeap + Send>,
    budget: Box<dyn VmBudget + Send>,
    execution_budget: Arc<ExecutionBudget>,
    runtime: Arc<RequestPendingRuntime>,
    wake_receiver: mpsc::Receiver<()>,
    mode: String,
    raw_http_adapter: bool,
}

impl ParkedBytecodeRequest {
    /// The completion authority for the currently parked effect.
    pub fn pending_completion(&self) -> RequestPendingCompletion {
        RequestPendingCompletion {
            runtime: Arc::clone(&self.runtime),
        }
    }

    /// Completes the parked effect deterministically and resumes the request.
    pub fn complete_and_resume(self) -> ControlledBytecodeDrive {
        let parked = self;
        if !parked.pending_completion().complete() {
            // The synchronous legacy seam has no authority to fabricate an
            // HTTP/stream value. Stop the shared terminal cell instead of
            // injecting `Empty`; production uses the async queue driver.
            let _ = parked.runtime.budget.request_internal_stop();
        }
        parked.resume()
    }

    /// Resumes after the caller has already claimed the wake signal.
    fn resume_with_claimed_signal(self) -> ControlledBytecodeDrive {
        let _ = self.refresh_pending_terminal_winner();
        let _ = self.wake_receiver.try_recv();
        let wake = self
            .runtime
            .wake_queue
            .claim()
            .expect("a claimed pending wake queue must hold exactly one wake");
        self.resume_wake(wake)
    }

    /// Drains exactly one claimed wake and restores the original VM site.
    ///
    /// The restored scheduler runs once; a second park suspends the chain
    /// again and returns a fresh [`ControlledBytecodeDrive::Parked`].
    pub fn resume(self) -> ControlledBytecodeDrive {
        self.wake_receiver
            .recv()
            .expect("a parked bytecode request must be completed before resume");
        let _ = self.refresh_pending_terminal_winner();
        self.runtime.wake_queue.consume_async_signal_if_present();
        let wake = self
            .runtime
            .wake_queue
            .claim()
            .expect("a signaled pending wake queue must hold exactly one wake");
        self.resume_wake(wake)
    }

    fn resume_wake(mut self, wake: ClaimedRequestPendingWake) -> ControlledBytecodeDrive {
        let resources = self.runtime.resources.clone();
        // A host completion may already own the queued wake when cancellation
        // or the admitted deadline becomes ready. Re-arbitrate immediately
        // before heap materialization so the request budget remains the one
        // terminal authority and a late heap-free payload is only dropped.
        let terminal_winner = self.refresh_pending_terminal_winner();
        let resumed = BytecodeScheduler::<VmFiber>::resume_from_claimed_pending_wake_with(
            wake,
            self.context.ports(),
            |resume, outcome| {
                materialize_pending_outcome_after_terminal_check(
                    terminal_winner,
                    outcome,
                    |outcome| {
                        materialize_request_pending_outcome(
                            resume,
                            &resources,
                            outcome,
                            &self.runtime.cleanup_roots,
                            &self.runtime.materialization_escrows,
                            &mut *self.heap,
                        )
                    },
                )
            },
        );
        match resumed {
            Ok(scheduler) => {
                let outcome =
                    self.context
                        .resume_drive(scheduler, &mut *self.heap, &mut *self.budget);
                self.finish_drive(outcome)
            }
            Err(error) => self.terminal(error),
        }
    }

    fn refresh_pending_terminal_winner(&self) -> Option<ExecutionWinner> {
        refresh_pending_terminal_winner(&self.runtime)
    }

    fn finish_drive(
        self,
        outcome: Result<BytecodeSchedulerOutcome<VmFiber>, BytecodeSchedulerFailure<VmFiber>>,
    ) -> ControlledBytecodeDrive {
        match outcome {
            Ok(BytecodeSchedulerOutcome::Complete(result)) => self.complete(result),
            Ok(BytecodeSchedulerOutcome::Parked) => ControlledBytecodeDrive::Parked(self),
            Err(error) => self.terminal(error),
        }
    }

    fn complete(self, completion: VmCompletion) -> ControlledBytecodeDrive {
        let ParkedBytecodeRequest {
            context,
            mut heap,
            budget,
            execution_budget,
            mode,
            raw_http_adapter,
            runtime,
            ..
        } = self;
        let result = project_completed_request(
            &mut *heap,
            &execution_budget,
            &completion,
            &mode,
            raw_http_adapter,
        );
        let (mut terminal_cause, mut terminal_escrow) = completion.into_terminal();
        if let Some(cause) = terminal_cause.as_mut() {
            let _ = cause.release_all(&mut *heap);
        }
        let _ = terminal_escrow.release_all(&mut *heap);
        let mut materialization_escrows = runtime.take_materialization_escrows();
        for escrow in &mut materialization_escrows {
            let _ = escrow.release_all(&mut *heap);
        }
        let snapshot = context.freeze_with_termination(resource_termination_for_result(&result));
        ControlledBytecodeDrive::Complete(DrivenBytecodeRequest {
            result,
            retention: BytecodeRequestRetention {
                scheduler_failure_owner: None,
                terminal_cause,
                terminal_escrow: Some(terminal_escrow),
                materialization_escrows,
                budget: Some(budget),
                cleanup_roots: runtime.take_cleanup_roots(),
                heap: Some(heap),
            },
            owner_inventory: DrivenBytecodeRequestOwnerInventory::Started(snapshot),
        })
    }

    fn terminal(self, failure: BytecodeSchedulerFailure<VmFiber>) -> ControlledBytecodeDrive {
        let ParkedBytecodeRequest {
            context,
            mut heap,
            budget,
            execution_budget,
            runtime,
            ..
        } = self;
        let result: RequestResult<BoundaryResponse> = Err(scheduler_error_to_request_error_ref(
            &execution_budget,
            failure.reason(),
        ));
        let (_reason, scheduler_failure_owner) = failure
            .normalize_terminal(|operation| {
                runtime.registry.abandon(operation.ticket());
            })
            .into_parts();
        let mut materialization_escrows = runtime.take_materialization_escrows();
        for escrow in &mut materialization_escrows {
            let _ = escrow.release_all(&mut *heap);
        }
        let snapshot = context.freeze_with_termination(resource_termination_for_result(&result));
        ControlledBytecodeDrive::Complete(DrivenBytecodeRequest {
            result,
            retention: BytecodeRequestRetention {
                scheduler_failure_owner: Some(scheduler_failure_owner),
                terminal_cause: None,
                terminal_escrow: None,
                materialization_escrows,
                budget: Some(budget),
                cleanup_roots: runtime.take_cleanup_roots(),
                heap: Some(heap),
            },
            owner_inventory: DrivenBytecodeRequestOwnerInventory::Started(snapshot),
        })
    }
}

fn refresh_pending_terminal_winner(runtime: &RequestPendingRuntime) -> Option<ExecutionWinner> {
    refresh_execution_terminal_winner(&runtime.execution_control, &runtime.budget)
}

fn refresh_execution_terminal_winner(
    execution_control: &crate::OwnedExecutionControl,
    budget: &ExecutionBudget,
) -> Option<ExecutionWinner> {
    if execution_control
        .cancelled()
        .load(std::sync::atomic::Ordering::Acquire)
    {
        let _ = budget.request_cancel();
    }
    // This also freezes an admitted deadline that became due after the
    // completion cell selected its host winner.
    budget.pending_terminal_winner()
}

fn materialize_pending_outcome_after_terminal_check<T>(
    terminal_winner: Option<ExecutionWinner>,
    outcome: T,
    materialize: impl FnOnce(T) -> ResumeOutcome,
) -> ResumeOutcome {
    match terminal_winner {
        Some(winner) => {
            drop(outcome);
            resume_outcome_from_winner(winner)
        }
        None => materialize(outcome),
    }
}

fn resource_termination_for_result(
    result: &RequestResult<BoundaryResponse>,
) -> RequestResourceTermination {
    match result {
        Ok(_) => RequestResourceTermination::RequestCompleted,
        Err(RequestError::Cancelled) => RequestResourceTermination::Cancelled,
        Err(RequestError::ExecutionBudgetExceeded {
            reason: ExecutionBudgetReason::DeadlineExceeded,
            ..
        }) => RequestResourceTermination::Deadline,
        Err(_) => RequestResourceTermination::RequestFailed,
    }
}

fn project_completed_request(
    heap: &mut dyn VmHeap,
    execution_budget: &ExecutionBudget,
    completion: &VmCompletion,
    mode: &str,
    raw_http_adapter: bool,
) -> RequestResult<BoundaryResponse> {
    if let Some(values) = completion.returned_values() {
        if mode == "serverStream" {
            if values.is_empty() {
                Ok(BoundaryResponse::StreamSent)
            } else {
                Err(RequestError::Decode(
                    "linked serverStream request returned unexpected scalar values".to_string(),
                ))
            }
        } else if raw_http_adapter {
            http_response_from_vm_values(heap, values.values())
        } else {
            json_payload_from_value_slots(heap, values.values()).map(BoundaryResponse::payload)
        }
    } else if completion.thrown_diagnostic().is_some() {
        Err(uncaught_throw_to_request_error_without_payload())
    } else if let Some(error) = completion.failure() {
        Err(vm_error_to_request_error_ref(execution_budget, error))
    } else {
        Err(RequestError::Decode(
            "bytecode VM completion has no primary diagnostic".to_string(),
        ))
    }
}

fn gateway_entry_arguments(
    request: &RequestEnvelope,
    entry: &DeploymentExecutionEntry,
    heap: &mut dyn VmHeap,
) -> RequestResult<Vec<ValueSlot>> {
    let Some(adapter) = &request.http_adapter else {
        if is_task_request(request) {
            return task_arguments(request, entry, heap);
        }
        return Ok(Vec::new());
    };
    let binary = request.binary_http.as_ref().ok_or_else(|| {
        RequestError::Decode("HTTP adapter request is missing binary HTTP metadata".to_string())
    })?;
    let typed_json_body = match adapter.kind {
        HttpAdapterKind::RawHttp => None,
        HttpAdapterKind::TypedJson => {
            let parameter_count = entry.signature().parameter_types().len();
            if adapter.adapter_args.len() != parameter_count {
                return Err(RequestError::Decode(format!(
                    "typedJson HTTP adapter has {} arguments but the exact pinned entry has {parameter_count} parameters",
                    adapter.adapter_args.len()
                )));
            }
            if !adapter
                .adapter_args
                .iter()
                .any(|arg| arg.source == GatewayAdapterSource::HttpBody)
            {
                return Err(RequestError::Decode(
                    "typedJson HTTP adapter has no http.body argument".to_string(),
                ));
            }
            Some(
                serde_json::from_slice::<serde_json::Value>(&binary.body).map_err(|error| {
                    RequestError::Decode(format!("typedJson HTTP body is not valid JSON: {error}"))
                })?,
            )
        }
    };
    let mut arguments = Vec::with_capacity(adapter.adapter_args.len());
    for (ordinal, arg) in adapter.adapter_args.iter().enumerate() {
        let value = match arg.source {
            GatewayAdapterSource::HttpRequest => {
                materialize_http_request(binary, entry, ordinal, heap)?
            }
            GatewayAdapterSource::HttpBody => match typed_json_body.as_ref() {
                Some(body) => materialize_typed_json_scalar(body, entry, ordinal)?,
                None => materialize_raw_http_body(binary, entry, ordinal, heap)?,
            },
            GatewayAdapterSource::HttpContext => ValueSlot::null(),
        };
        arguments.push(value);
    }
    Ok(arguments)
}

fn materialize_typed_json_scalar(
    body: &serde_json::Value,
    entry: &DeploymentExecutionEntry,
    ordinal: usize,
) -> RequestResult<ValueSlot> {
    if matches!(
        body,
        serde_json::Value::Array(_) | serde_json::Value::Object(_)
    ) {
        return Err(RequestError::Unsupported(format!(
            "typedJson HTTP body for parameter {ordinal} is non-scalar; Phase 1 supports only number, bool, and null"
        )));
    }

    let signature = entry.signature();
    let expected_type = signature.parameter_types().get(ordinal).ok_or_else(|| {
        RequestError::Decode(format!(
            "typedJson HTTP body parameter {ordinal} is absent from the exact pinned entry signature"
        ))
    })?;
    let expected_plan = signature.parameter_plans().get(ordinal).ok_or_else(|| {
        RequestError::Decode(format!(
            "typedJson HTTP body parameter {ordinal} has no exact pinned lifecycle plan"
        ))
    })?;
    let type_entry = entry
        .image()
        .types()
        .get(expected_type.get() as usize)
        .filter(|entry| entry.index() == *expected_type)
        .ok_or_else(|| {
            RequestError::Decode(format!(
                "typedJson HTTP body parameter {ordinal} has no exact pinned concrete type"
            ))
        })?;

    if !matches!(
        expected_plan,
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial
        }
    ) {
        return Err(RequestError::Unsupported(format!(
            "typedJson HTTP body parameter {ordinal} has an unsupported exact pinned lifecycle plan"
        )));
    }

    let TypeRefIr::Builtin { name, args } = type_entry.type_ref() else {
        return Err(unsupported_typed_json_parameter_type(ordinal));
    };
    if !args.is_empty() {
        return Err(unsupported_typed_json_parameter_type(ordinal));
    }

    match (name.as_str(), body) {
        ("number", serde_json::Value::Number(number)) => number
            .as_f64()
            .filter(|number| number.is_finite())
            .map(ValueSlot::number)
            .ok_or_else(|| {
                RequestError::Decode(format!(
                    "typedJson HTTP body number for parameter {ordinal} cannot be represented by the VM"
                ))
            }),
        ("bool", serde_json::Value::Bool(value)) => Ok(ValueSlot::bool(*value)),
        ("null", serde_json::Value::Null) => Ok(ValueSlot::null()),
        ("number" | "bool" | "null", _) => Err(RequestError::Decode(format!(
            "typedJson HTTP body does not match the exact pinned {name} type for parameter {ordinal}"
        ))),
        _ => Err(unsupported_typed_json_parameter_type(ordinal)),
    }
}

fn unsupported_typed_json_parameter_type(ordinal: usize) -> RequestError {
    RequestError::Unsupported(format!(
        "typedJson HTTP body parameter {ordinal} has an unsupported exact pinned type; Phase 1 supports only number, bool, and null"
    ))
}

fn exact_gateway_parameter_type(
    entry: &DeploymentExecutionEntry,
    ordinal: usize,
) -> RequestResult<TypeIndex> {
    entry
        .signature()
        .parameter_types()
        .get(ordinal)
        .copied()
        .ok_or_else(|| {
            RequestError::Decode(format!(
                "HTTP gateway argument {ordinal} is absent from the exact pinned entry signature"
            ))
        })
}

fn materialize_raw_http_body(
    binary: &BinaryHttpRequest,
    entry: &DeploymentExecutionEntry,
    ordinal: usize,
    heap: &mut dyn VmHeap,
) -> RequestResult<ValueSlot> {
    let body_type = exact_gateway_parameter_type(entry, ordinal)?;
    validate_builtin_type(entry.image(), body_type, "bytes")
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    heap.alloc_typed_bytes(
        binary.body.clone(),
        request_compact_type_tag(body_type, "raw HTTP body")?,
        ValueFlags::new(0),
    )
    .map_err(heap_error_to_request_error)
}

fn materialize_http_request(
    binary: &BinaryHttpRequest,
    entry: &DeploymentExecutionEntry,
    ordinal: usize,
    heap: &mut dyn VmHeap,
) -> RequestResult<ValueSlot> {
    let layout = raw_http_request_layout(entry, ordinal)?;
    let mut owned = Vec::new();
    let method = retain_materialized_root(
        heap.alloc_typed_string(
            binary.metadata.method.clone(),
            layout.string_tag,
            ValueFlags::new(0),
        ),
        heap,
        &mut owned,
    )?;
    let url = retain_materialized_root(
        heap.alloc_typed_string(
            binary.metadata.url.clone(),
            layout.string_tag,
            ValueFlags::new(0),
        ),
        heap,
        &mut owned,
    )?;
    let path = retain_materialized_root(
        heap.alloc_typed_string(
            binary.metadata.path.clone(),
            layout.string_tag,
            ValueFlags::new(0),
        ),
        heap,
        &mut owned,
    )?;
    let query = match materialize_name_values(
        &binary.metadata.query,
        "raw HTTP query",
        layout.query_tag,
        layout.string_tag,
        heap,
    ) {
        Ok(query) => {
            owned.push(query);
            query
        }
        Err(error) => return Err(cleanup_materialized_roots(heap, &mut owned, error)),
    };
    let headers = match materialize_name_values(
        &binary.metadata.headers,
        "raw HTTP headers",
        layout.header_tag,
        layout.string_tag,
        heap,
    ) {
        Ok(headers) => {
            owned.push(headers);
            headers
        }
        Err(error) => return Err(cleanup_materialized_roots(heap, &mut owned, error)),
    };
    let body = retain_materialized_root(
        heap.alloc_typed_bytes(binary.body.clone(), layout.body_tag, ValueFlags::new(0)),
        heap,
        &mut owned,
    )?;
    let fields = [
        VmRecordField {
            name: "body".to_string(),
            value: body,
        },
        VmRecordField {
            name: "headers".to_string(),
            value: headers,
        },
        VmRecordField {
            name: "method".to_string(),
            value: method,
        },
        VmRecordField {
            name: "path".to_string(),
            value: path,
        },
        VmRecordField {
            name: "query".to_string(),
            value: query,
        },
        VmRecordField {
            name: "url".to_string(),
            value: url,
        },
    ];
    let request = heap.allocate_record(&fields, layout.root_tag, ValueFlags::new(0));
    match request {
        Ok(request) => {
            owned.clear();
            Ok(request)
        }
        Err(error) => Err(cleanup_materialized_roots(
            heap,
            &mut owned,
            heap_error_to_request_error(error),
        )),
    }
}

#[derive(Clone, Copy)]
struct RawHttpRequestLayout {
    root_tag: CompactTypeTag,
    string_tag: CompactTypeTag,
    body_tag: CompactTypeTag,
    header_tag: CompactTypeTag,
    query_tag: CompactTypeTag,
}

fn raw_http_request_layout(
    entry: &DeploymentExecutionEntry,
    ordinal: usize,
) -> RequestResult<RawHttpRequestLayout> {
    if ordinal != 0 || entry.signature().parameter_types().len() != 1 {
        return Err(RequestError::Decode(format!(
            "raw HTTP request argument {ordinal} does not identify the gateway's sole exact parameter"
        )));
    }
    let image = entry.image();
    let request_type = exact_gateway_parameter_type(entry, ordinal)?;
    let request_ref = linked_type_ref(image, request_type, "raw HTTP request")
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    let request_abi =
        exact_std_http_symbol_abi(request_ref, HTTP_REQUEST_TYPE, "raw HTTP request parameter")
            .map_err(|error| RequestError::Decode(error.to_string()))?;
    let shape_index = entry.parameter_dense_record_shape().ok_or_else(|| {
        RequestError::Decode(
            "raw HTTP request parameter lacks compiler-owned dense materialization".to_string(),
        )
    })?;
    let shape = image
        .shapes()
        .get(shape_index.get() as usize)
        .filter(|shape| shape.index() == shape_index)
        .ok_or_else(|| {
            RequestError::Decode(format!(
                "raw HTTP request parameter references missing linked shape {}",
                shape_index.get()
            ))
        })?;
    validate_shape_fields(
        shape,
        &["body", "headers", "method", "path", "query", "url"],
    )
    .map_err(|error| RequestError::Decode(error.to_string()))?;
    if shape.privileged_affine_composite().is_some()
        || linked_type_ref(image, shape.nominal_type(), "raw HTTP request shape")
            .map_err(|error| RequestError::Decode(error.to_string()))?
            != request_ref
        || entry.signature().parameter_plans().get(ordinal) != Some(shape.plan())
    {
        return Err(RequestError::Decode(
            "raw HTTP request parameter dense materialization differs from its exact linked type/plan"
                .to_string(),
        ));
    }

    let body =
        shape_field_type(shape, "body").map_err(|error| RequestError::Decode(error.to_string()))?;
    validate_builtin_type(image, body, "bytes")
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    let method = shape_field_type(shape, "method")
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    validate_builtin_type(image, method, "string")
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    let string_ref = linked_type_ref(image, method, "raw HTTP request string")
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    for field in ["path", "url"] {
        let field_type = shape_field_type(shape, field)
            .map_err(|error| RequestError::Decode(error.to_string()))?;
        if linked_type_ref(image, field_type, "raw HTTP request string field")
            .map_err(|error| RequestError::Decode(error.to_string()))?
            != string_ref
        {
            return Err(RequestError::Decode(format!(
                "raw HTTP request field {field:?} differs from its exact builtin string carrier"
            )));
        }
    }

    let headers = shape_field_type(shape, "headers")
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    let header = exact_std_http_array_element_type(
        image,
        headers,
        request_abi,
        HTTP_HEADER_TYPE,
        "raw HTTP request headers",
    )
    .map_err(|error| RequestError::Decode(error.to_string()))?;
    let query = shape_field_type(shape, "query")
        .map_err(|error| RequestError::Decode(error.to_string()))?;
    let query = exact_std_http_array_element_type(
        image,
        query,
        request_abi,
        HTTP_QUERY_PARAM_TYPE,
        "raw HTTP request query",
    )
    .map_err(|error| RequestError::Decode(error.to_string()))?;

    Ok(RawHttpRequestLayout {
        root_tag: request_compact_type_tag(shape.nominal_type(), "raw HTTP request root")?,
        string_tag: request_compact_type_tag(method, "raw HTTP request string")?,
        body_tag: request_compact_type_tag(body, "raw HTTP request body")?,
        header_tag: request_compact_type_tag(header, "raw HTTP request header")?,
        query_tag: request_compact_type_tag(query, "raw HTTP request query")?,
    })
}

fn materialize_name_values(
    items: &[HttpNameValue],
    label: &str,
    record_tag: CompactTypeTag,
    string_tag: CompactTypeTag,
    heap: &mut dyn VmHeap,
) -> RequestResult<ValueSlot> {
    let mut owned = Vec::with_capacity(items.len());
    for item in items {
        let checkpoint = owned.len();
        let name = retain_materialized_root(
            heap.alloc_typed_string(item.name.clone(), string_tag, ValueFlags::new(0)),
            heap,
            &mut owned,
        )?;
        let value = retain_materialized_root(
            heap.alloc_typed_string(item.value.clone(), string_tag, ValueFlags::new(0)),
            heap,
            &mut owned,
        )?;
        let record = heap.allocate_record(
            &[
                VmRecordField {
                    name: "name".to_string(),
                    value: name,
                },
                VmRecordField {
                    name: "value".to_string(),
                    value,
                },
            ],
            record_tag,
            ValueFlags::new(0),
        );
        let record = match record {
            Ok(record) => record,
            Err(error) => {
                return Err(cleanup_materialized_roots(
                    heap,
                    &mut owned,
                    heap_error_to_request_error(error),
                ));
            }
        };
        owned.truncate(checkpoint);
        owned.push(record);
    }
    // Request VM arrays carry their exact element TypeIndex. The same tag is
    // therefore used for each canonical name/value record and its array.
    let array = heap.allocate_array(&owned, record_tag, ValueFlags::new(0));
    match array {
        Ok(array) => {
            owned.clear();
            Ok(array)
        }
        Err(error) => Err(cleanup_materialized_roots(
            heap,
            &mut owned,
            RequestError::Decode(format!(
                "{label} materialization failed on the request heap: {error}"
            )),
        )),
    }
}

fn retain_materialized_root(
    result: Result<ValueSlot, VmHeapError>,
    heap: &mut dyn VmHeap,
    owned: &mut Vec<ValueSlot>,
) -> RequestResult<ValueSlot> {
    match result {
        Ok(value) => {
            owned.push(value);
            Ok(value)
        }
        Err(error) => Err(cleanup_materialized_roots(
            heap,
            owned,
            heap_error_to_request_error(error),
        )),
    }
}

fn cleanup_materialized_roots(
    heap: &mut dyn VmHeap,
    owned: &mut Vec<ValueSlot>,
    primary: RequestError,
) -> RequestError {
    while let Some(root) = owned.last().copied() {
        match heap.release_snapshot(&root) {
            Ok(()) => {
                owned.pop();
            }
            Err(error) => {
                return RequestError::Decode(format!(
                    "{primary}; raw HTTP request heap cleanup failed: {error}; the failing owner and earlier roots remain escrowed"
                ));
            }
        }
    }
    primary
}

fn heap_error_to_request_error(error: VmHeapError) -> RequestError {
    RequestError::Decode(format!(
        "bytecode gateway heap materialization failed: {error}"
    ))
}

fn scheduler_error_to_request_error_ref(
    execution_budget: &ExecutionBudget,
    error: &BytecodeSchedulerError,
) -> RequestError {
    match error {
        BytecodeSchedulerError::UnsupportedChild => RequestError::Unsupported(
            "bytecode VM child invocation requires a child executor port".to_string(),
        ),
        BytecodeSchedulerError::UnsupportedAdapter => RequestError::Unsupported(
            "bytecode VM adapter invocation requires a child executor port".to_string(),
        ),
        BytecodeSchedulerError::UnsupportedStream => RequestError::Unsupported(
            "bytecode VM stream emission requires stream supervisor integration".to_string(),
        ),
        BytecodeSchedulerError::UnsupportedPark => RequestError::Unsupported(
            "bytecode VM parking requires stream supervisor integration".to_string(),
        ),
        BytecodeSchedulerError::ChildCapacityExceeded => RequestError::Decode(
            "bytecode scheduler blocked child capacity is exhausted".to_string(),
        ),
        BytecodeSchedulerError::ChildOwnerCreation(_) => {
            RequestError::Decode("bytecode scheduler owner creation failed".to_string())
        }
        BytecodeSchedulerError::Vm(error) => vm_error_to_request_error_ref(execution_budget, error),
        BytecodeSchedulerError::Port(message) => {
            RequestError::Unsupported(format!("bytecode scheduler port failed: {message}"))
        }
    }
}

fn validate_bytecode_request(
    request: &RequestEnvelope,
    entry: &DeploymentExecutionEntry,
    server_stream_writer: Option<SharedBytecodeServerStreamWriterPort>,
    resources: &RequestResourceTable,
    max_response_bytes: NonZeroUsize,
) -> RequestResult<Option<ServerStreamStart>> {
    validate_bytecode_request_metadata(request)?;
    let function = entry
        .image()
        .functions()
        .get(entry.function().get() as usize)
        .filter(|function| function.index() == entry.function())
        .ok_or_else(|| {
            RequestError::Decode(
                "bytecode ingress entry function is absent from its linked image".to_string(),
            )
        })?;
    match request.mode.as_str() {
        "unary" => {
            if function.stream_result_type_ref().is_some() {
                return Err(RequestError::Unsupported(
                    "unary bytecode ingress cannot execute a linked stream producer".to_string(),
                ));
            }
            Ok(None)
        }
        "serverStream" => {
            if !request
                .http_adapter
                .as_ref()
                .is_some_and(|adapter| adapter.kind == HttpAdapterKind::RawHttp)
            {
                return Err(RequestError::Unsupported(
                    "serverStream bytecode ingress requires the exact raw HTTP adapter".to_string(),
                ));
            }
            function.stream_result_type_ref().ok_or_else(|| {
                RequestError::Unsupported(
                    "serverStream bytecode ingress entry has no linked stream-result authority"
                        .to_string(),
                )
            })?;
            if !entry.signature().result_types().is_empty()
                || !entry.signature().result_plans().is_empty()
            {
                return Err(RequestError::Decode(
                    "linked server-stream entry retains a scalar result signature".to_string(),
                ));
            }
            if !function
                .instructions()
                .iter()
                .any(|instruction| instruction.opcode() == Opcode::EmitStream)
            {
                return Err(RequestError::Decode(
                    "linked serverStream entry has no linked EmitStream site".to_string(),
                ));
            }
            let writer = server_stream_writer.ok_or_else(|| {
                RequestError::Unsupported(
                    "serverStream bytecode ingress has no transport writer".to_string(),
                )
            })?;
            let handle = resources
                .register_server_response_stream(max_response_bytes)
                .map_err(|error| {
                    RequestError::Decode(format!(
                        "server response stream registration failed: {error}"
                    ))
                })?;
            Ok(Some(ServerStreamStart { handle, writer }))
        }
        mode => Err(RequestError::Unsupported(format!(
            "bytecode ingress request.start mode {mode:?} is unsupported"
        ))),
    }
}

fn validate_bytecode_request_metadata(request: &RequestEnvelope) -> RequestResult<()> {
    if request.ingress_selector.is_none() && !is_task_request(request) {
        return Err(RequestError::Unsupported(
            "bytecode ingress requires request.start ingress_selector".to_string(),
        ));
    }
    if request.extra.contains_key("actorCall") {
        return Err(RequestError::Unsupported(
            "actor.call request.start metadata is not supported by bytecode ingress".to_string(),
        ));
    }
    Ok(())
}

fn json_payload_from_value_slots(
    heap: &mut dyn VmHeap,
    values: &[ValueSlot],
) -> RequestResult<Vec<u8>> {
    match values {
        [] => Ok(b"null".to_vec()),
        [value] => serde_json::to_vec(&json_value_from_slot(heap, value, 0)?).map_err(|error| {
            RequestError::Decode(format!("bytecode VM JSON encode failed: {error}"))
        }),
        _ => Err(RequestError::Unsupported(format!(
            "bytecode VM returned {} results; expected zero or one",
            values.len()
        ))),
    }
}

fn json_value_from_slot(
    heap: &mut dyn VmHeap,
    value: &ValueSlot,
    depth: usize,
) -> RequestResult<serde_json::Value> {
    const MAX_DEPTH: usize = 1024;
    if value.is_null() {
        return Ok(serde_json::Value::Null);
    }
    if let Some(boolean) = value.as_bool() {
        return Ok(serde_json::Value::Bool(boolean));
    }
    if let Some(number) = value.as_number() {
        let number = serde_json::Number::from_f64(number).ok_or_else(|| {
            RequestError::Unsupported(format!("bytecode VM returned a non-JSON number: {number}"))
        })?;
        return Ok(serde_json::Value::Number(number));
    }
    if value.as_integer().is_some() {
        return Err(RequestError::Unsupported(
            "integer results are not supported by bytecode JSON ingress".to_string(),
        ));
    }
    if value.as_date().is_some() {
        return Err(RequestError::Unsupported(
            "Date results are not supported by bytecode JSON ingress".to_string(),
        ));
    }
    if value.kind() != Some(ValueKind::RequestHeapRef) {
        return Err(RequestError::Unsupported(format!(
            "bytecode VM returned unsupported value kind {:?}",
            value.kind()
        )));
    }
    if depth > MAX_DEPTH {
        return Err(RequestError::Unsupported(format!(
            "bytecode VM aggregate exceeds the JSON materialization depth {MAX_DEPTH}"
        )));
    }
    let container = heap
        .container_elements(value)
        .map_err(heap_error_to_request_error)?;
    match container.shape {
        VmContainerShape::Array => {
            let mut items = Vec::with_capacity(container.elements.len());
            for element in container.elements {
                items.push(json_value_from_slot(heap, &element.value, depth + 1)?);
            }
            Ok(serde_json::Value::Array(items))
        }
        VmContainerShape::Record => {
            let mut fields = serde_json::Map::with_capacity(container.elements.len());
            for element in container.elements {
                let name = element.field.ok_or_else(|| {
                    RequestError::Unsupported(
                        "bytecode VM record element has no canonical field name".to_string(),
                    )
                })?;
                fields.insert(name, json_value_from_slot(heap, &element.value, depth + 1)?);
            }
            Ok(serde_json::Value::Object(fields))
        }
    }
}

fn http_response_from_vm_values(
    heap: &mut dyn VmHeap,
    values: &[ValueSlot],
) -> RequestResult<BoundaryResponse> {
    let [record] = values else {
        return Err(RequestError::Unsupported(
            "HTTP gateway VM must return exactly one HTTP response record".to_string(),
        ));
    };
    let status_slot = heap
        .record_field(record, "status")
        .map_err(heap_error_to_request_error)?;
    let status = http_status_from_vm_slot(&status_slot).ok_or_else(|| {
        RequestError::Unsupported("HTTP gateway response status must be an integer".to_string())
    })?;
    let headers = http_headers_from_vm(heap, record, "headers")?
        .into_iter()
        .map(|header| HttpNameValue {
            name: header.name,
            value: header.value,
        })
        .collect();
    let body_slot = heap
        .record_field(record, "body")
        .map_err(heap_error_to_request_error)?;
    let body = heap
        .bytes_value(&body_slot)
        .map_err(heap_error_to_request_error)?;
    Ok(BoundaryResponse::http(
        body,
        HttpResponseMetadata::new(status, headers),
    ))
}

fn http_status_from_vm_slot(slot: &ValueSlot) -> Option<u16> {
    slot.as_integer()
        .and_then(|value| u16::try_from(value).ok())
        .or_else(|| {
            slot.as_number().and_then(|value| {
                (value.fract() == 0.0 && (0.0..=65535.0).contains(&value)).then_some(value as u16)
            })
        })
}

fn http_headers_from_vm(
    heap: &mut dyn VmHeap,
    record: &ValueSlot,
    field: &str,
) -> RequestResult<Vec<HttpBoundaryNameValue>> {
    let headers_slot = heap
        .record_field(record, field)
        .map_err(heap_error_to_request_error)?;
    let header_count = heap
        .array_len(&headers_slot)
        .map_err(heap_error_to_request_error)?;
    let mut headers = Vec::with_capacity(header_count);
    for index in 0..header_count {
        let header = heap
            .array_get(&headers_slot, index)
            .map_err(heap_error_to_request_error)?;
        let name = heap
            .record_field(&header, "name")
            .and_then(|slot| heap.string_value(&slot))
            .map_err(heap_error_to_request_error)?;
        let value = heap
            .record_field(&header, "value")
            .and_then(|slot| heap.string_value(&slot))
            .map_err(heap_error_to_request_error)?;
        headers.push(HttpBoundaryNameValue { name, value });
    }
    Ok(headers)
}

fn vm_error_to_request_error(execution_budget: &ExecutionBudget, error: VmError) -> RequestError {
    vm_error_to_request_error_ref(execution_budget, &error)
}

fn vm_error_to_request_error_ref(
    execution_budget: &ExecutionBudget,
    error: &VmError,
) -> RequestError {
    match error {
        VmError::BudgetClosed(error) => vm_budget_closed_to_request_error(execution_budget, *error),
        VmError::InternalTerminal(VmInternalTerminal::Budget(error)) => {
            vm_budget_closed_to_request_error(execution_budget, *error)
        }
        VmError::InternalTerminal(VmInternalTerminal::OwnerStopped) => RequestError::Cancelled,
        VmError::HostEffectFailure(payload) => RequestError::ExternalErrorPayload {
            code: payload.code.clone(),
            message: payload.message.clone(),
            status: payload.status,
            details: payload.details.clone(),
        },
        // A root throw is intercepted by the scheduler outcome before this
        // projection; reaching here means the envelope cannot be materialized
        // on this lane, so the canonical user error is projected without a
        // payload. Envelope-construction VmFailures must not leak VM
        // internals and project to the sanitized InternalError; every other
        // Phase 1 VM error keeps its existing user-facing projection.
        VmError::Thrown(_) => uncaught_throw_to_request_error_without_payload(),
        VmError::ThrowEnvelopeUnavailable { .. }
        | VmError::RethrowEnvelopeUnavailable { .. }
        | VmError::ResumeThrowEnvelopeUnavailable { .. } => {
            RequestError::Decode("bytecode VM execution failed".to_string())
        }
        error => RequestError::Unsupported(format!("bytecode VM execution failed: {error}")),
    }
}

fn bytecode_error_correlation(request: &RequestEnvelope) -> ErrorCorrelation {
    let request_id = if request.request_id.trim().is_empty() {
        request.target.as_str()
    } else {
        request.request_id.as_str()
    };
    ErrorCorrelation {
        trace_id: request_id.to_string(),
        error_id: request_id.to_string(),
    }
}

fn uncaught_throw_to_request_error_without_payload() -> RequestError {
    RequestError::ExternalErrorPayload {
        code: "std.service.InternalError".to_string(),
        message: "uncaught user exception".to_string(),
        status: None,
        details: None,
    }
}

fn vm_budget_closed_to_request_error(
    execution_budget: &ExecutionBudget,
    error: VmBudgetClosed,
) -> RequestError {
    let stats = execution_budget.stats_snapshot();
    match error {
        VmBudgetClosed::AlreadySettled(
            VmBudgetTerminal::Succeeded
            | VmBudgetTerminal::Failed
            | VmBudgetTerminal::Cancelled
            | VmBudgetTerminal::InternalStop,
        ) => RequestError::Cancelled,
        VmBudgetClosed::DeadlineExceeded
        | VmBudgetClosed::AlreadySettled(VmBudgetTerminal::DeadlineExceeded) => {
            RequestError::ExecutionBudgetExceeded {
                reason: ExecutionBudgetReason::DeadlineExceeded,
                instruction_count: stats.instruction_count,
                limit: stats.budget_limit,
                elapsed_ms: stats.elapsed_ms,
            }
        }
        VmBudgetClosed::InstructionLimitExceeded
        | VmBudgetClosed::AlreadySettled(VmBudgetTerminal::InstructionLimitExceeded) => {
            RequestError::ExecutionBudgetExceeded {
                reason: ExecutionBudgetReason::InstructionLimitExceeded,
                instruction_count: stats.instruction_count,
                limit: stats.budget_limit,
                elapsed_ms: stats.elapsed_ms,
            }
        }
        VmBudgetClosed::AccountingFailure
        | VmBudgetClosed::AlreadySettled(VmBudgetTerminal::AccountingFailure) => {
            RequestError::Unsupported(format!(
                "bytecode VM budget accounting failed closed: {error}"
            ))
        }
    }
}

fn vm_limits() -> VmLimits {
    VmLimits::new(
        NonZeroUsize::new(128).expect("VM frame limit is non-zero"),
        NonZeroUsize::new(4096).expect("VM value slot limit is non-zero"),
        NonZeroU32::new(1024).expect("VM segment instruction limit is non-zero"),
    )
}

#[cfg(test)]
mod absent_supervisor_tests;

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Barrier};

    use skiff_artifact_model::{IngressProtocol, IngressSelector};
    use skiff_runtime_model::{request_heap::RequestHeapLimits, vm_value::ValueSlot};

    use super::*;
    use crate::{
        BinaryHttpRequest, BinaryHttpRequestMetadata, HttpAdapter, HttpAdapterCallable,
        HttpAdapterKind, RequestEnvelope, ResponseEnd, ResponseEvent,
    };

    fn test_tag(type_index: u32) -> CompactTypeTag {
        CompactTypeTag::try_from_type_index(type_index).expect("test type index fits compact tag")
    }

    fn test_pending_runtime(
        budget: Arc<ExecutionBudget>,
        cancellation: CancellationToken,
    ) -> (Arc<RequestPendingRuntime>, RequestExecutionContext<VmFiber>) {
        let mut context =
            RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let (wake_queue, _wake_receiver) = RequestPendingWakeQueue::new();
        let resources = context.resource_table();
        let runtime = Arc::new(RequestPendingRuntime {
            registry: Arc::new(RequestPendingRegistry::new(
                context.take_pending_registration().unwrap(),
            )),
            wake_queue,
            budget: Arc::clone(&budget),
            resources: resources.clone(),
            http_client: None,
            execution_control: ExecutionControl::new(cancellation, &budget).owned(),
            stream_registrar: BytecodeHttpStreamRegistrar::new(resources),
            child_composition: Default::default(),
            cleanup_roots: Mutex::new(Vec::new()),
            materialization_escrows: Mutex::new(Vec::new()),
            manual_sleep_completion: Mutex::new(None),
        });
        (runtime, context)
    }

    #[test]
    fn phase_5_first_poll_async_driver_future_is_send() {
        fn accepts_send_future<F>(_: fn(BytecodeRequestExecutionInput) -> F)
        where
            F: Future<Output = DrivenBytecodeRequest> + Send,
        {
        }

        accepts_send_future(drive_runtime_bytecode_request_async);
    }

    #[test]
    fn phase_5_first_poll_ready_bypasses_pending_owner() {
        let mut context =
            RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let registry = PendingRegistry::<u8, &'static str, &'static str>::new(
            context.take_pending_registration().unwrap(),
        );
        let mut future = Box::pin(std::future::ready("ready"));

        assert_eq!(poll_future_once(future.as_mut()), Poll::Ready("ready"));
        assert_eq!(registry.live_count(), 0);
        drop(registry);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.pending.current, 0);
        assert!(!snapshot.pending.ever_created);
    }

    #[test]
    fn phase_5_first_poll_already_cancelled_ready_is_terminal_before_materialization() {
        let budget = Arc::new(ExecutionBudget::for_runtime_request(None));
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let (runtime, context) = test_pending_runtime(Arc::clone(&budget), cancellation);

        assert!(matches!(
            runtime.ready_terminal(),
            Some(ResumeOutcome::InternalTerminal(
                VmInternalTerminal::OwnerStopped
            ))
        ));
        assert_eq!(
            budget.settlement().unwrap().winner(),
            ExecutionWinner::Cancelled
        );
        drop(runtime);
        assert_eq!(context.into_not_started().pending.current, 0);
    }

    #[test]
    fn phase_5_first_poll_already_due_ready_uses_budget_terminal() {
        let now = std::time::Instant::now();
        let deadline = crate::execution_budget::AdmittedRequestDeadline::new(
            now.checked_sub(std::time::Duration::from_millis(1))
                .unwrap(),
        );
        let budget = Arc::new(ExecutionBudget::for_runtime_request(Some(deadline)));
        let (runtime, context) =
            test_pending_runtime(Arc::clone(&budget), CancellationToken::new());

        assert!(matches!(
            runtime.ready_terminal(),
            Some(ResumeOutcome::InternalTerminal(VmInternalTerminal::Budget(
                VmBudgetClosed::DeadlineExceeded
            )))
        ));
        assert_eq!(
            budget.settlement().unwrap().winner(),
            ExecutionWinner::DeadlineExceeded
        );
        drop(runtime);
        assert_eq!(context.into_not_started().pending.current, 0);
    }

    #[test]
    fn phase_5_first_poll_non_sleep_manual_completion_cannot_inject_empty() {
        let budget = Arc::new(ExecutionBudget::for_runtime_request(None));
        let (runtime, _context) =
            test_pending_runtime(Arc::clone(&budget), CancellationToken::new());
        let completion = runtime
            .registry
            .begin_with_resource_roots(RootEscrow::empty(), runtime.resources.root_pin())
            .unwrap();
        let sink: Arc<dyn RequestPendingSink> = Arc::new(PendingCellSink {
            completion: completion.clone(),
        });
        assert_eq!(budget.register_pending_sink(sink), None);
        let authority = RequestPendingCompletion {
            runtime: Arc::clone(&runtime),
        };

        assert!(!authority.complete());
        assert_eq!(
            completion.state(),
            skiff_runtime_scheduler::PendingCellState::Open
        );
        let _ = budget.request_internal_stop();
        assert_eq!(
            completion.state(),
            skiff_runtime_scheduler::PendingCellState::Settled
        );
        assert!(runtime.registry.abandon(completion.ticket()));
    }

    #[test]
    fn phase_5_first_poll_http_body_preserves_null_and_present_empty() {
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());
        let null = decode_optional_http_body(&mut heap, &ValueSlot::null()).unwrap();
        let empty_slot = heap
            .alloc_typed_bytes(Vec::new(), test_tag(1), ValueFlags::new(0))
            .unwrap();
        let present_empty = decode_optional_http_body(&mut heap, &empty_slot).unwrap();

        assert_eq!(null, None);
        assert_eq!(present_empty, Some(Vec::new()));
        assert_ne!(null, present_empty);
    }

    #[test]
    fn raw_http_boundary_materialization_failure_releases_partial_roots() {
        let mut heap = RequestVmHeap::new(RequestHeapLimits {
            max_nodes: 1,
            ..RequestHeapLimits::default()
        });
        let error = match materialize_name_values(
            &[HttpNameValue {
                name: "x-name".to_string(),
                value: "value".to_string(),
            }],
            "raw HTTP headers",
            test_tag(2),
            test_tag(1),
            &mut heap,
        ) {
            Err(error) => error,
            Ok(_) => panic!("the second carrier must exceed the one-node test heap"),
        };

        assert!(error.to_string().contains("resource limit"));
        assert_eq!(
            heap.live_value_count(),
            0,
            "failed boundary materialization must release every partial owner"
        );
    }

    #[test]
    fn phase_5_http_client_boundary_rejects_type_and_abi_mismatch() {
        let std_symbol = |path: &str, abi: Option<&str>| TypeRefIr::PackageSymbol {
            symbol: skiff_artifact_model::PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: HTTP_BOUNDARY_PACKAGE_ID.to_string(),
                },
                symbol_path: path.to_string(),
                abi_expectation: abi.map(str::to_string),
            },
        };
        let request = std_symbol(HTTP_CLIENT_REQUEST_TYPE, Some("abi-v1"));
        assert_eq!(
            exact_std_http_symbol_abi(&request, HTTP_CLIENT_REQUEST_TYPE, "test HTTP request")
                .unwrap(),
            "abi-v1"
        );

        let wrong_type = std_symbol(HTTP_CLIENT_RESPONSE_TYPE, Some("abi-v1"));
        assert!(exact_std_http_symbol_abi(
            &wrong_type,
            HTTP_CLIENT_REQUEST_TYPE,
            "test HTTP request"
        )
        .is_err());
        let missing_abi = std_symbol(HTTP_CLIENT_REQUEST_TYPE, None);
        assert!(exact_std_http_symbol_abi(
            &missing_abi,
            HTTP_CLIENT_REQUEST_TYPE,
            "test HTTP request"
        )
        .is_err());
        assert!(require_same_http_abi("abi-v2", "abi-v1", "test HTTP header").is_err());
    }

    fn test_http_result_layout() -> HttpResultLayout {
        HttpResultLayout {
            root_tag: test_tag(1),
            header_tag: test_tag(2),
            body_tag: test_tag(3),
            string_tag: test_tag(4),
        }
    }

    struct FailFirstHttpCleanupHeap {
        fail_next_release: bool,
        released: Vec<skiff_runtime_model::vm_value::VmHandle>,
    }

    impl VmHeap for FailFirstHttpCleanupHeap {
        fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }

        fn snapshot_share(&mut self, _source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Err(VmHeapError::InvalidValueMetadata)
        }

        fn transfer_owner(&mut self, _source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Err(VmHeapError::InvalidValueMetadata)
        }

        fn release_snapshot(&mut self, owner: &ValueSlot) -> Result<(), VmHeapError> {
            if self.fail_next_release {
                self.fail_next_release = false;
                return Err(VmHeapError::HeapOperationFailed {
                    operation: VmHeapOperation::ReleaseSnapshot,
                    message: "injected cleanup release failure".to_string(),
                });
            }
            self.released.push(
                owner
                    .as_request_heap_ref()
                    .ok_or(VmHeapError::InvalidValueMetadata)?,
            );
            Ok(())
        }

        fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            Err(VmHeapError::InvalidValueMetadata)
        }
    }

    #[test]
    fn phase_5_http_cleanup_release_failure_keeps_owner_and_suffix_escrowed() {
        let tag = test_tag(7);
        let mut owned = vec![
            ValueSlot::request_heap_ref(
                skiff_runtime_model::vm_value::VmHandle::new(1),
                tag,
                ValueFlags::new(0),
            ),
            ValueSlot::request_heap_ref(
                skiff_runtime_model::vm_value::VmHandle::new(2),
                tag,
                ValueFlags::new(0),
            ),
        ];
        let escrow = Mutex::new(Vec::new());
        let mut heap = FailFirstHttpCleanupHeap {
            fail_next_release: true,
            released: Vec::new(),
        };
        let primary = VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::AllocateRecord,
            message: "injected allocation failure".to_string(),
        };

        let error = cleanup_http_materialized_roots(&mut heap, &mut owned, primary, &escrow);

        assert!(matches!(
            error,
            VmHeapError::HeapOperationFailed {
                operation: VmHeapOperation::ReleaseSnapshot,
                ..
            }
        ));
        assert!(owned.is_empty());
        let mut retry = std::mem::take(&mut *escrow.lock().unwrap());
        assert_eq!(retry.len(), 2);
        assert!(heap.released.is_empty());

        let retry_escrow = Mutex::new(Vec::new());
        let retry_primary = VmHeapError::HeapOperationFailed {
            operation: VmHeapOperation::ReleaseSnapshot,
            message: "retry".to_string(),
        };
        let _ =
            cleanup_http_materialized_roots(&mut heap, &mut retry, retry_primary, &retry_escrow);
        assert!(retry.is_empty());
        assert!(retry_escrow.lock().unwrap().is_empty());
        assert_eq!(
            heap.released,
            [
                skiff_runtime_model::vm_value::VmHandle::new(2),
                skiff_runtime_model::vm_value::VmHandle::new(1),
            ]
        );
    }

    #[test]
    fn phase_5_http_response_partial_materialization_releases_every_heap_owner() {
        let cleanup_escrow = Mutex::new(Vec::new());
        let mut heap = RequestVmHeap::new(RequestHeapLimits {
            max_nodes: 2,
            ..RequestHeapLimits::default()
        });
        let error = match materialize_http_response_value(
            test_http_result_layout(),
            BytecodeHttpResponse {
                status: 200,
                headers: Vec::new(),
                body: b"body".to_vec(),
            },
            &cleanup_escrow,
            &mut heap,
        ) {
            Err(error) => error,
            Ok(_) => panic!("the response root must exceed the two-node test heap"),
        };

        assert!(matches!(
            error,
            VmHeapError::ResourceLimitExceeded {
                operation: VmHeapOperation::AllocateRecord,
                ..
            }
        ));
        assert_eq!(
            heap.live_value_count(),
            0,
            "failed response root allocation releases its header array and body"
        );
        assert!(cleanup_escrow.lock().unwrap().is_empty());
    }

    struct EndHttpBodySource;

    impl skiff_runtime_capability_context::StreamPullSource for EndHttpBodySource {
        fn next<'a>(
            &'a mut self,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = skiff_runtime_capability_context::StreamRuntimeResult<
                            Option<serde_json::Value>,
                        >,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Ok(None) })
        }
    }

    fn test_http_stream_route() -> (
        RequestExecutionContext<VmFiber>,
        RequestResourceTable,
        RequestResourceHandle,
        CancellationToken,
    ) {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let resources = context.resource_table();
        let registrar = BytecodeHttpStreamRegistrar::new(resources.clone());
        let cancellation = CancellationToken::new();
        let token = registrar
            .stream_runtime()
            .pull_stream_with_cancellation(EndHttpBodySource, cancellation.clone());
        let handle = registrar.take_exact_route(token).unwrap();
        (context, resources, handle, cancellation)
    }

    #[test]
    fn phase_5_http_stream_header_failure_releases_claimed_route() {
        let (context, resources, handle, cancellation) = test_http_stream_route();
        let cleanup_escrow = Mutex::new(Vec::new());
        let mut heap = RequestVmHeap::for_execution(
            resources.clone(),
            RequestHeapLimits {
                max_nodes: 0,
                ..RequestHeapLimits::default()
            },
        );

        let error = match materialize_http_stream_value(
            test_http_result_layout(),
            BytecodeHttpStreamResponse {
                status: 200,
                headers: Vec::new(),
                body: handle,
            },
            &resources,
            &cleanup_escrow,
            &mut heap,
        ) {
            Err(error) => error,
            Ok(_) => panic!("header allocation must hit the zero-node limit"),
        };

        assert!(matches!(
            error,
            VmHeapError::ResourceLimitExceeded {
                operation: VmHeapOperation::AllocateArray,
                ..
            }
        ));
        assert_eq!(heap.live_value_count(), 0);
        assert_eq!(resources.live_count(), 0);
        assert!(
            !cancellation.is_cancelled(),
            "releasing one HTTP body route must not cancel its request"
        );
        assert!(cleanup_escrow.lock().unwrap().is_empty());
        drop(heap);
        drop(resources);
        assert_eq!(context.into_not_started().resource.current, 0);
    }

    #[test]
    fn phase_5_http_stream_body_admission_failure_releases_headers_and_route() {
        let (context, resources, handle, cancellation) = test_http_stream_route();
        let cleanup_escrow = Mutex::new(Vec::new());
        // This heap deliberately has no resource-table authority, so body
        // admission fails after the header array has been allocated.
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());

        let error = match materialize_http_stream_value(
            test_http_result_layout(),
            BytecodeHttpStreamResponse {
                status: 200,
                headers: Vec::new(),
                body: handle,
            },
            &resources,
            &cleanup_escrow,
            &mut heap,
        ) {
            Err(error) => error,
            Ok(_) => panic!("resource admission without the request table must fail"),
        };

        assert!(matches!(
            error,
            VmHeapError::OperationKindMismatch {
                operation: VmHeapOperation::ValidateLive,
                kind: ValueKind::ResourceRef,
            }
        ));
        assert_eq!(heap.live_value_count(), 0);
        assert_eq!(resources.live_count(), 0);
        assert!(
            !cancellation.is_cancelled(),
            "failed route admission must not cancel its request"
        );
        assert!(cleanup_escrow.lock().unwrap().is_empty());
        drop(heap);
        drop(resources);
        assert_eq!(context.into_not_started().resource.current, 0);
    }

    #[test]
    fn phase_5_http_stream_partial_materialization_releases_resource_owner() {
        let (context, resources, handle, source_cancellation) = test_http_stream_route();
        let cleanup_escrow = Mutex::new(Vec::new());
        assert_eq!(resources.live_count(), 1);
        let mut heap = RequestVmHeap::for_execution(
            resources.clone(),
            RequestHeapLimits {
                max_nodes: 1,
                ..RequestHeapLimits::default()
            },
        );

        let error = match materialize_http_stream_value(
            test_http_result_layout(),
            BytecodeHttpStreamResponse {
                status: 200,
                headers: Vec::new(),
                body: handle,
            },
            &resources,
            &cleanup_escrow,
            &mut heap,
        ) {
            Err(error) => error,
            Ok(_) => panic!("the stream root must exceed the one-node test heap"),
        };

        assert!(matches!(
            error,
            VmHeapError::ResourceLimitExceeded {
                operation: VmHeapOperation::AllocateRecord,
                ..
            }
        ));
        assert_eq!(heap.live_value_count(), 0);
        assert_eq!(resources.live_count(), 0);
        assert!(
            !source_cancellation.is_cancelled(),
            "partial stream materialization cleanup must remain route-local"
        );
        assert!(cleanup_escrow.lock().unwrap().is_empty());
        drop(heap);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.resource.current, 0);
        assert!(snapshot.resource.ever_created);
    }

    #[test]
    fn user_throw_and_envelope_vm_failure_project_distinct_codes() {
        let budget = ExecutionBudget::for_runtime_request(None);
        let user_error = uncaught_throw_to_request_error_without_payload()
            .ordinary_payload()
            .expect("user throw is an ordinary response error");
        assert_eq!(user_error.code, "std.service.InternalError");
        assert_eq!(user_error.message, "uncaught user exception");

        let envelope_failure = vm_error_to_request_error(
            &budget,
            VmError::ThrowEnvelopeUnavailable {
                function: skiff_runtime_linked_bytecode::FunctionIndex::new(0),
                instruction: skiff_runtime_linked_bytecode::InstructionIndex::new(0),
                reason: "fixture".to_string(),
            },
        )
        .ordinary_payload()
        .expect("envelope VmFailure is an ordinary response error");
        assert_eq!(envelope_failure.code, "InternalError");
        assert_eq!(envelope_failure.message, "bytecode VM execution failed");
        assert_ne!(user_error.code, envelope_failure.code);
    }

    #[test]
    fn json_payload_encodes_scalar_immediates() {
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());
        assert_eq!(
            json_payload_from_value_slots(&mut heap, &[]).unwrap(),
            b"null"
        );
        assert_eq!(
            json_payload_from_value_slots(&mut heap, &[ValueSlot::null()]).unwrap(),
            b"null"
        );
        assert_eq!(
            json_payload_from_value_slots(&mut heap, &[ValueSlot::bool(true)]).unwrap(),
            b"true"
        );
        assert_eq!(
            json_payload_from_value_slots(&mut heap, &[ValueSlot::bool(false)]).unwrap(),
            b"false"
        );
        assert_eq!(
            json_payload_from_value_slots(&mut heap, &[ValueSlot::number(1.5)]).unwrap(),
            b"1.5"
        );
    }

    #[test]
    fn json_payload_rejects_unsupported_results() {
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());
        assert!(json_payload_from_value_slots(&mut heap, &[ValueSlot::integer(1)]).is_err());
        assert!(json_payload_from_value_slots(&mut heap, &[ValueSlot::date(1)]).is_err());
        assert!(json_payload_from_value_slots(
            &mut heap,
            &[ValueSlot::null(), ValueSlot::bool(true)]
        )
        .is_err());
        assert!(json_payload_from_value_slots(&mut heap, &[ValueSlot::number(f64::NAN)]).is_err());
    }

    #[test]
    fn json_payload_materializes_nested_record_and_array() {
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());
        let leaf = heap
            .allocate_record(
                &[VmRecordField {
                    name: "x".to_string(),
                    value: ValueSlot::number(1.0),
                }],
                test_tag(1),
                ValueFlags::new(0),
            )
            .unwrap();
        let tags = heap
            .allocate_array(
                &[ValueSlot::number(1.0), ValueSlot::number(2.0)],
                test_tag(2),
                ValueFlags::new(0),
            )
            .unwrap();
        let inner = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "leaf".to_string(),
                        value: leaf,
                    },
                    VmRecordField {
                        name: "tags".to_string(),
                        value: tags,
                    },
                ],
                test_tag(3),
                ValueFlags::new(0),
            )
            .unwrap();
        let payload = json_payload_from_value_slots(&mut heap, &[inner]).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "leaf": { "x": 1.0 },
                "tags": [1.0, 2.0],
            })
        );
    }

    #[test]
    fn http_response_from_vm_values_materializes_metadata_and_body() {
        let mut heap = RequestVmHeap::new(RequestHeapLimits::default());
        let name = heap
            .alloc_typed_string("content-type".to_string(), test_tag(1), ValueFlags::new(0))
            .unwrap();
        let value = heap
            .alloc_typed_string("text/plain".to_string(), test_tag(1), ValueFlags::new(0))
            .unwrap();
        let header = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "name".to_string(),
                        value: name,
                    },
                    VmRecordField {
                        name: "value".to_string(),
                        value,
                    },
                ],
                test_tag(2),
                ValueFlags::new(0),
            )
            .unwrap();
        let headers = heap
            .allocate_array(&[header], test_tag(2), ValueFlags::new(0))
            .unwrap();
        let body = heap
            .alloc_typed_bytes(b"ok".to_vec(), test_tag(3), ValueFlags::new(0))
            .unwrap();
        let response = heap
            .allocate_record(
                &[
                    VmRecordField {
                        name: "status".to_string(),
                        value: ValueSlot::number(201.0),
                    },
                    VmRecordField {
                        name: "headers".to_string(),
                        value: headers,
                    },
                    VmRecordField {
                        name: "body".to_string(),
                        value: body,
                    },
                ],
                test_tag(4),
                ValueFlags::new(0),
            )
            .unwrap();

        let boundary = http_response_from_vm_values(&mut heap, &[response]).unwrap();
        let BoundaryResponse::Event(ResponseEvent::End(ResponseEnd::Http { payload, metadata })) =
            boundary
        else {
            panic!("HTTP response conversion returned {boundary:?}");
        };
        assert_eq!(metadata.status, 201);
        assert_eq!(metadata.headers[0].name, "content-type");
        assert_eq!(metadata.headers[0].value, "text/plain");
        assert_eq!(payload, b"ok");
    }

    #[test]
    fn validation_requires_canonical_selector() {
        assert!(validate_bytecode_request_metadata(&request()).is_ok());

        let mut selector_request = request();
        selector_request.ingress_selector = None;
        let error = validate_bytecode_request_metadata(&selector_request)
            .expect_err("selector is required");
        assert!(error.to_string().contains("ingress_selector"));
    }

    #[test]
    fn validation_accepts_exact_task_marker_without_http_selector() {
        let mut task_request = request();
        task_request.ingress_selector = None;
        task_request.http_adapter = None;
        task_request.payload_bytes = vec![1, 2, 3];
        task_request
            .extra
            .insert("task".to_string(), serde_json::json!(true));
        validate_bytecode_request_metadata(&task_request)
            .expect("task marker must replace the HTTP ingress selector requirement");
    }

    #[test]
    fn validation_rejects_unsupported_ingress_metadata() {
        let mut binary_request = request();
        binary_request.binary_http = Some(BinaryHttpRequest {
            metadata: BinaryHttpRequestMetadata {
                method: "GET".to_string(),
                url: "http://example.test/entry".to_string(),
                path: "/entry".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
            },
            body: Vec::new(),
        });
        assert!(validate_bytecode_request_metadata(&binary_request).is_ok());

        let mut adapter_request = request();
        adapter_request.http_adapter = Some(HttpAdapter {
            kind: HttpAdapterKind::TypedJson,
            handler: HttpAdapterCallable::ServiceFunction {
                module_path: "main".to_string(),
                symbol: "run".to_string(),
            },
            guard: None,
            pre: None,
            adapter_args: Vec::new(),
        });
        assert!(validate_bytecode_request_metadata(&adapter_request).is_ok());

        let mut actor_request = request();
        actor_request
            .extra
            .insert("actorCall".to_string(), serde_json::json!({}));
        assert!(validate_bytecode_request_metadata(&actor_request).is_err());
    }

    #[test]
    fn scheduler_fail_closed_errors_map_to_unsupported() {
        let budget = ExecutionBudget::for_runtime_request(None);
        assert!(matches!(
            scheduler_error_to_request_error_ref(
                &budget,
                &BytecodeSchedulerError::UnsupportedChild
            ),
            RequestError::Unsupported(message) if message.contains("child executor port")
        ));
        assert!(matches!(
            scheduler_error_to_request_error_ref(
                &budget,
                &BytecodeSchedulerError::UnsupportedAdapter
            ),
            RequestError::Unsupported(message) if message.contains("child executor port")
        ));
        assert!(matches!(
            scheduler_error_to_request_error_ref(
                &budget,
                &BytecodeSchedulerError::UnsupportedStream
            ),
            RequestError::Unsupported(message) if message.contains("stream supervisor")
        ));
        assert!(matches!(
            scheduler_error_to_request_error_ref(
                &budget,
                &BytecodeSchedulerError::UnsupportedPark
            ),
            RequestError::Unsupported(message) if message.contains("stream supervisor")
        ));
        assert!(matches!(
            scheduler_error_to_request_error_ref(
                &budget,
                &BytecodeSchedulerError::ChildCapacityExceeded
            ),
            RequestError::Decode(message) if message == "bytecode scheduler blocked child capacity is exhausted"
        ));
    }

    fn request() -> RequestEnvelope {
        RequestEnvelope {
            request_id: "bytecode-request".to_string(),
            mode: "unary".to_string(),
            target: "display-only".to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: None,
            build_id: "legacy-build".to_string(),
            service_protocol_identity: "legacy-protocol".to_string(),
            contract_identity: None,
            activation_identity: None,
            ingress_selector: Some(IngressSelector {
                protocol: IngressProtocol::Http,
                method: Some("POST".to_string()),
                path: "/entry".to_string(),
            }),
            binary_http: None,
            http_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }

    fn pending_registry() -> (RequestPendingRegistry, RequestExecutionContext<VmFiber>) {
        let mut context =
            RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let registry = RequestPendingRegistry::new(context.take_pending_registration().unwrap());
        (registry, context)
    }

    #[test]
    fn cancellation_sink_settles_the_parked_cell_once_through_the_budget() {
        let budget = Arc::new(ExecutionBudget::for_runtime_request(None));
        let (registry, _context) = pending_registry();
        let completion = registry.begin(RootEscrow::empty()).unwrap();
        let sink: Arc<dyn RequestPendingSink> = Arc::new(PendingCellSink {
            completion: completion.clone(),
        });

        assert_eq!(budget.register_pending_sink(sink), None);
        assert_eq!(
            completion.state(),
            skiff_runtime_scheduler::PendingCellState::Open
        );
        budget.request_cancel();
        assert_eq!(
            completion.state(),
            skiff_runtime_scheduler::PendingCellState::Settled
        );

        // A second terminal source cannot re-settle the same cell.
        budget.request_internal_stop();
        assert_eq!(
            completion.state(),
            skiff_runtime_scheduler::PendingCellState::Settled
        );
        assert!(registry.abandon(completion.ticket()));
    }

    #[test]
    fn due_deadline_converts_the_parked_cell_to_one_deadline_terminal() {
        let now = std::time::Instant::now();
        let deadline = crate::execution_budget::AdmittedRequestDeadline::new(
            now.checked_sub(std::time::Duration::from_millis(1))
                .unwrap(),
        );
        let budget = Arc::new(ExecutionBudget::for_runtime_request(Some(deadline)));
        let (registry, _context) = pending_registry();
        let completion = registry.begin(RootEscrow::empty()).unwrap();
        let sink: Arc<dyn RequestPendingSink> = Arc::new(PendingCellSink {
            completion: completion.clone(),
        });

        let winner = budget.register_pending_sink(sink);
        assert_eq!(winner, Some(ExecutionWinner::DeadlineExceeded));
        let _ = complete_cell_from_winner(&completion, winner.unwrap());
        assert_eq!(
            completion.state(),
            skiff_runtime_scheduler::PendingCellState::Settled
        );
        assert_eq!(
            budget.settlement().unwrap().winner(),
            ExecutionWinner::DeadlineExceeded
        );
        assert!(registry.abandon(completion.ticket()));
    }

    #[test]
    fn deterministic_completion_claims_the_parked_cell_exactly_once() {
        let budget = Arc::new(ExecutionBudget::for_runtime_request(None));
        let (registry, _context) = pending_registry();
        let completion = registry.begin(RootEscrow::empty()).unwrap();
        let (wake_queue, _wake_receiver) = RequestPendingWakeQueue::new();
        let runtime = Arc::new(RequestPendingRuntime {
            registry: Arc::new(registry),
            wake_queue,
            budget: Arc::clone(&budget),
            resources: _context.resource_table(),
            http_client: None,
            execution_control: ExecutionControl::new(CancellationToken::new(), &budget).owned(),
            stream_registrar: BytecodeHttpStreamRegistrar::new(_context.resource_table()),
            child_composition: Default::default(),
            cleanup_roots: Mutex::new(Vec::new()),
            materialization_escrows: Mutex::new(Vec::new()),
            manual_sleep_completion: Mutex::new(Some(completion.clone())),
        });
        let authority = RequestPendingCompletion {
            runtime: Arc::clone(&runtime),
        };

        assert!(authority.complete());
        assert_eq!(
            completion.state(),
            skiff_runtime_scheduler::PendingCellState::Settled
        );
        assert!(!authority.complete());
    }

    #[test]
    fn phase_5_first_poll_manual_sleep_completion_reports_one_concurrent_winner() {
        let budget = Arc::new(ExecutionBudget::for_runtime_request(None));
        let (registry, context) = pending_registry();
        let completion = registry.begin(RootEscrow::empty()).unwrap();
        let (wake_queue, _wake_receiver) = RequestPendingWakeQueue::new();
        let resources = context.resource_table();
        let runtime = Arc::new(RequestPendingRuntime {
            registry: Arc::new(registry),
            wake_queue,
            budget: Arc::clone(&budget),
            resources: resources.clone(),
            http_client: None,
            execution_control: ExecutionControl::new(CancellationToken::new(), &budget).owned(),
            stream_registrar: BytecodeHttpStreamRegistrar::new(resources),
            child_composition: Default::default(),
            cleanup_roots: Mutex::new(Vec::new()),
            materialization_escrows: Mutex::new(Vec::new()),
            manual_sleep_completion: Mutex::new(Some(completion.clone())),
        });
        let authority = RequestPendingCompletion {
            runtime: Arc::clone(&runtime),
        };
        let barrier = Arc::new(Barrier::new(3));
        let workers = [authority.clone(), authority]
            .into_iter()
            .map(|authority| {
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    authority.complete()
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let winners = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(|won| *won)
            .count();

        assert_eq!(winners, 1);
        assert_eq!(
            completion.state(),
            skiff_runtime_scheduler::PendingCellState::Settled
        );
        assert!(runtime.registry.abandon(completion.ticket()));
        drop(runtime);
        let snapshot = context.into_not_started();
        assert_eq!(snapshot.pending.current, 0);
        assert_eq!(snapshot.resource.current, 0);
    }
}
