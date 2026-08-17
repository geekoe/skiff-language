//! Typed bytecode host-effect adapters over the production capability lowers.

use std::{
    collections::BTreeMap,
    future::{self, Future},
    pin::Pin,
    sync::Arc,
};

use serde_json::{Map, Value};
use skiff_artifact_model::{
    boundary::{classify_boundary_callback_position, BoundaryCallbackPosition},
    BoundaryValuePlan, ContractLiteral, ContractTypeRef, InterfaceInstantiationRef,
    PackageSchemaTypeRef, TypeRefIr,
};
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_boundary::recoverable::FailClosedRecoverableBehaviorHooks;
use skiff_runtime_boundary::value::{bytes_payload, bytes_value};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, CancellationToken, DbCapabilitySource, DbRecoverableRuntimeContext,
    DbRecoverableRuntimeExpectedPlans, HttpRuntimeOptions, OutboundControlMessage,
    OutboundRequestCancelSendError, OutboundRequestCancelSender, OutboundRequestRegistry,
    OutboundResponse, RequestCancelControl, RouterWriterMessage, TaskSubmitControlMessage,
    TaskSubmitResponseControl,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, LinkedCallableSignature, LinkedInterfaceTableKind, LinkedLocalInterfaceTable,
};
use skiff_runtime_linker::{DeploymentExecutionEntry, DeploymentExecutionImage};
use skiff_runtime_model::{
    addr::ExecutableAddr,
    callback_projection::CallbackLifetime,
    error::WirePayload,
    recoverable::{
        RuntimeRecoverableBoundaryContext, RuntimeRecoverableBoundaryKind,
        RuntimeRecoverableServiceRef, RuntimeRecoverableStorageLane,
        RuntimeRecoverableTrustBoundary,
    },
    request_heap::{deep_clone_runtime_value_between_heaps, RequestHeap},
    runtime_value::{
        InterfaceCarrier, InterfaceMethodTable, InterfaceMethodTarget, InterfaceMethodType,
        InterfaceReceiverCallAbi, InterfaceValue, RuntimeValue,
    },
    type_plan::leaf_bytes_plan,
    vm_value::{CompactTypeTag, ValueFlags, ValueSlot},
};
use skiff_runtime_request::{
    BytecodeActorChildComposition, BytecodeCallbackChildComposition, BytecodeCallbackChildError,
    BytecodeCallbackProjector, BytecodeCallbackResolver, BytecodeDbChildComposition,
    BytecodeHttpClientPort, BytecodeHttpFailure, BytecodeHttpFuture, BytecodeHttpRequest,
    BytecodeHttpResponse, BytecodeHttpStreamRegistrar, BytecodeHttpStreamResponse,
    BytecodeRequestChildComposition, BytecodeServiceChildError, BytecodeServiceResolver,
    BytecodeTaskChildComposition, BytecodeTaskSubmitError, BytecodeTaskSubmitter,
    CallbackExecution, CrossImageServiceChildThrowMaterializer, HttpNameValue,
    OwnedExecutionControl, RequestMemoryLedger, RequestVmHeap,
};
use skiff_runtime_transport::protocol::TaskSubmitResponseFrameHeader;
use tokio::sync::mpsc;

use crate::{
    capability_context::{
        BytecodeCallbackCapabilityHooks, BytecodeCallbackCapabilityTable, BytecodeCallbackError,
        EffectDispatchContext, HttpClientCapabilityContext, HttpEffectContext,
        TelemetryCapabilityContext,
    },
    error::{OrdinaryRuntimeError, RuntimeError},
};

use super::{http_client_runtime::CurrentScopeHttpFailure, http_runtime, RuntimeHost};

/// Production provider for the two exact Phase 5 HTTP executor identities.
///
/// The base context intentionally has neither a stream runtime nor test-effect
/// doubles. `stream` installs only the registrar's ResourceTable-backed
/// runtime, and both methods receive the one K5 execution scope as an input.
#[derive(Clone)]
pub(crate) struct ProductionBytecodeHttpClientPort {
    context: HttpClientCapabilityContext,
}

impl ProductionBytecodeHttpClientPort {
    fn new(
        cancellation: CancellationToken,
        response_max_bytes: usize,
        http_options: HttpRuntimeOptions,
    ) -> Self {
        let effects = EffectDispatchContext::new(
            HttpEffectContext::new(None, response_max_bytes, cancellation),
            TelemetryCapabilityContext::new(None),
            http_options.clone(),
        );
        Self {
            context: HttpClientCapabilityContext::production(effects, http_options),
        }
    }

    fn ready_invalid_input<T>(error: RuntimeError) -> BytecodeHttpFuture<T>
    where
        T: Send + 'static,
    {
        Box::pin(future::ready(Err(BytecodeHttpFailure::InvalidInput(
            ordinary_http_failure(error),
        ))))
    }
}

impl BytecodeHttpClientPort for ProductionBytecodeHttpClientPort {
    fn request(
        &self,
        request: BytecodeHttpRequest,
        execution: OwnedExecutionControl,
    ) -> BytecodeHttpFuture<BytecodeHttpResponse> {
        let input = request_value(request);
        if let Err(error) = http_runtime::validate_bytecode_request_input(&input) {
            return Self::ready_invalid_input(error);
        }

        let context = self.context.clone();
        // Clone the already-created scope; do not wrap or derive another
        // ExecutionControl/deadline/cancellation authority.
        let current_scope = execution.execution_scope().clone();
        Box::pin(async move {
            let output = context
                .dispatch_http_request_with_execution_scope(&input, current_scope)
                .await
                .map_err(map_current_scope_failure)?;
            strict_request_response(output)
        })
    }

    fn stream(
        &self,
        request: BytecodeHttpRequest,
        execution: OwnedExecutionControl,
        registrar: BytecodeHttpStreamRegistrar,
    ) -> BytecodeHttpFuture<BytecodeHttpStreamResponse> {
        let input = request_value(request);
        if let Err(error) = http_runtime::validate_bytecode_request_input(&input) {
            return Self::ready_invalid_input(error);
        }

        let context = self.context.with_stream_runtime(registrar.stream_runtime());
        let current_scope = execution.execution_scope().clone();
        Box::pin(async move {
            let item_plan = leaf_bytes_plan();
            let output = context
                .dispatch_http_stream_with_execution_scope(&input, Some(&item_plan), current_scope)
                .await
                .map_err(map_current_scope_failure)?;
            let (status, headers, body) = strict_stream_response_parts(output)?;
            // The registrar is the only route decoder/claim authority. The
            // lower's body carrier crosses this adapter unchanged.
            let body = registrar.take_exact_route(body)?;
            Ok(BytecodeHttpStreamResponse {
                status,
                headers,
                body,
            })
        })
    }
}

impl RuntimeHost {
    pub(super) fn bytecode_http_client_port(
        &self,
        cancellation: CancellationToken,
        response_max_bytes: usize,
    ) -> Arc<dyn BytecodeHttpClientPort> {
        Arc::new(ProductionBytecodeHttpClientPort::new(
            cancellation,
            response_max_bytes,
            self.http_runtime_options.clone(),
        ))
    }
}

fn request_value(request: BytecodeHttpRequest) -> Value {
    let headers = request
        .headers
        .into_iter()
        .map(|header| {
            Value::Object(Map::from_iter([
                ("name".to_string(), Value::String(header.name)),
                ("value".to_string(), Value::String(header.value)),
            ]))
        })
        .collect();
    Value::Object(Map::from_iter([
        ("method".to_string(), Value::String(request.method)),
        ("url".to_string(), Value::String(request.url)),
        ("headers".to_string(), Value::Array(headers)),
        (
            "body".to_string(),
            request.body.map_or(Value::Null, |body| bytes_value(&body)),
        ),
        (
            "timeoutMs".to_string(),
            request
                .timeout_ms
                .map_or(Value::Null, |timeout| Value::Number(timeout.into())),
        ),
    ]))
}

fn strict_request_response(output: Value) -> Result<BytecodeHttpResponse, BytecodeHttpFailure> {
    let (status, headers, body) = strict_response_parts(output)?;
    let body = bytes_payload(&body).ok_or_else(|| {
        invalid_provider_contract("HTTP request response body is not canonical bytes")
    })?;
    Ok(BytecodeHttpResponse {
        status,
        headers,
        body,
    })
}

fn strict_stream_response_parts(
    output: Value,
) -> Result<(u16, Vec<HttpNameValue>, Value), BytecodeHttpFailure> {
    strict_response_parts(output)
}

fn strict_response_parts(
    output: Value,
) -> Result<(u16, Vec<HttpNameValue>, Value), BytecodeHttpFailure> {
    let Value::Object(mut response) = output else {
        return Err(invalid_provider_contract("HTTP response is not an object"));
    };
    if response.len() != 3
        || !response.contains_key("status")
        || !response.contains_key("headers")
        || !response.contains_key("body")
    {
        return Err(invalid_provider_contract(
            "HTTP response must contain exactly status, headers, and body",
        ));
    }
    let status = response
        .remove("status")
        .and_then(|value| value.as_u64())
        .and_then(|status| u16::try_from(status).ok())
        .ok_or_else(|| invalid_provider_contract("HTTP response status is not a u16"))?;
    let headers = strict_headers(
        response
            .remove("headers")
            .expect("exact response keys were checked above"),
    )?;
    let body = response
        .remove("body")
        .expect("exact response keys were checked above");
    Ok((status, headers, body))
}

fn strict_headers(value: Value) -> Result<Vec<HttpNameValue>, BytecodeHttpFailure> {
    let Value::Array(headers) = value else {
        return Err(invalid_provider_contract(
            "HTTP response headers are not an array",
        ));
    };
    headers
        .into_iter()
        .map(|header| {
            let Value::Object(mut header) = header else {
                return Err(invalid_provider_contract(
                    "HTTP response header is not an object",
                ));
            };
            if header.len() != 2 || !header.contains_key("name") || !header.contains_key("value") {
                return Err(invalid_provider_contract(
                    "HTTP response header must contain exactly name and value",
                ));
            }
            let name = header
                .remove("name")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| {
                    invalid_provider_contract("HTTP response header name is not a string")
                })?;
            let value = header
                .remove("value")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| {
                    invalid_provider_contract("HTTP response header value is not a string")
                })?;
            Ok(HttpNameValue { name, value })
        })
        .collect()
}

fn invalid_provider_contract(message: impl Into<String>) -> BytecodeHttpFailure {
    BytecodeHttpFailure::InvalidProviderContract(message.into())
}

fn map_current_scope_failure(error: CurrentScopeHttpFailure) -> BytecodeHttpFailure {
    match error {
        CurrentScopeHttpFailure::Cancelled => BytecodeHttpFailure::Cancelled,
        CurrentScopeHttpFailure::ScopeDeadlineExceeded
        | CurrentScopeHttpFailure::PrimitiveTimeout => BytecodeHttpFailure::DeadlineExceeded,
        CurrentScopeHttpFailure::ResponseLimitExceeded {
            limit_bytes,
            received_bytes,
        } => BytecodeHttpFailure::ResponseLimitExceeded {
            limit_bytes,
            received_bytes,
        },
        CurrentScopeHttpFailure::Runtime(RuntimeError::ExecutionBudgetExceeded {
            reason: skiff_runtime_capability_context::ExecutionBudgetReason::DeadlineExceeded,
            ..
        }) => BytecodeHttpFailure::DeadlineExceeded,
        CurrentScopeHttpFailure::Runtime(error) => {
            BytecodeHttpFailure::Transport(ordinary_http_failure(error))
        }
    }
}

fn ordinary_http_failure(error: RuntimeError) -> Box<dyn WirePayload> {
    Box::new(
        OrdinaryRuntimeError::try_new(error)
            .expect("bytecode HTTP cancellation was split before ordinary trait erasure"),
    )
}

#[derive(Clone)]
struct CallbackMethodBinding {
    function: FunctionIndex,
    provider_abi: String,
    source_abi: String,
}

struct HostCallbackCapabilityPayload {
    adapter: Arc<skiff_runtime_native::callback_adapter::InProcessCallbackAdapter>,
    contract: String,
    provider_image: Arc<DeploymentExecutionImage>,
    provider_interface: InterfaceInstantiationRef,
    methods: BTreeMap<(u32, String), CallbackMethodBinding>,
}

struct HostCallbackExecution {
    contract: String,
    adapter: Arc<skiff_runtime_native::callback_adapter::InProcessCallbackAdapter>,
    provider_image: Arc<DeploymentExecutionImage>,
    binding: CallbackMethodBinding,
}

impl CallbackExecution for HostCallbackExecution {
    fn canonical_contract(&self) -> &str {
        &self.contract
    }

    fn operation(
        &self,
        slot: u32,
        method_abi_id: &str,
    ) -> Result<
        &skiff_runtime_model::callback_projection::CallbackContractOperationProjection,
        BytecodeCallbackChildError,
    > {
        if self.binding.provider_abi != method_abi_id {
            return Err(BytecodeCallbackChildError::WrongOperation {
                slot,
                method_abi_id: method_abi_id.to_string(),
            });
        }
        self.adapter
            .operation(slot, &self.binding.source_abi)
            .map_err(|_| BytecodeCallbackChildError::WrongOperation {
                slot,
                method_abi_id: method_abi_id.to_string(),
            })
    }

    fn receiver(&self) -> &RuntimeValue {
        self.adapter.receiver()
    }

    fn owner_heap_arena(&self) -> Arc<tokio::sync::Mutex<RequestHeap>> {
        self.adapter.owner_heap_arena()
    }

    fn provider_entry(&self) -> Result<DeploymentExecutionEntry, BytecodeCallbackChildError> {
        self.provider_image
            .function_entry(self.binding.function)
            .map_err(|error| BytecodeCallbackChildError::MissingFacts {
                message: format!("callback provider function is absent: {error}"),
            })
    }
}

#[derive(Clone)]
struct ProductionBytecodeCallbackResolver {
    table: BytecodeCallbackCapabilityTable,
}

impl BytecodeCallbackResolver for ProductionBytecodeCallbackResolver {
    fn resolve_callback(
        &self,
        carrier: &skiff_runtime_model::runtime_value::CallbackCapabilityCarrier,
        expected_runtime_replica_id: &str,
        table: &skiff_runtime_linked_bytecode::LinkedInterfaceTable,
        method_ordinal: u32,
        method_abi_id: &str,
    ) -> Result<Arc<dyn CallbackExecution>, BytecodeCallbackChildError> {
        let payload = self.table.lookup(carrier).map_err(callback_lookup_error)?;
        let payload = payload
            .downcast::<HostCallbackCapabilityPayload>()
            .map_err(|_| BytecodeCallbackChildError::MissingFacts {
                message: "callback table payload is not the VM host execution payload".to_string(),
            })?;
        if payload.provider_interface != *table.interface().artifact() {
            return Err(BytecodeCallbackChildError::MissingFacts {
                message: format!(
                    "callback provider interface drifts from the linked provider table"
                ),
            });
        }
        let binding = payload
            .methods
            .get(&(method_ordinal, method_abi_id.to_string()))
            .cloned()
            .ok_or_else(|| BytecodeCallbackChildError::WrongOperation {
                slot: method_ordinal,
                method_abi_id: method_abi_id.to_string(),
            })?;
        let _ = expected_runtime_replica_id;
        Ok(Arc::new(HostCallbackExecution {
            contract: payload.contract.clone(),
            adapter: Arc::clone(&payload.adapter),
            provider_image: Arc::clone(&payload.provider_image),
            binding,
        }))
    }
}

fn callback_lookup_error(error: BytecodeCallbackError) -> BytecodeCallbackChildError {
    match error {
        BytecodeCallbackError::CrossRuntimeRejected { expected, actual } => {
            BytecodeCallbackChildError::CrossRuntimeRejected { expected, actual }
        }
        BytecodeCallbackError::CapabilityExpired | BytecodeCallbackError::Cancelled => {
            BytecodeCallbackChildError::CapabilityExpired
        }
        BytecodeCallbackError::WrongContract => BytecodeCallbackChildError::WrongContract,
        BytecodeCallbackError::CapabilityUnavailable => {
            BytecodeCallbackChildError::CapabilityUnavailable
        }
        other => BytecodeCallbackChildError::MissingFacts {
            message: other.to_string(),
        },
    }
}

#[derive(Clone)]
struct ProductionBytecodeCallbackProjector {
    hooks: BytecodeCallbackCapabilityHooks,
}

impl BytecodeCallbackProjector for ProductionBytecodeCallbackProjector {
    fn project_callback_argument(
        &self,
        source_heap: &mut dyn skiff_runtime_model::vm_heap::VmHeap,
        source: &ValueSlot,
        caller_image: &Arc<DeploymentExecutionImage>,
        destination_heap: &mut dyn skiff_runtime_model::vm_heap::VmHeap,
        provider_image: &DeploymentExecutionImage,
        provider_type: skiff_runtime_linked_bytecode::TypeIndex,
        plan: &skiff_runtime_linked_bytecode::LinkedServiceBoundaryValue,
    ) -> Result<ValueSlot, BytecodeCallbackChildError> {
        let source_vm = source_heap
            .as_any()
            .and_then(|heap| heap.downcast_ref::<RequestVmHeap>())
            .ok_or_else(|| projection_error("callback source heap is not a request VM heap"))?;
        let local = source_vm
            .local_interface_linked_table(source)
            .map_err(|error| projection_error(error.to_string()))?;
        let vm_table = source_heap
            .local_interface_table(source)
            .map_err(|error| projection_error(error.to_string()))?;
        let interface_row = caller_image
            .interface_tables()
            .iter()
            .find(|row| {
                row.index().get() == vm_table.table_index()
                    && matches!(row.kind(), LinkedInterfaceTableKind::Local(_))
            })
            .ok_or_else(|| projection_error("callback local interface row is absent"))?;
        let payload_slot = source_heap
            .local_interface_payload(source)
            .map_err(|error| projection_error(error.to_string()))?;
        let payload_value = source_vm
            .runtime_value_for_slot(&payload_slot)
            .map_err(|error| projection_error(error.to_string()))?;
        let mut temporary_heap = RequestHeap::new(source_vm.limits().clone());
        let cloned_payload = deep_clone_runtime_value_between_heaps(
            source_vm.request_heap(),
            &mut temporary_heap,
            &payload_value,
        )
        .map_err(|error| projection_error(error.to_string()))?;

        let records: PackageSchemaRecords = provider_image
            .schema_records()
            .iter()
            .map(|(type_id, record)| (type_id.clone(), Arc::new(record.clone())))
            .collect();
        let package_schema = package_schema_type(plan.contract_type())?;
        let operations = callback_operations(plan.contract_type(), &records)?;
        let interface_value = local_interface_value(
            &local,
            interface_row
                .interface()
                .artifact()
                .interface_abi_id
                .as_str(),
            &operations,
            cloned_payload,
        )?;
        let temporary_handle = temporary_heap
            .alloc_interface(interface_value)
            .map_err(|error| projection_error(error.to_string()))?;
        let temporary_interface = match temporary_heap.get(temporary_handle) {
            Ok(skiff_runtime_model::value::HeapNode::Interface(value)) => value,
            _ => {
                return Err(projection_error(
                    "temporary callback interface allocation is absent",
                ));
            }
        };
        let adapter =
            skiff_runtime_native::callback_adapter::InProcessCallbackAdapter::from_local_interface(
                package_schema,
                temporary_interface,
                &operations,
                &records,
                &temporary_heap,
            )
            .map_err(|error| projection_error(error.to_string()))?;
        let contract = adapter
            .canonical_contract_identity()
            .map_err(|error| projection_error(error.to_string()))?;
        let correlation = callback_methods(
            local.as_ref(),
            interface_row.interface().artifact(),
            provider_image.interface_tables(),
            caller_image.as_ref(),
            provider_image,
        )?;
        let payload: Arc<dyn std::any::Any + Send + Sync> =
            Arc::new(HostCallbackCapabilityPayload {
                adapter: Arc::new(adapter),
                contract: contract.clone(),
                provider_image: Arc::clone(caller_image),
                provider_interface: correlation.provider_interface,
                methods: correlation.methods,
            });
        let lifetime = callback_lifetime(plan)?;
        let projection = self
            .hooks
            .register_payload(
                lifetime,
                contract,
                interface_row
                    .interface()
                    .artifact()
                    .interface_abi_id
                    .clone(),
                payload,
            )
            .map_err(callback_registration_error)?;

        let destination_vm = destination_heap
            .as_any_mut()
            .and_then(|heap| heap.downcast_mut::<RequestVmHeap>())
            .ok_or_else(|| {
                projection_error("callback destination heap is not a request VM heap")
            })?;
        let tag = CompactTypeTag::try_from_type_index(provider_type.get()).ok_or_else(|| {
            projection_error(format!(
                "callback provider type {} does not fit compact tag",
                provider_type.get()
            ))
        })?;
        let capability = projection.capability().clone();
        let receiver_abi = projection.receiver_interface_abi_id().to_string();
        let handle = destination_vm
            .request_heap_mut()
            .alloc_interface(InterfaceValue::new(
                receiver_abi,
                InterfaceCarrier::CallbackCapability(capability),
            ))
            .map_err(|error| projection_error(error.to_string()))?;
        let slot = destination_vm
            .heap_ref(handle, tag, ValueFlags::new(0))
            .map_err(|error| projection_error(error.to_string()))?;
        projection.commit();
        Ok(slot)
    }
}

fn projection_error(message: impl Into<String>) -> BytecodeCallbackChildError {
    BytecodeCallbackChildError::Materialization {
        message: message.into(),
    }
}

fn callback_registration_error(error: BytecodeCallbackError) -> BytecodeCallbackChildError {
    match error {
        BytecodeCallbackError::CrossRuntimeRejected { expected, actual } => {
            BytecodeCallbackChildError::CrossRuntimeRejected { expected, actual }
        }
        BytecodeCallbackError::CapabilityExpired | BytecodeCallbackError::Cancelled => {
            BytecodeCallbackChildError::CapabilityExpired
        }
        BytecodeCallbackError::WrongContract => BytecodeCallbackChildError::WrongContract,
        BytecodeCallbackError::CapabilityUnavailable => {
            BytecodeCallbackChildError::CapabilityUnavailable
        }
        other => BytecodeCallbackChildError::MissingFacts {
            message: other.to_string(),
        },
    }
}

fn package_schema_type(
    ty: &ContractTypeRef,
) -> Result<PackageSchemaTypeRef, BytecodeCallbackChildError> {
    match classify_boundary_callback_position(ty) {
        BoundaryCallbackPosition::Exact { interface_type } => Ok(interface_type),
        _ => Err(projection_error(
            "callback capability requires an exact non-generic any interface",
        )),
    }
}

fn callback_operations(
    ty: &ContractTypeRef,
    records: &PackageSchemaRecords,
) -> Result<
    BTreeMap<String, skiff_artifact_model::BoundaryCallbackOperation>,
    BytecodeCallbackChildError,
> {
    let reference = package_schema_type(ty)?;
    let record = records
        .get(&reference.package_schema_type_id)
        .ok_or_else(|| projection_error("callback package schema record is absent"))?;
    let skiff_artifact_model::ContractTypeDescriptor::CallbackInterface { operations } =
        &record.canonical_descriptor.descriptor
    else {
        return Err(projection_error(
            "package schema type is not a callback interface",
        ));
    };
    Ok(operations.clone())
}

fn callback_lifetime(
    plan: &skiff_runtime_linked_bytecode::LinkedServiceBoundaryValue,
) -> Result<CallbackLifetime, BytecodeCallbackChildError> {
    let BoundaryValuePlan::Linkable { lifetime, .. } = plan.value_plan() else {
        return Err(projection_error(
            "callback service argument has no linkable boundary plan",
        ));
    };
    CallbackLifetime::from_boundary(*lifetime)
        .map_err(|error| projection_error(format!("callback lifetime is unsupported: {error}")))
}

struct CallbackMethodCorrelation {
    provider_interface: InterfaceInstantiationRef,
    methods: BTreeMap<(u32, String), CallbackMethodBinding>,
}

fn callback_methods(
    local: &LinkedLocalInterfaceTable,
    local_interface: &InterfaceInstantiationRef,
    provider_tables: &[skiff_runtime_linked_bytecode::LinkedInterfaceTable],
    caller_image: &DeploymentExecutionImage,
    provider_image: &DeploymentExecutionImage,
) -> Result<CallbackMethodCorrelation, BytecodeCallbackChildError> {
    let mut candidates = provider_tables.iter().filter_map(|row| {
        let LinkedInterfaceTableKind::Callback(requirement) = row.kind() else {
            return None;
        };
        (row.interface().artifact() == local_interface).then_some((row, requirement))
    });
    let (row, requirement) =
        candidates
            .next()
            .ok_or_else(|| BytecodeCallbackChildError::MissingFacts {
                message: "provider callback interface table is absent".to_string(),
            })?;
    if candidates.next().is_some() {
        return Err(BytecodeCallbackChildError::MissingFacts {
            message: "provider callback interface table correlation is ambiguous".to_string(),
        });
    }
    if requirement.methods().len() != local.methods().len() {
        return Err(BytecodeCallbackChildError::SignatureMismatch {
            message: format!(
                "provider callback method count {} differs from caller method count {}",
                requirement.methods().len(),
                local.methods().len()
            ),
        });
    }
    let mut methods = BTreeMap::new();
    for local_method in local.methods() {
        let provider_method = requirement
            .methods()
            .iter()
            .find(|method| {
                method.method_slot() == local_method.method_slot()
                    && method.method_abi_id().as_str() == local_method.method_abi_id().as_str()
            })
            .ok_or_else(|| BytecodeCallbackChildError::WrongOperation {
                slot: local_method.method_slot(),
                method_abi_id: local_method.method_abi_id().as_str().to_string(),
            })?;
        if !callback_provider_receiver_matches(
            provider_image,
            provider_method.signature(),
            local_interface,
        ) || !linked_signatures_types_match(
            caller_image,
            local_method.signature(),
            provider_image,
            provider_method.signature(),
        ) {
            return Err(BytecodeCallbackChildError::SignatureMismatch {
                message: format!(
                    "provider callback method {} signature drifts from caller method {}",
                    provider_method.method_abi_id().as_str(),
                    local_method.method_abi_id().as_str()
                ),
            });
        }
        let provider_abi = provider_method.method_abi_id().as_str().to_string();
        let key = (provider_method.method_slot(), provider_abi.clone());
        if methods
            .insert(
                key,
                CallbackMethodBinding {
                    function: local_method.function(),
                    provider_abi,
                    source_abi: local_method.method_abi_id().as_str().to_string(),
                },
            )
            .is_some()
        {
            return Err(BytecodeCallbackChildError::MissingFacts {
                message: "provider callback method table repeats an exact method".to_string(),
            });
        }
    }
    Ok(CallbackMethodCorrelation {
        provider_interface: row.interface().artifact().clone(),
        methods,
    })
}

fn callback_provider_receiver_matches(
    provider_image: &DeploymentExecutionImage,
    provider: &LinkedCallableSignature,
    interface: &InterfaceInstantiationRef,
) -> bool {
    let Some(&receiver) = provider.parameter_types().first() else {
        return false;
    };
    matches!(
        linked_type_ref(provider_image, receiver),
        Some(TypeRefIr::AnyInterface {
            interface: actual,
        }) if actual == interface
    )
}

fn linked_signatures_types_match(
    caller_image: &DeploymentExecutionImage,
    caller: &LinkedCallableSignature,
    provider_image: &DeploymentExecutionImage,
    provider: &LinkedCallableSignature,
) -> bool {
    let parameters_match = caller.parameter_types().len() == provider.parameter_types().len()
        && linked_callback_parameters_match(caller_image, caller, provider_image, provider);
    let results_match = caller.result_types().len() == provider.result_types().len()
        && caller.result_plans() == provider.result_plans()
        && caller
            .result_types()
            .iter()
            .zip(provider.result_types())
            .all(|(caller_type, provider_type)| {
                linked_type_ref(caller_image, *caller_type)
                    == linked_type_ref(provider_image, *provider_type)
            });
    parameters_match && results_match
}

fn linked_callback_parameters_match(
    caller_image: &DeploymentExecutionImage,
    caller: &LinkedCallableSignature,
    provider_image: &DeploymentExecutionImage,
    provider: &LinkedCallableSignature,
) -> bool {
    let Some(caller_types) = caller.parameter_types().get(1..) else {
        return provider.parameter_types().get(1..).is_none();
    };
    let Some(provider_types) = provider.parameter_types().get(1..) else {
        return false;
    };
    caller_types.len() == provider_types.len()
        && caller.parameter_modes().get(1..) == provider.parameter_modes().get(1..)
        && caller.parameter_plans().get(1..) == provider.parameter_plans().get(1..)
        && caller_types
            .iter()
            .zip(provider_types)
            .all(|(caller_type, provider_type)| {
                linked_type_ref(caller_image, *caller_type)
                    == linked_type_ref(provider_image, *provider_type)
            })
}

fn linked_type_ref(
    image: &DeploymentExecutionImage,
    index: skiff_runtime_linked_bytecode::TypeIndex,
) -> Option<&TypeRefIr> {
    let position = usize::try_from(index.get()).ok()?;
    image
        .types()
        .get(position)
        .filter(|entry| entry.index() == index)
        .map(|entry| entry.type_ref())
}

fn local_interface_value(
    local: &LinkedLocalInterfaceTable,
    interface_abi_id: &str,
    operations: &BTreeMap<String, skiff_artifact_model::BoundaryCallbackOperation>,
    payload: RuntimeValue,
) -> Result<InterfaceValue, BytecodeCallbackChildError> {
    let mut slots = Vec::with_capacity(local.methods().len());
    for method in local.methods() {
        let contract_operation = operations
            .get(method.method_name())
            .ok_or_else(|| projection_error("callback local method has no contract operation"))?;
        let mut parameters = Vec::with_capacity(contract_operation.parameters.len() + 1);
        parameters.push(InterfaceMethodType::builtin("Self"));
        parameters.extend(
            contract_operation
                .parameters
                .iter()
                .map(contract_type_to_interface_method_type)
                .collect::<Result<Vec<_>, _>>()?,
        );
        let signature = skiff_runtime_model::runtime_value::InterfaceMethodSignature::new(
            parameters,
            contract_type_to_interface_method_type(&contract_operation.return_type)?,
        );
        slots.push(
            skiff_runtime_model::runtime_value::InterfaceMethodSlot::from_admitted_metadata(
                method.method_slot(),
                method.method_name().to_string(),
                method.method_abi_id().as_str().to_string(),
                signature,
                InterfaceMethodTarget::LocalExecutable {
                    executable: ExecutableAddr::service(0, method.method_slot() as usize),
                    receiver_call_abi: InterfaceReceiverCallAbi::ExplicitSelfFirst,
                },
            ),
        );
    }
    let method_table = InterfaceMethodTable::new(
        interface_abi_id.to_string(),
        interface_abi_id.to_string(),
        slots,
    );
    Ok(InterfaceValue::new(
        interface_abi_id.to_string(),
        InterfaceCarrier::Local {
            concrete_type: format!("local:{}", local.concrete_type().get()),
            method_table,
            payload,
        },
    ))
}

fn contract_type_to_interface_method_type(
    ty: &ContractTypeRef,
) -> Result<InterfaceMethodType, BytecodeCallbackChildError> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => Ok(InterfaceMethodType::Builtin {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(contract_type_to_interface_method_type)
                .collect::<Result<Vec<_>, _>>()?,
        }),
        ContractTypeRef::Record { fields } => Ok(InterfaceMethodType::Record(
            fields
                .iter()
                .map(|(name, ty)| Ok((name.clone(), contract_type_to_interface_method_type(ty)?)))
                .collect::<Result<BTreeMap<_, _>, BytecodeCallbackChildError>>()?,
        )),
        ContractTypeRef::StructuralUnion { variants } => Ok(InterfaceMethodType::Union(
            variants
                .iter()
                .map(contract_type_to_interface_method_type)
                .collect::<Result<Vec<_>, _>>()?,
        )),
        ContractTypeRef::Nullable { inner } => Ok(InterfaceMethodType::Nullable(Box::new(
            contract_type_to_interface_method_type(inner)?,
        ))),
        ContractTypeRef::Literal {
            value: ContractLiteral::String { value },
        } => Ok(InterfaceMethodType::Literal(
            skiff_runtime_model::runtime_value::InterfaceMethodLiteral::String(value.clone()),
        )),
        other => Err(projection_error(format!(
            "callback contract type is not supported by the local method projection: {other:?}"
        ))),
    }
}

pub(crate) struct ProductionBytecodeServiceResolver {
    host: RuntimeHost,
}

impl ProductionBytecodeServiceResolver {
    pub(crate) fn new(host: RuntimeHost) -> Self {
        Self { host }
    }
}

impl BytecodeServiceResolver for ProductionBytecodeServiceResolver {
    fn resolve_service(
        &self,
        slot: &skiff_runtime_deployment_image::ServiceDependencySlot,
        _operation: &skiff_artifact_model::ContractOperationId,
        expected_protocol: &skiff_artifact_model::ServiceProtocolIdentity,
    ) -> Result<
        std::sync::Arc<skiff_runtime_linker::DeploymentExecutionImage>,
        BytecodeServiceChildError,
    > {
        let root = self.host.bootstrap_artifact_root().ok_or_else(|| {
            BytecodeServiceChildError::ProviderMissing {
                service_id: slot.contract().service_id.clone(),
                contract_version: slot.contract().contract_version.clone(),
            }
        })?;
        let profile = self.host.frozen_profile.get().ok_or_else(|| {
            BytecodeServiceChildError::ProviderMissing {
                service_id: slot.contract().service_id.clone(),
                contract_version: slot.contract().contract_version.clone(),
            }
        })?;
        let store =
            skiff_deployment::storage::CanonicalArtifactStore::open(std::path::Path::new(&root))
                .map_err(|error| BytecodeServiceChildError::Load {
                    message: error.to_string(),
                })?;
        let pointer = store
            .read_release_pointer(
                profile,
                &slot.contract().service_id,
                &slot.contract().contract_version,
            )
            .map_err(|error| BytecodeServiceChildError::Load {
                message: error.to_string(),
            })?
            .ok_or_else(|| BytecodeServiceChildError::ProviderMissing {
                service_id: slot.contract().service_id.clone(),
                contract_version: slot.contract().contract_version.clone(),
            })?;
        if &pointer.deployment.service_id != &slot.contract().service_id
            || &pointer.deployment.contract_version != &slot.contract().contract_version
        {
            return Err(BytecodeServiceChildError::DeploymentDrift);
        }
        let image = self
            .host
            .bytecode_deployments
            .loaded_or_failed_sync(&pointer.deployment)
            .ok_or_else(|| BytecodeServiceChildError::ProviderMissing {
                service_id: slot.contract().service_id.clone(),
                contract_version: slot.contract().contract_version.clone(),
            })?
            .map_err(|message| BytecodeServiceChildError::Load { message })?;
        if image.owner().deployment() != &pointer.deployment {
            return Err(BytecodeServiceChildError::DeploymentDrift);
        }
        if image.service_protocol_identity() != expected_protocol {
            return Err(BytecodeServiceChildError::ProtocolMismatch {
                expected: expected_protocol.clone(),
                actual: image.service_protocol_identity().clone(),
            });
        }
        Ok(image)
    }
}

#[derive(Clone)]
struct ProductionBytecodeTaskSubmitter {
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
    outbound_requests: Arc<OutboundRequestRegistry>,
}

impl ProductionBytecodeTaskSubmitter {
    fn new(
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
        outbound_requests: Arc<OutboundRequestRegistry>,
    ) -> Self {
        Self {
            sender,
            outbound_requests,
        }
    }
}

impl BytecodeTaskSubmitter for ProductionBytecodeTaskSubmitter {
    fn submit(
        &self,
        message: TaskSubmitControlMessage,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<TaskSubmitResponseControl, BytecodeTaskSubmitError>>
                + Send
                + 'static,
        >,
    > {
        let sender = self.sender.clone();
        let outbound_requests = Arc::clone(&self.outbound_requests);
        Box::pin(async move {
            let rpc_id = message.request.rpc_id.clone();
            let (response_tx, mut response_rx) = mpsc::unbounded_channel();
            let cancel_sender: Option<OutboundRequestCancelSender> = {
                let sender = sender.clone();
                Some(Arc::new(move |request_id, reason| {
                    sender
                        .send(RouterWriterMessage::Control(
                            OutboundControlMessage::RequestCancel {
                                request: RequestCancelControl {
                                    request_id: request_id.to_string(),
                                    reason: reason.to_string(),
                                },
                            },
                        ))
                        .map_err(|_| OutboundRequestCancelSendError::Closed)
                }))
            };
            let lease = outbound_requests
                .insert_with_lease(
                    rpc_id.clone(),
                    response_tx,
                    cancel_sender,
                    "task_child_submit",
                )
                .map_err(|error| BytecodeTaskSubmitError::Protocol(error.to_string()))?;
            if sender
                .send(RouterWriterMessage::TaskSubmit(message))
                .is_err()
            {
                let _ = lease.cancel("runtime_disconnect");
                return Err(BytecodeTaskSubmitError::Closed);
            }
            match response_rx.recv().await {
                Some(OutboundResponse::End { payload }) => {
                    lease.complete();
                    parse_task_submit_response(&payload, &rpc_id)
                }
                Some(OutboundResponse::Error(error)) => {
                    lease.complete();
                    Err(BytecodeTaskSubmitError::Rejected {
                        code: error.code,
                        message: error.message,
                    })
                }
                None => {
                    let _ = lease.cancel("response_channel_closed");
                    Err(BytecodeTaskSubmitError::Protocol(
                        "task submit response channel closed".to_string(),
                    ))
                }
            }
        })
    }
}

fn parse_task_submit_response(
    payload: &[u8],
    expected_rpc_id: &str,
) -> Result<TaskSubmitResponseControl, BytecodeTaskSubmitError> {
    let header: TaskSubmitResponseFrameHeader =
        serde_json::from_slice(payload).map_err(|error| {
            BytecodeTaskSubmitError::Protocol(format!(
                "task.submit.response header is not valid JSON: {error}"
            ))
        })?;
    if header.envelope_type != "task.submit.response" {
        return Err(BytecodeTaskSubmitError::Protocol(format!(
            "task.submit.response envelope type is {}, expected task.submit.response",
            header.envelope_type
        )));
    }
    if header.rpc_id != expected_rpc_id {
        return Err(BytecodeTaskSubmitError::Protocol(format!(
            "task.submit.response rpcId {} does not match request {}",
            header.rpc_id, expected_rpc_id
        )));
    }
    if header.status != "submitted" {
        return Err(BytecodeTaskSubmitError::Protocol(format!(
            "task.submit.response status must be submitted, got {}",
            header.status
        )));
    }
    Ok(TaskSubmitResponseControl {
        task_ref: header.task_ref.into_string(),
        task_id: header.task_id,
        request_id: header.request_id,
    })
}

pub(crate) fn bytecode_request_child_composition(
    host: &RuntimeHost,
    image: &DeploymentExecutionImage,
    db_source: Option<&DbCapabilitySource>,
    request_id: &str,
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
    activation_identity: Option<ActivationIdentityControl>,
) -> BytecodeRequestChildComposition {
    let owner = image.owner();
    let deployment = owner.deployment();
    let db_child = BytecodeDbChildComposition {
        capability_context: db_source
            .map(|source| source.context_for_request(deployment.service_id.clone(), request_id)),
        recoverable_context: Some(bytecode_db_recoverable_context(image)),
        // F6 has not yet emitted DbObjectTargetId into the execution image;
        // an absent exact target must fail closed before any provider call.
        exact_target: None,
    };
    let task_child = BytecodeTaskChildComposition {
        submitter: Arc::new(ProductionBytecodeTaskSubmitter::new(
            sender,
            Arc::clone(&host.outbound_requests),
        )),
        caller_request_id: request_id.to_string(),
        runtime_id: host.base_runtime_id.clone(),
        activation_identity,
    };
    let mut composition =
        bytecode_request_child_composition_with_parts(host, db_child, request_id, task_child);
    let actor_executor = Arc::clone(&host.bytecode_actor_executor);
    composition.actor_child = BytecodeActorChildComposition {
        exact_build: Some(image.owner().build_id().as_str().to_string()),
        arena_lease_root: Some(actor_executor.arena_lease_root()),
        executor: Some(actor_executor),
    };
    composition
}

pub(crate) fn bytecode_request_child_composition_with_db_child(
    host: &RuntimeHost,
    db_child: BytecodeDbChildComposition,
    request_id: &str,
) -> BytecodeRequestChildComposition {
    bytecode_request_child_composition_with_parts(
        host,
        db_child,
        request_id,
        BytecodeTaskChildComposition::default(),
    )
}

fn bytecode_request_child_composition_with_parts(
    host: &RuntimeHost,
    db_child: BytecodeDbChildComposition,
    request_id: &str,
    task_child: BytecodeTaskChildComposition,
) -> BytecodeRequestChildComposition {
    let limits = host.request_heap_limits();
    let callback_hooks = bytecode_callback_hooks(host, request_id);
    let callback_table = callback_hooks.table().clone();
    // A same-Runtime callback is a nested child heap: the provider service
    // child and the callback child are both live while the callback executes.
    // Keep the aggregate cap bounded but allow more than one owner-local heap
    // to be live at once.
    let aggregate_hard_cap = limits.max_estimated_bytes.saturating_mul(4);
    BytecodeRequestChildComposition {
        memory_ledger: Arc::new(RequestMemoryLedger::new(aggregate_hard_cap)),
        service_resolver: Arc::new(ProductionBytecodeServiceResolver::new(host.clone())),
        child_heap_factory: None,
        heap_limits: limits,
        throw_materializer: Arc::new(CrossImageServiceChildThrowMaterializer),
        callback_hooks: Some(Arc::new(callback_hooks.clone())),
        callback_child: BytecodeCallbackChildComposition {
            runtime_replica_id: host.base_runtime_id.clone(),
            resolver: Some(Arc::new(ProductionBytecodeCallbackResolver {
                table: callback_table,
            })),
        },
        callback_projector: Some(Arc::new(ProductionBytecodeCallbackProjector {
            hooks: callback_hooks,
        })),
        actor_child: BytecodeActorChildComposition::default(),
        db_child,
        task_child,
        child_streams: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
    }
}

fn bytecode_callback_hooks(
    host: &RuntimeHost,
    request_id: &str,
) -> BytecodeCallbackCapabilityHooks {
    let table = BytecodeCallbackCapabilityTable::new(
        host.base_runtime_id.clone(),
        format!("{}-{}", host.base_runtime_id, request_id),
    );
    BytecodeCallbackCapabilityHooks::new(table, 1)
}

fn bytecode_db_recoverable_context(
    image: &DeploymentExecutionImage,
) -> DbRecoverableRuntimeContext {
    let owner = image.owner();
    let deployment = owner.deployment();
    let build_id = owner.build_id().as_str().to_string();
    DbRecoverableRuntimeContext {
        behavior_hooks: Arc::new(FailClosedRecoverableBehaviorHooks),
        expected_plans: DbRecoverableRuntimeExpectedPlans::default(),
        artifact_identity: deployment.deployment_artifact_identity.as_str().to_string(),
        build_id: build_id.clone(),
        boundary_context: RuntimeRecoverableBoundaryContext::new(
            RuntimeRecoverableBoundaryKind::DbValue,
            RuntimeRecoverableTrustBoundary::OwnerInternal,
            RuntimeRecoverableStorageLane::RecoverableEnvelope,
        )
        .with_origin_service(RuntimeRecoverableServiceRef {
            service_id: deployment.service_id.clone(),
            version: Some(deployment.contract_version.clone()),
            build_id: Some(build_id),
        })
        .with_explicit_recoverable_slot(),
        retention_expires_at_epoch_millis: None,
    }
}

impl RuntimeHost {
    pub(super) async fn preload_service_dependencies(
        &self,
        caller_image: &std::sync::Arc<skiff_runtime_linker::DeploymentExecutionImage>,
    ) {
        let Some(root) = self.bootstrap_artifact_root() else {
            return;
        };
        let Some(profile) = self.frozen_profile.get() else {
            return;
        };
        let Ok(store) =
            skiff_deployment::storage::CanonicalArtifactStore::open(std::path::Path::new(&root))
        else {
            return;
        };
        for slot in caller_image.dependency_slots() {
            let contract = slot.contract();
            let Ok(Some(pointer)) = store.read_release_pointer(
                profile,
                &contract.service_id,
                &contract.contract_version,
            ) else {
                continue;
            };
            let _ = self
                .bytecode_deployments
                .get_or_load(&pointer.deployment, std::path::Path::new(&root))
                .await;
        }
    }
}

#[cfg(test)]
mod tests;
