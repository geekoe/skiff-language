use std::{fmt, sync::Arc};

use skiff_artifact_model::{
    BoundaryOperationDescriptor, ContractOperationId, OperationTargetRef, ServiceContract,
    ServiceContractRef,
};
use skiff_runtime_activation::{
    ActivationContext, ActivationContextError, ActivationId, RequestActivationContext,
};
use skiff_runtime_linked_program::{
    ActivationRelativeServiceCall, AssemblyExecutionImage, ExecutableAddr, LinkedPackageDirectCall,
};
use skiff_runtime_linked_type_plan::{
    RuntimeAssemblyTypePlanSeamError, RuntimeAssemblyTypePlanTarget,
};

use crate::assembly_execution::RuntimeAssemblyExecutionProjection;

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

    fn operation_target(
        &self,
        activation_id: &ActivationId,
        operation: &ContractOperationId,
    ) -> Option<OperationTargetRef>;
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
        let execution_projection =
            RuntimeAssemblyExecutionProjection::from_image(Arc::clone(&execution_image));
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
        let executable = self
            .execution_image
            .entry_executable(
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
            operation: instruction.operation_id().clone(),
            executable_addr: executable.addr().clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeAssemblyServiceCallTarget {
    provider_request: RequestActivationContext,
    contract: Arc<ServiceContract>,
    operation: ContractOperationId,
    executable_addr: ExecutableAddr,
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

    pub fn descriptor(&self) -> &BoundaryOperationDescriptor {
        self.contract
            .operations
            .get(&self.operation)
            .expect("service-call target is constructed from this contract operation")
    }

    pub fn executable_addr(&self) -> &ExecutableAddr {
        &self.executable_addr
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
