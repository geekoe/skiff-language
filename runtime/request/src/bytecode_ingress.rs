use std::{
    num::{NonZeroU32, NonZeroUsize},
    sync::{mpsc, Arc, Mutex},
};

use skiff_artifact_model::TypeRefIr;
use skiff_runtime_boundary::http::HttpBoundaryNameValue;
use skiff_runtime_capability_context::{CancellationToken, ExecutionBudgetReason};
use skiff_runtime_linked_bytecode::{LinkedValueDropPlan, LinkedValueTransferPlan};
use skiff_runtime_linker::DeploymentExecutionEntry;
use skiff_runtime_model::{
    bytecode_execution_observation::{
        BytecodeExecutionObserver, RequestExecutionOwnerInventorySnapshot,
    },
    request_heap::RequestHeapLimits,
    service_error::{ErrorCorrelation, RequestException},
    vm_heap::{VmContainerShape, VmHeap, VmHeapError, VmRecordField},
    vm_root::{VmRootSource, VmRootVisitor},
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};
use skiff_runtime_scheduler::{
    BytecodeAdapterHandoff, BytecodeChildExecutor, BytecodeChildStart, BytecodeScheduler,
    BytecodeSchedulerError, BytecodeSchedulerOutcome, BytecodeSchedulerPorts, PendingWakeQueue,
    RequestExecutionContext, RootDisposition, RootEscrow, RootEscrowBacking, SuspendedTrampoline,
    VmCompletionHandle, VmPendingRegistry, VmPendingWake,
};
use skiff_runtime_vm::{
    AdapterInvocation, PendingOperation, ResumeOutcome, Vm, VmBudget, VmBudgetClosed,
    VmBudgetTerminal, VmError, VmFiber, VmInternalTerminal, VmLimits, VmResult, VmResumeToken,
};

use crate::{
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

/// The sole pinned host effect admitted into the Phase 4 actual-Pending lane.
const SLEEP_BINDING_KEY: &str = "std.time.sleep";

/// Everything needed to project and retain one started request across any
/// number of park/resume cycles.
struct BytecodeStart {
    fiber: VmFiber,
    heap: Box<dyn VmHeap + Send>,
    budget: Box<dyn VmBudget + Send>,
    execution_budget: Arc<ExecutionBudget>,
    mode: String,
    raw_http_adapter: bool,
}

fn start_bytecode_request(input: BytecodeRequestExecutionInput) -> RequestResult<BytecodeStart> {
    let BytecodeRequestExecutionInput {
        target,
        request,
        observer,
        cancellation,
        execution_budget,
        handles,
        heap: injected_heap,
    } = input;

    let mode = request.mode.clone();
    let raw_http_adapter = request
        .http_adapter
        .as_ref()
        .is_some_and(|adapter| adapter.kind == HttpAdapterKind::RawHttp);
    validate_bytecode_request(&request)?;
    ExecutionControl::new(cancellation.clone(), &execution_budget)
        .check_cancelled()
        .map_err(RequestError::from)?;
    let mut heap: Box<dyn VmHeap + Send> = match injected_heap {
        Some(heap) => heap,
        None => Box::new(RequestVmHeap::new(handles.request_heap_limits)),
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
    let start = match start_bytecode_request(input) {
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
    let runtime = Arc::new(RequestPendingRuntime {
        registry: Arc::new(VmPendingRegistry::new(context.pending_registration())),
        wake_queue,
        budget: Arc::clone(&start.execution_budget),
        completion: Mutex::new(None),
    });
    let mut context = context.with_ports(BytecodeSchedulerPorts {
        child_executor: Some(Arc::new(SleepHostExecutor {
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
struct SleepArgumentRoots(Vec<ValueSlot>);

impl VmRootSource for SleepArgumentRoots {
    fn visit_roots(&self, visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
        for root in &self.0 {
            visitor.visit_root(root)?;
        }
        Ok(())
    }
}

impl RootEscrowBacking for SleepArgumentRoots {
    fn root_count(&self) -> usize {
        self.0.len()
    }

    fn restore_roots(self: Box<Self>) {}

    fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {}
}

/// Runtime-neutral runnable queue for claimed pending wakes.
///
/// Every wake stays root-enumerable while queued; `enqueue` also signals the
/// single parked-request receiver so a resume can drain exactly one wake.
struct RequestPendingWakeQueue {
    wakes: Mutex<Vec<VmPendingWake<VmSuspended>>>,
    signal: mpsc::Sender<()>,
}

impl RequestPendingWakeQueue {
    fn new() -> (Arc<Self>, mpsc::Receiver<()>) {
        let (signal, receiver) = mpsc::channel();
        (
            Arc::new(Self {
                wakes: Mutex::new(Vec::new()),
                signal,
            }),
            receiver,
        )
    }

    fn pop(&self) -> Option<VmPendingWake<VmSuspended>> {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
    }
}

impl PendingWakeQueue<VmResumeToken, VmSuspended, ResumeOutcome> for RequestPendingWakeQueue {
    fn enqueue(&self, wake: VmPendingWake<VmSuspended>) {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(wake);
        let _ = self.signal.send(());
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
    registry: Arc<VmPendingRegistry<VmSuspended>>,
    wake_queue: Arc<RequestPendingWakeQueue>,
    budget: Arc<ExecutionBudget>,
    completion: Mutex<Option<VmCompletionHandle<VmSuspended>>>,
}

/// Converts the budget's single authoritative winner into the exact pending
/// cell settlement. The cell arbiter drops every duplicate, so this path can
/// never produce a second terminal.
fn complete_cell_from_winner(
    completion: &VmCompletionHandle<VmSuspended>,
    winner: ExecutionWinner,
) {
    let outcome = ResumeOutcome::InternalTerminal(match winner {
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
    });
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

struct PendingCellSink {
    completion: VmCompletionHandle<VmSuspended>,
}

impl RequestPendingSink for PendingCellSink {
    fn on_terminal(&self, winner: ExecutionWinner) {
        complete_cell_from_winner(&self.completion, winner);
    }
}

/// Typed executor slot for the sole pinned pending host effect.
///
/// Every other host effect fails closed with `UnsupportedAdapter`, exactly as
/// it did before Phase 4; only `std.time.sleep` enters the pending registry.
struct SleepHostExecutor {
    runtime: Arc<RequestPendingRuntime>,
}

impl BytecodeChildExecutor<VmFiber> for SleepHostExecutor {
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
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeAdapterHandoff<VmFiber>, BytecodeSchedulerError> {
        let adapter_index = invocation.adapter();
        let image = invocation.resume().image();
        let adapter = image
            .host_effect_adapters()
            .get(adapter_index.get() as usize)
            .filter(|row| row.index() == adapter_index)
            .ok_or_else(|| {
                BytecodeSchedulerError::Port(
                    "pending host effect adapter row is absent from the pinned image".to_string(),
                )
            })?;
        if adapter.binding_key().as_str() != SLEEP_BINDING_KEY {
            return Err(BytecodeSchedulerError::UnsupportedAdapter);
        }
        let (_adapter, arguments, resume) = invocation.into_parts();
        let escrow = RootEscrow::new(Box::new(SleepArgumentRoots(arguments.values().to_vec())));
        let completion = self
            .runtime
            .registry
            .begin(escrow)
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
        let sink: Arc<dyn RequestPendingSink> = Arc::new(PendingCellSink {
            completion: completion.clone(),
        });
        if let Some(winner) = self.runtime.budget.register_pending_sink(sink) {
            complete_cell_from_winner(&completion, winner);
        }
        *self
            .runtime
            .completion
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(completion.clone());
        Ok(BytecodeAdapterHandoff::Pending(
            resume.into_pending(completion.ticket()),
        ))
    }

    fn park_adapter(
        &self,
        operation: PendingOperation,
        suspended: VmSuspended,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<(), BytecodeSchedulerError> {
        let queue: Arc<dyn PendingWakeQueue<VmResumeToken, VmSuspended, ResumeOutcome>> =
            self.runtime.wake_queue.clone();
        self.runtime
            .registry
            .publish_operation(operation, suspended, queue)
            .map(|_| ())
            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))
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
            .completion
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
                let _ = completion.complete(ResumeOutcome::Empty);
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
        parked.pending_completion().complete();
        parked.resume()
    }

    /// Drains exactly one claimed wake and restores the original VM site.
    ///
    /// The restored scheduler runs once; a second park suspends the chain
    /// again and returns a fresh [`ControlledBytecodeDrive::Parked`].
    pub fn resume(mut self) -> ControlledBytecodeDrive {
        self.wake_receiver
            .recv()
            .expect("a parked bytecode request must be completed before resume");
        let wake = self
            .runtime
            .wake_queue
            .pop()
            .expect("a signaled pending wake queue must hold exactly one wake");
        match BytecodeScheduler::<VmFiber>::resume_from_pending_wake(wake, self.context.ports()) {
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
        let snapshot = context.freeze();
        let result = project_completed_request(
            &mut *heap,
            &execution_budget,
            result,
            &mode,
            raw_http_adapter,
        );
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
        let snapshot = context.freeze();
        ControlledBytecodeDrive::Complete(DrivenBytecodeRequest {
            result: Err(scheduler_error_to_request_error(&execution_budget, error)),
            retention: BytecodeRequestRetention {
                heap: Some(heap),
                budget: Some(budget),
            },
            owner_inventory: DrivenBytecodeRequestOwnerInventory::Started(snapshot),
        })
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

    fn pending_registry() -> (
        VmPendingRegistry<VmSuspended>,
        RequestExecutionContext<VmFiber>,
    ) {
        let context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());
        let registry = VmPendingRegistry::new(context.pending_registration());
        (registry, context)
    }

    #[test]
    fn cancellation_sink_settles_the_parked_cell_once_through_the_budget() {
        let budget = Arc::new(ExecutionBudget::for_runtime_request(None));
        let (registry, _context) = pending_registry();
        let completion = registry
            .begin(RootEscrow::new(Box::new(SleepArgumentRoots(Vec::new()))))
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
            .begin(RootEscrow::new(Box::new(SleepArgumentRoots(Vec::new()))))
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
            .begin(RootEscrow::new(Box::new(SleepArgumentRoots(Vec::new()))))
            .unwrap();
        let (wake_queue, _wake_receiver) = RequestPendingWakeQueue::new();
        let runtime = Arc::new(RequestPendingRuntime {
            registry: Arc::new(registry),
            wake_queue,
            budget: Arc::clone(&budget),
            completion: Mutex::new(Some(completion.clone())),
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
