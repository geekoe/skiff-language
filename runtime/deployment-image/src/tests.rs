use std::sync::Arc;

use skiff_artifact_model::{
    ContractOperationId, DeploymentArtifactIdentity, DeploymentRevision, PackageBuildId,
    ServiceContractRef, ServiceDeploymentRef, ServiceProtocolIdentity, ServiceRequirementKey,
};

use crate::{
    DeploymentImage, DeploymentImageError, DeploymentLoadFailure, DeploymentLoadResult,
    DeploymentOwnerIdentity, LoadAttemptId, PinnedProviderImage, ServiceDependencySlot,
    ServiceDependencySlotError,
};

#[test]
fn owner_preserves_the_exact_deployment_build() {
    let deployment = deployment("build:consumer:exact");
    let owner = DeploymentOwnerIdentity::new(deployment.clone());

    assert_eq!(owner.deployment(), &deployment);
    assert_eq!(
        owner.build_id(),
        &DeploymentArtifactIdentity::new("build:consumer:exact")
    );
}

#[test]
fn dependency_slot_canonicalizes_operations() {
    let slot = ServiceDependencySlot::try_new(
        dependency_key(1),
        contract("payments"),
        ["operation:z", "operation:a", "operation:m"].map(ContractOperationId::new),
    )
    .expect("unique operations must construct a slot");

    let operations = slot
        .used_operations()
        .iter()
        .map(ContractOperationId::as_str)
        .collect::<Vec<_>>();
    assert_eq!(operations, ["operation:a", "operation:m", "operation:z"]);
}

#[test]
fn dependency_slot_rejects_duplicate_operations() {
    let error = ServiceDependencySlot::try_new(
        dependency_key(1),
        contract("payments"),
        ["operation:a", "operation:z", "operation:a"].map(ContractOperationId::new),
    )
    .expect_err("duplicate operations must be rejected");

    assert_eq!(
        error,
        ServiceDependencySlotError::DuplicateOperation {
            operation_id: ContractOperationId::new("operation:a"),
        }
    );
}

#[test]
fn deployment_image_rejects_duplicate_dependency_keys() {
    let first = dependency_slot(3, "payments");
    let duplicate = dependency_slot(3, "payments-v2");
    let error = DeploymentImage::try_new(
        DeploymentOwnerIdentity::new(deployment("build:consumer:exact")),
        Arc::new("verified-program"),
        [first, duplicate],
    )
    .expect_err("duplicate requirement keys must be rejected");

    assert_eq!(
        error,
        DeploymentImageError::DuplicateDependencyKey {
            key: dependency_key(3),
        }
    );
}

#[test]
fn provider_pin_keeps_the_same_image_arc() {
    let program = Arc::new("verified-program");
    let image = Arc::new(
        DeploymentImage::try_new(
            DeploymentOwnerIdentity::new(deployment("build:provider:exact")),
            Arc::clone(&program),
            [dependency_slot(2, "ledger")],
        )
        .expect("unique dependency slots must construct an image"),
    );
    let invocation_pin = PinnedProviderImage::new(Arc::clone(&image));
    let stream_pin = invocation_pin.clone();
    let callback_pin = invocation_pin.clone();

    assert!(Arc::ptr_eq(invocation_pin.image(), &image));
    assert!(Arc::ptr_eq(stream_pin.image(), callback_pin.image()));
    assert!(Arc::ptr_eq(invocation_pin.image().program(), &program));
    assert_eq!(
        invocation_pin.owner().build_id().as_str(),
        "build:provider:exact"
    );
}

#[test]
fn load_waiters_share_the_same_failure_arc() {
    let error = Arc::new("decode failed".to_string());
    let failure = Arc::new(DeploymentLoadFailure::new(
        LoadAttemptId::new(41),
        Arc::clone(&error),
    ));
    let first: DeploymentLoadResult<(), String> = Err(Arc::clone(&failure));
    let second: DeploymentLoadResult<(), String> = Err(Arc::clone(&failure));

    let first = first.expect_err("fixture is a failed attempt");
    let second = second.expect_err("fixture is a failed attempt");
    assert!(Arc::ptr_eq(&first, &second));
    assert!(Arc::ptr_eq(first.error(), &error));
    assert_eq!(first.attempt_id().get(), 41);
}

fn deployment(build_id: &str) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "consumer".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("revision:consumer"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(build_id),
    }
}

fn dependency_key(slot: u32) -> ServiceRequirementKey {
    ServiceRequirementKey {
        caller_package_build_id: PackageBuildId::new("package:consumer"),
        service_requirement_slot: slot,
    }
}

fn contract(service_id: &str) -> ServiceContractRef {
    ServiceContractRef {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        service_protocol_identity: ServiceProtocolIdentity::new(format!("protocol:{service_id}")),
    }
}

fn dependency_slot(slot: u32, service_id: &str) -> ServiceDependencySlot {
    ServiceDependencySlot::try_new(
        dependency_key(slot),
        contract(service_id),
        [ContractOperationId::new(format!("operation:{service_id}"))],
    )
    .expect("fixture operations are unique")
}
