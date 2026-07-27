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

    fn project(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
        native: bool,
    ) -> std::result::Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError>
    {
        let target = self
            .context
            .runtime_assembly_target()
            .map_err(callback_materialization_error)?;
        let interface = interface_value(request.value, request.source_heap)?;
        let (callback_type, operations) =
            callback_contract(request.ty, request.package_schema_records)?;
        let adapter = if native {
            InProcessCallbackAdapter::from_registered_explicit_native_interface(
                request.ty,
                callback_type,
                operations,
                &interface,
                request.package_schema_records,
                request.source_heap,
            )
        } else {
            InProcessCallbackAdapter::from_local_interface(
                callback_type,
                &interface,
                operations,
                request.package_schema_records,
                request.source_heap,
            )
        }
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
        self.project(request, false)
    }

    fn project_native_adapter_capability(
        &self,
        request: ServiceLinkableCapabilityRequest<'_>,
    ) -> std::result::Result<ServiceLinkableCapabilityProjection, ServiceLinkableMaterializationError>
    {
        self.project(request, true)
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
    callback_contract_inner(ty, schema, &mut HashSet::new())
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
mod tests {
    use super::*;
    use skiff_artifact_model::{PackageSchemaCanonicalDescriptor, PackageSchemaTypeRecord};

    #[test]
    fn in_process_callback_resolves_only_declared_callback_contract_operations() {
        let callback_id = PackageSchemaTypeId::new("package-schema:callback");
        let callback_ty = ContractTypeRef::package_schema(
            "example.callback",
            "api.Callback",
            callback_id.clone(),
        );
        let operations = BTreeMap::from([(
            "invoke".to_string(),
            BoundaryCallbackOperation {
                parameters: Vec::new(),
                return_type: ContractTypeRef::builtin("bool"),
            },
        )]);
        let callback_record = Arc::new(PackageSchemaTypeRecord {
            package_id: "example.callback".to_string(),
            stable_schema_key: "api.Callback".to_string(),
            package_schema_type_id: callback_id.clone(),
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: ContractTypeDescriptor::CallbackInterface {
                    operations: operations.clone(),
                },
            },
        });
        let schema = BTreeMap::from([(callback_id, Arc::clone(&callback_record))]);
        let strong_count_before = Arc::strong_count(&callback_record);
        let (resolved_id, resolved_operations) = callback_contract(&callback_ty, &schema).unwrap();
        assert_eq!(resolved_id.package_id, "example.callback");
        assert_eq!(resolved_id.stable_schema_key, "api.Callback");
        assert_eq!(
            resolved_id.package_schema_type_id.as_str(),
            "package-schema:callback"
        );
        assert_eq!(resolved_operations, &operations);
        assert_eq!(Arc::strong_count(&callback_record), strong_count_before);
        assert!(matches!(
            callback_contract(&ContractTypeRef::builtin("string"), &schema),
            Err(ServiceLinkableMaterializationError::TypeMismatch)
        ));
    }

    #[test]
    fn in_process_callback_maps_wrong_tuple_to_stable_unavailable_error() {
        let error = callback_capability_error(CallbackCapabilityError::CapabilityUnavailable);
        assert!(matches!(
            error,
            RuntimeError::ProviderUnavailable { ref reason, .. }
                if reason == "CapabilityUnavailable"
        ));
    }

    #[test]
    fn callback_contract_rejects_owner_key_id_descriptor_and_missing_alias_record() {
        let type_id = PackageSchemaTypeId::new("schema:callback-validation");
        let ty =
            ContractTypeRef::package_schema("example.callback", "api.Callback", type_id.clone());
        let callback_record = || PackageSchemaTypeRecord {
            package_id: "example.callback".to_string(),
            stable_schema_key: "api.Callback".to_string(),
            package_schema_type_id: type_id.clone(),
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: ContractTypeDescriptor::CallbackInterface {
                    operations: BTreeMap::new(),
                },
            },
        };
        for mutate in [
            |record: &mut PackageSchemaTypeRecord| record.package_id.push_str(".wrong"),
            |record: &mut PackageSchemaTypeRecord| record.stable_schema_key.push_str(".wrong"),
            |record: &mut PackageSchemaTypeRecord| {
                record.package_schema_type_id = PackageSchemaTypeId::new("wrong")
            },
        ] as [fn(&mut PackageSchemaTypeRecord); 3]
        {
            let mut record = callback_record();
            mutate(&mut record);
            assert!(
                callback_contract(&ty, &BTreeMap::from([(type_id.clone(), Arc::new(record))]))
                    .is_err()
            );
        }

        let mut non_callback = callback_record();
        non_callback.canonical_descriptor.descriptor = ContractTypeDescriptor::Enumeration {
            variants: vec!["value".to_string()],
        };
        assert!(matches!(
            callback_contract(
                &ty,
                &BTreeMap::from([(type_id.clone(), Arc::new(non_callback))])
            ),
            Err(ServiceLinkableMaterializationError::TypeMismatch)
        ));

        let child_id = PackageSchemaTypeId::new("schema:missing-callback");
        let alias = PackageSchemaTypeRecord {
            package_id: "example.callback".to_string(),
            stable_schema_key: "api.Callback".to_string(),
            package_schema_type_id: type_id.clone(),
            canonical_descriptor: PackageSchemaCanonicalDescriptor {
                type_params: Vec::new(),
                descriptor: ContractTypeDescriptor::Alias {
                    target: ContractTypeRef::package_schema(
                        "example.callback",
                        "api.MissingCallback",
                        child_id,
                    ),
                },
            },
        };
        assert!(matches!(
            callback_contract(&ty, &BTreeMap::from([(type_id, Arc::new(alias))])),
            Err(ServiceLinkableMaterializationError::MissingSchema { .. })
        ));
    }
}
