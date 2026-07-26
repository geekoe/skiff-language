use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Serialize};

use crate::{
    BoundaryOperationContract, GatewayAdapterArg, GatewayAdapterKind, GatewayEntryKey,
    PackageSchemaTypeId, PackageTypeRequirement, ServiceDeploymentInput, ServiceDeploymentRef,
    SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};

/// Shared source-level dependency alias vectors.  Package and contract
/// dependencies deliberately consume this leaf owner instead of maintaining
/// separate lexical or reserved-word tables.
pub const DEPENDENCY_ALIAS_POSITIVE_VECTORS: &[&str] = &["a", "accounts", "a0", "a_B9"];
pub const DEPENDENCY_ALIAS_LEXICAL_NEGATIVE_VECTORS: &[&str] = &[
    "",
    "Accounts",
    "9accounts",
    "_accounts",
    "account-service",
    "account.service",
    "账户",
];
pub const DEPENDENCY_ALIAS_RESERVED_VECTORS: &[&str] = &[
    "package", "service", "std", "ext", "connect", "config", "root",
];

pub fn is_dependency_alias_lexically_valid(alias: &str) -> bool {
    let mut chars = alias.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

pub fn is_dependency_alias_reserved(alias: &str) -> bool {
    DEPENDENCY_ALIAS_RESERVED_VECTORS.contains(&alias)
}

pub fn is_dependency_alias_valid(alias: &str) -> bool {
    is_dependency_alias_lexically_valid(alias) && !is_dependency_alias_reserved(alias)
}

#[derive(Debug)]
pub enum EcosystemAuthoringError {
    Yaml(serde_yaml::Error),
    Validation(String),
}

impl std::fmt::Display for EcosystemAuthoringError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Yaml(source) => source.fmt(formatter),
            Self::Validation(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for EcosystemAuthoringError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Yaml(source) => Some(source),
            Self::Validation(_) => None,
        }
    }
}

impl From<serde_yaml::Error> for EcosystemAuthoringError {
    fn from(source: serde_yaml::Error) -> Self {
        Self::Yaml(source)
    }
}

/// Service-only source manifest. A service source root remains a normal package
/// root; this DTO deliberately has no version, dependency or API surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceAuthoringKind {
    #[default]
    Service,
    Test,
}

fn default_http_host() -> String {
    "*".to_string()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HttpGatewayEntryAuthoring {
    #[serde(default = "default_http_host")]
    pub host: String,
    pub method: String,
    pub path: String,
    pub kind: GatewayAdapterKind,
    pub handler: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapter_args: Vec<GatewayAdapterArg>,
}

struct HttpGatewayEntriesVisitor;

impl<'de> Visitor<'de> for HttpGatewayEntriesVisitor {
    type Value = BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a mapping of unique HTTP gateway entry keys")
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = BTreeMap::new();
        while let Some((key, entry)) =
            access.next_entry::<GatewayEntryKey, HttpGatewayEntryAuthoring>()?
        {
            if entries.insert(key.clone(), entry).is_some() {
                return Err(serde::de::Error::custom(format!(
                    "duplicate HTTP gateway entry key {key:?}"
                )));
            }
        }
        Ok(entries)
    }
}

fn deserialize_http_gateway_entry_map<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserializer.deserialize_map(HttpGatewayEntriesVisitor)
}

fn deserialize_http_gateway_entries<'de, D>(
    deserializer: D,
) -> Result<Option<BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OptionalEntriesVisitor;

    impl<'de> Visitor<'de> for OptionalEntriesVisitor {
        type Value = Option<BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("null or a mapping of unique HTTP gateway entry keys")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserialize_http_gateway_entry_map(deserializer).map(Some)
        }
    }

    deserializer.deserialize_option(OptionalEntriesVisitor)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceManifestAuthoring {
    pub id: String,
    #[serde(default)]
    pub kind: ServiceAuthoringKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_calls: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_http_gateway_entries"
    )]
    pub http: Option<BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<serde_json::Value>,
}

/// One environment profile. Profiles bind already-declared runtime
/// requirements and never own code dependencies.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceConfigProfileAuthoring {
    #[serde(default)]
    pub config: serde_json::Value,
    #[serde(default)]
    pub secrets: serde_json::Value,
    #[serde(default)]
    pub state: serde_json::Value,
    #[serde(default)]
    pub resources: serde_json::Value,
    #[serde(default)]
    pub timeout: serde_json::Value,
    #[serde(default)]
    pub quota: serde_json::Value,
    #[serde(default)]
    pub principal: serde_json::Value,
    #[serde(default)]
    pub lifecycle: serde_json::Value,
}

/// Diagnostic strings keyed by authoring stable keys. They never enter protocol identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractDefinitionDiagnosticText {
    pub service: String,
    pub operations: BTreeMap<String, String>,
    pub types: BTreeMap<PackageSchemaTypeId, String>,
}

/// Code-free `contract.yml` input. Operations may reference only package-owned
/// schema identities listed by the exact package requirements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractDefinition {
    pub schema_version: String,
    pub service_id: String,
    pub contract_version: String,
    pub operations: BTreeMap<String, BoundaryOperationContract>,
    pub package_type_requirements: Vec<PackageTypeRequirement>,
    pub diagnostic_text: ServiceContractDefinitionDiagnosticText,
}

/// `deployment.yml` is already represented by the strict, source-free
/// projection input. This alias prevents a second copy of that body.
pub type ServiceDeploymentAuthoring = ServiceDeploymentInput;

/// The complete canonical `assembly.yml` surface. Closure and identity are
/// resolved from the exact root deployment references, never from "latest".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeAssemblyAuthoring {
    pub environment: String,
    pub root_deployments: Vec<ServiceDeploymentRef>,
}

pub fn parse_service_contract_definition_yml(
    source: &str,
) -> Result<ServiceContractDefinition, EcosystemAuthoringError> {
    let definition = serde_yaml::from_str::<ServiceContractDefinition>(source)?;
    definition
        .validate()
        .map_err(EcosystemAuthoringError::Validation)?;
    Ok(definition)
}

pub fn parse_service_deployment_yml(
    source: &str,
) -> Result<ServiceDeploymentAuthoring, EcosystemAuthoringError> {
    let deployment = serde_yaml::from_str::<ServiceDeploymentAuthoring>(source)?;
    validate_deployment_authoring(&deployment).map_err(EcosystemAuthoringError::Validation)?;
    Ok(deployment)
}

pub fn parse_runtime_assembly_yml(
    source: &str,
) -> Result<RuntimeAssemblyAuthoring, EcosystemAuthoringError> {
    let assembly = serde_yaml::from_str::<RuntimeAssemblyAuthoring>(source)?;
    assembly
        .validate()
        .map_err(EcosystemAuthoringError::Validation)?;
    Ok(assembly)
}

impl ServiceContractDefinition {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION {
            return Err(format!(
                "contract.yml schemaVersion must be {SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION}"
            ));
        }
        for (label, value) in [
            ("serviceId", self.service_id.as_str()),
            ("contractVersion", self.contract_version.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("contract.yml {label} must not be empty"));
            }
        }
        if self.operations.is_empty() {
            return Err("contract.yml operations must not be empty".to_string());
        }
        if self.operations.keys().any(|key| key.trim().is_empty()) {
            return Err("contract.yml stable keys must not be empty".to_string());
        }
        if let Some(key) = self
            .diagnostic_text
            .operations
            .keys()
            .find(|key| !self.operations.contains_key(*key))
        {
            return Err(format!(
                "contract.yml diagnosticText references unknown operation {key}"
            ));
        }
        if let Some(key) = self.diagnostic_text.types.keys().find(|key| {
            !self
                .package_type_requirements
                .iter()
                .any(|requirement| requirement.required_type_ids.contains(key))
        }) {
            return Err(format!(
                "contract.yml diagnosticText references unknown type {key}"
            ));
        }
        Ok(())
    }
}

impl RuntimeAssemblyAuthoring {
    pub fn validate(&self) -> Result<(), String> {
        if self.environment.trim().is_empty() {
            return Err("assembly.yml environment must not be empty".to_string());
        }
        if !is_safe_token(&self.environment) {
            return Err(
                "assembly.yml environment must use only letters, digits, dot, dash, or underscore"
                    .to_string(),
            );
        }
        if self.root_deployments.is_empty() {
            return Err("assembly.yml rootDeployments must not be empty".to_string());
        }
        let mut roots = BTreeSet::new();
        for root in &self.root_deployments {
            if !roots.insert(root) {
                return Err(format!(
                    "assembly.yml contains duplicate root deployment {root:?}"
                ));
            }
        }
        Ok(())
    }
}

fn validate_deployment_authoring(deployment: &ServiceDeploymentAuthoring) -> Result<(), String> {
    if deployment.schema_version != SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION {
        return Err(format!(
            "deployment.yml schemaVersion must be {SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION}"
        ));
    }
    for (label, value) in [
        (
            "contract.serviceId",
            deployment.contract.service_id.as_str(),
        ),
        (
            "contract.contractVersion",
            deployment.contract.contract_version.as_str(),
        ),
        (
            "contract.serviceProtocolIdentity",
            deployment.contract.service_protocol_identity.as_str(),
        ),
        (
            "deploymentRevision",
            deployment.deployment_revision.as_str(),
        ),
        (
            "implementation.packageId",
            deployment.implementation.package_id.as_str(),
        ),
        (
            "implementation.packageVersion",
            deployment.implementation.package_version.as_str(),
        ),
        (
            "implementation.packageBuildId",
            deployment.implementation.package_build_id.as_str(),
        ),
        (
            "implementation.packageLocalAbiIdentity",
            deployment
                .implementation
                .package_local_abi_identity
                .as_str(),
        ),
    ] {
        if value.trim().is_empty() {
            return Err(format!("deployment.yml {label} must not be empty"));
        }
    }
    Ok(())
}

fn is_safe_token(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.bytes().all(|byte| {
            matches!(
                byte,
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'_'
            )
        })
}

#[cfg(test)]
mod tests {
    use crate::{
        ActivationPolicy, BoundaryCallbackContract, BoundaryCancellationContract,
        BoundaryEffectGuarantee, BoundaryReturn, BoundaryStreamContract, BoundaryValueCarrier,
        BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan,
        ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText, DeploymentPolicy,
        DeploymentRevision, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity,
        ResourcePolicy, ServiceContractRef, ServiceProtocolIdentity,
        SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn dependency_alias_vectors_have_one_shared_leaf_owner() {
        for alias in DEPENDENCY_ALIAS_POSITIVE_VECTORS {
            assert!(is_dependency_alias_lexically_valid(alias), "{alias}");
            assert!(!is_dependency_alias_reserved(alias), "{alias}");
            assert!(is_dependency_alias_valid(alias), "{alias}");
        }
        for alias in DEPENDENCY_ALIAS_LEXICAL_NEGATIVE_VECTORS {
            assert!(!is_dependency_alias_lexically_valid(alias), "{alias}");
            assert!(!is_dependency_alias_valid(alias), "{alias}");
        }
        for alias in DEPENDENCY_ALIAS_RESERVED_VECTORS {
            assert!(is_dependency_alias_lexically_valid(alias), "{alias}");
            assert!(is_dependency_alias_reserved(alias), "{alias}");
            assert!(!is_dependency_alias_valid(alias), "{alias}");
        }
    }

    #[test]
    fn service_manifest_missing_and_empty_service_calls_are_equivalent() {
        let missing =
            serde_yaml::from_str::<ServiceManifestAuthoring>("id: example.com/users\n").unwrap();
        let empty = serde_yaml::from_str::<ServiceManifestAuthoring>(
            "id: example.com/users\nserviceCalls: []\n",
        )
        .unwrap();
        assert_eq!(missing, empty);
        assert!(missing.service_calls.is_empty());
        assert_eq!(
            serde_json::to_value(&missing).unwrap(),
            serde_json::json!({
                "id": "example.com/users",
                "kind": "service"
            })
        );

        let unvalidated = serde_yaml::from_str::<ServiceManifestAuthoring>(
            "id: example.com/users\nserviceCalls:\n  - users.get\n  - not validated here\n",
        )
        .unwrap();
        assert_eq!(
            unvalidated.service_calls,
            vec!["users.get".to_string(), "not validated here".to_string()]
        );
        assert_eq!(
            serde_json::to_value(&unvalidated).unwrap()["serviceCalls"],
            serde_json::json!(["users.get", "not validated here"])
        );

        assert!(serde_yaml::from_str::<ServiceManifestAuthoring>(
            "id: example.com/users\nserviceCallRoots: []\n"
        )
        .is_err());
    }

    #[test]
    fn service_manifest_decodes_named_http_entries_in_canonical_key_order() {
        let manifest = serde_yaml::from_str::<ServiceManifestAuthoring>(
            r#"
id: example.com/users
http:
  zRaw:
    host: api.example.com
    method: GET
    path: /raw
    kind: rawHttp
    handler: handlers.raw
  createUser:
    method: POST
    path: /users
    kind: typedJson
    handler: users.create
    guard: users.guard
    pre: users.prepare
    adapterArgs:
      - param: body
        source: { kind: http.body }
      - param: context
        source: { kind: http.context }
"#,
        )
        .unwrap();
        let http = manifest.http.as_ref().unwrap();
        assert_eq!(
            http.keys().map(GatewayEntryKey::as_str).collect::<Vec<_>>(),
            vec!["createUser", "zRaw"]
        );
        assert_eq!(
            http[&GatewayEntryKey::parse("createUser").unwrap()].host,
            "*"
        );

        let encoded = serde_json::to_string(&manifest).unwrap();
        assert!(encoded.find("createUser").unwrap() < encoded.find("zRaw").unwrap());
        assert_eq!(
            serde_json::from_str::<ServiceManifestAuthoring>(&encoded).unwrap(),
            manifest
        );
        assert!(serde_yaml::from_str::<ServiceManifestAuthoring>(
            "id: example.com/users\nhttp: null\n"
        )
        .unwrap()
        .http
        .is_none());
    }

    #[test]
    fn service_manifest_http_mapping_rejects_duplicate_keys_and_recursive_unknown_fields() {
        let duplicate = r#"
id: example.com/users
http:
  createUser:
    method: POST
    path: /users
    kind: typedJson
    handler: users.create
  createUser:
    method: PUT
    path: /users
    kind: typedJson
    handler: users.replace
"#;
        assert!(serde_yaml::from_str::<ServiceManifestAuthoring>(duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate HTTP gateway entry key"));

        for invalid in [
            "unknown: true",
            "operation: createUser",
            "handlerArgs: []",
            "id: duplicate",
        ] {
            let source = format!(
                "id: example.com/users\nhttp:\n  createUser:\n    method: POST\n    path: /users\n    kind: typedJson\n    handler: users.create\n    {invalid}\n"
            );
            assert!(
                serde_yaml::from_str::<ServiceManifestAuthoring>(&source).is_err(),
                "{invalid}"
            );
        }

        let unknown_source_field = r#"
id: example.com/users
http:
  createUser:
    method: POST
    path: /users
    kind: typedJson
    handler: users.create
    adapterArgs:
      - param: body
        source: { kind: http.body, field: nested }
"#;
        assert!(serde_yaml::from_str::<ServiceManifestAuthoring>(unknown_source_field).is_err());
    }

    #[test]
    fn contract_deployment_and_assembly_documents_have_exact_top_level_fields() {
        let contract = ServiceContractDefinition {
            schema_version: SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION.to_string(),
            service_id: "example.com/echo".to_string(),
            contract_version: "1.0.0".to_string(),
            operations: BTreeMap::from([("health".to_string(), operation_contract())]),
            package_type_requirements: Vec::new(),
            diagnostic_text: ServiceContractDefinitionDiagnosticText {
                service: "Echo".to_string(),
                operations: BTreeMap::new(),
                types: BTreeMap::new(),
            },
        };
        let contract_yml = serde_yaml::to_string(&contract).unwrap();
        assert_eq!(
            parse_service_contract_definition_yml(&contract_yml).unwrap(),
            contract
        );
        assert!(parse_service_contract_definition_yml(&format!(
            "{contract_yml}providerBuildId: forbidden\n"
        ))
        .is_err());
        assert!(parse_service_contract_definition_yml(&contract_yml.replace(
            SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION,
            "skiff-service-contract-definition-v2"
        ))
        .is_err());

        let deployment = ServiceDeploymentInput {
            schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
            contract: ServiceContractRef {
                service_id: "example.com/echo".to_string(),
                contract_version: "1.0.0".to_string(),
                service_protocol_identity: ServiceProtocolIdentity::new("protocol"),
            },
            deployment_revision: DeploymentRevision::new("revision-1"),
            implementation: PackageArtifactRef {
                package_id: "example.com/provider".to_string(),
                package_version: "1.0.0".to_string(),
                package_build_id: PackageBuildId::new("build"),
                package_local_abi_identity: PackageLocalAbiIdentity::new("abi"),
            },
            operation_bindings: Vec::new(),
            package_bindings: Vec::new(),
            service_selectors: Vec::new(),
            gateway_entries: BTreeMap::new(),
            ingress: Vec::new(),
            config_literals: Vec::new(),
            secret_refs: Vec::new(),
            state_bindings: Vec::new(),
            resource_bindings: Vec::new(),
            runtime_capability_bindings: Vec::new(),
            policy: DeploymentPolicy {
                timeout_ms: Some(1_000),
                resources: ResourcePolicy {
                    cpu_millis: 100,
                    memory_bytes: 1_024,
                },
                activation: ActivationPolicy {
                    max_concurrency: 1,
                    idle_timeout_ms: None,
                },
                principal: "service:echo".to_string(),
            },
            diagnostic_text: DeploymentDiagnosticText {
                display_name: "Echo".to_string(),
                notes: BTreeMap::new(),
            },
        };
        let deployment_yml = serde_yaml::to_string(&deployment).unwrap();
        assert_eq!(
            parse_service_deployment_yml(&deployment_yml).unwrap(),
            deployment
        );
        assert!(parse_service_deployment_yml(&deployment_yml.replace(
            SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
            "skiff-service-deployment-input-v2"
        ))
        .is_err());
        assert!(
            parse_service_deployment_yml(&format!("{deployment_yml}sourceRoot: forbidden\n"))
                .is_err()
        );

        let assembly = RuntimeAssemblyAuthoring {
            environment: "test".to_string(),
            root_deployments: vec![ServiceDeploymentRef {
                service_id: "example.com/echo".to_string(),
                contract_version: "1.0.0".to_string(),
                deployment_revision: DeploymentRevision::new("revision-1"),
                deployment_artifact_identity: DeploymentArtifactIdentity::new("deployment"),
            }],
        };
        let assembly_yml = serde_yaml::to_string(&assembly).unwrap();
        assert_eq!(parse_runtime_assembly_yml(&assembly_yml).unwrap(), assembly);
        assert!(parse_runtime_assembly_yml(&format!("{assembly_yml}artifactRoots: []\n")).is_err());
    }

    fn operation_contract() -> BoundaryOperationContract {
        BoundaryOperationContract {
            parameters: Vec::new(),
            return_value: BoundaryReturn {
                ty: ContractTypeRef::builtin("bool"),
                value_plan: BoundaryValuePlan::Linkable {
                    carrier: BoundaryValueCarrier::DetachedValueGraph,
                    encoding: BoundaryValueEncoding::CanonicalValue,
                    owner: BoundaryValueOwner::Provider,
                    lifetime: BoundaryValueLifetime::Call,
                },
            },
            stream: BoundaryStreamContract::Unary,
            cancellation: BoundaryCancellationContract::NotCancellable,
            callbacks: BoundaryCallbackContract::None,
            may_suspend: false,
            effect_guarantee: BoundaryEffectGuarantee {
                detached_parameters: true,
                detached_return: true,
                detached_error: true,
                no_caller_reachable_mutation: true,
                no_caller_value_escape: true,
                no_same_heap_identity: true,
            },
        }
    }
}
