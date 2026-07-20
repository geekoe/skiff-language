use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{
    BoundaryOperationContract, ContractTypeShape, ServiceDeploymentInput, ServiceDeploymentRef,
    SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};

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

/// One `package.yml` compile dependency on an independently published contract.
///
/// Protocol identity is deliberately absent: the compiler obtains it from the
/// selected, validated ServiceContract record before constructing its
/// `ContractRequirement`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageContractAuthoring {
    pub alias: String,
    pub service_id: String,
    pub contract_version: String,
}

/// Strict projection of the `contracts` field in `package.yml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PackageContractsAuthoring {
    pub contracts: Vec<PackageContractAuthoring>,
}

/// Diagnostic strings keyed by authoring stable keys. They never enter protocol identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractDefinitionDiagnosticText {
    pub service: String,
    pub operations: BTreeMap<String, String>,
    pub types: BTreeMap<String, String>,
}

/// Code-free `contract.yml` input. Stable keys are replaced with canonical
/// operation/type identities while materializing the ServiceContract record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceContractDefinition {
    pub schema_version: String,
    pub service_id: String,
    pub contract_version: String,
    pub operations: BTreeMap<String, BoundaryOperationContract>,
    pub boundary_schema: BTreeMap<String, ContractTypeShape>,
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

pub fn parse_package_contracts_yml(
    source: &str,
) -> Result<PackageContractsAuthoring, EcosystemAuthoringError> {
    let authoring = serde_yaml::from_str::<PackageContractsAuthoring>(source)?;
    authoring
        .validate()
        .map_err(EcosystemAuthoringError::Validation)?;
    Ok(authoring)
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

impl PackageContractsAuthoring {
    pub fn validate(&self) -> Result<(), String> {
        let mut aliases = BTreeSet::new();
        for dependency in &self.contracts {
            for (label, value) in [
                ("alias", dependency.alias.as_str()),
                ("serviceId", dependency.service_id.as_str()),
                ("contractVersion", dependency.contract_version.as_str()),
            ] {
                if value.trim().is_empty() {
                    return Err(format!(
                        "package.yml contracts entry {label} must not be empty"
                    ));
                }
            }
            if !is_dependency_alias(&dependency.alias) {
                return Err(format!(
                    "package.yml contract alias {} must match [a-z][A-Za-z0-9_]* and not be reserved",
                    dependency.alias
                ));
            }
            if !aliases.insert(dependency.alias.as_str()) {
                return Err(format!(
                    "package.yml contracts contains duplicate alias {}",
                    dependency.alias
                ));
            }
        }
        Ok(())
    }
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
        if self.operations.keys().any(|key| key.trim().is_empty())
            || self.boundary_schema.keys().any(|key| key.trim().is_empty())
        {
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
        if let Some(key) = self
            .diagnostic_text
            .types
            .keys()
            .find(|key| !self.boundary_schema.contains_key(*key))
        {
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

fn is_dependency_alias(alias: &str) -> bool {
    let mut chars = alias.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
        && !matches!(
            alias,
            "package" | "service" | "std" | "ext" | "connect" | "config" | "root"
        )
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
        BoundaryEffectGuarantee, BoundaryErrorContract, BoundaryReturn, BoundaryStreamContract,
        BoundaryValueCarrier, BoundaryValueEncoding, BoundaryValueLifetime, BoundaryValueOwner,
        BoundaryValuePlan, ContractTypeRef, DeploymentArtifactIdentity, DeploymentDiagnosticText,
        DeploymentPolicy, DeploymentRevision, PackageArtifactRef, PackageBuildId,
        PackageLocalAbiIdentity, ResourcePolicy, ServiceContractRef, ServiceProtocolIdentity,
        SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn package_contract_section_is_strict_and_provider_free() {
        let source = r#"
contracts:
  - alias: accounts
    serviceId: skiff.run/account
    contractVersion: 1.0.0
"#;
        let parsed = parse_package_contracts_yml(source).expect("contracts section");
        parsed.validate().expect("valid contracts section");
        assert_eq!(parsed.contracts[0].alias, "accounts");

        for forbidden in [
            "expectedProtocolIdentity",
            "providerPackageId",
            "providerBuildId",
            "deploymentRevision",
        ] {
            let invalid = source.replace(
                "    contractVersion: 1.0.0",
                &format!("    contractVersion: 1.0.0\n    {forbidden}: forbidden"),
            );
            assert!(
                parse_package_contracts_yml(&invalid).is_err(),
                "{forbidden}"
            );
        }
    }

    #[test]
    fn authoring_parsers_reject_unknown_missing_and_duplicate_fields() {
        assert!(parse_package_contracts_yml("contracts: []\nlegacy: true\n").is_err());
        assert!(parse_package_contracts_yml(
            "contracts:\n  - alias: a\n    alias: b\n    serviceId: s\n    contractVersion: v\n"
        )
        .is_err());
        assert!(
            parse_package_contracts_yml("contracts:\n  - alias: a\n    serviceId: s\n").is_err()
        );
    }

    #[test]
    fn contract_deployment_and_assembly_documents_have_exact_top_level_fields() {
        let contract = ServiceContractDefinition {
            schema_version: SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION.to_string(),
            service_id: "example.com/echo".to_string(),
            contract_version: "1.0.0".to_string(),
            operations: BTreeMap::from([("health".to_string(), operation_contract())]),
            boundary_schema: BTreeMap::new(),
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
            ingress: Vec::new(),
            config_literals: Vec::new(),
            secret_refs: Vec::new(),
            state_bindings: Vec::new(),
            resource_bindings: Vec::new(),
            runtime_capability_bindings: Vec::new(),
            policy: DeploymentPolicy {
                timeout_ms: 1_000,
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
            errors: BoundaryErrorContract::None,
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
