use skiff_artifact_model::{
    BoundaryOperationDescriptor, BoundaryValueCarrier, BoundaryValueLifetime, BoundaryValueOwner,
    BoundaryValuePlan, ContractTypeRef,
};
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_boundary::service_linkable::{
    ServiceLinkableCapabilityHooks, ServiceLinkableContractPlan,
    ServiceLinkableMaterializationError, ServiceLinkableMaterializationScope,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};

use crate::error::{Result, RuntimeError};

/// Canonical, lane-neutral materialization plan for one resolved service operation.
///
/// Lanes retain scheduling, cancellation, stream and callback orchestration. This owner performs
/// the common descriptor/schema preflight and every caller/provider heap transition.
pub(crate) struct CanonicalServiceBoundaryPlan<'a> {
    operation: &'a BoundaryOperationDescriptor,
    parameter_plans: Vec<DirectionalMaterializationPlan<'a>>,
    return_plan: DirectionalMaterializationPlan<'a>,
}

impl<'a> CanonicalServiceBoundaryPlan<'a> {
    pub(crate) fn new(
        operation: &'a BoundaryOperationDescriptor,
        schema: &'a PackageSchemaRecords,
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
        Ok(Self {
            operation,
            parameter_plans,
            return_plan,
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

    /// Detaches a successful result or forwards an already-fixed failure.
    ///
    /// A provider lane must export its actual error while the provider heap is
    /// alive. Reaching this shared boundary with any other error is an
    /// execution invariant violation, never a legacy error pass-through.
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
            Err(RuntimeError::FixedServiceFailure(error)) => {
                Err(RuntimeError::FixedServiceFailure(error))
            }
            Err(_) => Err(RuntimeError::InvalidArtifact(format!(
                "canonical service operation {} returned an unfixed provider failure",
                self.operation.operation_id
            ))),
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
        schema: &'a PackageSchemaRecords,
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
