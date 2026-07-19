use skiff_artifact_model::ServiceDeploymentRef;
use skiff_runtime_linked_program::SharedPackageLinkedImage;
use skiff_runtime_linker::{AssemblyLinkedCandidate, LinkedActivationTemplate};

/// Immutable Phase 03 handoff for one activation in a linked runtime assembly.
///
/// This view deliberately contains no request-owned state. Phase 04 is responsible for turning
/// the template into an `ActivationContext` and propagating that owner through execution.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeAssemblyActivationTemplate<'a> {
    image: &'a SharedPackageLinkedImage,
    template: &'a LinkedActivationTemplate,
}

impl<'a> RuntimeAssemblyActivationTemplate<'a> {
    pub fn from_candidate(
        candidate: &'a AssemblyLinkedCandidate,
        deployment: &ServiceDeploymentRef,
    ) -> Result<Self, RuntimeAssemblyActivationSeamError> {
        let template = candidate.activation(deployment).ok_or_else(|| {
            RuntimeAssemblyActivationSeamError::MissingActivation {
                deployment: deployment.clone(),
            }
        })?;
        Ok(Self {
            image: candidate.shared_image().as_ref(),
            template,
        })
    }

    pub fn image(&self) -> &'a SharedPackageLinkedImage {
        self.image
    }

    pub fn template(&self) -> &'a LinkedActivationTemplate {
        self.template
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeAssemblyActivationSeamError {
    MissingActivation { deployment: ServiceDeploymentRef },
}

impl std::fmt::Display for RuntimeAssemblyActivationSeamError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingActivation { deployment } => write!(
                formatter,
                "linked runtime assembly has no activation for {deployment:?}"
            ),
        }
    }
}

impl std::error::Error for RuntimeAssemblyActivationSeamError {}
