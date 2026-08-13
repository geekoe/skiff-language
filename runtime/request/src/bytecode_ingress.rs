use std::{
    num::{NonZeroU32, NonZeroUsize},
    sync::Arc,
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
    vm_value::{CompactTypeTag, ValueFlags, ValueKind, ValueSlot},
};
use skiff_runtime_scheduler::{
    BytecodeSchedulerError, BytecodeSchedulerOutcome, BytecodeSchedulerPorts,
    RequestExecutionContext,
};
use skiff_runtime_vm::{
    Vm, VmBudget, VmBudgetClosed, VmBudgetTerminal, VmError, VmFiber, VmInternalTerminal, VmLimits,
};

use crate::{
    vm_heap::RequestVmHeap, BinaryHttpRequest, BoundaryResponse, ExecutionBudget, ExecutionControl,
    GatewayAdapterSource, HttpAdapterKind, HttpNameValue, HttpResponseMetadata, RequestEnvelope,
    RequestError, RequestResult,
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

/// The only public composition of one bytecode request: create the owner-bound
/// execution context, start the fiber exactly once, then drive the scheduler
/// exactly once.
///
/// A failure during the start phase returns an empty retention carrier and a
/// frozen `NotStarted` owner inventory. A completed drive freezes the actual
/// `Started` snapshot on every outcome, success, parked or failed.
pub fn drive_runtime_bytecode_request(
    input: BytecodeRequestExecutionInput,
) -> DrivenBytecodeRequest {
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

    let mut context = RequestExecutionContext::<VmFiber>::create(BytecodeSchedulerPorts::default());

    let start =
        (|| -> RequestResult<(VmFiber, Box<dyn VmHeap + Send>, Box<dyn VmBudget + Send>)> {
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
            let budget: Box<dyn VmBudget + Send> = Box::new(budget);
            Ok((fiber, heap, budget))
        })();

    let (fiber, mut heap, mut budget) = match start {
        Ok(parts) => parts,
        Err(error) => {
            return DrivenBytecodeRequest {
                result: Err(error),
                retention: BytecodeRequestRetention {
                    heap: None,
                    budget: None,
                },
                owner_inventory: DrivenBytecodeRequestOwnerInventory::NotStarted(
                    context.into_not_started(),
                ),
            };
        }
    };

    context.install_root(fiber);
    let (outcome, snapshot) = context.drive(&mut *heap, &mut *budget);
    let result = match outcome {
        Ok(BytecodeSchedulerOutcome::Complete(result)) => match result {
            Ok(values) => {
                if mode == "serverStream" {
                    Err(RequestError::Decode(
                        "serverStream request completed without a response stream".to_string(),
                    ))
                } else if raw_http_adapter {
                    http_response_from_vm_values(&mut *heap, values.values())
                } else {
                    json_payload_from_value_slots(&mut *heap, values.values())
                        .map(BoundaryResponse::payload)
                }
            }
            Err(VmError::Thrown(envelope)) => {
                Err(uncaught_throw_to_request_error(&mut *heap, &envelope))
            }
            Err(error) => Err(vm_error_to_request_error(&execution_budget, error)),
        },
        Ok(BytecodeSchedulerOutcome::Parked) => Err(RequestError::Unsupported(
            "bytecode VM parked on the synchronous Phase 1 request lane".to_string(),
        )),
        Err(error) => Err(scheduler_error_to_request_error(&execution_budget, error)),
    };
    DrivenBytecodeRequest {
        result,
        retention: BytecodeRequestRetention {
            heap: Some(heap),
            budget: Some(budget),
        },
        owner_inventory: DrivenBytecodeRequestOwnerInventory::Started(snapshot),
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
}
