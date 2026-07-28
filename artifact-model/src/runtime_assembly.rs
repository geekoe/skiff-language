use serde::{de, Deserialize, Deserializer, Serialize};

use crate::{
    validate_runtime_assembly_identity, AssemblyIdentity, ConfigLiteralBinding,
    ContractOperationId, DeploymentPolicy, GatewayEntryIdentity, GatewayEntryKey, IngressSelector,
    PackageArtifactRef, PackageBinding, PackageBuildId, ResourceBinding, SecretRefBinding,
    ServiceContractRef, ServiceDeploymentRef, ServiceRequirementKey, StateBinding,
};

/// Exact reference to one immutable RuntimeAssembly record.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyRef {
    pub assembly_identity: AssemblyIdentity,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeAssemblyRef {
    assembly_identity: AssemblyIdentity,
}

impl<'de> Deserialize<'de> for RuntimeAssemblyRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawRuntimeAssemblyRef::deserialize(deserializer)?;
        validate_runtime_assembly_identity(raw.assembly_identity.as_str())
            .map_err(de::Error::custom)?;
        Ok(Self {
            assembly_identity: raw.assembly_identity,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageCodeSlot {
    pub package: PackageArtifactRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalPackageLinkPlan {
    pub code_slots: Vec<PackageCodeSlot>,
    pub package_links: Vec<PackageBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedServiceBinding {
    pub key: ServiceRequirementKey,
    pub contract: ServiceContractRef,
    pub provider: ServiceDeploymentRef,
    pub used_operations: Vec<ContractOperationId>,
}

/// All service edges for one activation. The activation key prevents global slot patching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceBindingTemplate {
    pub activation: ServiceDeploymentRef,
    pub bindings: Vec<ResolvedServiceBinding>,
}

/// Deployment-owned values and state handles used to create one ActivationContext.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationTemplate {
    pub deployment: ServiceDeploymentRef,
    pub implementation_package_build_id: PackageBuildId,
    pub config_literals: Vec<ConfigLiteralBinding>,
    pub secret_refs: Vec<SecretRefBinding>,
    pub state_bindings: Vec<StateBinding>,
    pub resource_bindings: Vec<ResourceBinding>,
    pub policy: DeploymentPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayIngressBinding {
    pub selector: IngressSelector,
    pub deployment: ServiceDeploymentRef,
    pub gateway_entry_key: GatewayEntryKey,
    pub gateway_entry_identity: GatewayEntryIdentity,
}

/// Canonical assembly-scoped ingress identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceIngressKey {
    pub deployment: ServiceDeploymentRef,
    pub selector: IngressSelector,
}

impl GatewayIngressBinding {
    #[must_use]
    pub fn service_ingress_key(&self) -> ServiceIngressKey {
        ServiceIngressKey {
            deployment: self.deployment.clone(),
            selector: self.selector.clone(),
        }
    }
}

/// Canonical resolved deployment/package graph loaded identically by every replica.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssembly {
    pub schema_version: String,
    pub assembly_identity: AssemblyIdentity,
    pub roots: Vec<ServiceDeploymentRef>,
    pub resolved_deployments: Vec<ServiceDeploymentRef>,
    pub resolved_contracts: Vec<ServiceContractRef>,
    pub resolved_packages: Vec<PackageArtifactRef>,
    pub package_link_plan: CanonicalPackageLinkPlan,
    pub service_binding_templates: Vec<ServiceBindingTemplate>,
    pub activation_templates: Vec<ActivationTemplate>,
    pub gateway_ingress: Vec<GatewayIngressBinding>,
}

#[cfg(test)]
mod tests {
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
}
