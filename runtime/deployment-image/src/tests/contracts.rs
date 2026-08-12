use skiff_artifact_model::{
    ContractOperationId, PackageBuildId, ServiceContractRef, ServiceProtocolIdentity,
    ServiceRequirementKey,
};

use crate::{ServiceDependencySlot, ServiceDependencySlotError};

use super::owner;

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
