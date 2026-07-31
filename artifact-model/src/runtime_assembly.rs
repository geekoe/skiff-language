use serde::{de, Deserialize, Deserializer, Serialize};

use crate::{
    validate_runtime_assembly_identity, AssemblyIdentity, ContractOperationId,
    GatewayEntryIdentity, GatewayEntryKey, IngressSelector, PackageArtifactRef, PackageBinding,
    PackageBuildId, ServiceContractRef, ServiceDeploymentRef, ServiceRequirementKey,
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

/// Deployment-owned non-config inputs used to create one ActivationContext.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationTemplate {
    pub deployment: ServiceDeploymentRef,
    pub implementation_package_build_id: PackageBuildId,
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
mod tests;
