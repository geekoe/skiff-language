use std::{
    future::Future,
    num::{NonZeroU32, NonZeroUsize},
    pin::Pin,
    sync::{mpsc, Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
};

use skiff_artifact_model::{HostEffectExecutorIdentity, TypeRefIr};
use skiff_runtime_boundary::http::HttpBoundaryNameValue;
use skiff_runtime_capability_context::{CancellationToken, ExecutionBudgetReason};
use skiff_runtime_linked_bytecode::{
    LinkedNativeCallableSignature, LinkedShapeEntry, LinkedValueDropPlan, LinkedValueTransferPlan,
    TypeIndex,
};
use skiff_runtime_linker::{DeploymentExecutionEntry, DeploymentExecutionImage};
use skiff_runtime_model::{
    bytecode_execution_observation::{
        BytecodeExecutionObserver, RequestExecutionOwnerInventorySnapshot,
    },
    error::RuntimeErrorPayload,
    request_heap::RequestHeapLimits,
    service_error::{ErrorCorrelation, RequestException},
    vm_heap::{VmContainerShape, VmHeap, VmHeapError, VmRecordField},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};
use skiff_runtime_scheduler::{
    BytecodeAdapterHandoff, BytecodeChildExecutor, BytecodeChildStart, BytecodeHandoff,
    BytecodeScheduler, BytecodeSchedulerError, BytecodeSchedulerOutcome, BytecodeSchedulerPorts,
    BytecodeStreamHandoff, CompletionHandle, PendingRegistry, PendingWake, PendingWakeQueue,
    RequestByteStreamFailure, RequestExecutionContext, RequestResourceFinishReason,
    RequestResourceHandle, RequestResourceTable, RequestResourceTermination, RootDisposition,
    RootEscrow, RootEscrowBacking, SuspendedTrampoline,
};
use skiff_runtime_vm::{
    AdapterInvocation, ChildInvocation, PendingOperation, ResumeOutcome, Vm, VmBudget,
    VmBudgetClosed, VmBudgetTerminal, VmError, VmFiber, VmInternalTerminal, VmLimits,
    VmOwnedValues, VmResult, VmResumeToken,
};

use crate::{
    bytecode_host_effects::{
        BytecodeHttpFailure, BytecodeHttpRequest, BytecodeHttpResponse,
        BytecodeHttpStreamRegistrar, BytecodeHttpStreamResponse, SharedBytecodeHttpClientPort,
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
    /// Optional injected VM heap (production composition or a recording heap
    /// spy). When `None`, the driver constructs the production
    /// [`RequestVmHeap`] from `handles.request_heap_limits`. The injected heap
    /// is exactly the heap driven into the VM and retained for the boundary
    /// result lifetime.
    pub heap: Option<Box<dyn VmHeap + Send>>,
}

pub struct BytecodeRequestExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
}

/// Opaque retention carrier for one driven bytecode request.
///
/// The carrier keeps the VM heap and budget alive for the lifetime of the
/// boundary result; dropping it releases all remaining heap owners and
/// detaches the VM budget. The fields are intentionally never read: the
/// carrier's entire contract is its Drop lifetime.
#[allow(dead_code)]
pub struct BytecodeRequestRetention {
    heap: Option<Box<dyn VmHeap + Send>>,
    budget: Option<Box<dyn VmBudget + Send>>,
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
type VmSuspended = SuspendedTrampoline<VmFiber, VmResumeToken>;

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
        heap: injected_heap,
    } = input;

    let mode = request.mode.clone();
    let raw_http_adapter = request
        .http_adapter
        .as_ref()
        .is_some_and(|adapter| adapter.kind == HttpAdapterKind::RawHttp);
    validate_bytecode_request(&request)?;
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
    let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
    let start = match start_bytecode_request(input, context.resource_table()) {
        Ok(start) => start,
        Err(error) => {
            return ControlledBytecodeDrive::Complete(DrivenBytecodeRequest {
                result: Err(error),
                retention: BytecodeRequestRetention {
                    heap: None,
                    budget: None,
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
        registry: Arc::new(RequestPendingRegistry::new(context.pending_registration())),
        wake_queue,
        budget: Arc::clone(&start.execution_budget),
        resources,
        http_client: start.http_client.clone(),
        execution_control: start.execution_control.clone(),
        stream_registrar,
        manual_sleep_completion: Mutex::new(None),
    });
    let mut context = context.with_ports(BytecodeSchedulerPorts {
        child_executor: Some(Arc::new(BytecodeHostExecutor {
            runtime: Arc::clone(&runtime),
        })),
        stream_supervisor: None,
    });

    let BytecodeStart {
        fiber,
        mut heap,
        mut budget,
        execution_budget,
        mode,
        raw_http_adapter,
        http_client: _,
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

/// Roots transferred out of a parked fiber as the argument of a pinned
/// pending host effect.
///
/// The slots are already popped from the fiber operand stack, so neither
/// terminal path can "restore" them back into a live owner. The escrow keeps
/// them enumerable during a safepoint walk; the request heap releases their
/// storage at boundary teardown.
struct HostEffectArgumentRoots(Vec<ValueSlot>);

impl VmRootSource for HostEffectArgumentRoots {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        for root in &self.0 {
            visitor.visit_root(root)?;
        }
        Ok(())
    }
}

impl RootEscrowBacking for HostEffectArgumentRoots {
    fn root_count(&self) -> usize {
        self.0.len()
    }

    fn restore_roots(self: Box<Self>) {}

    fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {}
}

#[derive(Clone, Copy, Debug)]
struct HttpResultLayout {
    root: TypeIndex,
    headers: TypeIndex,
    header: TypeIndex,
    header_name: TypeIndex,
    header_value: TypeIndex,
    body: TypeIndex,
}

enum RequestPendingOutcome {
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
}

impl VmRootSource for RequestPendingOutcome {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        match self {
            Self::Vm(outcome) => outcome.visit_roots(visitor),
            Self::HttpRequest { .. } | Self::HttpStream { .. } | Self::StreamNext { .. } => Ok(()),
        }
    }
}

type RequestPendingRegistry = PendingRegistry<VmResumeToken, VmSuspended, RequestPendingOutcome>;
type RequestCompletionHandle = CompletionHandle<VmResumeToken, VmSuspended, RequestPendingOutcome>;
type RequestPendingWake = PendingWake<VmResumeToken, VmSuspended, RequestPendingOutcome>;

/// Runtime-neutral runnable queue for claimed pending wakes.
///
/// Every wake stays root-enumerable while queued; `enqueue` also signals the
/// single parked-request receiver so a resume can drain exactly one wake.
struct RequestPendingWakeQueue {
    wakes: Mutex<Vec<RequestPendingWake>>,
    signal: mpsc::Sender<()>,
    async_signal: tokio::sync::Semaphore,
}

impl RequestPendingWakeQueue {
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

    fn pop(&self) -> Option<RequestPendingWake> {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
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

impl PendingWakeQueue<VmResumeToken, VmSuspended, RequestPendingOutcome>
    for RequestPendingWakeQueue
{
    fn enqueue(&self, wake: RequestPendingWake) {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(wake);
        let _ = self.signal.send(());
        self.async_signal.add_permits(1);
    }
}

impl VmRootSource for RequestPendingWakeQueue {
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
struct RequestPendingRuntime {
    registry: Arc<RequestPendingRegistry>,
    wake_queue: Arc<RequestPendingWakeQueue>,
    budget: Arc<ExecutionBudget>,
    resources: RequestResourceTable,
    http_client: Option<SharedBytecodeHttpClientPort>,
    #[allow(dead_code)]
    execution_control: crate::OwnedExecutionControl,
    #[allow(dead_code)]
    stream_registrar: BytecodeHttpStreamRegistrar,
    /// Deterministic Phase 4 regression authority. Only typed Sleep may
    /// install a handle here; HTTP and StreamNext are host-owned futures and
    /// can never accept an injected empty result.
    manual_sleep_completion: Mutex<Option<RequestCompletionHandle>>,
}

/// Converts the budget's single authoritative winner into the exact pending
/// cell settlement. The cell arbiter drops every duplicate, so this path can
/// never produce a second terminal.
fn complete_cell_from_winner(completion: &RequestCompletionHandle, winner: ExecutionWinner) {
    let outcome = RequestPendingOutcome::Vm(resume_outcome_from_winner(winner));
    match winner {
        ExecutionWinner::DeadlineExceeded => {
            let _ = completion.deadline(outcome);
        }
        ExecutionWinner::Cancelled => {
            let _ = completion.cancel(outcome);
        }
        _ => {
            let _ = completion.internal_stop(outcome);
        }
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
        complete_cell_from_winner(&self.completion, winner);
    }
}

#[derive(Debug)]
struct FirstPollWake;

impl Wake for FirstPollWake {
    fn wake(self: Arc<Self>) {}
}

fn poll_future_once<F>(future: Pin<&mut F>) -> Poll<F::Output>
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
}

impl BytecodeHostExecutor {
    fn begin_pending<T, F, M>(
        &self,
        roots: Vec<ValueSlot>,
        resume: VmResumeToken,
        future: Pin<Box<F>>,
        allow_manual_sleep_completion: bool,
        map: M,
    ) -> Result<PendingOperation, BytecodeSchedulerError>
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static + ?Sized,
        M: FnOnce(T) -> RequestPendingOutcome + Send + 'static,
    {
        let spawner = tokio::runtime::Handle::try_current().map_err(|_| {
            BytecodeSchedulerError::Port(
                "actual-Pending host effect requires the current request Tokio runtime".to_string(),
            )
        })?;
        let escrow = RootEscrow::new(Box::new(HostEffectArgumentRoots(roots)));
        let completion = self
            .runtime
            .registry
            .begin(escrow)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let sink: Arc<dyn RequestPendingSink> = Arc::new(PendingCellSink {
            completion: completion.clone(),
        });
        let winner = self.runtime.budget.register_pending_sink(sink);
        *self
            .runtime
            .manual_sleep_completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            allow_manual_sleep_completion.then(|| completion.clone());
        if let Some(winner) = winner {
            complete_cell_from_winner(&completion, winner);
            drop(future);
        } else {
            let budget = Arc::clone(&self.runtime.budget);
            let execution_control = self.runtime.execution_control.clone();
            let completion_for_task = completion.clone();
            spawner.spawn(async move {
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
                let output = tokio::select! {
                    output = future => Some(output),
                    _ = cancellation.wait_cancelled() => {
                        let _ = budget.request_cancel();
                        None
                    }
                    _ = &mut deadline_wait => {
                        let _ = budget.pending_terminal_winner();
                        None
                    }
                };
                let Some(output) = output else {
                    return;
                };
                let outcome = map(output);
                if request_pending_outcome_is_cancelled(&outcome) {
                    let _ = budget.request_cancel();
                    drop(outcome);
                } else {
                    if execution_control
                        .cancelled()
                        .load(std::sync::atomic::Ordering::Acquire)
                    {
                        let _ = budget.request_cancel();
                    }
                    if let Some(winner) = budget.pending_terminal_winner() {
                        complete_cell_from_winner(&completion_for_task, winner);
                        drop(outcome);
                    } else {
                        let _ = completion_for_task.complete(outcome);
                    }
                }
            });
        }
        Ok(resume.into_pending(completion.ticket()))
    }

    fn ready_adapter(
        resume: VmResumeToken,
        outcome: ResumeOutcome,
    ) -> BytecodeAdapterHandoff<VmFiber> {
        BytecodeAdapterHandoff::Ready(BytecodeHandoff { resume, outcome })
    }

    fn ready_terminal(&self) -> Option<ResumeOutcome> {
        if self
            .runtime
            .execution_control
            .cancelled()
            .load(std::sync::atomic::Ordering::Acquire)
        {
            let _ = self.runtime.budget.request_cancel();
        }
        self.runtime
            .budget
            .pending_terminal_winner()
            .map(resume_outcome_from_winner)
    }
}

impl BytecodeChildExecutor<VmFiber> for BytecodeHostExecutor {
    fn execute_child(
        &self,
        _invocation: skiff_runtime_vm::ChildInvocation,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeChildStart<VmFiber>, BytecodeSchedulerError> {
        Err(BytecodeSchedulerError::UnsupportedChild)
    }

    fn execute_adapter(
        &self,
        invocation: AdapterInvocation,
        heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let adapter_index = invocation.adapter();
        let image = Arc::clone(invocation.resume().image());
        let target = image.host_effect_target(adapter_index).ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "pending host effect adapter row is absent from the pinned image".to_string(),
            )
        })?;
        let identity = target.executor_identity();
        let signature = target.signature().clone();
        let (_adapter, arguments, resume) = invocation.into_parts();
        let roots = arguments.values().to_vec();
        match identity {
            HostEffectExecutorIdentity::Sleep => {
                validate_native_arity(&signature, 1, 0)?;
                let argument = arguments.values().first().ok_or_else(|| {
                    BytecodeSchedulerError::Port(
                        "typed sleep invocation is missing its duration".to_string(),
                    )
                })?;
                let millis = heap
                    .representation_payload(argument)
                    .and_then(|payload| {
                        payload
                            .as_integer()
                            .ok_or(VmHeapError::InvalidValueMetadata)
                    })
                    .and_then(|millis| {
                        u64::try_from(millis).map_err(|_| VmHeapError::InvalidValueMetadata)
                    })
                    .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                tokio::runtime::Handle::try_current().map_err(|_| {
                    BytecodeSchedulerError::Port(
                        "typed sleep requires the current request Tokio runtime".to_string(),
                    )
                })?;
                let mut future: Pin<Box<dyn Future<Output = ()> + Send>> = Box::pin(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(millis)).await;
                });
                match poll_future_once(future.as_mut()) {
                    Poll::Ready(()) => Ok(Self::ready_adapter(
                        resume,
                        self.ready_terminal().unwrap_or(ResumeOutcome::Empty),
                    )),
                    Poll::Pending => self
                        .begin_pending(roots, resume, future, true, |_| {
                            RequestPendingOutcome::Vm(ResumeOutcome::Empty)
                        })
                        .map(BytecodeAdapterHandoff::Pending),
                }
            }
            HostEffectExecutorIdentity::HttpClientRequest => {
                let provider = self.runtime.http_client.clone().ok_or_else(|| {
                    BytecodeSchedulerError::Port(
                        "typed bytecode HTTP provider is unavailable".to_string(),
                    )
                })?;
                let request = decode_http_request(&image, &signature, arguments.values(), heap)?;
                let layout = http_result_layout(&image, &signature, false)?;
                let mut future = provider.request(request, self.runtime.execution_control.clone());
                match poll_future_once(future.as_mut()) {
                    Poll::Ready(result) => {
                        if matches!(&result, Err(BytecodeHttpFailure::Cancelled)) {
                            let _ = self.runtime.budget.request_cancel();
                        }
                        let outcome = self.ready_terminal().unwrap_or_else(|| {
                            materialize_http_request_outcome(&image, layout, result, heap)
                        });
                        Ok(Self::ready_adapter(resume, outcome))
                    }
                    Poll::Pending => self
                        .begin_pending(roots, resume, future, false, move |result| {
                            RequestPendingOutcome::HttpRequest { layout, result }
                        })
                        .map(BytecodeAdapterHandoff::Pending),
                }
            }
            HostEffectExecutorIdentity::HttpClientStream => {
                let provider = self.runtime.http_client.clone().ok_or_else(|| {
                    BytecodeSchedulerError::Port(
                        "typed bytecode HTTP provider is unavailable".to_string(),
                    )
                })?;
                let request = decode_http_request(&image, &signature, arguments.values(), heap)?;
                let layout = http_result_layout(&image, &signature, true)?;
                let mut future = provider.stream(
                    request,
                    self.runtime.execution_control.clone(),
                    self.runtime.stream_registrar.clone(),
                );
                match poll_future_once(future.as_mut()) {
                    Poll::Ready(result) => {
                        if matches!(&result, Err(BytecodeHttpFailure::Cancelled)) {
                            let _ = self.runtime.budget.request_cancel();
                        }
                        let outcome = self.ready_terminal().unwrap_or_else(|| {
                            materialize_http_stream_outcome(&image, layout, result, heap)
                        });
                        Ok(Self::ready_adapter(resume, outcome))
                    }
                    Poll::Pending => self
                        .begin_pending(roots, resume, future, false, move |result| {
                            RequestPendingOutcome::HttpStream { layout, result }
                        })
                        .map(BytecodeAdapterHandoff::Pending),
                }
            }
        }
    }

    fn park_adapter(
        &self,
        operation: PendingOperation,
        suspended: VmSuspended,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError> {
        let queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, RequestPendingOutcome>> =
            self.runtime.wake_queue.clone();
        self.runtime
            .registry
            .publish_operation(operation, suspended, queue)
            .map(|_| ())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
    }

    fn execute_stream_next(
        &self,
        invocation: ChildInvocation,
        heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeStreamHandoff<VmFiber>, BytecodeSchedulerError> {
        let (_target, arguments, endpoint, resume) = invocation.into_parts();
        let endpoint = endpoint.ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "StreamNext invocation is missing its exact endpoint route".to_string(),
            )
        })?;
        let handle = self
            .runtime
            .resources
            .validate_vm_route(endpoint.route())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let item_type = stream_next_item_type(&resume)?;
        let mut future = self
            .runtime
            .resources
            .start_byte_stream_pull(&handle)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        match poll_future_once(future.as_mut()) {
            Poll::Ready(result) => {
                if matches!(&result, Err(RequestByteStreamFailure::Cancelled)) {
                    let _ = self.runtime.budget.request_cancel();
                }
                let outcome = self.ready_terminal().unwrap_or_else(|| {
                    materialize_stream_next_outcome(
                        resume.image(),
                        &self.runtime.resources,
                        handle,
                        item_type,
                        result,
                        heap,
                    )
                });
                Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    outcome,
                    resume,
                }))
            }
            Poll::Pending => self
                .begin_pending(
                    arguments.values().to_vec(),
                    resume,
                    future,
                    false,
                    move |result| RequestPendingOutcome::StreamNext {
                        handle,
                        item_type,
                        result,
                    },
                )
                .map(BytecodeStreamHandoff::Pending),
        }
    }

    fn park_stream_next(
        &self,
        operation: PendingOperation,
        suspended: VmSuspended,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError> {
        let queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, RequestPendingOutcome>> =
            self.runtime.wake_queue.clone();
        self.runtime
            .registry
            .publish_operation(operation, suspended, queue)
            .map(|_| ())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
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

fn exact_shape(
    image: &DeploymentExecutionImage,
    ty: TypeIndex,
) -> Result<&LinkedShapeEntry, BytecodeSchedulerError> {
    let mut matches = image
        .shapes()
        .iter()
        .filter(|shape| shape.nominal_type() == ty);
    let shape = matches.next().ok_or_else(|| {
        BytecodeSchedulerError::Port(format!(
            "typed host value type {} has no verified dense shape",
            ty.get()
        ))
    })?;
    if matches.next().is_some() {
        return Err(BytecodeSchedulerError::Port(format!(
            "typed host value type {} has more than one dense shape",
            ty.get()
        )));
    }
    Ok(shape)
}

fn validate_shape_fields(
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

fn shape_field_type(
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

fn array_element_type(
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

fn validate_builtin_type(
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
    let certificate = resume
        .image()
        .resume_sites()
        .get(resume.resume_site())
        .filter(|certificate| {
            certificate.function() == resume.function()
                && certificate.site() == resume.instruction()
        })
        .ok_or_else(|| {
            BytecodeSchedulerError::Port(
                "StreamNext resume token has no matching verified certificate".to_string(),
            )
        })?;
    let [item_type] = certificate.result_types() else {
        return Err(BytecodeSchedulerError::Port(
            "StreamNext verified certificate does not carry exactly one item type".to_string(),
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
) -> Result<BytecodeHttpRequest, BytecodeSchedulerError> {
    validate_native_arity(signature, 1, 1)?;
    let request_type = signature.parameter_types()[0];
    let shape = exact_shape(image, request_type)?;
    validate_shape_fields(shape, &["body", "headers", "method", "timeoutMs", "url"])?;
    let request = arguments.first().ok_or_else(|| {
        BytecodeSchedulerError::Port("typed HTTP invocation is missing its request".to_string())
    })?;
    if request.compact_type_tag().get() != request_type.get() {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP request does not carry its verified concrete type".to_string(),
        ));
    }
    let method = heap
        .record_field(request, "method")
        .and_then(|value| heap.string_value(&value))
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let url = heap
        .record_field(request, "url")
        .and_then(|value| heap.string_value(&value))
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let headers_value = heap
        .record_field(request, "headers")
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let headers_type = shape_field_type(shape, "headers")?;
    let header_type = array_element_type(image, headers_type)?;
    let header_shape = exact_shape(image, header_type)?;
    validate_shape_fields(header_shape, &["name", "value"])?;
    let header_count = heap
        .array_len(&headers_value)
        .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
    let mut headers = Vec::with_capacity(header_count);
    for index in 0..header_count {
        let header = heap
            .array_get(&headers_value, index)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        if header.compact_type_tag().get() != header_type.get() {
            return Err(BytecodeSchedulerError::Port(
                "typed HTTP header does not carry its verified concrete type".to_string(),
            ));
        }
        let name = heap
            .record_field(&header, "name")
            .and_then(|value| heap.string_value(&value))
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let value = heap
            .record_field(&header, "value")
            .and_then(|value| heap.string_value(&value))
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
                .ok_or_else(|| {
                    BytecodeSchedulerError::Port(
                        "typed HTTP timeoutMs is not a non-negative integer".to_string(),
                    )
                })?,
        )
    };
    Ok(BytecodeHttpRequest {
        method,
        url,
        headers,
        body,
        timeout_ms,
    })
}

fn http_result_layout(
    image: &DeploymentExecutionImage,
    signature: &LinkedNativeCallableSignature,
    stream: bool,
) -> Result<HttpResultLayout, BytecodeSchedulerError> {
    validate_native_arity(signature, 1, 1)?;
    let root = signature.result_types()[0];
    let shape = exact_shape(image, root)?;
    validate_shape_fields(shape, &["body", "headers", "status"])?;
    let headers = shape_field_type(shape, "headers")?;
    let header = array_element_type(image, headers)?;
    let header_shape = exact_shape(image, header)?;
    validate_shape_fields(header_shape, &["name", "value"])?;
    let header_name = shape_field_type(header_shape, "name")?;
    let header_value = shape_field_type(header_shape, "value")?;
    validate_builtin_type(image, header_name, "string")?;
    validate_builtin_type(image, header_value, "string")?;
    let body = shape_field_type(shape, "body")?;
    if stream {
        validate_byte_stream_type(image, body)?;
    } else {
        validate_builtin_type(image, body, "bytes")?;
    }
    if stream && signature.result_plans().len() != 1 {
        return Err(BytecodeSchedulerError::Port(
            "typed HTTP stream has no exact result lifecycle plan".to_string(),
        ));
    }
    Ok(HttpResultLayout {
        root,
        headers,
        header,
        header_name,
        header_value,
        body,
    })
}

fn allocate_http_headers(
    heap: &mut dyn VmHeap,
    layout: HttpResultLayout,
    headers: Vec<HttpNameValue>,
) -> Result<ValueSlot, VmHeapError> {
    let mut values = Vec::with_capacity(headers.len());
    for header in headers {
        let name = heap.alloc_typed_string(
            header.name,
            CompactTypeTag::new(layout.header_name.get()),
            ValueFlags::new(0),
        )?;
        let value = heap.alloc_typed_string(
            header.value,
            CompactTypeTag::new(layout.header_value.get()),
            ValueFlags::new(0),
        )?;
        values.push(heap.allocate_record(
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
            CompactTypeTag::new(layout.header.get()),
            ValueFlags::new(0),
        )?);
    }
    heap.allocate_array(
        &values,
        CompactTypeTag::new(layout.headers.get()),
        ValueFlags::new(0),
    )
}

fn materialize_http_request_outcome(
    image: &Arc<DeploymentExecutionImage>,
    layout: HttpResultLayout,
    result: Result<BytecodeHttpResponse, BytecodeHttpFailure>,
    heap: &mut dyn VmHeap,
) -> ResumeOutcome {
    let result = match result {
        Ok(result) => result,
        Err(error) => return http_failure_outcome(error),
    };
    let materialized = (|| {
        let headers = allocate_http_headers(heap, layout, result.headers)?;
        let body = heap.alloc_typed_bytes(
            result.body,
            CompactTypeTag::new(layout.body.get()),
            ValueFlags::new(0),
        )?;
        heap.allocate_record(
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
            CompactTypeTag::new(layout.root.get()),
            ValueFlags::new(0),
        )
    })();
    match materialized {
        Ok(value) => ResumeOutcome::Values(VmOwnedValues::from_values(
            Arc::clone(image),
            vec![value].into_boxed_slice(),
        )),
        Err(error) => ResumeOutcome::Failure(VmError::Heap(error)),
    }
}

fn materialize_http_stream_outcome(
    image: &Arc<DeploymentExecutionImage>,
    layout: HttpResultLayout,
    result: Result<BytecodeHttpStreamResponse, BytecodeHttpFailure>,
    heap: &mut dyn VmHeap,
) -> ResumeOutcome {
    let result = match result {
        Ok(result) => result,
        Err(error) => return http_failure_outcome(error),
    };
    let materialized = (|| {
        let headers = allocate_http_headers(heap, layout, result.headers)?;
        let body = heap.admit_resource_ref(
            result.body.vm_handle(),
            CompactTypeTag::new(layout.body.get()),
            ValueFlags::new(0),
        )?;
        heap.allocate_record(
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
            CompactTypeTag::new(layout.root.get()),
            ValueFlags::new(0),
        )
    })();
    match materialized {
        Ok(value) => ResumeOutcome::Values(VmOwnedValues::from_values(
            Arc::clone(image),
            vec![value].into_boxed_slice(),
        )),
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
    image: &Arc<DeploymentExecutionImage>,
    resources: &RequestResourceTable,
    handle: RequestResourceHandle,
    item_type: TypeIndex,
    result: Result<Option<Vec<u8>>, RequestByteStreamFailure>,
    heap: &mut dyn VmHeap,
) -> ResumeOutcome {
    match result {
        Ok(Some(bytes)) => match heap.alloc_typed_bytes(
            bytes,
            CompactTypeTag::new(item_type.get()),
            ValueFlags::new(0),
        ) {
            Ok(value) => ResumeOutcome::Values(VmOwnedValues::from_values(
                Arc::clone(image),
                vec![value].into_boxed_slice(),
            )),
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

fn resource_failure_outcome(message: String) -> ResumeOutcome {
    ResumeOutcome::Failure(VmError::HostEffectFailure(RuntimeErrorPayload {
        code: "InternalError".to_string(),
        message,
        status: None,
        details: None,
    }))
}

fn materialize_request_pending_outcome(
    image: &Arc<DeploymentExecutionImage>,
    resources: &RequestResourceTable,
    outcome: RequestPendingOutcome,
    heap: &mut dyn VmHeap,
) -> ResumeOutcome {
    match outcome {
        RequestPendingOutcome::Vm(outcome) => outcome,
        RequestPendingOutcome::HttpRequest { layout, result } => {
            materialize_http_request_outcome(image, layout, result, heap)
        }
        RequestPendingOutcome::HttpStream { layout, result } => {
            materialize_http_stream_outcome(image, layout, result, heap)
        }
        RequestPendingOutcome::StreamNext {
            handle,
            item_type,
            result,
        } => materialize_stream_next_outcome(image, resources, handle, item_type, result, heap),
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
        let before = completion.state();
        if matches!(before, skiff_runtime_scheduler::PendingCellState::Claimed) {
            return false;
        }
        match self.runtime.budget.pending_terminal_winner() {
            None => {
                let _ = completion.complete(RequestPendingOutcome::Vm(ResumeOutcome::Empty));
            }
            Some(winner) => complete_cell_from_winner(&completion, winner),
        }
        completion.state() != before
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
        let _ = self.wake_receiver.try_recv();
        let wake = self
            .runtime
            .wake_queue
            .pop()
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
        self.runtime.wake_queue.consume_async_signal_if_present();
        let wake = self
            .runtime
            .wake_queue
            .pop()
            .expect("a signaled pending wake queue must hold exactly one wake");
        self.resume_wake(wake)
    }

    fn resume_wake(mut self, wake: RequestPendingWake) -> ControlledBytecodeDrive {
        let resources = self.runtime.resources.clone();
        let resumed = BytecodeScheduler::<VmFiber>::resume_from_pending_wake_with(
            wake,
            self.context.ports(),
            |resume, outcome| {
                materialize_request_pending_outcome(
                    resume.image(),
                    &resources,
                    outcome,
                    &mut *self.heap,
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

    fn finish_drive(
        self,
        outcome: Result<BytecodeSchedulerOutcome<VmFiber>, BytecodeSchedulerError>,
    ) -> ControlledBytecodeDrive {
        match outcome {
            Ok(BytecodeSchedulerOutcome::Complete(result)) => self.complete(result),
            Ok(BytecodeSchedulerOutcome::Parked) => ControlledBytecodeDrive::Parked(self),
            Err(error) => self.terminal(error),
        }
    }

    fn complete(self, result: VmResult) -> ControlledBytecodeDrive {
        let ParkedBytecodeRequest {
            context,
            mut heap,
            budget,
            execution_budget,
            mode,
            raw_http_adapter,
            ..
        } = self;
        let result = project_completed_request(
            &mut *heap,
            &execution_budget,
            result,
            &mode,
            raw_http_adapter,
        );
        let snapshot = context.freeze_with_termination(resource_termination_for_result(&result));
        ControlledBytecodeDrive::Complete(DrivenBytecodeRequest {
            result,
            retention: BytecodeRequestRetention {
                heap: Some(heap),
                budget: Some(budget),
            },
            owner_inventory: DrivenBytecodeRequestOwnerInventory::Started(snapshot),
        })
    }

    fn terminal(self, error: BytecodeSchedulerError) -> ControlledBytecodeDrive {
        let ParkedBytecodeRequest {
            context,
            heap,
            budget,
            execution_budget,
            ..
        } = self;
        let result: RequestResult<BoundaryResponse> =
            Err(scheduler_error_to_request_error(&execution_budget, error));
        let snapshot = context.freeze_with_termination(resource_termination_for_result(&result));
        ControlledBytecodeDrive::Complete(DrivenBytecodeRequest {
            result,
            retention: BytecodeRequestRetention {
                heap: Some(heap),
                budget: Some(budget),
            },
            owner_inventory: DrivenBytecodeRequestOwnerInventory::Started(snapshot),
        })
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
    result: VmResult,
    mode: &str,
    raw_http_adapter: bool,
) -> RequestResult<BoundaryResponse> {
    match result {
        Ok(values) => {
            if mode == "serverStream" {
                Err(RequestError::Decode(
                    "serverStream request completed without a response stream".to_string(),
                ))
            } else if raw_http_adapter {
                http_response_from_vm_values(heap, values.values())
            } else {
                json_payload_from_value_slots(heap, values.values()).map(BoundaryResponse::payload)
            }
        }
        Err(VmError::Thrown(envelope)) => Err(uncaught_throw_to_request_error(heap, &envelope)),
        Err(error) => Err(vm_error_to_request_error(execution_budget, error)),
    }
}

fn gateway_entry_arguments(
    request: &RequestEnvelope,
    entry: &DeploymentExecutionEntry,
    heap: &mut dyn VmHeap,
) -> RequestResult<Vec<ValueSlot>> {
    let Some(adapter) = &request.http_adapter else {
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
            GatewayAdapterSource::HttpRequest => materialize_http_request(binary, heap)?,
            GatewayAdapterSource::HttpBody => match typed_json_body.as_ref() {
                Some(body) => materialize_typed_json_scalar(body, entry, ordinal)?,
                None => heap
                    .alloc_bytes(binary.body.clone())
                    .map_err(heap_error_to_request_error)?,
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

fn materialize_http_request(
    binary: &BinaryHttpRequest,
    heap: &mut dyn VmHeap,
) -> RequestResult<ValueSlot> {
    let method = heap
        .alloc_string(binary.metadata.method.clone())
        .map_err(heap_error_to_request_error)?;
    let url = heap
        .alloc_string(binary.metadata.url.clone())
        .map_err(heap_error_to_request_error)?;
    let path = heap
        .alloc_string(binary.metadata.path.clone())
        .map_err(heap_error_to_request_error)?;
    let query = materialize_name_values(&binary.metadata.query, heap)?;
    let headers = materialize_name_values(&binary.metadata.headers, heap)?;
    let body = heap
        .alloc_bytes(binary.body.clone())
        .map_err(heap_error_to_request_error)?;
    let fields = vec![
        VmRecordField {
            name: "method".to_string(),
            value: method,
        },
        VmRecordField {
            name: "url".to_string(),
            value: url,
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
            name: "headers".to_string(),
            value: headers,
        },
        VmRecordField {
            name: "body".to_string(),
            value: body,
        },
    ];
    heap.allocate_record(&fields, CompactTypeTag::new(0), ValueFlags::new(0))
        .map_err(heap_error_to_request_error)
}

fn materialize_name_values(
    items: &[HttpNameValue],
    heap: &mut dyn VmHeap,
) -> RequestResult<ValueSlot> {
    let mut records = Vec::with_capacity(items.len());
    for item in items {
        let name = heap
            .alloc_string(item.name.clone())
            .map_err(heap_error_to_request_error)?;
        let value = heap
            .alloc_string(item.value.clone())
            .map_err(heap_error_to_request_error)?;
        let record = heap
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
                CompactTypeTag::new(0),
                ValueFlags::new(0),
            )
            .map_err(heap_error_to_request_error)?;
        records.push(record);
    }
    heap.allocate_array(&records, CompactTypeTag::new(0), ValueFlags::new(0))
        .map_err(heap_error_to_request_error)
}

fn heap_error_to_request_error(error: VmHeapError) -> RequestError {
    RequestError::Decode(format!(
        "bytecode gateway heap materialization failed: {error}"
    ))
}

fn scheduler_error_to_request_error(
    execution_budget: &ExecutionBudget,
    error: BytecodeSchedulerError,
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
        BytecodeSchedulerError::Vm(error) => vm_error_to_request_error(execution_budget, error),
        BytecodeSchedulerError::Port(message) => {
            RequestError::Unsupported(format!("bytecode scheduler port failed: {message}"))
        }
    }
}

fn validate_bytecode_request(request: &RequestEnvelope) -> RequestResult<()> {
    if request.mode != "unary" {
        return Err(RequestError::Unsupported(format!(
            "bytecode scalar ingress only supports unary request.start, got {}",
            request.mode
        )));
    }
    validate_bytecode_request_metadata(request)
}

fn validate_bytecode_request_metadata(request: &RequestEnvelope) -> RequestResult<()> {
    if request.ingress_selector.is_none() {
        return Err(RequestError::Unsupported(
            "bytecode scalar ingress requires request.start ingress_selector".to_string(),
        ));
    }
    if request.extra.contains_key("actorCall") {
        return Err(RequestError::Unsupported(
            "actor.call request.start metadata is not supported by bytecode scalar ingress"
                .to_string(),
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
    match error {
        VmError::BudgetClosed(error) => vm_budget_closed_to_request_error(execution_budget, error),
        VmError::InternalTerminal(VmInternalTerminal::Budget(error)) => {
            vm_budget_closed_to_request_error(execution_budget, error)
        }
        VmError::InternalTerminal(VmInternalTerminal::OwnerStopped) => RequestError::Cancelled,
        VmError::HostEffectFailure(payload) => RequestError::ExternalErrorPayload {
            code: payload.code,
            message: payload.message,
            status: payload.status,
            details: payload.details,
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

/// Projects a root uncaught user throw to the canonical ordinary error. The
/// payload is materialized to JSON when the Phase 2 surface can encode it;
/// private or unencodable payloads are suppressed rather than leaking fields
/// or strings.
fn uncaught_throw_to_request_error(
    heap: &mut dyn VmHeap,
    envelope: &RequestException,
) -> RequestError {
    let details = envelope
        .vm_local_slot()
        .and_then(|slot| json_value_from_slot(heap, &slot, 0).ok());
    RequestError::ExternalErrorPayload {
        code: "std.service.InternalError".to_string(),
        message: "uncaught user exception".to_string(),
        status: None,
        details,
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
mod tests {
    use std::collections::HashMap;

    use skiff_artifact_model::{IngressProtocol, IngressSelector};
    use skiff_runtime_model::{request_heap::RequestHeapLimits, vm_value::ValueSlot};

    use super::*;
    use crate::{
        BinaryHttpRequest, BinaryHttpRequestMetadata, HttpAdapter, HttpAdapterCallable,
        HttpAdapterKind, RequestEnvelope, ResponseEnd, ResponseEvent,
    };

    fn test_pending_runtime(
        budget: Arc<ExecutionBudget>,
        cancellation: CancellationToken,
    ) -> (Arc<RequestPendingRuntime>, RequestExecutionContext<VmFiber>) {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let (wake_queue, _wake_receiver) = RequestPendingWakeQueue::new();
        let resources = context.resource_table();
        let runtime = Arc::new(RequestPendingRuntime {
            registry: Arc::new(RequestPendingRegistry::new(context.pending_registration())),
            wake_queue,
            budget: Arc::clone(&budget),
            resources: resources.clone(),
            http_client: None,
            execution_control: ExecutionControl::new(cancellation, &budget).owned(),
            stream_registrar: BytecodeHttpStreamRegistrar::new(resources),
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
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let registry =
            PendingRegistry::<u8, &'static str, &'static str>::new(context.pending_registration());
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
        let executor = BytecodeHostExecutor {
            runtime: Arc::clone(&runtime),
        };

        assert!(matches!(
            executor.ready_terminal(),
            Some(ResumeOutcome::InternalTerminal(
                VmInternalTerminal::OwnerStopped
            ))
        ));
        assert_eq!(
            budget.settlement().unwrap().winner(),
            ExecutionWinner::Cancelled
        );
        drop(executor);
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
        let executor = BytecodeHostExecutor {
            runtime: Arc::clone(&runtime),
        };

        assert!(matches!(
            executor.ready_terminal(),
            Some(ResumeOutcome::InternalTerminal(VmInternalTerminal::Budget(
                VmBudgetClosed::DeadlineExceeded
            )))
        ));
        assert_eq!(
            budget.settlement().unwrap().winner(),
            ExecutionWinner::DeadlineExceeded
        );
        drop(executor);
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
            .begin(RootEscrow::new(Box::new(HostEffectArgumentRoots(
                Vec::new(),
            ))))
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
        let empty_slot = heap.alloc_bytes(Vec::new()).unwrap();
        let present_empty = decode_optional_http_body(&mut heap, &empty_slot).unwrap();

        assert_eq!(null, None);
        assert_eq!(present_empty, Some(Vec::new()));
        assert_ne!(null, present_empty);
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
                CompactTypeTag::new(0),
                ValueFlags::new(0),
            )
            .unwrap();
        let tags = heap
            .allocate_array(
                &[ValueSlot::number(1.0), ValueSlot::number(2.0)],
                CompactTypeTag::new(0),
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
                CompactTypeTag::new(0),
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
        let name = heap.alloc_string("content-type".to_string()).unwrap();
        let value = heap.alloc_string("text/plain".to_string()).unwrap();
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
                CompactTypeTag::new(0),
                ValueFlags::new(0),
            )
            .unwrap();
        let headers = heap
            .allocate_array(&[header], CompactTypeTag::new(0), ValueFlags::new(0))
            .unwrap();
        let body = heap.alloc_bytes(b"ok".to_vec()).unwrap();
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
                CompactTypeTag::new(0),
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
    fn validation_requires_unary_and_canonical_selector() {
        assert!(validate_bytecode_request(&request()).is_ok());

        let mut selector_request = request();
        selector_request.ingress_selector = None;
        let error = validate_bytecode_request(&selector_request).expect_err("selector is required");
        assert!(error.to_string().contains("ingress_selector"));

        let mut mode_request = request();
        mode_request.mode = "serverStream".to_string();
        let error = validate_bytecode_request(&mode_request).expect_err("mode is validated");
        assert!(error.to_string().contains("unary"));
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
        assert!(validate_bytecode_request(&binary_request).is_ok());

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
        assert!(validate_bytecode_request(&adapter_request).is_ok());

        let mut actor_request = request();
        actor_request
            .extra
            .insert("actorCall".to_string(), serde_json::json!({}));
        assert!(validate_bytecode_request(&actor_request).is_err());
    }

    #[test]
    fn scheduler_fail_closed_errors_map_to_unsupported() {
        let budget = ExecutionBudget::for_runtime_request(None);
        assert!(matches!(
            scheduler_error_to_request_error(&budget, BytecodeSchedulerError::UnsupportedChild),
            RequestError::Unsupported(message) if message.contains("child executor port")
        ));
        assert!(matches!(
            scheduler_error_to_request_error(&budget, BytecodeSchedulerError::UnsupportedAdapter),
            RequestError::Unsupported(message) if message.contains("child executor port")
        ));
        assert!(matches!(
            scheduler_error_to_request_error(&budget, BytecodeSchedulerError::UnsupportedStream),
            RequestError::Unsupported(message) if message.contains("stream supervisor")
        ));
        assert!(matches!(
            scheduler_error_to_request_error(&budget, BytecodeSchedulerError::UnsupportedPark),
            RequestError::Unsupported(message) if message.contains("stream supervisor")
        ));
        assert!(matches!(
            scheduler_error_to_request_error(
                &budget,
                BytecodeSchedulerError::ChildCapacityExceeded
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
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let registry = RequestPendingRegistry::new(context.pending_registration());
        (registry, context)
    }

    #[test]
    fn cancellation_sink_settles_the_parked_cell_once_through_the_budget() {
        let budget = Arc::new(ExecutionBudget::for_runtime_request(None));
        let (registry, _context) = pending_registry();
        let completion = registry
            .begin(RootEscrow::new(Box::new(HostEffectArgumentRoots(
                Vec::new(),
            ))))
            .unwrap();
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
        let completion = registry
            .begin(RootEscrow::new(Box::new(HostEffectArgumentRoots(
                Vec::new(),
            ))))
            .unwrap();
        let sink: Arc<dyn RequestPendingSink> = Arc::new(PendingCellSink {
            completion: completion.clone(),
        });

        let winner = budget.register_pending_sink(sink);
        assert_eq!(winner, Some(ExecutionWinner::DeadlineExceeded));
        complete_cell_from_winner(&completion, winner.unwrap());
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
        let completion = registry
            .begin(RootEscrow::new(Box::new(HostEffectArgumentRoots(
                Vec::new(),
            ))))
            .unwrap();
        let (wake_queue, _wake_receiver) = RequestPendingWakeQueue::new();
        let runtime = Arc::new(RequestPendingRuntime {
            registry: Arc::new(registry),
            wake_queue,
            budget: Arc::clone(&budget),
            resources: _context.resource_table(),
            http_client: None,
            execution_control: ExecutionControl::new(CancellationToken::new(), &budget).owned(),
            stream_registrar: BytecodeHttpStreamRegistrar::new(_context.resource_table()),
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
}
