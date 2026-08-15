use std::sync::Arc;

use skiff_artifact_model::{
    PackageBuildId, ServiceContractRef, ServiceProtocolIdentity, ServiceRequirementKey,
};
use skiff_runtime_deployment_image::ServiceDependencySlot;
use skiff_runtime_request::{BytecodeRequestChildComposition, BytecodeServiceChildError};

#[test]
fn default_service_resolver_fails_closed_for_missing_provider() {
    let composition = BytecodeRequestChildComposition::default();
    let slot = ServiceDependencySlot::try_new(
        ServiceRequirementKey {
            caller_package_build_id: PackageBuildId::new("build:caller"),
            service_requirement_slot: 0,
        },
        ServiceContractRef {
            service_id: "example.com/provider".to_string(),
            contract_version: "1.0.0".to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("unassigned"),
        },
        Vec::<skiff_artifact_model::ContractOperationId>::new(),
    )
    .expect("dependency slot accepts an empty operation set");
    let operation =
        skiff_artifact_identity::contract_operation_id("example.com/provider", "1.0.0", "run")
            .unwrap();
    let error = composition
        .service_resolver
        .resolve_service(
            &slot,
            &operation,
            &ServiceProtocolIdentity::new("unassigned"),
        )
        .expect_err("default resolver must fail closed");
    assert!(matches!(
        error,
        BytecodeServiceChildError::ProviderMissing {
            service_id,
            contract_version,
        } if service_id == "example.com/provider" && contract_version == "1.0.0"
    ));
    let _ = Arc::new(composition);
}
