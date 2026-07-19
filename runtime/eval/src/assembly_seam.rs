use skiff_artifact_model::ServiceDeploymentRef;
use skiff_runtime_activation::RuntimeAssemblyActivationTemplate;
use skiff_runtime_linked_type_plan::{
    RuntimeAssemblyTypePlanSeamError, RuntimeAssemblyTypePlanTarget,
};

/// Immutable assembly input pinned for eval without constructing a service-specific program.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeAssemblyEvalTarget<'a> {
    activation: RuntimeAssemblyActivationTemplate<'a>,
    type_plan: RuntimeAssemblyTypePlanTarget<'a>,
}

impl<'a> RuntimeAssemblyEvalTarget<'a> {
    pub fn new(
        activation: RuntimeAssemblyActivationTemplate<'a>,
    ) -> Result<Self, RuntimeAssemblyEvalSeamError> {
        let type_plan = RuntimeAssemblyTypePlanTarget::from_shared_image(
            activation.image(),
            activation.template().implementation_package_build_id(),
        )?;
        Ok(Self {
            activation,
            type_plan,
        })
    }

    pub fn activation(&self) -> RuntimeAssemblyActivationTemplate<'a> {
        self.activation
    }

    pub fn type_plan(&self) -> RuntimeAssemblyTypePlanTarget<'a> {
        self.type_plan
    }

    /// Phase 03 terminal execution seam.
    ///
    /// Eval must not project this target into the legacy [`crate::EvalRuntimeProgram`] because
    /// doing so would erase the activation-relative service binding vector. Phase 04 replaces
    /// this error with construction and propagation of the activation owner.
    pub fn ensure_execution_ready(&self) -> Result<(), RuntimeAssemblyEvalSeamError> {
        Err(
            RuntimeAssemblyEvalSeamError::ActivationContextExecutionUnavailable {
                deployment: self.activation.template().deployment_ref().clone(),
            },
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeAssemblyEvalSeamError {
    #[error(transparent)]
    TypePlan(#[from] RuntimeAssemblyTypePlanSeamError),
    #[error(
        "runtime assembly activation {deployment:?} cannot enter eval before Phase 04 ActivationContext execution"
    )]
    ActivationContextExecutionUnavailable { deployment: ServiceDeploymentRef },
}
