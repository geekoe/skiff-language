use skiff_artifact_model::{DeploymentArtifactIdentity, ServiceDeploymentRef};

/// Exact identity of the deployment that owns an execution image.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeploymentOwnerIdentity {
    deployment: ServiceDeploymentRef,
}

impl DeploymentOwnerIdentity {
    pub fn new(deployment: ServiceDeploymentRef) -> Self {
        Self { deployment }
    }

    pub fn deployment(&self) -> &ServiceDeploymentRef {
        &self.deployment
    }

    pub fn build_id(&self) -> &DeploymentArtifactIdentity {
        &self.deployment.deployment_artifact_identity
    }
}
