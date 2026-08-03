use std::{
    collections::{BTreeMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use skiff_artifact_model::{
    BoundaryCallbackOperation, BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime,
    BoundaryValueOwner, BoundaryValuePlan, ContractTypeDescriptor, ContractTypeRef,
    PackageSchemaTypeId, PackageSchemaTypeRef,
};
use skiff_runtime_activation::{CallbackCapabilityError, CallbackLifetime};
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_boundary::service_linkable::{
    FailClosedServiceLinkableCapabilityHooks, ServiceLinkableCapabilityHooks,
    ServiceLinkableCapabilityProjection, ServiceLinkableCapabilityRequest,
    ServiceLinkableContractPlan, ServiceLinkableMaterializationError,
    ServiceLinkableMaterializationScope,
};
use skiff_runtime_linked_program::CallIr;
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{CallbackCapabilityCarrier, HeapNode, InterfaceValue, RuntimeValue},
};
use skiff_runtime_native::callback_adapter::InProcessCallbackAdapter;

use crate::{
    error::{Result, RuntimeError},
    eval_context::EvalContext,
    program_execution::ProgramExecutionContext,
    program_ir::executable_has_explicit_self_binding,
};

static CALLBACK_CAPABILITY_ID: AtomicU64 = AtomicU64::new(1);

fn callback_capability_error(error: CallbackCapabilityError) -> RuntimeError {
    RuntimeError::ProviderUnavailable {
        target: "in-process callback-interface capability".to_string(),
        reason: error.to_string(),
    }
}

fn callback_materialization_error(
    error: impl std::fmt::Display,
) -> ServiceLinkableMaterializationError {
    ServiceLinkableMaterializationError::RuntimeModel {
        message: error.to_string(),
    }
}

/// Adapter passed by ordinary/stream materializers whenever a value plan requires an explicit
/// callback or native capability. Registration always belongs to the current activation and the
/// current top-level request generation.
pub(crate) struct CallbackNativeCapabilityHooks<'context, 'execution> {
    context: &'context ProgramExecutionContext<'execution>,
}

impl<'context, 'execution> CallbackNativeCapabilityHooks<'context, 'execution> {
    pub(crate) fn new(context: &'context ProgramExecutionContext<'execution>) -> Self {
        Self { context }
    }

    fn project_callback(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
    ) -> std::result::Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError>
    {
        let target = self
            .context
            .runtime_assembly_target()
            .map_err(callback_materialization_error)?;
        let interface = interface_value(request.value, request.source_heap)?;
        let (callback_type, operations) =
            callback_contract(request.ty, request.package_schema_records)?;
        let adapter = InProcessCallbackAdapter::from_local_interface(
            callback_type,
            &interface,
            operations,
            request.package_schema_records,
            request.source_heap,
        )
        .map_err(callback_materialization_error)?;
        validate_adapter_preimage(target, &adapter)?;
        let contract = serde_json::to_string(adapter.canonical_package_schema_type())
            .map_err(callback_materialization_error)?;
        let receiver_interface_abi_id = adapter.source_interface().to_string();
        let lifetime = match request.lifetime {
            BoundaryValueLifetime::Request => CallbackLifetime::Request,
            BoundaryValueLifetime::Stream => CallbackLifetime::Stream,
            BoundaryValueLifetime::Call => {
                return Err(ServiceLinkableMaterializationError::InvalidPlan {
                    message: "callback capability cannot have call lifetime",
                });
            }
        };
        let owner = target.activation_context();
        let opaque_id = format!(
            "callback:{}:{}",
            target.request_activation().generation(),
            CALLBACK_CAPABILITY_ID.fetch_add(1, Ordering::Relaxed)
        );
        let table = owner.callback_capabilities().clone();
        let carrier = table
            .register(
                owner,
                target.request_activation(),
                contract,
                opaque_id,
                lifetime,
                Arc::new(adapter),
            )
            .map_err(callback_materialization_error)?;
        let rollback_carrier = carrier.clone();
        Ok(
            ServiceLinkableCapabilityProjection::new_with_receiver_interface(
                carrier,
                receiver_interface_abi_id,
                move || {
                    let _ = table.revoke(&rollback_carrier);
                },
            ),
        )
    }
}

impl ServiceLinkableCapabilityHooks for CallbackNativeCapabilityHooks<'_, '_> {
    fn project_callback_capability(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
    ) -> std::result::Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError>
    {
        self.project_callback(request)
    }

    fn project_native_adapter_capability(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
    ) -> std::result::Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError>
    {
        let _ = request;
        Err(ServiceLinkableMaterializationError::InvalidPlan {
            message: "native callback adapters require an explicit non-service capability path",
        })
    }
}

mod prepared;
#[allow(unused_imports)]
pub(crate) use prepared::{
    prepare_interface_call, CompletedCallbackInvocation, PreparedCallbackInvocation,
};

pub(crate) async fn execute_interface_call(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    carrier: &CallbackCapabilityCarrier,
    method_abi_id: &str,
    slot: u32,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let prepared = prepare_interface_call(context, call, carrier, method_abi_id, slot, args)?;
    prepared
        .wait(context.interpreter)
        .await
        .finalize(context.heap)
}

fn interface_value(
    value: &RuntimeValue,
    heap: &RequestHeap,
) -> std::result::Result<InterfaceValue, ServiceLinkableMaterializationError> {
    let RuntimeValue::Heap(handle) = value else {
        return Err(ServiceLinkableMaterializationError::TypeMismatch);
    };
    match heap.get(*handle).map_err(callback_materialization_error)? {
        HeapNode::Interface(interface) => Ok(interface.clone()),
        _ => Err(ServiceLinkableMaterializationError::TypeMismatch),
    }
}

fn callback_contract<'a>(
    ty: &ContractTypeRef,
    schema: &'a PackageSchemaRecords,
) -> std::result::Result<
    (
        PackageSchemaTypeRef,
        &'a BTreeMap<String, BoundaryCallbackOperation>,
    ),
    ServiceLinkableMaterializationError,
> {
    let ContractTypeRef::AnyInterface {
        interface,
        arguments,
    } = ty
    else {
        return Err(ServiceLinkableMaterializationError::TypeMismatch);
    };
    if !arguments.is_empty() {
        return Err(ServiceLinkableMaterializationError::TypeMismatch);
    }
    let ContractTypeRef::PackageSchema { .. } = interface.as_ref() else {
        return Err(ServiceLinkableMaterializationError::TypeMismatch);
    };
    callback_contract_inner(interface, schema, &mut HashSet::new())
}

fn callback_contract_inner<'a>(
    ty: &ContractTypeRef,
    schema: &'a PackageSchemaRecords,
    active: &mut HashSet<PackageSchemaTypeId>,
) -> std::result::Result<
    (
        PackageSchemaTypeRef,
        &'a BTreeMap<String, BoundaryCallbackOperation>,
    ),
    ServiceLinkableMaterializationError,
> {
    let ContractTypeRef::PackageSchema {
        package_id,
        stable_schema_key,
        package_schema_type_id,
    } = ty
    else {
        return Err(ServiceLinkableMaterializationError::TypeMismatch);
    };
    if !active.insert(package_schema_type_id.clone()) {
        return Err(ServiceLinkableMaterializationError::CyclicSchema {
            package_schema_type_id: package_schema_type_id.clone(),
        });
    }
    let schema_type = schema.get(package_schema_type_id).ok_or_else(|| {
        ServiceLinkableMaterializationError::MissingSchema {
            package_schema_type_id: package_schema_type_id.clone(),
        }
    })?;
    if schema_type.package_schema_type_id != *package_schema_type_id {
        return Err(
            ServiceLinkableMaterializationError::SchemaIdentityMismatch {
                requested: package_schema_type_id.clone(),
                actual: schema_type.package_schema_type_id.clone(),
            },
        );
    }
    if schema_type.package_id != *package_id || schema_type.stable_schema_key != *stable_schema_key
    {
        return Err(
            ServiceLinkableMaterializationError::SchemaOwnerOrKeyMismatch {
                package_schema_type_id: package_schema_type_id.clone(),
                expected_package_id: package_id.clone(),
                expected_stable_schema_key: stable_schema_key.clone(),
                actual_package_id: schema_type.package_id.clone(),
                actual_stable_schema_key: schema_type.stable_schema_key.clone(),
            },
        );
    }
    let result = match &schema_type.canonical_descriptor.descriptor {
        ContractTypeDescriptor::CallbackInterface { operations } => Ok((
            PackageSchemaTypeRef {
                package_id: package_id.clone(),
                stable_schema_key: stable_schema_key.clone(),
                package_schema_type_id: package_schema_type_id.clone(),
            },
            operations,
        )),
        ContractTypeDescriptor::Alias { target }
        | ContractTypeDescriptor::Representation { target } => {
            callback_contract_inner(target, schema, active)
        }
        _ => Err(ServiceLinkableMaterializationError::TypeMismatch),
    };
    active.remove(package_schema_type_id);
    result
}

fn validate_adapter_preimage(
    target: &crate::RuntimeAssemblyEvalTarget,
    adapter: &InProcessCallbackAdapter,
) -> std::result::Result<(), ServiceLinkableMaterializationError> {
    let detached_plan = BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner: BoundaryValueOwner::Caller,
        lifetime: BoundaryValueLifetime::Call,
    };
    for operation in adapter.operations() {
        for ty in operation
            .parameters()
            .iter()
            .chain(std::iter::once(operation.return_type()))
        {
            ServiceLinkableContractPlan::new(ty, adapter.package_schema_records(), &detached_plan)?;
        }
        let executable = target
            .execution_image()
            .executable_at(operation.executable())
            .map_err(callback_materialization_error)?;
        let executable = executable.executable();
        let explicit_self = usize::from(executable_has_explicit_self_binding(executable));
        if !executable.type_params.is_empty()
            || executable.params.len().saturating_sub(explicit_self) != operation.parameters().len()
        {
            return Err(ServiceLinkableMaterializationError::InvalidPlan {
                message: "callback adapter executable does not match its contract operation",
            });
        }
    }
    Ok(())
}

fn materialize_callback_value(
    ty: &ContractTypeRef,
    schema: &PackageSchemaRecords,
    value: &RuntimeValue,
    source_heap: &RequestHeap,
    destination_heap: &mut RequestHeap,
    owner: BoundaryValueOwner,
) -> Result<RuntimeValue> {
    let value_plan = BoundaryValuePlan::Linkable {
        carrier: BoundaryValueCarrier::DetachedValueGraph,
        encoding: BoundaryValueEncoding::CanonicalValue,
        owner,
        lifetime: BoundaryValueLifetime::Call,
    };
    let plan = ServiceLinkableContractPlan::new(ty, schema, &value_plan).map_err(|error| {
        RuntimeError::Protocol {
            target: "in-process callback capability".to_string(),
            message: error.to_string(),
        }
    })?;
    plan.materialize(
        value,
        source_heap,
        destination_heap,
        ServiceLinkableMaterializationScope {
            owner,
            lifetime: BoundaryValueLifetime::Call,
        },
        &FailClosedServiceLinkableCapabilityHooks,
    )
    .map_err(|error| RuntimeError::Protocol {
        target: "in-process callback capability".to_string(),
        message: error.to_string(),
    })
}

#[cfg(test)]
mod tests;
