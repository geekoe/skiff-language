use std::sync::Arc;

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::fixture::{build_capability, BuildOutcome, Capability, PublishedFixture};

pub fn published_positive(capability: Capability, prefix: &str) -> PublishedFixture {
    match build_capability(capability, prefix) {
        BuildOutcome::Published(fixture) => fixture,
        BuildOutcome::Rejected { error_chain, .. } => panic!(
            "production Phase 7 {capability:?} source did not reach the executable carrier: {error_chain}"
        ),
    }
}

pub fn admitted_artifact(capability: Capability, prefix: &str) -> Arc<ValidatedBytecodeArtifact> {
    let fixture = published_positive(capability, prefix);
    fixture.bytecode()
}

pub(super) fn linked_image(
    capability: Capability,
    prefix: &str,
) -> Arc<skiff_runtime_linker::DeploymentExecutionImage> {
    let fixture = published_positive(capability, prefix);
    fixture.link()
}

pub(super) fn link_input(capability: Capability, prefix: &str) -> HydratedDeploymentBytecode {
    let fixture = published_positive(capability, prefix);
    fixture.link_input()
}

/// Disabled capability negative: the only reachable outcome is the compiler /
/// admission rejection that publishes no package or release pointer.
pub fn assert_capability_rejected(capability: Capability, prefix: &str) {
    match build_capability(capability, prefix) {
        BuildOutcome::Rejected {
            package_pointer_absent,
            release_pointer_absent,
            ..
        } => {
            assert!(
                package_pointer_absent,
                "disabled Phase 7 {capability:?} surface wrote a package pointer"
            );
            assert!(
                release_pointer_absent,
                "disabled Phase 7 {capability:?} surface wrote a release pointer"
            );
        }
        BuildOutcome::Published(_) => {
            panic!("disabled Phase 7 {capability:?} surface published an executable image")
        }
    }
}
