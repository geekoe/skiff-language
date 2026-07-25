use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    ContractOperationId, DeploymentArtifactIdentity, DeploymentRevision, MetadataValue,
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

/// Human-authored operation target consumed only by deployment projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceDeploymentOperationInput {
    pub contract_operation_id: ContractOperationId,
    pub package_public_path: String,
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

/// Global external selector. It points to a contract operation, never a package path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IngressSelector {
    pub protocol: IngressProtocol,
    pub host: String,
    pub method: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentIngressBinding {
    pub selector: IngressSelector,
    pub contract_operation_id: ContractOperationId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigLiteralBinding {
    pub path: String,
    pub value: MetadataValue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SecretRefBinding {
    pub path: String,
    pub secret_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StateBindingKind {
    Database,
    Redis,
    Actor,
    Queue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StateBinding {
    pub requirement_key: String,
    pub kind: StateBindingKind,
    pub namespace: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceBinding {
    pub requirement_key: String,
    pub capability: String,
    pub resource_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeCapabilityBinding {
    pub capability: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourcePolicy {
    pub cpu_millis: u32,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationPolicy {
    pub max_concurrency: u32,
    pub idle_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentPolicy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    pub resources: ResourcePolicy,
    pub activation: ActivationPolicy,
    pub principal: String,
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
    pub ingress: Vec<DeploymentIngressBinding>,
    pub config_literals: Vec<ConfigLiteralBinding>,
    pub secret_refs: Vec<SecretRefBinding>,
    pub state_bindings: Vec<StateBinding>,
    pub resource_bindings: Vec<ResourceBinding>,
    pub runtime_capability_bindings: Vec<RuntimeCapabilityBinding>,
    pub policy: DeploymentPolicy,
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
    pub ingress: Vec<DeploymentIngressBinding>,
    pub config_literals: Vec<ConfigLiteralBinding>,
    pub secret_refs: Vec<SecretRefBinding>,
    pub state_bindings: Vec<StateBinding>,
    pub resource_bindings: Vec<ResourceBinding>,
    pub runtime_capability_bindings: Vec<RuntimeCapabilityBinding>,
    pub policy: DeploymentPolicy,
    pub diagnostic_text: DeploymentDiagnosticText,
}
