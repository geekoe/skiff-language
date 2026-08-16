//! Exact immutable activation identity projected from release-resolved
//! deployment facts for Router request construction.

use sha2::{Digest, Sha256};
use skiff_artifact_model::{
    AssemblyIdentity, ServiceDeploymentRef, RUNTIME_ASSEMBLY_IDENTITY_PREFIX,
};

/// The Phase 6 router activation generation. This is the current immutable
/// deployment-generation wire value; it is not inferred from mutable session
/// connection generations.
pub(crate) const ACTIVATION_GENERATION: u64 = 1;

/// Projects the exact activation identity tuple for one release-resolved
/// deployment. The deployment artifact identity is the immutable release fact
/// consumed by the Router and Runtime lazy loader; the assembly identity is
/// derived from that exact build and never from display text or a hand-built
/// fixture value.
pub(crate) fn activation_identity_for_deployment(
    deployment: &ServiceDeploymentRef,
) -> Option<(AssemblyIdentity, u64)> {
    let build = deployment.deployment_artifact_identity.as_str();
    if build.is_empty() || deployment.deployment_revision.as_str().is_empty() {
        return None;
    }
    let digest = Sha256::digest(build);
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let identity = AssemblyIdentity::new(format!("{RUNTIME_ASSEMBLY_IDENTITY_PREFIX}:{hex}"));
    Some((identity, ACTIVATION_GENERATION))
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::{
        DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef,
    };

    use super::{activation_identity_for_deployment, ACTIVATION_GENERATION};

    fn deployment(build: &str, revision: &str) -> ServiceDeploymentRef {
        ServiceDeploymentRef {
            service_id: "example.com/service".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: DeploymentRevision::new(revision),
            deployment_artifact_identity: DeploymentArtifactIdentity::new(build),
        }
    }

    #[test]
    fn activation_identity_is_exact_for_deployment_build() {
        let build = "skiff-deployment-artifact-v4:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let (identity, generation) =
            activation_identity_for_deployment(&deployment(build, "revision-1"))
                .expect("release-resolved deployment must project exact activation facts");
        assert!(identity
            .as_str()
            .starts_with("skiff-runtime-assembly-v3:sha256:"));
        assert_eq!(
            identity.as_str().len(),
            "skiff-runtime-assembly-v3:sha256:".len() + 64
        );
        assert_eq!(generation, ACTIVATION_GENERATION);
    }

    #[test]
    fn activation_identity_missing_deployment_facts_fails_closed() {
        assert!(
            activation_identity_for_deployment(&deployment("", "revision-1")).is_none(),
            "empty deployment artifact identity must fail closed"
        );
        assert!(
            activation_identity_for_deployment(&deployment("skiff-deployment-artifact-v4:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", ""))
                .is_none(),
            "empty deployment revision must fail closed"
        );
    }
}
