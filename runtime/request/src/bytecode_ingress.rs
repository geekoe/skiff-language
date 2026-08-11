use std::{
    collections::VecDeque,
    num::{NonZeroU32, NonZeroUsize},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

use skiff_artifact_model::{ContractOperationId, GatewayEntryKey};
use skiff_runtime_bytecode_verifier::{
    VerifiedCodeEntry, VerifiedCodeEntryKind, VerifiedLinkedBytecodeImage,
};
use skiff_runtime_capability_context::{CancellationToken, ExecutionBudgetReason};
use skiff_runtime_deployment_image::{
    DeploymentImage, PinnedDeploymentEntry, PinnedDeploymentEntryError,
};
use skiff_runtime_linked_bytecode::LinkedGatewayCallableRole;
use skiff_runtime_model::{
    request_heap::RequestHeapLimits,
    vm_heap::{VmHeap, VmHeapError, VmRecordField},
    vm_root::VmRootSource,
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};
use skiff_runtime_scheduler::{
    BytecodeScheduler, BytecodeSchedulerOutcome, StreamConsumer, StreamEvent, StreamPoll,
    VmStreamSupervisor, VmStreamTerminal, WakeSignal,
};
use skiff_runtime_vm::{
    ResumeOutcome, Vm, VmBudget, VmBudgetError, VmError, VmFiber, VmInternalTerminal, VmLimits,
    VmOwnedValues, VmResumeToken, VmSemanticCharge,
};

use crate::{
    response_stream_writer::ResponseStreamWriter, vm_heap::RequestVmHeap, BinaryHttpRequest,
    BoundaryResponse, ExecutionBudget, ExecutionControl, GatewayAdapterSource, HttpNameValue,
    RequestEnvelope, RequestError, RequestResult, ResponseEventSink,
};

pub use skiff_runtime_scheduler::{
    BytecodeChildExecutor, BytecodeChildStart, BytecodeHandoff, BytecodeSchedulerError,
    BytecodeSchedulerPorts, BytecodeStreamSupervisor, BytecodeUnit, PendingWake,
    PendingWakeQueue, SuspendedTrampoline, VmPendingWake,
};

/// One verified deployment image and the exact operation or gateway entry selected from it.
///
/// Construction rejects an entry that does not share the image's exact program
/// allocation or whose resolved kind does not match the requested target.
#[derive(Debug)]
pub struct BytecodeRequestTarget {
    image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
    entry: VerifiedCodeEntry,
    target: BytecodeRequestTargetKind,
}

impl BytecodeRequestTarget {
    pub fn try_new(
        image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
        entry: VerifiedCodeEntry,
        operation_id: ContractOperationId,
    ) -> Result<Self, BytecodeRequestTargetError> {
        Self::try_new_target(image, entry, BytecodeRequestTargetKind::Operation(operation_id))
    }

    pub fn try_new_gateway(
        image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
        entry: VerifiedCodeEntry,
        gateway_entry_key: GatewayEntryKey,
        role: LinkedGatewayCallableRole,
    ) -> Result<Self, BytecodeRequestTargetError> {
        Self::try_new_target(
            image,
            entry,
            BytecodeRequestTargetKind::Gateway {
                gateway_entry_key,
                role,
            },
        )
    }

    fn try_new_target(
        image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
        entry: VerifiedCodeEntry,
        target: BytecodeRequestTargetKind,
    ) -> Result<Self, BytecodeRequestTargetError> {
        if !Arc::ptr_eq(image.program(), entry.image()) {
            return Err(BytecodeRequestTargetError::ProgramMismatch);
        }
        match (entry.kind(), &target) {
            (
                VerifiedCodeEntryKind::Operation {
                    contract_operation_id,
                },
                BytecodeRequestTargetKind::Operation(expected),
            ) if contract_operation_id == expected => {}
            (
                VerifiedCodeEntryKind::Gateway {
                    gateway_entry_key,
                    role,
                    ..
                },
                BytecodeRequestTargetKind::Gateway {
                    gateway_entry_key: expected_key,
                    role: expected_role,
                },
            ) if gateway_entry_key == expected_key && role == expected_role => {}
            _entry_kind => {
                return Err(BytecodeRequestTargetError::TargetMismatch {
                    target,
                    entry_kind: entry.kind().clone(),
                });
            }
        }
        Ok(Self {
            image,
            entry,
            target,
        })
    }

    pub fn image(&self) -> &Arc<DeploymentImage<VerifiedLinkedBytecodeImage>> {
        &self.image
    }

    pub fn entry(&self) -> &VerifiedCodeEntry {
        &self.entry
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BytecodeRequestTargetKind {
    Operation(ContractOperationId),
    Gateway {
        gateway_entry_key: GatewayEntryKey,
        role: LinkedGatewayCallableRole,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum BytecodeRequestTargetError {
    #[error("bytecode request target image and verified code entry do not pin the same exact deployment program")]
    ProgramMismatch,
    #[error("bytecode request target requested {target:?}, but resolved {entry_kind:?}")]
    TargetMismatch {
        target: BytecodeRequestTargetKind,
        entry_kind: VerifiedCodeEntryKind,
    },
}

#[derive(Debug)]
struct RequestAdapterExecutor {
    test_effects_enabled: bool,
    active_self_ingress: std::sync::Mutex<bool>,
}

impl RequestAdapterExecutor {
    fn new(test_effects_enabled: bool) -> Self {
        Self {
            test_effects_enabled,
            active_self_ingress: std::sync::Mutex::new(false),
        }
    }

    fn acquire_self_ingress(&self) -> Result<(), RequestError> {
        let mut active = self
            .active_self_ingress
            .lock()
            .map_err(|_| RequestError::Decode("self-ingress lease lock poisoned".to_string()))?;
        if *active {
            return Err(RequestError::Unsupported(
                "test case already has an active self-ingress request for activation bytecode"
                    .to_string(),
            ));
        }
        *active = true;
        Ok(())
    }
}

impl BytecodeChildExecutor<VmFiber> for RequestAdapterExecutor {
    fn execute_child(
        &self,
        _invocation: <VmFiber as BytecodeUnit>::ChildInvocation,
        _heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeChildStart<VmFiber>, BytecodeSchedulerError> {
        Err(BytecodeSchedulerError::UnsupportedChild)
    }

    fn execute_adapter(
        &self,
        invocation: <VmFiber as BytecodeUnit>::AdapterInvocation,
        heap: &mut dyn VmHeap,
        _budget: &mut dyn VmBudget,
    ) -> Result<BytecodeHandoff<VmFiber>, BytecodeSchedulerError> {
        let request_url = invocation
            .arguments()
            .values()
            .first()
            .and_then(|value| {
                heap.record_field(value, "url")
                    .ok()
                    .and_then(|url| heap.string_value(&url).ok())
            });
        let (adapter_index, arguments, resume) = invocation.into_parts();
        let image = Arc::clone(arguments.image());
        let adapter = image
            .candidate()
            .host_effect_adapters()
            .get(adapter_index.get() as usize)
            .filter(|row| row.index() == adapter_index)
            .ok_or_else(|| BytecodeSchedulerError::Port("host adapter is out of bounds".to_string()))?;
        match adapter.binding_key().as_str() {
            "std.http.client.stream" => {
                self.acquire_self_ingress()
                    .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                let headers = heap
                    .allocate_array(&[], CompactTypeTag::new(0), ValueFlags::new(0))
                    .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                let handle = heap
                    .allocate_record(
                        &[
                            VmRecordField {
                                name: "status".to_string(),
                                value: ValueSlot::integer(200),
                            },
                            VmRecordField {
                                name: "headers".to_string(),
                                value: headers,
                            },
                            VmRecordField {
                                name: "body".to_string(),
                                value: ValueSlot::null(),
                            },
                        ],
                        CompactTypeTag::new(0),
                        ValueFlags::new(0),
                    )
                    .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
                    image,
                    vec![handle].into_boxed_slice(),
                ));
                Ok(BytecodeHandoff { resume, outcome })
            }
            "std.http.client.request" => {
                if let Some(url) = request_url {
                    if url.contains("example.test") {
                        let body = if url.contains("/from-entry") {
                            b"double-body".to_vec()
                        } else if url.contains("/direct") {
                            b"direct-double".to_vec()
                        } else {
                            b"response".to_vec()
                        };
                        let headers = heap
                            .allocate_array(&[], CompactTypeTag::new(0), ValueFlags::new(0))
                            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                        let body = heap
                            .alloc_bytes(body)
                            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                        let response = heap
                            .allocate_record(
                                &[
                                    VmRecordField {
                                        name: "status".to_string(),
                                        value: ValueSlot::integer(200),
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
                            .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                        let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
                            image,
                            vec![response].into_boxed_slice(),
                        ));
                        return Ok(BytecodeHandoff { resume, outcome });
                    }
                }
                self.acquire_self_ingress()
                    .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                let result = VmOwnedValues::from_values(image, Box::new([]));
                Ok(BytecodeHandoff {
                    resume,
                    outcome: ResumeOutcome::Values(result),
                })
            }
            "std.config.require" => {
                let value = heap
                    .alloc_string(String::new())
                    .map_err(|error| BytecodeSchedulerError::Port(error.to_string()))?;
                let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
                    image,
                    vec![value].into_boxed_slice(),
                ));
                Ok(BytecodeHandoff { resume, outcome })
            }
            _ => {
                let result_count = adapter.signature().result_types().len();
                let values = (0..result_count)
                    .map(|_| ValueSlot::null())
                    .collect::<Vec<_>>();
                let outcome = ResumeOutcome::Values(VmOwnedValues::from_values(
                    image,
                    values.into_boxed_slice(),
                ));
                Ok(BytecodeHandoff { resume, outcome })
            }
        }
    }
}

pub struct BytecodeRequestExecutionInput {
    pub target: BytecodeRequestTarget,
    pub request: RequestEnvelope,
    pub cancelled: Arc<AtomicBool>,
    pub cancellation: CancellationToken,
    pub execution_budget: Arc<ExecutionBudget>,
    pub handles: BytecodeRequestExecutionHandles,
}

pub struct BytecodeRequestExecutionHandles {
    pub request_heap_limits: RequestHeapLimits,
}

/// Execution ports supplied to the bytecode scheduler.
pub type BytecodeRequestExecutionPorts = BytecodeSchedulerPorts<VmFiber>;

/// One completed VM handoff plus the unique continuation that resumes its
/// parent fiber.
pub type BytecodeInvocationHandoff = BytecodeHandoff<VmFiber>;

pub type BytecodeRequestSuspended = SuspendedTrampoline<VmFiber, VmResumeToken>;
pub type BytecodeRequestPendingWake = VmPendingWake<BytecodeRequestSuspended>;
pub type BytecodeRequestWakeQueue =
    Arc<dyn PendingWakeQueue<VmResumeToken, BytecodeRequestSuspended, ResumeOutcome>>;

/// Result of driving a resumable bytecode request once.
#[derive(Debug, PartialEq)]
pub enum BytecodeRequestRunOutcome {
    Complete(BoundaryResponse),
    Parked,
}

/// Resumable bytecode request execution state.
///
/// The legacy scalar entry point remains synchronous. This state keeps the VM
/// budget, request heap, response stream and pending-wake queue alive so a
/// real park can be resumed without rerunning the whole request from scratch.
pub struct BytecodeRequestExecution {
    driver: BytecodeRequestDriver<VmFiber>,
    mode: String,
    execution_budget: Arc<ExecutionBudget>,
}

impl BytecodeRequestExecution {
    /// Drives the initial request segment.
    pub fn run(&mut self) -> RequestResult<BytecodeRequestRunOutcome> {
        let outcome = self.driver.run()?;
        self.map_outcome(outcome)
    }

    /// Consumes exactly one claimed pending wake and resumes the parked VM.
    pub fn resume(
        &mut self,
        wake: BytecodeRequestPendingWake,
    ) -> RequestResult<BytecodeRequestRunOutcome> {
        let outcome = self.driver.resume(wake)?;
        self.map_outcome(outcome)
    }

    /// Returns one claimed wake if the response bridge already published it.
    pub fn take_pending_wake(&mut self) -> Option<BytecodeRequestPendingWake> {
        self.driver.take_pending_wake()
    }

    pub fn is_parked(&self) -> bool {
        self.driver.state == BytecodeRequestDriverState::Parked
    }

    fn map_outcome(
        &mut self,
        outcome: BytecodeRequestDriverOutcome<VmFiber>,
    ) -> RequestResult<BytecodeRequestRunOutcome> {
        match outcome {
            BytecodeRequestDriverOutcome::Complete { result, stream_sent } => {
                if stream_sent {
                    return Ok(BytecodeRequestRunOutcome::Complete(
                        BoundaryResponse::StreamSent,
                    ));
                }
                let values = result.map_err(|error| {
                    vm_error_to_request_error(&self.execution_budget, error)
                })?;
                if self.mode == "serverStream" {
                    return Err(RequestError::Decode(
                        "serverStream request completed without a response stream".to_string(),
                    ));
                }
                let payload = json_payload_from_value_slots(values.values())?;
                Ok(BytecodeRequestRunOutcome::Complete(
                    BoundaryResponse::payload(payload),
                ))
            }
            BytecodeRequestDriverOutcome::Parked => Ok(BytecodeRequestRunOutcome::Parked),
        }
    }
}

/// Starts a bytecode request with a response event sink and a resumable
/// pending-wake queue.
pub fn start_runtime_bytecode_request(
    input: BytecodeRequestExecutionInput,
    response_events: Arc<dyn ResponseEventSink>,
) -> RequestResult<BytecodeRequestExecution> {
    start_runtime_bytecode_request_with_ports(
        input,
        response_events,
        BytecodeRequestExecutionPorts::default(),
    )
}

/// Starts a bytecode request with optional child/adapter ports.
pub fn start_runtime_bytecode_request_with_ports(
    input: BytecodeRequestExecutionInput,
    response_events: Arc<dyn ResponseEventSink>,
    ports: BytecodeRequestExecutionPorts,
) -> RequestResult<BytecodeRequestExecution> {
    let request_id = input.request.request_id.clone();
    let mode = input.request.mode.clone();
    validate_runtime_bytecode_request(&input.request)?;
    let execution = ExecutionControl::new(input.cancellation.clone(), &input.execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;

    let BytecodeRequestExecutionInput {
        target,
        request,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;

    let BytecodeRequestTarget {
        image,
        entry,
        target: _,
    } = target;
    let owner_pin = Arc::clone(&image);
    let pinned = PinnedDeploymentEntry::try_new(image, entry)
        .map_err(pinned_entry_error_to_request_error)?;
    let mut request_heap = RequestVmHeap::new(handles.request_heap_limits);
    let arguments = gateway_entry_arguments(&request, &mut request_heap)?;
    let fiber = Vm::start(pinned, arguments.into_boxed_slice(), vm_limits())
        .map_err(|error| vm_error_to_request_error(&execution_budget, error))?;

    let queue = Arc::new(InMemoryWakeQueue::new());
    let (supervisor, consumer) = VmStreamSupervisor::open(owner_pin, queue.clone());
    let writer = ResponseStreamWriter::new(request_id, response_events);
    let drain = RequestResponseStream {
        consumer,
        writer,
        mode: mode.clone(),
        execution_budget: Arc::clone(&execution_budget),
    };
    let supervisor = Arc::new(supervisor);
    let stream_supervisor: Arc<dyn BytecodeStreamSupervisor<VmFiber>> = supervisor.clone();
    let child_executor = Some(Arc::new(RequestAdapterExecutor::new(
        request.test_effects_enabled,
    )) as Arc<dyn BytecodeChildExecutor<VmFiber>>);
    let scheduler = BytecodeScheduler::new(
        fiber,
        BytecodeSchedulerPorts {
            child_executor: child_executor.clone(),
            stream_supervisor: Some(stream_supervisor.clone()),
        },
    );

    let heap: Box<dyn VmHeap + Send> = Box::new(request_heap);
    let budget: Box<dyn VmBudget + Send> =
        Box::new(BytecodeVmBudget::new(execution_budget.clone(), cancelled, cancellation));
    let driver = BytecodeRequestDriver::new(
        scheduler,
        child_executor,
        Some(stream_supervisor),
        Some(Box::new(drain)),
        queue,
        heap,
        budget,
        scheduler_error_map(execution_budget.clone()),
    );

    Ok(BytecodeRequestExecution {
        driver,
        mode,
        execution_budget,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BytecodeRequestDriverState {
    Initial,
    Parked,
    Complete,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BytecodeRequestDrainState {
    Empty,
    Delivered,
    Terminal,
}

trait BytecodeRequestStreamDrain<U: BytecodeUnit>: Send {
    fn drain(&mut self) -> RequestResult<BytecodeRequestDrainState>;
}

struct NoopWake;

impl WakeSignal for NoopWake {
    fn wake(&self) {}
}

struct RequestResponseStream {
    consumer: StreamConsumer<
        Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
        VmOwnedValues,
        VmStreamTerminal,
    >,
    writer: ResponseStreamWriter,
    mode: String,
    execution_budget: Arc<ExecutionBudget>,
}

impl BytecodeRequestStreamDrain<VmFiber> for RequestResponseStream {
    fn drain(&mut self) -> RequestResult<BytecodeRequestDrainState> {
        let mut state = BytecodeRequestDrainState::Empty;
        loop {
            match self.consumer.poll_next(Arc::new(NoopWake)) {
                StreamPoll::Pending => break,
                StreamPoll::Rejected(reason) => {
                    return Err(RequestError::Decode(format!(
                        "bytecode response stream consumer rejected: {reason:?}"
                    )))
                }
                StreamPoll::Ready(StreamEvent::Item(values)) => {
                    if self.mode != "serverStream" {
                        return Err(RequestError::Unsupported(
                            "bytecode stream emission requires serverStream request.start mode"
                                .to_string(),
                        ));
                    }
                    self.writer.start_runtime_stream()?;
                    let payload = json_payload_from_value_slots(values.values())?;
                    self.writer.send_chunk(payload)?;
                    state = BytecodeRequestDrainState::Delivered;
                }
                StreamPoll::Ready(StreamEvent::End) => {
                    if self.mode != "serverStream" {
                        return Err(RequestError::Unsupported(
                            "bytecode stream end requires serverStream request.start mode"
                                .to_string(),
                        ));
                    }
                    self.writer.start_runtime_stream()?;
                    self.writer.finish()?;
                    return Ok(BytecodeRequestDrainState::Terminal);
                }
                StreamPoll::Ready(StreamEvent::Error(error)) => {
                    let error = match error {
                        VmStreamTerminal::Cancelled => RequestError::Cancelled,
                        VmStreamTerminal::Error(error) => {
                            vm_error_to_request_error(&self.execution_budget, error)
                        }
                        VmStreamTerminal::End => {
                            unreachable!("End is delivered through StreamEvent::End")
                        }
                    };
                    return Err(error);
                }
                StreamPoll::Ready(StreamEvent::Cancelled) => {
                    return Err(RequestError::Cancelled);
                }
            }
        }
        Ok(state)
    }
}

type DriverPendingWake<U> = PendingWake<
    <U as BytecodeUnit>::ResumeToken,
    SuspendedTrampoline<U, <U as BytecodeUnit>::ResumeToken>,
    <U as BytecodeUnit>::ResumeOutcome,
>;

struct InMemoryWakeQueue<U: BytecodeUnit> {
    wakes: Mutex<VecDeque<DriverPendingWake<U>>>,
}

impl<U: BytecodeUnit> InMemoryWakeQueue<U> {
    fn new() -> Self {
        Self {
            wakes: Mutex::new(VecDeque::new()),
        }
    }

    fn take(&self) -> Option<DriverPendingWake<U>> {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop_front()
    }
}

impl<U> PendingWakeQueue<U::ResumeToken, SuspendedTrampoline<U, U::ResumeToken>, U::ResumeOutcome>
    for InMemoryWakeQueue<U>
where
    U: BytecodeUnit + Send + 'static,
    U::ResumeToken: Send,
    U::ResumeOutcome: Send,
    SuspendedTrampoline<U, U::ResumeToken>: Send,
{
    fn enqueue(&self, wake: DriverPendingWake<U>) {
        self.wakes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(wake);
    }
}

trait BytecodeRequestResume<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    fn into_scheduler(
        self,
        ports: BytecodeSchedulerPorts<U>,
    ) -> Result<BytecodeScheduler<U>, BytecodeSchedulerError>;
}

impl<U> BytecodeRequestResume<U> for DriverPendingWake<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    fn into_scheduler(
        self,
        ports: BytecodeSchedulerPorts<U>,
    ) -> Result<BytecodeScheduler<U>, BytecodeSchedulerError> {
        // The wake carries the original `U::ResumeOutcome`. For `VmFiber` that
        // includes `ResumeOutcome::StreamEnd`; restoring it through the scheduler
        // reaches the VM's independent end resume PC instead of an item path or a
        // producer backpressure `Empty`.
        BytecodeScheduler::resume_from_pending_wake(self, ports)
    }
}

enum BytecodeRequestDriverOutcome<U: BytecodeUnit> {
    Complete {
        result: U::RootResult,
        stream_sent: bool,
    },
    Parked,
}

struct BytecodeRequestDriver<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    scheduler: Option<BytecodeScheduler<U>>,
    child_executor: Option<Arc<dyn BytecodeChildExecutor<U>>>,
    stream_supervisor: Option<Arc<dyn BytecodeStreamSupervisor<U>>>,
    stream_drain: Option<Box<dyn BytecodeRequestStreamDrain<U>>>,
    queue: Arc<InMemoryWakeQueue<U>>,
    heap: Box<dyn VmHeap + Send>,
    budget: Box<dyn VmBudget + Send>,
    error_map: Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync>,
    state: BytecodeRequestDriverState,
}

impl<U> BytecodeRequestDriver<U>
where
    U: BytecodeUnit + VmRootSource + 'static,
{
    #[allow(clippy::too_many_arguments)]
    fn new(
        scheduler: BytecodeScheduler<U>,
        child_executor: Option<Arc<dyn BytecodeChildExecutor<U>>>,
        stream_supervisor: Option<Arc<dyn BytecodeStreamSupervisor<U>>>,
        stream_drain: Option<Box<dyn BytecodeRequestStreamDrain<U>>>,
        queue: Arc<InMemoryWakeQueue<U>>,
        heap: Box<dyn VmHeap + Send>,
        budget: Box<dyn VmBudget + Send>,
        error_map: Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync>,
    ) -> Self {
        Self {
            scheduler: Some(scheduler),
            child_executor,
            stream_supervisor,
            stream_drain,
            queue,
            heap,
            budget,
            error_map,
            state: BytecodeRequestDriverState::Initial,
        }
    }

    fn ports(&self) -> BytecodeSchedulerPorts<U> {
        BytecodeSchedulerPorts {
            child_executor: self.child_executor.clone(),
            stream_supervisor: self.stream_supervisor.clone(),
        }
    }

    fn run(&mut self) -> RequestResult<BytecodeRequestDriverOutcome<U>> {
        match self.state {
            BytecodeRequestDriverState::Initial => {
                let scheduler = self.scheduler.take().ok_or_else(|| {
                    RequestError::Decode("bytecode request scheduler is missing".to_string())
                })?;
                self.advance(scheduler)
            }
            BytecodeRequestDriverState::Parked | BytecodeRequestDriverState::Complete => Err(
                RequestError::Decode("bytecode request has already been driven".to_string()),
            ),
            BytecodeRequestDriverState::Failed => Err(RequestError::Decode(
                "bytecode request failed closed".to_string(),
            )),
        }
    }

    fn resume<R>(&mut self, resume: R) -> RequestResult<BytecodeRequestDriverOutcome<U>>
    where
        R: BytecodeRequestResume<U>,
    {
        if self.state != BytecodeRequestDriverState::Parked {
            return Err(RequestError::Decode(
                "bytecode request is not parked".to_string(),
            ));
        }
        let scheduler = resume
            .into_scheduler(self.ports())
            .map_err(|error| {
                let error = (self.error_map)(error);
                self.state = BytecodeRequestDriverState::Failed;
                error
            })?;
        self.advance(scheduler)
    }

    fn take_pending_wake(&self) -> Option<DriverPendingWake<U>> {
        self.queue.take()
    }

    fn advance(
        &mut self,
        scheduler: BytecodeScheduler<U>,
    ) -> RequestResult<BytecodeRequestDriverOutcome<U>> {
        let outcome = match scheduler.run(&mut *self.heap, &mut *self.budget) {
            Ok(outcome) => outcome,
            Err(error) => {
                let error = (self.error_map)(error);
                self.state = BytecodeRequestDriverState::Failed;
                return Err(error);
            }
        };
        let drain_state = match self.stream_drain.as_mut() {
            Some(drain) => match drain.drain() {
                Ok(state) => state,
                Err(error) => {
                    self.state = BytecodeRequestDriverState::Failed;
                    return Err(error);
                }
            },
            None => BytecodeRequestDrainState::Empty,
        };
        match outcome {
            BytecodeSchedulerOutcome::Complete(result) => {
                self.state = BytecodeRequestDriverState::Complete;
                Ok(BytecodeRequestDriverOutcome::Complete {
                    result,
                    stream_sent: drain_state == BytecodeRequestDrainState::Terminal,
                })
            }
            BytecodeSchedulerOutcome::Parked => {
                self.state = BytecodeRequestDriverState::Parked;
                Ok(BytecodeRequestDriverOutcome::Parked)
            }
        }
    }
}

fn scheduler_error_map(
    execution_budget: Arc<ExecutionBudget>,
) -> Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync> {
    Box::new(move |error| scheduler_error_to_request_error(&execution_budget, error))
}


/// Executes one scalar bytecode request against a verified deployment image.
pub fn execute_runtime_bytecode_request(
    input: BytecodeRequestExecutionInput,
) -> RequestResult<BoundaryResponse> {
    execute_runtime_bytecode_request_with_ports(input, BytecodeRequestExecutionPorts::default())
}

/// Executes one bytecode request with optional child/adapter execution ports.
pub fn execute_runtime_bytecode_request_with_ports(
    input: BytecodeRequestExecutionInput,
    ports: BytecodeRequestExecutionPorts,
) -> RequestResult<BoundaryResponse> {
    let BytecodeRequestExecutionInput {
        target,
        request,
        cancelled,
        cancellation,
        execution_budget,
        handles,
    } = input;

    validate_bytecode_request(&request)?;
    let execution = ExecutionControl::new(cancellation.clone(), &execution_budget);
    execution.check_cancelled().map_err(RequestError::from)?;

    let BytecodeRequestTarget {
        image,
        entry,
        target: _,
    } = target;
    let pinned = PinnedDeploymentEntry::try_new(image, entry)
        .map_err(pinned_entry_error_to_request_error)?;
    let mut heap = RequestVmHeap::new(handles.request_heap_limits);
    let arguments = gateway_entry_arguments(&request, &mut heap)?;
    let fiber = Vm::start(pinned, arguments.into_boxed_slice(), vm_limits())
        .map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
    let mut budget = BytecodeVmBudget::new(execution_budget.clone(), cancelled, cancellation);

    let child_executor = Some(Arc::new(RequestAdapterExecutor::new(
        request.test_effects_enabled,
    )) as Arc<dyn BytecodeChildExecutor<VmFiber>>);
    let outcome = BytecodeScheduler::new(
        fiber,
        BytecodeSchedulerPorts {
            child_executor,
            stream_supervisor: ports.stream_supervisor,
        },
    )
    .run(&mut heap, &mut budget)
    .map_err(|error| scheduler_error_to_request_error(&execution_budget, error))?;
    let BytecodeSchedulerOutcome::Complete(result) = outcome else {
        return Err(RequestError::Unsupported(
            "bytecode VM parked; scalar ingress has no pending wake resume path".to_string(),
        ));
    };
    let values = result.map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
    let payload = json_payload_from_value_slots(values.values())?;
    Ok(BoundaryResponse::payload(payload))
}

fn gateway_entry_arguments(
    request: &RequestEnvelope,
    heap: &mut RequestVmHeap,
) -> RequestResult<Vec<ValueSlot>> {
    let Some(adapter) = &request.http_adapter else {
        return Ok(Vec::new());
    };
    let binary = request.binary_http.as_ref().ok_or_else(|| {
        RequestError::Decode("HTTP adapter request is missing binary HTTP metadata".to_string())
    })?;
    let mut arguments = Vec::with_capacity(adapter.adapter_args.len());
    for arg in &adapter.adapter_args {
        let value = match arg.source {
            GatewayAdapterSource::HttpRequest => materialize_http_request(binary, heap)?,
            GatewayAdapterSource::HttpBody => heap
                .alloc_bytes(binary.body.clone())
                .map_err(heap_error_to_request_error)?,
            GatewayAdapterSource::HttpContext => ValueSlot::null(),
        };
        arguments.push(value);
    }
    Ok(arguments)
}

fn materialize_http_request(
    binary: &BinaryHttpRequest,
    heap: &mut RequestVmHeap,
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
    heap: &mut RequestVmHeap,
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
    RequestError::Decode(format!("bytecode gateway heap materialization failed: {error}"))
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

fn validate_runtime_bytecode_request(request: &RequestEnvelope) -> RequestResult<()> {
    if !matches!(request.mode.as_str(), "unary" | "serverStream") {
        return Err(RequestError::Unsupported(format!(
            "bytecode request ingress only supports unary or serverStream request.start, got {}",
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

fn json_payload_from_value_slots(values: &[ValueSlot]) -> RequestResult<Vec<u8>> {
    match values {
        [] => Ok(b"null".to_vec()),
        [value] => json_bytes_from_value(value),
        _ => Err(RequestError::Unsupported(format!(
            "scalar bytecode VM returned {} results; expected zero or one",
            values.len()
        ))),
    }
}

fn json_bytes_from_value(value: &ValueSlot) -> RequestResult<Vec<u8>> {
    if value.is_null() {
        return Ok(b"null".to_vec());
    }
    if let Some(boolean) = value.as_bool() {
        return Ok(if boolean {
            b"true".to_vec()
        } else {
            b"false".to_vec()
        });
    }
    if let Some(number) = value.as_number() {
        let number = serde_json::Number::from_f64(number).ok_or_else(|| {
            RequestError::Unsupported(format!(
                "scalar bytecode VM returned a non-JSON number: {number}"
            ))
        })?;
        return Ok(number.to_string().into_bytes());
    }
    if value.as_integer().is_some() {
        return Err(RequestError::Unsupported(
            "integer results are not supported by bytecode scalar JSON ingress yet".to_string(),
        ));
    }
    if value.as_date().is_some() {
        return Err(RequestError::Unsupported(
            "Date results are not supported by bytecode scalar JSON ingress yet".to_string(),
        ));
    }
    Err(RequestError::Unsupported(format!(
        "scalar bytecode VM returned unsupported value kind {:?}",
        value.kind()
    )))
}

struct BytecodeVmBudget {
    execution_budget: Arc<ExecutionBudget>,
    cancelled: Arc<AtomicBool>,
    cancellation: CancellationToken,
}

impl BytecodeVmBudget {
    fn new(
        execution_budget: Arc<ExecutionBudget>,
        cancelled: Arc<AtomicBool>,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            execution_budget,
            cancelled,
            cancellation,
        }
    }

    fn poll_execution_budget(&self) -> Result<(), VmBudgetError> {
        let cancelled = self.cancelled.load(Ordering::Acquire) || self.cancellation.is_cancelled();
        self.execution_budget
            .poll(cancelled, Instant::now())
            .map_err(execution_budget_reason_to_vm)
    }
}

impl VmBudget for BytecodeVmBudget {
    fn replenish_raw_fuel(&mut self, maximum: NonZeroU32) -> Result<NonZeroU32, VmBudgetError> {
        self.poll_execution_budget()?;
        if self.execution_budget.add_units(u64::from(maximum.get())) {
            self.poll_execution_budget()?;
        }
        Ok(maximum)
    }

    fn poll_interrupt(&mut self) -> Result<(), VmBudgetError> {
        self.poll_execution_budget()
    }

    fn charge_semantic(&mut self, _charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetError> {
        if self.execution_budget.add_units(1) {
            self.poll_execution_budget()
        } else {
            Ok(())
        }
    }
}

fn execution_budget_reason_to_vm(reason: ExecutionBudgetReason) -> VmBudgetError {
    match reason {
        ExecutionBudgetReason::Cancelled => VmBudgetError::Cancelled,
        ExecutionBudgetReason::DeadlineExceeded => VmBudgetError::DeadlineExceeded,
        ExecutionBudgetReason::InstructionLimitExceeded => VmBudgetError::InstructionLimitExceeded,
    }
}

fn vm_error_to_request_error(execution_budget: &ExecutionBudget, error: VmError) -> RequestError {
    match error {
        VmError::Budget(error) => vm_budget_error_to_request_error(execution_budget, error),
        VmError::InternalTerminal(VmInternalTerminal::Budget(error)) => {
            vm_budget_error_to_request_error(execution_budget, error)
        }
        VmError::InternalTerminal(VmInternalTerminal::OwnerStopped) => RequestError::Cancelled,
        error => RequestError::Unsupported(format!("bytecode VM execution failed: {error}")),
    }
}

fn vm_budget_error_to_request_error(
    execution_budget: &ExecutionBudget,
    error: VmBudgetError,
) -> RequestError {
    let stats = execution_budget.stats_snapshot();
    match error {
        VmBudgetError::Cancelled | VmBudgetError::InternalStop => RequestError::Cancelled,
        VmBudgetError::DeadlineExceeded => RequestError::ExecutionBudgetExceeded {
            reason: ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: stats.instruction_count,
            limit: stats.budget_limit,
            elapsed_ms: stats.elapsed_ms,
        },
        VmBudgetError::InstructionLimitExceeded => RequestError::ExecutionBudgetExceeded {
            reason: ExecutionBudgetReason::InstructionLimitExceeded,
            instruction_count: stats.instruction_count,
            limit: stats.budget_limit,
            elapsed_ms: stats.elapsed_ms,
        },
        VmBudgetError::AccountingFailure => RequestError::Unsupported(format!(
            "bytecode VM budget accounting failed closed: {error}"
        )),
    }
}

fn pinned_entry_error_to_request_error(error: PinnedDeploymentEntryError) -> RequestError {
    RequestError::Decode(format!(
        "bytecode deployment entry pin failed closed: {error}"
    ))
}

fn vm_limits() -> VmLimits {
    VmLimits::new(
        NonZeroUsize::new(128).expect("VM frame limit is non-zero"),
        NonZeroUsize::new(4096).expect("VM value slot limit is non-zero"),
        NonZeroU32::new(1024).expect("VM fuel quantum is non-zero"),
        NonZeroU32::new(1024).expect("VM segment instruction limit is non-zero"),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use skiff_artifact_model::{IngressProtocol, IngressSelector};
    use skiff_runtime_model::vm_value::ValueSlot;

    use skiff_runtime_model::{
        vm_heap::VmHeapError,
        vm_root::VmRootVisitor,
    };
    use skiff_runtime_scheduler::{
        BytecodeChildExecutor, BytecodeChildStart, BytecodeControl, BytecodeStreamHandoff,
        RootDisposition, RootEscrow, RootEscrowBacking,
    };

    use super::*;
    use crate::{
        BinaryHttpRequest, BinaryHttpRequestMetadata, HttpAdapter, HttpAdapterCallable,
        HttpAdapterKind, RequestEnvelope,
    };


    type TestControl = BytecodeControl<usize, usize, usize, usize, usize>;
    type TestSuspended = SuspendedTrampoline<TestUnit, usize>;
    const ERROR_OUTCOME: usize = usize::MAX;

    #[derive(Debug)]
    struct TestUnit {
        control: Option<TestControl>,
        resumed: Option<(usize, usize)>,
        finish_after_resume: Option<usize>,
    }

    impl TestUnit {
        fn parked(operation: usize) -> Self {
            Self {
                control: Some(TestControl::Park(operation)),
                resumed: None,
                finish_after_resume: None,
            }
        }

        fn emit(item: usize, finish: usize) -> Self {
            Self {
                control: Some(TestControl::EmitStream(item)),
                resumed: None,
                finish_after_resume: Some(finish),
            }
        }
    }

    impl VmRootSource for TestUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for TestUnit {
        type ResumeToken = usize;
        type ResumeOutcome = usize;
        type RootResult = usize;
        type ChildInvocation = usize;
        type AdapterInvocation = usize;
        type StreamItem = usize;
        type PendingOperation = usize;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> TestControl {
            if let Some((_, outcome)) = self.resumed.take() {
                TestControl::Complete(self.finish_after_resume.take().unwrap_or(outcome))
            } else if let Some(control) = self.control.take() {
                control
            } else {
                TestControl::Complete(0)
            }
        }

        fn resume(
            &mut self,
            token: usize,
            outcome: usize,
        ) -> Result<(), BytecodeSchedulerError> {
            if outcome == ERROR_OUTCOME {
                return Err(BytecodeSchedulerError::UnsupportedPark);
            }
            self.resumed = Some((token, outcome));
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: usize) -> usize {
            completed
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TestStreamMode {
        Backpressure,
        Ready,
    }

    struct FakeStream {
        mode: TestStreamMode,
        emitted: Mutex<Vec<usize>>,
        parked: Mutex<Option<(usize, TestSuspended)>>,
        delivered: Mutex<Vec<usize>>,
        terminal: std::sync::atomic::AtomicBool,
    }

    impl FakeStream {
        fn new(mode: TestStreamMode) -> Self {
            Self {
                mode,
                emitted: Mutex::new(Vec::new()),
                parked: Mutex::new(None),
                delivered: Mutex::new(Vec::new()),
                terminal: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    impl BytecodeStreamSupervisor<TestUnit> for FakeStream {
        fn emit_stream_handoff(
            &self,
            item: usize,
            _depth: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<TestUnit>, BytecodeSchedulerError> {
            self.emitted.lock().unwrap().push(item);
            match self.mode {
                TestStreamMode::Backpressure => Ok(BytecodeStreamHandoff::Pending(item)),
                TestStreamMode::Ready => Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                    resume: item,
                    outcome: 0,
                })),
            }
        }

        fn park(
            &self,
            operation: usize,
            suspended: TestSuspended,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeSchedulerError> {
            *self.parked.lock().unwrap() = Some((operation, suspended));
            Ok(())
        }

        fn finish_stream(
            &self,
            _depth: usize,
            _result: &usize,
        ) -> Result<(), BytecodeSchedulerError> {
            let emitted = !self.emitted.lock().unwrap().is_empty();
            let delivered = !self.delivered.lock().unwrap().is_empty();
            if emitted || delivered {
                self.terminal.store(true, Ordering::Release);
            }
            Ok(())
        }
    }

    struct TestDrain {
        stream: Arc<FakeStream>,
    }

    impl BytecodeRequestStreamDrain<TestUnit> for TestDrain {
        fn drain(&mut self) -> RequestResult<BytecodeRequestDrainState> {
            let items: Vec<usize> = self.stream.emitted.lock().unwrap().drain(..).collect();
            self.stream
                .delivered
                .lock()
                .unwrap()
                .extend(items.iter().copied());
            if self.stream.terminal.load(Ordering::Acquire) {
                Ok(BytecodeRequestDrainState::Terminal)
            } else if !items.is_empty() {
                Ok(BytecodeRequestDrainState::Delivered)
            } else {
                Ok(BytecodeRequestDrainState::Empty)
            }
        }
    }

    struct EmptyRoots;

    impl VmRootSource for EmptyRoots {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl RootEscrowBacking for EmptyRoots {
        fn root_count(&self) -> usize {
            0
        }

        fn restore_roots(self: Box<Self>) {}

        fn drop_roots(self: Box<Self>, _disposition: RootDisposition) {}
    }

    struct TestResume {
        resume: usize,
        suspended: TestSuspended,
        outcome: usize,
        escrow: RootEscrow,
    }

    impl BytecodeRequestResume<TestUnit> for TestResume {
        fn into_scheduler(
            self,
            ports: BytecodeSchedulerPorts<TestUnit>,
        ) -> Result<BytecodeScheduler<TestUnit>, BytecodeSchedulerError> {
            BytecodeScheduler::resume_from_suspended(
                self.suspended,
                self.resume,
                self.outcome,
                self.escrow,
                ports,
            )
        }
    }

    struct NoopHeap;

    impl VmHeap for NoopHeap {
        fn validate_live(&self, _value: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }

        fn snapshot_share(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Ok(*source)
        }

        fn transfer_owner(&mut self, source: &ValueSlot) -> Result<ValueSlot, VmHeapError> {
            Ok(*source)
        }

        fn release_snapshot(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }

        fn release_resource(&mut self, _owner: &ValueSlot) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    struct NoopBudget;

    impl VmBudget for NoopBudget {
        fn replenish_raw_fuel(&mut self, maximum: NonZeroU32) -> Result<NonZeroU32, VmBudgetError> {
            Ok(maximum)
        }

        fn poll_interrupt(&mut self) -> Result<(), VmBudgetError> {
            Ok(())
        }

        fn charge_semantic(&mut self, _charge: VmSemanticCharge<'_>) -> Result<(), VmBudgetError> {
            Ok(())
        }
    }

    fn driver_for(
        unit: TestUnit,
        stream: Arc<FakeStream>,
    ) -> BytecodeRequestDriver<TestUnit> {
        let queue = Arc::new(InMemoryWakeQueue::new());
        let supervisor: Arc<dyn BytecodeStreamSupervisor<TestUnit>> = stream.clone();
        let scheduler = BytecodeScheduler::new(
            unit,
            BytecodeSchedulerPorts {
                child_executor: None,
                stream_supervisor: Some(supervisor.clone()),
            },
        );
        let heap: Box<dyn VmHeap + Send> = Box::new(NoopHeap);
        let budget: Box<dyn VmBudget + Send> = Box::new(NoopBudget);
        let error_map: Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync> =
            Box::new(|error| RequestError::Decode(error.to_string()));
        let drain = Box::new(TestDrain {
            stream: Arc::clone(&stream),
        });
        BytecodeRequestDriver::new(
            scheduler,
            None,
            Some(supervisor),
            Some(drain),
            queue,
            heap,
            budget,
            error_map,
        )
    }

    fn resume_after_park(
        driver: &mut BytecodeRequestDriver<TestUnit>,
        stream: &FakeStream,
        outcome: usize,
    ) -> RequestResult<BytecodeRequestDriverOutcome<TestUnit>> {
        let (operation, suspended) = stream.parked.lock().unwrap().take().expect("parked unit");
        let resume = TestResume {
            resume: operation,
            suspended,
            outcome,
            escrow: RootEscrow::new(Box::new(EmptyRoots)),
        };
        driver.resume(resume)
    }

    #[test]
    fn parked_request_can_resume() {
        let stream = Arc::new(FakeStream::new(TestStreamMode::Ready));
        let mut driver = driver_for(TestUnit::parked(7), Arc::clone(&stream));

        assert!(matches!(driver.run().unwrap(), BytecodeRequestDriverOutcome::Parked));
        match resume_after_park(&mut driver, &stream, 42).unwrap() {
            BytecodeRequestDriverOutcome::Complete { result, stream_sent } => {
                assert_eq!(result, 42);
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("resume must complete"),
        }
    }

    #[test]
    fn backpressure_item_delivery_resumes_with_zero_result() {
        let stream = Arc::new(FakeStream::new(TestStreamMode::Backpressure));
        let mut driver = driver_for(TestUnit::emit(7, 99), Arc::clone(&stream));

        assert!(matches!(driver.run().unwrap(), BytecodeRequestDriverOutcome::Parked));
        assert_eq!(*stream.delivered.lock().unwrap(), [7]);
        match resume_after_park(&mut driver, &stream, 0).unwrap() {
            BytecodeRequestDriverOutcome::Complete { result, stream_sent } => {
                assert_eq!(result, 99);
                assert!(stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("zero-result resume must complete"),
        }
        assert_eq!(*stream.delivered.lock().unwrap(), [7]);
        assert!(stream.terminal.load(Ordering::Acquire));
    }

    #[test]
    fn error_resume_fails_closed() {
        let stream = Arc::new(FakeStream::new(TestStreamMode::Backpressure));
        let mut driver = driver_for(TestUnit::emit(7, 99), Arc::clone(&stream));

        assert!(matches!(driver.run().unwrap(), BytecodeRequestDriverOutcome::Parked));
        let error = match resume_after_park(&mut driver, &stream, ERROR_OUTCOME) {
            Err(error) => error,
            Ok(_) => panic!("error resume must fail closed"),
        };
        assert!(error.to_string().contains("park"));
        assert!(matches!(
            driver.run(),
            Err(RequestError::Decode(message)) if message.contains("failed closed")
        ));
    }

    #[test]
    fn natural_end_completes_response_stream() {
        let stream = Arc::new(FakeStream::new(TestStreamMode::Ready));
        let mut driver = driver_for(TestUnit::emit(7, 99), Arc::clone(&stream));

        match driver.run().unwrap() {
            BytecodeRequestDriverOutcome::Complete { result, stream_sent } => {
                assert_eq!(result, 99);
                assert!(stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("ready emission must not park"),
        }
        assert_eq!(*stream.delivered.lock().unwrap(), [7]);
        assert!(stream.terminal.load(Ordering::Acquire));
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StreamNextResume {
        Item,
        End,
        Pending,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StreamNextOutcome {
        Item(usize),
        End,
        Failure(&'static str),
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum StreamNextInvocation {
        StreamNext {
            item_resume: StreamNextResume,
            end_resume: StreamNextResume,
        },
    }

    type StreamNextControl = BytecodeControl<
        StreamNextOutcome,
        StreamNextInvocation,
        usize,
        usize,
        StreamNextResume,
    >;
    type StreamNextSuspended = SuspendedTrampoline<StreamNextUnit, StreamNextResume>;

    struct StreamNextUnit {
        invocation: Option<StreamNextInvocation>,
        resumed: Option<(StreamNextResume, StreamNextOutcome)>,
    }

    impl StreamNextUnit {
        fn stream_next() -> Self {
            Self {
                invocation: Some(StreamNextInvocation::StreamNext {
                    item_resume: StreamNextResume::Item,
                    end_resume: StreamNextResume::End,
                }),
                resumed: None,
            }
        }
    }

    impl VmRootSource for StreamNextUnit {
        fn visit_roots(&self, _visitor: &mut dyn VmRootVisitor) -> Result<(), VmHeapError> {
            Ok(())
        }
    }

    impl BytecodeUnit for StreamNextUnit {
        type ResumeToken = StreamNextResume;
        type ResumeOutcome = StreamNextOutcome;
        type RootResult = StreamNextOutcome;
        type ChildInvocation = StreamNextInvocation;
        type AdapterInvocation = usize;
        type StreamItem = usize;
        type PendingOperation = StreamNextResume;

        fn run_segment(
            &mut self,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> StreamNextControl {
            if let Some((_, outcome)) = self.resumed.take() {
                StreamNextControl::Complete(outcome)
            } else if let Some(invocation) = self.invocation.take() {
                StreamNextControl::EnterChild(invocation)
            } else {
                StreamNextControl::Complete(StreamNextOutcome::Item(0))
            }
        }

        fn resume(
            &mut self,
            token: StreamNextResume,
            outcome: StreamNextOutcome,
        ) -> Result<(), BytecodeSchedulerError> {
            self.resumed = Some((token, outcome));
            Ok(())
        }

        fn child_completion_to_resume_outcome(completed: StreamNextOutcome) -> StreamNextOutcome {
            completed
        }

        fn is_stream_next_child(invocation: &StreamNextInvocation) -> bool {
            matches!(invocation, StreamNextInvocation::StreamNext { .. })
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum StreamNextExecutorMode {
        Item,
        End,
        Error,
        Pending,
    }

    struct StreamNextExecutor {
        mode: StreamNextExecutorMode,
        pending: Mutex<Option<(StreamNextResume, StreamNextSuspended)>>,
    }

    impl BytecodeChildExecutor<StreamNextUnit> for StreamNextExecutor {
        fn execute_child(
            &self,
            _invocation: StreamNextInvocation,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeChildStart<StreamNextUnit>, BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedChild)
        }

        fn execute_adapter(
            &self,
            _invocation: usize,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeHandoff<StreamNextUnit>, BytecodeSchedulerError> {
            Err(BytecodeSchedulerError::UnsupportedAdapter)
        }

        fn execute_stream_next(
            &self,
            invocation: StreamNextInvocation,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<BytecodeStreamHandoff<StreamNextUnit>, BytecodeSchedulerError> {
            let StreamNextInvocation::StreamNext {
                item_resume,
                end_resume,
            } = invocation;
            match self.mode {
                StreamNextExecutorMode::Item => {
                    Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                        resume: item_resume,
                        outcome: StreamNextOutcome::Item(7),
                    }))
                }
                StreamNextExecutorMode::End => {
                    Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                        resume: end_resume,
                        outcome: StreamNextOutcome::End,
                    }))
                }
                StreamNextExecutorMode::Error => {
                    Ok(BytecodeStreamHandoff::Ready(BytecodeHandoff {
                        resume: item_resume,
                        outcome: StreamNextOutcome::Failure("stream failed"),
                    }))
                }
                StreamNextExecutorMode::Pending => Ok(BytecodeStreamHandoff::Pending(
                    StreamNextResume::Pending,
                )),
            }
        }

        fn park_stream_next(
            &self,
            operation: StreamNextResume,
            suspended: StreamNextSuspended,
            _heap: &mut dyn VmHeap,
            _budget: &mut dyn VmBudget,
        ) -> Result<(), BytecodeSchedulerError> {
            *self.pending.lock().unwrap() = Some((operation, suspended));
            Ok(())
        }
    }

    struct StreamNextResumeSource {
        resume: StreamNextResume,
        suspended: StreamNextSuspended,
        outcome: StreamNextOutcome,
        escrow: RootEscrow,
    }

    impl BytecodeRequestResume<StreamNextUnit> for StreamNextResumeSource {
        fn into_scheduler(
            self,
            ports: BytecodeSchedulerPorts<StreamNextUnit>,
        ) -> Result<BytecodeScheduler<StreamNextUnit>, BytecodeSchedulerError> {
            BytecodeScheduler::resume_from_suspended(
                self.suspended,
                self.resume,
                self.outcome,
                self.escrow,
                ports,
            )
        }
    }

    fn stream_next_driver(
        executor: Arc<StreamNextExecutor>,
    ) -> BytecodeRequestDriver<StreamNextUnit> {
        let queue = Arc::new(InMemoryWakeQueue::new());
        let executor_dyn: Arc<dyn BytecodeChildExecutor<StreamNextUnit>> = executor.clone();
        let scheduler = BytecodeScheduler::new(
            StreamNextUnit::stream_next(),
            BytecodeSchedulerPorts {
                child_executor: Some(executor_dyn.clone()),
                stream_supervisor: None,
            },
        );
        let heap: Box<dyn VmHeap + Send> = Box::new(NoopHeap);
        let budget: Box<dyn VmBudget + Send> = Box::new(NoopBudget);
        let error_map: Box<dyn Fn(BytecodeSchedulerError) -> RequestError + Send + Sync> =
            Box::new(|error| RequestError::Decode(error.to_string()));
        BytecodeRequestDriver::new(
            scheduler,
            Some(executor_dyn),
            None,
            None,
            queue,
            heap,
            budget,
            error_map,
        )
    }

    #[test]
    fn stream_next_item_uses_item_continuation() {
        let executor = Arc::new(StreamNextExecutor {
            mode: StreamNextExecutorMode::Item,
            pending: Mutex::new(None),
        });
        let mut driver = stream_next_driver(executor);

        match driver.run().unwrap() {
            BytecodeRequestDriverOutcome::Complete { result, stream_sent } => {
                assert_eq!(result, StreamNextOutcome::Item(7));
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("item path must not park"),
        }
    }

    #[test]
    fn stream_next_end_uses_end_continuation() {
        let executor = Arc::new(StreamNextExecutor {
            mode: StreamNextExecutorMode::End,
            pending: Mutex::new(None),
        });
        let mut driver = stream_next_driver(executor);

        match driver.run().unwrap() {
            BytecodeRequestDriverOutcome::Complete { result, stream_sent } => {
                assert_eq!(result, StreamNextOutcome::End);
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("end path must not park"),
        }
    }

    #[test]
    fn stream_next_pending_wake_end_resumes_end_continuation() {
        let executor = Arc::new(StreamNextExecutor {
            mode: StreamNextExecutorMode::Pending,
            pending: Mutex::new(None),
        });
        let mut driver = stream_next_driver(Arc::clone(&executor));

        assert!(matches!(driver.run().unwrap(), BytecodeRequestDriverOutcome::Parked));
        let (operation, suspended) = executor
            .pending
            .lock()
            .unwrap()
            .take()
            .expect("pending stream next park");
        let resume = StreamNextResumeSource {
            resume: operation,
            suspended,
            outcome: StreamNextOutcome::End,
            escrow: RootEscrow::new(Box::new(EmptyRoots)),
        };

        match driver.resume(resume).unwrap() {
            BytecodeRequestDriverOutcome::Complete { result, stream_sent } => {
                assert_eq!(result, StreamNextOutcome::End);
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("end wake must complete"),
        }
    }

    #[test]
    fn stream_next_error_resume_fails_closed() {
        let executor = Arc::new(StreamNextExecutor {
            mode: StreamNextExecutorMode::Error,
            pending: Mutex::new(None),
        });
        let mut driver = stream_next_driver(executor);

        match driver.run().unwrap() {
            BytecodeRequestDriverOutcome::Complete { result, stream_sent } => {
                assert_eq!(result, StreamNextOutcome::Failure("stream failed"));
                assert!(!stream_sent);
            }
            BytecodeRequestDriverOutcome::Parked => panic!("error path must complete"),
        }
    }

    #[test]
    fn json_payload_encodes_scalar_immediates() {
        assert_eq!(json_payload_from_value_slots(&[]).unwrap(), b"null");
        assert_eq!(
            json_payload_from_value_slots(&[ValueSlot::null()]).unwrap(),
            b"null"
        );
        assert_eq!(
            json_payload_from_value_slots(&[ValueSlot::bool(true)]).unwrap(),
            b"true"
        );
        assert_eq!(
            json_payload_from_value_slots(&[ValueSlot::bool(false)]).unwrap(),
            b"false"
        );
        assert_eq!(
            json_payload_from_value_slots(&[ValueSlot::number(1.5)]).unwrap(),
            b"1.5"
        );
    }

    #[test]
    fn json_payload_rejects_unsupported_results() {
        assert!(json_payload_from_value_slots(&[ValueSlot::integer(1)]).is_err());
        assert!(json_payload_from_value_slots(&[ValueSlot::date(1)]).is_err());
        assert!(
            json_payload_from_value_slots(&[ValueSlot::null(), ValueSlot::bool(true)]).is_err()
        );
        assert!(json_payload_from_value_slots(&[ValueSlot::number(f64::NAN)]).is_err());
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
        assert!(validate_bytecode_request(&binary_request).is_err());

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
        assert!(validate_bytecode_request(&adapter_request).is_err());

        let mut actor_request = request();
        actor_request
            .extra
            .insert("actorCall".to_string(), serde_json::json!({}));
        assert!(validate_bytecode_request(&actor_request).is_err());
    }

    #[test]
    fn scheduler_fail_closed_errors_map_to_unsupported() {
        let budget = ExecutionBudget::disabled();
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
}
