use std::sync::Arc;

use skiff_artifact_model::{
    ContractOperationId, PackageBuildId, ServiceContractRef, ServiceProtocolIdentity,
    ServiceRequirementKey,
};

use crate::{
    DeploymentImage, DeploymentImageError, PinnedProviderImage, ServiceDependencySlot,
    ServiceDependencySlotError,
};

use super::{owner, owner_with};

#[test]
fn owner_preserves_the_exact_deployment_build() {
    let owner = owner("build:consumer:exact");

    assert_eq!(
        &owner.deployment().deployment_artifact_identity,
        owner.build_id()
    );
    assert_eq!(owner.build_id().as_str(), "build:consumer:exact");
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
        owner("build:consumer:exact"),
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
fn provider_pin_keeps_the_same_image_arc_without_requiring_program_clone() {
    #[derive(Debug)]
    struct NonCloneProgram;

    let owner = owner_with("build:provider:exact", "provider", "revision:provider");
    let image = Arc::new(
        DeploymentImage::try_new(
            owner.clone(),
            Arc::new(NonCloneProgram),
            [dependency_slot(2, "ledger")],
        )
        .expect("unique dependency slots must construct an image"),
    );
    let invocation_pin = PinnedProviderImage::new(Arc::clone(&image));
    let stream_pin = invocation_pin.clone();
    let callback_pin = invocation_pin.clone();

    assert!(Arc::ptr_eq(invocation_pin.image(), &image));
    assert!(Arc::ptr_eq(stream_pin.image(), callback_pin.image()));
    assert_eq!(invocation_pin.owner(), &owner);
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
