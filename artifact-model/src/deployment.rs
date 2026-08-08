use std::collections::BTreeMap;
use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::{
    ContractOperationId, DeploymentArtifactIdentity, DeploymentRevision, GatewayAdapterArg,
    GatewayAdapterKind, GatewayEntryIdentity, GatewayEntryKey, GatewayEntryProtocolSurface,
    PackageBuildId, PackageCallableId, PackageLocalAbiIdentity, ServiceProtocolIdentity,
};

/// Exact, path-free reference to one immutable package artifact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageArtifactRef {
    pub package_id: String,
    pub package_version: String,
    pub package_build_id: PackageBuildId,
    pub package_local_abi_identity: PackageLocalAbiIdentity,
}

/// Exact, path-free reference to one immutable service contract.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractRef {
    pub service_id: String,
    pub contract_version: String,
    pub service_protocol_identity: ServiceProtocolIdentity,
}

/// Exact, path-free reference to one immutable service deployment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentRef {
    pub service_id: String,
    pub contract_version: String,
    pub deployment_revision: DeploymentRevision,
    pub deployment_artifact_identity: DeploymentArtifactIdentity,
}

/// Activation-independent key for a direct package dependency edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageRequirementKey {
    pub caller_package_build_id: PackageBuildId,
    pub package_requirement_alias: String,
}

/// Activation-relative key for a service requirement edge.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceRequirementKey {
    pub caller_package_build_id: PackageBuildId,
    pub service_requirement_slot: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageBinding {
    pub key: PackageRequirementKey,
    pub package: PackageArtifactRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceSelectorBinding {
    pub key: ServiceRequirementKey,
    pub contract: ServiceContractRef,
}

/// Exact operation target consumed only by deployment projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentOperationInput {
    pub contract_operation_id: ContractOperationId,
    pub package_callable_id: PackageCallableId,
}

/// Canonical operation binding. Public/display paths never enter this artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentOperationBinding {
    pub contract_operation_id: ContractOperationId,
    pub package_callable_id: PackageCallableId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IngressProtocol {
    Http,
    WebSocket,
}

/// Service-local ingress selector. It points to a gateway entry, never a package path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngressSelector {
    pub protocol: IngressProtocol,
    pub method: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentIngressBinding {
    pub selector: IngressSelector,
    pub gateway_entry_key: GatewayEntryKey,
}

/// Complete, ordered mapping from a gateway callable's target parameters to
/// the typed adapter sources that provide them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GatewayAdapterPlan {
    pub kind: GatewayAdapterKind,
    pub args: Vec<GatewayAdapterArg>,
}

/// Source-free deployment-owned gateway target and execution plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentGatewayEntry {
    pub gateway_entry_identity: GatewayEntryIdentity,
    pub protocol_surface: GatewayEntryProtocolSurface,
    pub handler: Option<PackageCallableId>,
    pub pre: Option<PackageCallableId>,
    pub guard: Option<PackageCallableId>,
    pub adapter_plan: GatewayAdapterPlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_handler: Option<PackageCallableId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_adapter_plan: Option<GatewayAdapterPlan>,
}

struct GatewayEntriesVisitor;

impl<'de> Visitor<'de> for GatewayEntriesVisitor {
    type Value = BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a mapping of unique deployment gateway entry keys")
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = BTreeMap::new();
        while let Some((key, entry)) =
            access.next_entry::<GatewayEntryKey, DeploymentGatewayEntry>()?
        {
            if entries.insert(key.clone(), entry).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate deployment gateway entry key {key:?}"
                )));
            }
        }
        Ok(entries)
    }
}

fn deserialize_gateway_entries<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_map(GatewayEntriesVisitor)
}

/// Non-semantic text retained for diagnostics and excluded from identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentDiagnosticText {
    pub display_name: String,
    pub notes: BTreeMap<String, String>,
}

/// Source-free typed input to service deployment projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentInput {
    pub schema_version: String,
    pub contract: ServiceContractRef,
    pub deployment_revision: DeploymentRevision,
    pub implementation: PackageArtifactRef,
    pub operation_bindings: Vec<ServiceDeploymentOperationInput>,
    pub package_bindings: Vec<PackageBinding>,
    pub service_selectors: Vec<ServiceSelectorBinding>,
    #[serde(deserialize_with = "deserialize_gateway_entries")]
    pub gateway_entries: BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>,
    pub ingress: Vec<DeploymentIngressBinding>,
    pub diagnostic_text: DeploymentDiagnosticText,
}

/// Canonical, source-free deployment artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeployment {
    pub schema_version: String,
    pub contract: ServiceContractRef,
    pub deployment_revision: DeploymentRevision,
    pub deployment_artifact_identity: DeploymentArtifactIdentity,
    pub implementation: PackageArtifactRef,
    pub operation_bindings: Vec<DeploymentOperationBinding>,
    pub package_bindings: Vec<PackageBinding>,
    pub service_selectors: Vec<ServiceSelectorBinding>,
    #[serde(deserialize_with = "deserialize_gateway_entries")]
    pub gateway_entries: BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>,
    pub ingress: Vec<DeploymentIngressBinding>,
    pub diagnostic_text: DeploymentDiagnosticText,
}

#[cfg(test)]
mod tests;
