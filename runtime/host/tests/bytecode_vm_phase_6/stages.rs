use std::sync::Arc;

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_runtime_loader::HydratedDeploymentBytecode;

use super::{
    fixture::{
        build_capability, build_interface_local_named, BuildOutcome, Capability, PublishedFixture,
    },
    host_chain,
};

pub fn published_positive(capability: Capability, prefix: &str) -> PublishedFixture {
    match build_capability(capability, false, prefix) {
        BuildOutcome::Published(fixture) => fixture,
        BuildOutcome::Rejected { error_chain, .. } => panic!(
            "production Phase 6 {capability:?} source did not reach the executable carrier: {error_chain}"
        ),
    }
}

pub fn published_interface_local_named(
    directory: &str,
    package_id: &str,
    prefix: &str,
) -> PublishedFixture {
    match build_interface_local_named(directory, package_id, prefix) {
        BuildOutcome::Published(fixture) => fixture,
        BuildOutcome::Rejected { error_chain, .. } => panic!(
            "production Phase 6 interface-local {directory} source did not reach the executable carrier: {error_chain}"
        ),
    }
}

pub fn assert_interface_local_named_rejected(directory: &str, package_id: &str, prefix: &str) {
    match build_interface_local_named(directory, package_id, prefix) {
        BuildOutcome::Rejected {
            package_pointer_absent,
            release_pointer_absent,
            ..
        } => {
            assert!(
                package_pointer_absent,
                "{directory} wrote a package pointer"
            );
            assert!(
                release_pointer_absent,
                "{directory} wrote a release pointer"
            );
        }
        BuildOutcome::Published(_) => {
            panic!("disabled interface-local {directory} source published an executable image")
        }
    }
}

pub fn assert_callback_negative_rejected(prefix: &str) {
    match build_capability(Capability::Callback, true, prefix) {
        BuildOutcome::Rejected {
            package_pointer_absent,
            release_pointer_absent,
            ..
        } => {
            assert!(
                package_pointer_absent,
                "disabled callback surface wrote a package pointer"
            );
            assert!(
                release_pointer_absent,
                "disabled callback surface wrote a release pointer"
            );
        }
        BuildOutcome::Published(_) => {
            panic!("disabled callback surface published an executable image")
        }
    }
}

pub fn assert_interface_remote_negative_rejected(prefix: &str) {
    match build_capability(Capability::InterfaceRemote, true, prefix) {
        BuildOutcome::Rejected {
            package_pointer_absent,
            release_pointer_absent,
            ..
        } => {
            assert!(
                package_pointer_absent,
                "disabled remote interface source wrote a package pointer"
            );
            assert!(
                release_pointer_absent,
                "disabled remote interface source wrote a release pointer"
            );
        }
        BuildOutcome::Published(_) => {
            panic!("disabled remote interface source published an executable image")
        }
    }
}

pub fn admitted_artifact(capability: Capability, prefix: &str) -> Arc<ValidatedBytecodeArtifact> {
    let fixture = published_positive(capability, prefix);
    fixture.bytecode()
}

pub(super) fn linked_image(
    capability: Capability,
    prefix: &str,
) -> std::sync::Arc<skiff_runtime_linker::DeploymentExecutionImage> {
    let fixture = published_positive(capability, prefix);
    fixture.link()
}

pub(super) fn link_input(capability: Capability, prefix: &str) -> HydratedDeploymentBytecode {
    let fixture = published_positive(capability, prefix);
    fixture.link_input()
}

pub async fn scheduler_to_request(capability: Capability, prefix: &str) {
    host_chain::scheduler_to_request(capability, prefix).await;
}

pub async fn request_to_terminal(capability: Capability, prefix: &str) {
    host_chain::request_to_terminal(capability, prefix).await;
}

pub fn assert_containment_rejected(prefix: &str) {
    for (capability, negative) in [
        (Capability::Containment, false),
        (Capability::Containment, true),
    ] {
        match build_capability(capability, negative, prefix) {
            BuildOutcome::Rejected {
                package_pointer_absent,
                release_pointer_absent,
                ..
            } => {
                assert!(package_pointer_absent, "disabled surface wrote a package pointer");
                assert!(release_pointer_absent, "disabled surface wrote a release pointer");
            }
            BuildOutcome::Published(_) => panic!(
                "disabled containment surface {capability:?}/{negative} published an executable image"
            ),
        }
    }
}
