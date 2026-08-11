use std::{collections::BTreeMap, fmt, sync::Arc};

use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractOperationId, OperationTargetRef, PackageSchemaTypeId,
    PackageSchemaTypeRecord, ServiceContract, ServiceContractRef,
};
use skiff_runtime_activation::{
    ActivationContext, ActivationContextError, ActivationId, RequestActivationContext,
};
use skiff_runtime_linked_program::{
    ActivationRelativeServiceCall, AssemblyExecutionImage, ConstAddr, DbObjectTargetId,
    ExecutableAddr, LinkedPackageCallableTarget, LinkedPackageDirectCall,
};
use skiff_runtime_linked_type_plan::{
    RuntimeAssemblyTypePlanSeamError, RuntimeAssemblyTypePlanTarget,
};

use crate::assembly_execution::RuntimeAssemblyExecutionProjection;

/// Eval-owned, loader-independent view of one already-admitted contract schema closure.
///
/// Both the map and every record payload remain immutable and shared. Execution paths never
/// receive an artifact resolver or reconstruct admission facts.
pub type AdmittedPackageSchemaRecords =
    Arc<BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>>;

/// One activation-time `db contract` to host `db object` binding.
///
/// Built by host admission, shared immutably with eval. `implementer` names the host collection
/// target every contract db target resolves to; `contract_view` marks the engine side of the
/// shared collection so the runtime can reject whole-document writes.
#[derive(Debug, Clone)]
pub struct DbContractBinding {
    pub contract: DbObjectTargetId,
    pub implementer: DbObjectTargetId,
    pub contract_view: bool,
}

impl DbContractBinding {
    pub fn new(contract: DbObjectTargetId, implementer: DbObjectTargetId) -> Self {
        Self {
            contract,
            implementer,
            contract_view: true,
        }
    }
}

/// Host-owned lookup surface needed after an activation-relative service instruction is decoded.
///
/// The resolver returns only typed, already-admitted assembly facts. It cannot load artifacts,
/// resolve display names, select a remote runtime, or manufacture a legacy program.
pub trait RuntimeAssemblyEvalResolver: Send + Sync {
    fn activation(&self, activation_id: &ActivationId) -> Option<Arc<ActivationContext>>;

    /// Resolves the opaque activation id carried by a callback capability without parsing it
    /// into a display name or consulting a route registry.
    fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>>;

    fn contract(&self, contract: &ServiceContractRef) -> Option<Arc<ServiceContract>>;

    fn admitted_schema_records(
        &self,
        contract: &ServiceContractRef,
    ) -> Option<AdmittedPackageSchemaRecords>;

    fn operation_target(
        &self,
        activation_id: &ActivationId,
        operation: &ContractOperationId,
    ) -> Option<OperationTargetRef>;

    /// Resolves the host implementation binding for one `db contract` target. Returns `None`
    /// for plain `db object` targets. Legacy eval resolvers (no assembly contracts) keep the
    /// default `None`.
    fn db_contract_binding(
        &self,
        _contract_target: &DbObjectTargetId,
    ) -> Option<Arc<DbContractBinding>> {
        None
    }
}

/// Runtime-ready eval input pinned to one immutable execution image and one request generation.
///
/// There is deliberately no conversion to `EvalRuntimeProgram`: canonical package and service
/// calls retain their distinct linked forms all the way into the lane hooks.
#[derive(Clone)]
pub struct RuntimeAssemblyEvalTarget {
    execution_image: Arc<AssemblyExecutionImage>,
    execution_projection: RuntimeAssemblyExecutionProjection,
    request_activation: RequestActivationContext,
    resolver: Arc<dyn RuntimeAssemblyEvalResolver>,
}

impl fmt::Debug for RuntimeAssemblyEvalTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeAssemblyEvalTarget")
            .field(
                "assembly_identity",
                self.execution_image.assembly_identity(),
            )
            .field(
                "current_activation_id",
                self.request_activation.current().activation_id(),
            )
            .field("request_generation", &self.request_activation.generation())
            .finish_non_exhaustive()
    }
}

impl RuntimeAssemblyEvalTarget {
    pub fn new(
        execution_image: Arc<AssemblyExecutionImage>,
        request_activation: RequestActivationContext,
        resolver: Arc<dyn RuntimeAssemblyEvalResolver>,
    ) -> Result<Self, RuntimeAssemblyEvalSeamError> {
        Self::validate_request_activation(&execution_image, &request_activation, &resolver)?;
        let execution_projection = RuntimeAssemblyExecutionProjection::from_image(
            Arc::clone(&execution_image),
        )
        .with_db_contract_binding_source(Arc::clone(&resolver));
        Ok(Self {
            execution_image,
            execution_projection,
            request_activation,
            resolver,
        })
    }

    fn validate_request_activation(
        execution_image: &AssemblyExecutionImage,
        request_activation: &RequestActivationContext,
        resolver: &Arc<dyn RuntimeAssemblyEvalResolver>,
    ) -> Result<(), RuntimeAssemblyEvalSeamError> {
        let current = request_activation.current();
        if execution_image.assembly_identity() != &current.identity().assembly_identity {
            return Err(RuntimeAssemblyEvalSeamError::AssemblyIdentityMismatch);
        }
        let resolved_current = resolver
            .activation(current.activation_id())
            .ok_or_else(|| RuntimeAssemblyEvalSeamError::MissingActivationOwner {
                activation_id: current.activation_id().as_str().to_string(),
            })?;
        if !Arc::ptr_eq(current, &resolved_current) {
            return Err(RuntimeAssemblyEvalSeamError::ActivationOwnerMismatch {
                activation_id: current.activation_id().as_str().to_string(),
            });
        }
        RuntimeAssemblyTypePlanTarget::from_execution_image(
            execution_image,
            current.implementation_package_build_id(),
        )?;
        Ok(())
    }

    pub fn execution_image(&self) -> &Arc<AssemblyExecutionImage> {
        &self.execution_image
    }

    pub(crate) fn execution_projection(&self) -> &RuntimeAssemblyExecutionProjection {
        &self.execution_projection
    }

    pub fn request_activation(&self) -> &RequestActivationContext {
        &self.request_activation
    }

    pub fn activation_context(&self) -> &Arc<ActivationContext> {
        self.request_activation.current()
    }

    pub fn type_plan(
        &self,
    ) -> Result<RuntimeAssemblyTypePlanTarget<'_>, RuntimeAssemblyEvalSeamError> {
        Ok(RuntimeAssemblyTypePlanTarget::from_execution_image(
            &self.execution_image,
            self.activation_context().implementation_package_build_id(),
        )?)
    }

    pub fn ensure_execution_ready(&self) -> Result<(), RuntimeAssemblyEvalSeamError> {
        self.type_plan()?;
        Ok(())
    }

    pub fn with_request_activation(
        &self,
        request_activation: RequestActivationContext,
    ) -> Result<Self, RuntimeAssemblyEvalSeamError> {
        Self::validate_request_activation(
            &self.execution_image,
            &request_activation,
            &self.resolver,
        )?;
        Ok(Self {
            execution_image: Arc::clone(&self.execution_image),
            execution_projection: self.execution_projection.clone(),
            request_activation,
            resolver: Arc::clone(&self.resolver),
        })
    }

    pub fn activation_by_opaque_id(&self, activation_id: &str) -> Option<Arc<ActivationContext>> {
        self.resolver.activation_by_opaque_id(activation_id)
    }

    pub fn ensure_package_direct_target(
        &self,
        call: &LinkedPackageDirectCall,
    ) -> Result<(), RuntimeAssemblyEvalSeamError> {
        self.execution_image
            .executable_at(call.executable_addr())
            .map_err(
                |error| RuntimeAssemblyEvalSeamError::InvalidPackageDirectTarget {
                    detail: error.to_string(),
                },
            )?;
        Ok(())
    }

    #[cfg(any(test, feature = "legacy-eval"))]
    pub fn resolve_service_call(
        &self,
        instruction: &ActivationRelativeServiceCall,
    ) -> Result<RuntimeAssemblyServiceCallTarget, RuntimeAssemblyEvalSeamError> {
        let caller = self.request_activation.current();
        let binding = caller.resolve_service_binding(
            instruction.caller_package_build_id(),
            instruction.service_requirement_slot(),
            instruction.expected_protocol_identity(),
            instruction.operation_id(),
        )?;
        let provider = self
            .resolver
            .activation(binding.provider_activation_id())
            .ok_or_else(|| RuntimeAssemblyEvalSeamError::MissingProviderActivation {
                activation_id: binding.provider_activation_id().as_str().to_string(),
            })?;
        if provider.activation_id() != binding.provider_activation_id() {
            return Err(RuntimeAssemblyEvalSeamError::ProviderActivationMismatch {
                activation_id: binding.provider_activation_id().as_str().to_string(),
            });
        }
        if provider.identity().deployment.service_id != binding.contract().service_id
            || provider.identity().deployment.contract_version
                != binding.contract().contract_version
        {
            return Err(RuntimeAssemblyEvalSeamError::ProviderContractMismatch {
                activation_id: provider.activation_id().as_str().to_string(),
                contract: binding.contract().clone(),
            });
        }
        let provider_request = self.request_activation.switch_to(Arc::clone(&provider))?;
        let contract = self.resolver.contract(binding.contract()).ok_or_else(|| {
            RuntimeAssemblyEvalSeamError::MissingContract {
                contract: binding.contract().clone(),
            }
        })?;
        if contract.service_id != binding.contract().service_id
            || contract.contract_version != binding.contract().contract_version
            || contract.service_protocol_identity != binding.contract().service_protocol_identity
        {
            return Err(RuntimeAssemblyEvalSeamError::ContractIdentityMismatch {
                contract: binding.contract().clone(),
            });
        }
        let schema_records = self
            .resolver
            .admitted_schema_records(binding.contract())
            .ok_or_else(|| RuntimeAssemblyEvalSeamError::MissingAdmittedSchema {
                contract: binding.contract().clone(),
            })?;
        let descriptor = contract
            .operations
            .get(instruction.operation_id())
            .ok_or_else(|| RuntimeAssemblyEvalSeamError::MissingContractOperation {
                contract: binding.contract().clone(),
                operation: instruction.operation_id().clone(),
            })?;
        if descriptor.operation_id != *instruction.operation_id() {
            return Err(
                RuntimeAssemblyEvalSeamError::ContractOperationIdentityMismatch {
                    operation: instruction.operation_id().clone(),
                },
            );
        }
        let operation_target = self
            .resolver
            .operation_target(provider.activation_id(), instruction.operation_id())
            .ok_or_else(|| RuntimeAssemblyEvalSeamError::MissingProviderOperation {
                activation_id: provider.activation_id().as_str().to_string(),
                operation: instruction.operation_id().clone(),
            })?;
        let callable_target = self
            .execution_image
            .entry_callable_target(
                provider.implementation_package_build_id(),
                &operation_target,
            )
            .map_err(
                |error| RuntimeAssemblyEvalSeamError::InvalidProviderTarget {
                    activation_id: provider.activation_id().as_str().to_string(),
                    operation: instruction.operation_id().clone(),
                    detail: error.to_string(),
                },
            )?;
        Ok(RuntimeAssemblyServiceCallTarget {
            provider_request,
            contract,
            schema_records,
            operation: instruction.operation_id().clone(),
            callable_target,
        })
    }

    /// Adapts one already-pinned global ingress route into the same in-process boundary target
    /// consumed by activation-relative internal service calls.
    ///
    /// Host admission has already selected the activation, canonical contract and exact provider
    /// operation target in one generation. This adapter validates those typed facts without
    /// consulting the resolver again and never accepts build/display/ABI fallback identities.
    #[cfg(any(test, feature = "legacy-eval"))]
    pub fn resolve_ingress_target(
        &self,
        contract_ref: &ServiceContractRef,
        operation: &ContractOperationId,
        contract: Arc<ServiceContract>,
        operation_target: &OperationTargetRef,
    ) -> Result<RuntimeAssemblyServiceCallTarget, RuntimeAssemblyEvalSeamError> {
        let provider = self.request_activation.current();
        if !Arc::ptr_eq(provider, self.request_activation.receiver()) {
            return Err(RuntimeAssemblyEvalSeamError::IngressActivationOwnerMismatch);
        }
        let deployment = &provider.identity().deployment;
        if deployment.service_id != contract_ref.service_id
            || deployment.contract_version != contract_ref.contract_version
        {
            return Err(RuntimeAssemblyEvalSeamError::ProviderContractMismatch {
                activation_id: provider.activation_id().as_str().to_string(),
                contract: contract_ref.clone(),
            });
        }
        if contract.service_id != contract_ref.service_id
            || contract.contract_version != contract_ref.contract_version
            || contract.service_protocol_identity != contract_ref.service_protocol_identity
        {
            return Err(RuntimeAssemblyEvalSeamError::ContractIdentityMismatch {
                contract: contract_ref.clone(),
            });
        }
        let admitted_contract = self.resolver.contract(contract_ref).ok_or_else(|| {
            RuntimeAssemblyEvalSeamError::MissingContract {
                contract: contract_ref.clone(),
            }
        })?;
        if !Arc::ptr_eq(&contract, &admitted_contract) {
            return Err(RuntimeAssemblyEvalSeamError::ContractGenerationMismatch {
                contract: contract_ref.clone(),
            });
        }
        let schema_records = self
            .resolver
            .admitted_schema_records(contract_ref)
            .ok_or_else(|| RuntimeAssemblyEvalSeamError::MissingAdmittedSchema {
                contract: contract_ref.clone(),
            })?;
        let descriptor = contract.operations.get(operation).ok_or_else(|| {
            RuntimeAssemblyEvalSeamError::MissingContractOperation {
                contract: contract_ref.clone(),
                operation: operation.clone(),
            }
        })?;
        if descriptor.operation_id != *operation {
            return Err(
                RuntimeAssemblyEvalSeamError::ContractOperationIdentityMismatch {
                    operation: operation.clone(),
                },
            );
        }
        let callable_target = self
            .execution_image
            .entry_callable_target(provider.implementation_package_build_id(), operation_target)
            .map_err(
                |error| RuntimeAssemblyEvalSeamError::InvalidProviderTarget {
                    activation_id: provider.activation_id().as_str().to_string(),
                    operation: operation.clone(),
                    detail: error.to_string(),
                },
            )?;
        Ok(RuntimeAssemblyServiceCallTarget {
            provider_request: self.request_activation.clone(),
            contract,
            schema_records,
            operation: operation.clone(),
            callable_target,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeAssemblyServiceCallTarget {
    provider_request: RequestActivationContext,
    contract: Arc<ServiceContract>,
    schema_records: AdmittedPackageSchemaRecords,
    operation: ContractOperationId,
    callable_target: LinkedPackageCallableTarget,
}

impl RuntimeAssemblyServiceCallTarget {
    pub fn provider_request(&self) -> &RequestActivationContext {
        &self.provider_request
    }

    pub fn provider_activation(&self) -> &Arc<ActivationContext> {
        self.provider_request.current()
    }

    pub fn contract(&self) -> &Arc<ServiceContract> {
        &self.contract
    }

    pub fn schema_records(&self) -> &AdmittedPackageSchemaRecords {
        &self.schema_records
    }

    pub fn descriptor(&self) -> &BoundaryOperationDescriptor {
        self.contract
            .operations
            .get(&self.operation)
            .expect("service-call target is constructed from this contract operation")
    }

    pub fn executable_addr(&self) -> &ExecutableAddr {
        self.callable_target.executable_addr()
    }

    pub fn receiver_const(&self) -> Option<&ConstAddr> {
        self.callable_target.receiver_const()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeAssemblyEvalSeamError {
    #[error(transparent)]
    TypePlan(#[from] RuntimeAssemblyTypePlanSeamError),
    #[error(transparent)]
    Activation(#[from] ActivationContextError),
    #[error("assembly execution image and current activation have different assembly identities")]
    AssemblyIdentityMismatch,
    #[error("canonical ingress target must begin with its provider as receiver/current owner")]
    IngressActivationOwnerMismatch,
    #[error("runtime assembly resolver has no activation owner {activation_id}")]
    MissingActivationOwner { activation_id: String },
    #[error("runtime assembly resolver returned a different owner for activation {activation_id}")]
    ActivationOwnerMismatch { activation_id: String },
    #[error("runtime assembly package-direct target is invalid: {detail}")]
    InvalidPackageDirectTarget { detail: String },
    #[error("runtime assembly resolver has no provider activation {activation_id}")]
    MissingProviderActivation { activation_id: String },
    #[error("runtime assembly resolver returned the wrong provider activation {activation_id}")]
    ProviderActivationMismatch { activation_id: String },
    #[error("provider activation {activation_id} does not implement contract {contract:?}")]
    ProviderContractMismatch {
        activation_id: String,
        contract: ServiceContractRef,
    },
    #[error("runtime assembly resolver has no canonical contract {contract:?}")]
    MissingContract { contract: ServiceContractRef },
    #[error("runtime assembly resolver returned a mismatched canonical contract {contract:?}")]
    ContractIdentityMismatch { contract: ServiceContractRef },
    #[error("canonical contract {contract:?} belongs to a different admitted assembly generation")]
    ContractGenerationMismatch { contract: ServiceContractRef },
    #[error("runtime assembly resolver has no admitted Package schema for {contract:?}")]
    MissingAdmittedSchema { contract: ServiceContractRef },
    #[error("canonical contract {contract:?} has no operation {operation}")]
    MissingContractOperation {
        contract: ServiceContractRef,
        operation: ContractOperationId,
    },
    #[error("canonical contract operation {operation} has a mismatched embedded identity")]
    ContractOperationIdentityMismatch { operation: ContractOperationId },
    #[error("provider activation {activation_id} has no operation {operation}")]
    MissingProviderOperation {
        activation_id: String,
        operation: ContractOperationId,
    },
    #[error(
        "provider activation {activation_id} operation {operation} has an invalid target: {detail}"
    )]
    InvalidProviderTarget {
        activation_id: String,
        operation: ContractOperationId,
        detail: String,
    },
    #[error("program execution context has no runtime assembly target")]
    MissingExecutionTarget,
}
