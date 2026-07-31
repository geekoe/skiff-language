use serde_json::json;

use super::*;

#[test]
fn service_deployment_operation_input_requires_exact_callable_id() {
    let canonical = json!({
        "contractOperationId": "operation",
        "packageCallableId": "pkg-callable:example.provider:echo"
    });
    let decoded =
        serde_json::from_value::<ServiceDeploymentOperationInput>(canonical.clone()).unwrap();
    assert_eq!(decoded.contract_operation_id.as_str(), "operation");
    assert_eq!(
        decoded.package_callable_id.as_str(),
        "pkg-callable:example.provider:echo"
    );
    assert_eq!(serde_json::to_value(decoded).unwrap(), canonical);

    let legacy_path = json!({
        "contractOperationId": "operation",
        "packagePublicPath": "echo"
    });
    assert!(serde_json::from_value::<ServiceDeploymentOperationInput>(legacy_path).is_err());

    let mut both = canonical.clone();
    both["packagePublicPath"] = json!("echo");
    assert!(serde_json::from_value::<ServiceDeploymentOperationInput>(both).is_err());

    let mut missing_exact_id = canonical.clone();
    missing_exact_id
        .as_object_mut()
        .unwrap()
        .remove("packageCallableId");
    assert!(serde_json::from_value::<ServiceDeploymentOperationInput>(missing_exact_id).is_err());

    let mut unknown = canonical;
    unknown["sourcePath"] = json!("echo");
    assert!(serde_json::from_value::<ServiceDeploymentOperationInput>(unknown).is_err());
}

#[test]
fn service_deployment_rejects_retired_runtime_config_state_and_policy_fields() {
    let deployment = ServiceDeployment {
        schema_version: crate::SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: ServiceContractRef {
            service_id: "example.com/users".to_string(),
            contract_version: "1.0.0".to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("protocol"),
        },
        deployment_revision: DeploymentRevision::new("revision"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new("identity"),
        implementation: PackageArtifactRef {
            package_id: "example.com/users".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("build"),
            package_local_abi_identity: PackageLocalAbiIdentity::new("abi"),
        },
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "users".to_string(),
            notes: BTreeMap::new(),
        },
    };
    let canonical = serde_json::to_value(deployment).unwrap();

    for field in [
        "configLiterals",
        "secretRefs",
        "stateBindings",
        "resourceBindings",
        "runtimeCapabilityBindings",
        "policy",
    ] {
        let mut retired = canonical.clone();
        retired[field] = json!([]);
        assert!(
            serde_json::from_value::<ServiceDeployment>(retired).is_err(),
            "{field} unexpectedly survived the deployment hard cut"
        );
    }
}
