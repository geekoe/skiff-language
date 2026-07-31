use serde_json::json;

use super::*;
use crate::{
    GatewayEntryIdentity, GatewayEntryKey, GATEWAY_ENTRY_IDENTITY_PREFIX,
    RUNTIME_ASSEMBLY_IDENTITY_PREFIX, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};

fn empty_runtime_assembly() -> RuntimeAssembly {
    RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new(format!(
            "{RUNTIME_ASSEMBLY_IDENTITY_PREFIX}:{}",
            "a".repeat(64)
        )),
        roots: Vec::new(),
        resolved_deployments: Vec::new(),
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    }
}

#[test]
fn runtime_assembly_v3_wire_requires_gateway_ingress_and_rejects_legacy_fields() {
    let assembly = empty_runtime_assembly();
    let value = serde_json::to_value(&assembly).unwrap();
    assert_eq!(
        value["schemaVersion"],
        json!(RUNTIME_ASSEMBLY_SCHEMA_VERSION)
    );
    assert_eq!(value["gatewayIngress"], json!([]));
    assert!(value.get("globalIngress").is_none());

    let mut missing = value.clone();
    missing.as_object_mut().unwrap().remove("gatewayIngress");
    assert!(serde_json::from_value::<RuntimeAssembly>(missing).is_err());

    let mut legacy = value.clone();
    legacy.as_object_mut().unwrap().remove("gatewayIngress");
    legacy
        .as_object_mut()
        .unwrap()
        .insert("globalIngress".to_string(), json!([]));
    assert!(serde_json::from_value::<RuntimeAssembly>(legacy).is_err());
}

#[test]
fn runtime_assembly_gateway_binding_wire_is_strict_and_operation_free() {
    let binding = GatewayIngressBinding {
        selector: IngressSelector {
            protocol: crate::IngressProtocol::Http,
            method: Some("POST".to_string()),
            path: "/users".to_string(),
        },
        deployment: ServiceDeploymentRef {
            service_id: "example.users".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: "revision-1".into(),
            deployment_artifact_identity: "skiff-service-deployment-v2:sha256:fixture".into(),
        },
        gateway_entry_key: GatewayEntryKey::parse("create-user").unwrap(),
        gateway_entry_identity: GatewayEntryIdentity::parse(format!(
            "{GATEWAY_ENTRY_IDENTITY_PREFIX}:{}",
            "b".repeat(64)
        ))
        .unwrap(),
    };
    let value = serde_json::to_value(&binding).unwrap();
    assert_eq!(value["gatewayEntryKey"], json!("create-user"));
    assert!(value.get("contract").is_none());
    assert!(value.get("contractOperationId").is_none());
    assert_eq!(
        serde_json::from_value::<GatewayIngressBinding>(value.clone()).unwrap(),
        binding
    );

    for legacy_field in ["contract", "contractOperationId", "operationDescriptor"] {
        let mut legacy = value.clone();
        legacy
            .as_object_mut()
            .unwrap()
            .insert(legacy_field.to_string(), json!("legacy"));
        assert!(
            serde_json::from_value::<GatewayIngressBinding>(legacy).is_err(),
            "{legacy_field}"
        );
    }
}

#[test]
fn ingress_selector_rejects_legacy_host_wire() {
    let current = json!({
        "protocol": "http",
        "method": "GET",
        "path": "/items"
    });
    assert!(serde_json::from_value::<IngressSelector>(current.clone()).is_ok());

    let mut legacy = current;
    legacy["host"] = json!("api.example.test");
    assert!(serde_json::from_value::<IngressSelector>(legacy).is_err());
}

#[test]
fn activation_template_rejects_retired_runtime_config_state_and_policy_fields() {
    let template = ActivationTemplate {
        deployment: ServiceDeploymentRef {
            service_id: "example.com/users".to_string(),
            contract_version: "1.0.0".to_string(),
            deployment_revision: "revision".into(),
            deployment_artifact_identity: "deployment".into(),
        },
        implementation_package_build_id: PackageBuildId::new("build"),
    };
    let canonical = serde_json::to_value(template).unwrap();

    for field in [
        "configLiterals",
        "secretRefs",
        "stateBindings",
        "resourceBindings",
        "policy",
    ] {
        let mut retired = canonical.clone();
        retired[field] = json!([]);
        assert!(
            serde_json::from_value::<ActivationTemplate>(retired).is_err(),
            "{field} unexpectedly survived the activation template hard cut"
        );
    }
}
