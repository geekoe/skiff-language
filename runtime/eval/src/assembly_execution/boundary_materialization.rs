use skiff_artifact_model::{
    BoundaryErrorContract, BoundaryOperationDescriptor, BoundaryValueCarrier,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef,
};
use skiff_runtime_boundary::service_linkable::{
    ServiceLinkableCapabilityHooks, ServiceLinkableContractPlan,
    ServiceLinkableMaterializationError, ServiceLinkableMaterializationScope,
};
use skiff_runtime_boundary::service_schema_records::ServiceSchemaRecords;
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};

use crate::{
    error::{replace_user_exception_preserving_diagnostics, Result, RuntimeError, UserException},
    exceptions::user_exception_for_catch,
    runtime_ops::{runtime_from_wire, runtime_to_wire},
};

/// Canonical, lane-neutral materialization plan for one resolved service operation.
///
/// Lanes retain scheduling, cancellation, stream and callback orchestration. This owner performs
/// the common descriptor/schema preflight and every caller/provider heap transition.
pub(crate) struct CanonicalServiceBoundaryPlan<'a> {
    operation: &'a BoundaryOperationDescriptor,
    parameter_plans: Vec<DirectionalMaterializationPlan<'a>>,
    return_plan: DirectionalMaterializationPlan<'a>,
    error_plan: Option<DirectionalMaterializationPlan<'a>>,
}

impl<'a> CanonicalServiceBoundaryPlan<'a> {
    pub(crate) fn new(
        operation: &'a BoundaryOperationDescriptor,
        schema: &'a ServiceSchemaRecords,
        arg_count: usize,
    ) -> Result<Self> {
        preflight_boundary_contract(operation, arg_count)?;
        let parameter_plans = operation
            .contract
            .parameters
            .iter()
            .map(|parameter| {
                DirectionalMaterializationPlan::new(
                    &parameter.ty,
                    schema,
                    &parameter.value_plan,
                    BoundaryValueOwner::Caller,
                    operation.operation_id.as_str(),
                    "parameter",
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let return_plan = DirectionalMaterializationPlan::new(
            &operation.contract.return_value.ty,
            schema,
            &operation.contract.return_value.value_plan,
            BoundaryValueOwner::Provider,
            operation.operation_id.as_str(),
            "return",
        )?;
        let error_plan = match &operation.contract.errors {
            BoundaryErrorContract::None => None,
            BoundaryErrorContract::Typed {
                payload_type,
                value_plan,
            } => Some(DirectionalMaterializationPlan::new(
                payload_type,
                schema,
                value_plan,
                BoundaryValueOwner::Provider,
                operation.operation_id.as_str(),
                "typed error",
            )?),
            BoundaryErrorContract::Unsupported { reason } => {
                return Err(RuntimeError::Unsupported(format!(
                    "canonical service operation {} has unsupported error semantics: {reason:?}",
                    operation.operation_id
                )));
            }
        };
        Ok(Self {
            operation,
            parameter_plans,
            return_plan,
            error_plan,
        })
    }

    pub(crate) fn fresh_provider_heap(&self, limits: RequestHeapLimits) -> RequestHeap {
        RequestHeap::new(limits)
    }

    pub(crate) fn materialize_parameters(
        &self,
        args: &[RuntimeValue],
        caller_heap: &RequestHeap,
        provider_heap: &mut RequestHeap,
        caller_hooks: &dyn ServiceLinkableCapabilityHooks,
    ) -> Result<Vec<RuntimeValue>> {
        self.parameter_plans
            .iter()
            .zip(args)
            .map(|(plan, value)| plan.materialize(value, caller_heap, provider_heap, caller_hooks))
            .collect()
    }

    pub(crate) fn materialize_provider_result(
        &self,
        result: Result<RuntimeValue>,
        provider_heap: &mut RequestHeap,
        caller_heap: &mut RequestHeap,
        provider_hooks: &dyn ServiceLinkableCapabilityHooks,
    ) -> Result<RuntimeValue> {
        match result {
            Ok(value) => {
                self.materialize_success(&value, provider_heap, caller_heap, provider_hooks)
            }
            Err(error) => {
                self.materialize_provider_error(error, provider_heap, caller_heap, provider_hooks)
            }
        }
    }

    /// Detaches a successful provider value into the caller heap using the declared return plan.
    pub(crate) fn materialize_success(
        &self,
        value: &RuntimeValue,
        provider_heap: &RequestHeap,
        caller_heap: &mut RequestHeap,
        provider_hooks: &dyn ServiceLinkableCapabilityHooks,
    ) -> Result<RuntimeValue> {
        self.return_plan
            .materialize(value, provider_heap, caller_heap, provider_hooks)
    }

    /// Classifies a provider failure and detaches only a contract-declared typed payload.
    ///
    /// Runtime failures retain their original class, undeclared typed throws become protocol
    /// failures, and declared payload mismatches are reported by the same directional planner.
    pub(crate) fn materialize_provider_error(
        &self,
        error: RuntimeError,
        provider_heap: &mut RequestHeap,
        caller_heap: &mut RequestHeap,
        provider_hooks: &dyn ServiceLinkableCapabilityHooks,
    ) -> Result<RuntimeValue> {
        let Some(exception) = user_exception_for_catch(&error).cloned() else {
            return Err(error);
        };
        let Some(error_plan) = &self.error_plan else {
            return Err(RuntimeError::Protocol {
                target: self.operation.operation_id.to_string(),
                message:
                    "provider threw a typed business error but the contract declares no typed error"
                        .to_string(),
            });
        };
        let mut envelope = exception.envelope();
        let payload = envelope
            .as_object()
            .and_then(|object| object.get("error"))
            .cloned()
            .ok_or_else(|| RuntimeError::Protocol {
                target: self.operation.operation_id.to_string(),
                message: "provider typed error has no payload".to_string(),
            })?;
        let provider_value = runtime_from_wire(&payload, provider_heap).map_err(|error| {
            self.protocol_error(format!("typed error payload decode failed: {error}"))
        })?;
        let caller_value =
            error_plan.materialize(&provider_value, provider_heap, caller_heap, provider_hooks)?;
        let detached_payload = runtime_to_wire(&caller_value, caller_heap).map_err(|error| {
            self.protocol_error(format!("typed error payload encode failed: {error}"))
        })?;
        envelope
            .as_object_mut()
            .expect("validated user exception envelope is an object")
            .insert("error".to_string(), detached_payload);
        let detached_exception =
            UserException::from_runtime_parts(exception.actual_payload_type().clone(), envelope);
        Err(replace_user_exception_preserving_diagnostics(
            error,
            detached_exception,
        ))
    }

    fn protocol_error(&self, message: impl Into<String>) -> RuntimeError {
        RuntimeError::Protocol {
            target: self.operation.operation_id.to_string(),
            message: message.into(),
        }
    }
}

struct DirectionalMaterializationPlan<'a> {
    operation: String,
    role: &'static str,
    plan: ServiceLinkableContractPlan<'a>,
    scope: ServiceLinkableMaterializationScope,
}

impl<'a> DirectionalMaterializationPlan<'a> {
    fn new(
        ty: &'a ContractTypeRef,
        schema: &'a ServiceSchemaRecords,
        value_plan: &'a BoundaryValuePlan,
        detached_owner: BoundaryValueOwner,
        operation: &str,
        role: &'static str,
    ) -> Result<Self> {
        let plan = ServiceLinkableContractPlan::new(ty, schema, value_plan)
            .map_err(|error| invalid_materialization_plan(operation, role, error))?;
        let scope = directional_scope(value_plan, detached_owner).map_err(|message| {
            RuntimeError::InvalidArtifact(format!(
                "canonical service operation {operation} {role} value plan is invalid: {message}"
            ))
        })?;
        Ok(Self {
            operation: operation.to_string(),
            role,
            plan,
            scope,
        })
    }

    fn materialize(
        &self,
        value: &RuntimeValue,
        source_heap: &RequestHeap,
        destination_heap: &mut RequestHeap,
        hooks: &dyn ServiceLinkableCapabilityHooks,
    ) -> Result<RuntimeValue> {
        self.plan
            .materialize(value, source_heap, destination_heap, self.scope, hooks)
            .map_err(|error| RuntimeError::Protocol {
                target: self.operation.clone(),
                message: format!("{} materialization failed: {error}", self.role),
            })
    }
}

fn preflight_boundary_contract(
    operation: &BoundaryOperationDescriptor,
    arg_count: usize,
) -> Result<()> {
    if arg_count != operation.contract.parameters.len() {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical service operation {} expected {} parameters, got {arg_count}",
            operation.operation_id,
            operation.contract.parameters.len()
        )));
    }
    let guarantee = &operation.contract.effect_guarantee;
    if !(guarantee.detached_parameters
        && guarantee.detached_return
        && guarantee.detached_error
        && guarantee.no_caller_reachable_mutation
        && guarantee.no_caller_value_escape
        && guarantee.no_same_heap_identity)
    {
        return Err(RuntimeError::InvalidArtifact(format!(
            "canonical service operation {} lacks detached in-process boundary guarantees",
            operation.operation_id
        )));
    }
    Ok(())
}

fn directional_scope(
    plan: &BoundaryValuePlan,
    detached_owner: BoundaryValueOwner,
) -> std::result::Result<ServiceLinkableMaterializationScope, &'static str> {
    match plan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            owner,
            lifetime: BoundaryValueLifetime::Call,
            ..
        } if *owner == detached_owner => Ok(ServiceLinkableMaterializationScope {
            owner: detached_owner,
            lifetime: BoundaryValueLifetime::Call,
        }),
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Request,
            ..
        } => Ok(ServiceLinkableMaterializationScope {
            owner: BoundaryValueOwner::CapabilityOwner,
            lifetime: BoundaryValueLifetime::Request,
        }),
        BoundaryValuePlan::Unsupported { .. } => Err("unsupported value plan"),
        _ => Err("owner or lifetime does not match the service boundary direction"),
    }
}

fn invalid_materialization_plan(
    operation: &str,
    role: &str,
    error: ServiceLinkableMaterializationError,
) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "canonical service operation {operation} {role} value plan is invalid: {error}"
    ))
}

#[cfg(test)]
mod tests;
