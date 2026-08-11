use std::{
    num::{NonZeroU32, NonZeroUsize},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};

use skiff_artifact_model::ContractOperationId;
use skiff_runtime_bytecode_verifier::{
    VerifiedCodeEntry, VerifiedCodeEntryKind, VerifiedLinkedBytecodeImage,
};
use skiff_runtime_capability_context::{CancellationToken, ExecutionBudgetReason};
use skiff_runtime_deployment_image::{
    DeploymentImage, PinnedDeploymentEntry, PinnedDeploymentEntryError,
};
use skiff_runtime_model::{request_heap::RequestHeapLimits, vm_heap::VmHeap, vm_value::ValueSlot};
use skiff_runtime_vm::{
    AdapterInvocation, ChildInvocation, PendingOperation, ResumeOutcome, StreamItem, Vm, VmBudget,
    VmBudgetError, VmControl, VmError, VmLimits, VmResumeToken, VmSemanticCharge,
};

use crate::{
    vm_heap::RequestVmHeap, BoundaryResponse, ExecutionBudget, ExecutionControl, RequestEnvelope,
    RequestError, RequestResult,
};

/// One verified deployment image and the exact operation entry selected from it.
///
/// Construction rejects an entry that does not share the image's exact program
/// allocation or whose resolved kind is not the supplied operation.
#[derive(Debug)]
pub struct BytecodeRequestTarget {
    image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
    entry: VerifiedCodeEntry,
    operation_id: ContractOperationId,
}

impl BytecodeRequestTarget {
    pub fn try_new(
        image: Arc<DeploymentImage<VerifiedLinkedBytecodeImage>>,
        entry: VerifiedCodeEntry,
        operation_id: ContractOperationId,
    ) -> Result<Self, BytecodeRequestTargetError> {
        if !Arc::ptr_eq(image.program(), entry.image()) {
            return Err(BytecodeRequestTargetError::ProgramMismatch);
        }
        match entry.kind() {
            VerifiedCodeEntryKind::Operation {
                contract_operation_id,
            } if contract_operation_id == &operation_id => {}
            entry_kind => {
                return Err(BytecodeRequestTargetError::OperationMismatch {
                    operation: operation_id.clone(),
                    entry_kind: entry_kind.clone(),
                })
            }
        }
        Ok(Self {
            image,
            entry,
            operation_id,
        })
    }

    pub fn image(&self) -> &Arc<DeploymentImage<VerifiedLinkedBytecodeImage>> {
        &self.image
    }

    pub fn entry(&self) -> &VerifiedCodeEntry {
        &self.entry
    }

    pub fn operation_id(&self) -> &ContractOperationId {
        &self.operation_id
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BytecodeRequestTargetError {
    #[error("bytecode request target image and verified code entry do not pin the same exact deployment program")]
    ProgramMismatch,
    #[error(
        "bytecode request target requested operation {operation}, but resolved {entry_kind:?}"
    )]
    OperationMismatch {
        operation: ContractOperationId,
        entry_kind: VerifiedCodeEntryKind,
    },
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

/// Execution ports that a host or scheduler can supply for VM handoffs.
///
/// The existing host path constructs [`BytecodeRequestExecutionHandles`] with
/// only heap limits and calls [`execute_runtime_bytecode_request`]. Ports are
/// an additive seam so that same call can be upgraded without changing the
/// host's current struct literal.
#[derive(Default)]
pub struct BytecodeRequestExecutionPorts {
    pub child_executor: Option<Arc<dyn BytecodeChildExecutor>>,
    pub stream_supervisor: Option<Arc<dyn BytecodeStreamSupervisor>>,
}

/// One completed VM handoff plus the unique continuation that resumes its
/// parent fiber.
pub struct BytecodeInvocationHandoff {
    pub resume: VmResumeToken,
    pub outcome: ResumeOutcome,
}

/// Flat execution seam for child and host-adapter invocations.
///
/// The request crate never starts a child VM itself. Implementations receive
/// the same heap and budget as the parent so child result slots remain valid
/// for the parent's continuation.
pub trait BytecodeChildExecutor: Send + Sync + 'static {
    fn execute_child(
        &self,
        invocation: ChildInvocation,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> RequestResult<BytecodeInvocationHandoff>;

    fn execute_adapter(
        &self,
        invocation: AdapterInvocation,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> RequestResult<BytecodeInvocationHandoff>;
}

/// Future seam for stream emission and actual-Pending parking.
///
/// This path is currently fail-closed because the bytecode HTTP entry has no
/// stream response sink or pending registry yet.
pub trait BytecodeStreamSupervisor: Send + Sync + 'static {
    fn emit_stream(
        &self,
        item: StreamItem,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> RequestResult<BytecodeInvocationHandoff>;

    fn park(
        &self,
        operation: PendingOperation,
        heap: &mut dyn VmHeap,
        budget: &mut dyn VmBudget,
    ) -> RequestResult<BytecodeInvocationHandoff>;
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
        operation_id: _,
    } = target;
    let pinned = PinnedDeploymentEntry::try_new(image, entry)
        .map_err(pinned_entry_error_to_request_error)?;
    let mut fiber = Vm::start(pinned, Box::new([]), vm_limits())
        .map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
    let mut heap = RequestVmHeap::new(handles.request_heap_limits);
    let mut budget = BytecodeVmBudget::new(execution_budget.clone(), cancelled, cancellation);

    loop {
        match fiber.run_segment(&mut heap, &mut budget) {
            VmControl::Continue => {}
            VmControl::Complete(result) => {
                let values =
                    result.map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
                let payload = json_payload_from_value_slots(values.values())?;
                fiber
                    .discard_terminal_roots(&mut heap)
                    .map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
                return Ok(BoundaryResponse::payload(payload));
            }
            VmControl::EnterChild(invocation) => {
                let handoff = match &ports.child_executor {
                    Some(executor) => executor.execute_child(invocation, &mut heap, &mut budget)?,
                    None => {
                        return Err(RequestError::Unsupported(
                            "bytecode VM child invocation requires a child executor port"
                                .to_string(),
                        ))
                    }
                };
                fiber
                    .resume(handoff.resume, handoff.outcome)
                    .map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
            }
            VmControl::EnterAdapter(invocation) => {
                let handoff = match &ports.child_executor {
                    Some(executor) => {
                        executor.execute_adapter(invocation, &mut heap, &mut budget)?
                    }
                    None => {
                        return Err(RequestError::Unsupported(
                            "bytecode VM adapter invocation requires a child executor port"
                                .to_string(),
                        ))
                    }
                };
                fiber
                    .resume(handoff.resume, handoff.outcome)
                    .map_err(|error| vm_error_to_request_error(&execution_budget, error))?;
            }
            VmControl::EmitStream(_) => {
                return Err(RequestError::Unsupported(
                    "bytecode VM stream emission requires stream supervisor integration"
                        .to_string(),
                ))
            }
            VmControl::Park(_) => {
                return Err(RequestError::Unsupported(
                    "bytecode VM parking requires stream supervisor integration".to_string(),
                ))
            }
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
    if request.ingress_selector.is_none() {
        return Err(RequestError::Unsupported(
            "bytecode scalar ingress requires request.start ingress_selector".to_string(),
        ));
    }
    if request.binary_http.is_some() {
        return Err(RequestError::Unsupported(
            "binary HTTP metadata is not supported by bytecode scalar ingress".to_string(),
        ));
    }
    if request.http_adapter.is_some() {
        return Err(RequestError::Unsupported(
            "HTTP callable adapter metadata is not supported by bytecode scalar ingress"
                .to_string(),
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
        error => RequestError::Unsupported(format!("scalar bytecode VM execution failed: {error}")),
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

    use super::*;
    use crate::{
        BinaryHttpRequest, BinaryHttpRequestMetadata, HttpAdapter, HttpAdapterCallable,
        HttpAdapterKind, RequestEnvelope,
    };

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
