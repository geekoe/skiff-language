use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use sha2::{Digest, Sha256};
use skiff_artifact_identity::{package_artifact_ref, service_contract_ref};
use skiff_artifact_model::{
    ActivationPolicy, ConfigLiteralBinding, DeploymentDiagnosticText, DeploymentIngressBinding,
    DeploymentPolicy, DeploymentRevision, IngressProtocol, IngressSelector, MetadataValue,
    PackageArtifact, PackageBinding, PackageRequirementKey, ResourceBinding, ResourcePolicy,
    RuntimeCapabilityBinding, SecretRefBinding, ServiceConfigProfileAuthoring, ServiceContractRef,
    ServiceDeployment, ServiceDeploymentInput, ServiceDeploymentOperationInput,
    ServiceManifestAuthoring, ServiceRequirementKey, ServiceSelectorBinding, StateBinding,
    StateBindingKind, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler_contract::ServiceApiProjection;
use skiff_deployment::projection::{project_service_deployment, ProjectionError};
use thiserror::Error;

/// Exact typed inputs used to generate one deployment. There is deliberately no
/// `deployment.yml` or manually-authored operation map in this seam.
pub struct GeneratedServiceDeploymentInput<'a> {
    pub service: &'a ServiceManifestAuthoring,
    pub profile_name: &'a str,
    pub profile: &'a ServiceConfigProfileAuthoring,
    pub service_api: &'a ServiceApiProjection,
    pub implementation: &'a PackageArtifact,
    pub package_closure: &'a [PackageArtifact],
}

#[derive(Debug, Error)]
pub enum GeneratedServiceDeploymentError {
    #[error("generated deployment input is inconsistent: {0}")]
    InvalidInput(String),
    #[error("service manifest field {field} is invalid: {message}")]
    InvalidManifest {
        field: &'static str,
        message: String,
    },
    #[error("config profile field {field} is invalid: {message}")]
    InvalidProfile {
        field: &'static str,
        message: String,
    },
    #[error(transparent)]
    Identity(#[from] skiff_artifact_identity::ArtifactIdentityError),
    #[error(transparent)]
    Projection(#[from] ProjectionError),
}

pub fn generate_service_deployment(
    input: GeneratedServiceDeploymentInput<'_>,
) -> Result<ServiceDeployment, GeneratedServiceDeploymentError> {
    validate_exact_api(&input)?;
    let typed = ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract: service_contract_ref(&input.service_api.contract)?,
        deployment_revision: generated_revision(&input)?,
        implementation: package_artifact_ref(input.implementation)?,
        operation_bindings: operation_bindings(&input)?,
        package_bindings: package_bindings(&input)?,
        service_selectors: service_selectors(&input),
        ingress: ingress_bindings(&input)?,
        config_literals: keyed_values("config", &input.profile.config)?
            .into_iter()
            .map(|(path, value)| ConfigLiteralBinding {
                path,
                value: MetadataValue::from_json(value),
            })
            .collect(),
        secret_refs: keyed_strings("secrets", &input.profile.secrets)?
            .into_iter()
            .map(|(path, secret_ref)| SecretRefBinding { path, secret_ref })
            .collect(),
        state_bindings: state_bindings(&input.profile.state)?,
        resource_bindings: resource_bindings(&input.profile.resources)?,
        runtime_capability_bindings: runtime_capability_bindings(&input),
        policy: deployment_policy(input.profile)?,
        diagnostic_text: DeploymentDiagnosticText {
            display_name: format!(
                "{}@{} ({})",
                input.service.id, input.implementation.package_version, input.profile_name
            ),
            notes: BTreeMap::new(),
        },
    };
    let mut artifacts = input.package_closure.to_vec();
    if !artifacts
        .iter()
        .any(|candidate| candidate.package_build_id == input.implementation.package_build_id)
    {
        artifacts.push(input.implementation.clone());
    }
    Ok(project_service_deployment(
        typed,
        &input.service_api.contract,
        &artifacts,
    )?)
}

fn validate_exact_api(
    input: &GeneratedServiceDeploymentInput<'_>,
) -> Result<(), GeneratedServiceDeploymentError> {
    let contract = &input.service_api.contract;
    if contract.service_id != input.service.id {
        return Err(invalid(format!(
            "service.yml id {} does not match ServiceContract id {}",
            input.service.id, contract.service_id
        )));
    }
    if contract.contract_version != input.implementation.package_version {
        return Err(invalid(format!(
            "package version label {} does not match ServiceContract label {}",
            input.implementation.package_version, contract.contract_version
        )));
    }
    let descriptor_paths = contract
        .operations
        .values()
        .map(|descriptor| descriptor.stable_key.as_str())
        .collect::<BTreeSet<_>>();
    let available_paths = input
        .service_api
        .available
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if descriptor_paths != available_paths {
        return Err(invalid(
            "automatic service API operation map is missing or contains an extra mapping",
        ));
    }
    let unique_callables = input
        .service_api
        .available
        .values()
        .collect::<BTreeSet<_>>();
    if unique_callables.len() != input.service_api.available.len() {
        return Err(invalid(
            "automatic service API has a duplicate source callable mapping",
        ));
    }
    Ok(())
}

fn operation_bindings(
    input: &GeneratedServiceDeploymentInput<'_>,
) -> Result<Vec<ServiceDeploymentOperationInput>, GeneratedServiceDeploymentError> {
    input
        .service_api
        .contract
        .operations
        .iter()
        .map(|(operation_id, descriptor)| {
            let callable_id = input
                .service_api
                .available
                .get(&descriptor.stable_key)
                .ok_or_else(|| invalid("automatic operation mapping is missing"))?;
            let public_path = input
                .implementation
                .package_local_abi
                .public_symbols
                .iter()
                .find_map(|(path, symbol)| match symbol {
                    skiff_artifact_model::PackageLocalAbiSymbol::Callable {
                        callable_id: actual,
                        ..
                    } if actual == callable_id => Some(path.clone()),
                    _ => None,
                })
                .ok_or_else(|| {
                    invalid(format!(
                        "source callable {callable_id} is absent from implementation API"
                    ))
                })?;
            Ok(ServiceDeploymentOperationInput {
                contract_operation_id: operation_id.clone(),
                package_public_path: public_path,
            })
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouteAuthoring {
    #[serde(default = "default_host")]
    host: String,
    #[serde(default)]
    method: Option<String>,
    path: String,
    operation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutesAuthoring {
    routes: Vec<RouteAuthoring>,
}

fn default_host() -> String {
    "*".to_string()
}

fn ingress_bindings(
    input: &GeneratedServiceDeploymentInput<'_>,
) -> Result<Vec<DeploymentIngressBinding>, GeneratedServiceDeploymentError> {
    let mut result = Vec::new();
    for (field, protocol, value) in [
        ("http", IngressProtocol::Http, input.service.http.as_ref()),
        (
            "websocket",
            IngressProtocol::WebSocket,
            input.service.websocket.as_ref(),
        ),
    ] {
        let Some(value) = value else { continue };
        let routes: RoutesAuthoring = serde_json::from_value(value.clone()).map_err(|error| {
            GeneratedServiceDeploymentError::InvalidManifest {
                field,
                message: error.to_string(),
            }
        })?;
        for route in routes.routes {
            result.push(resolve_route(input, protocol, route)?);
        }
    }
    Ok(result)
}

fn resolve_route(
    input: &GeneratedServiceDeploymentInput<'_>,
    protocol: IngressProtocol,
    route: RouteAuthoring,
) -> Result<DeploymentIngressBinding, GeneratedServiceDeploymentError> {
    if input.service_api.unavailable.contains_key(&route.operation) {
        return Err(invalid(format!(
            "ingress operation {} is boundary unavailable",
            route.operation
        )));
    }
    let descriptor = input
        .service_api
        .contract
        .operations
        .values()
        .find(|descriptor| descriptor.stable_key == route.operation)
        .ok_or_else(|| {
            invalid(format!(
                "ingress operation {} is not an Available service API operation",
                route.operation
            ))
        })?;
    Ok(DeploymentIngressBinding {
        selector: IngressSelector {
            protocol,
            host: route.host,
            method: route.method,
            path: route.path,
        },
        contract_operation_id: descriptor.operation_id.clone(),
    })
}

fn package_bindings(
    input: &GeneratedServiceDeploymentInput<'_>,
) -> Result<Vec<PackageBinding>, GeneratedServiceDeploymentError> {
    input
        .package_closure
        .iter()
        .chain(std::iter::once(input.implementation))
        .flat_map(|caller| {
            caller
                .package_requirements
                .iter()
                .map(move |requirement| (caller, requirement))
        })
        .map(|(caller, requirement)| {
            let package = input
                .package_closure
                .iter()
                .chain(std::iter::once(input.implementation))
                .find(|candidate| {
                    candidate.package_id == requirement.package_id
                        && candidate.package_version == requirement.exact_version
                        && candidate.package_local_abi.local_abi_identity
                            == requirement.expected_local_abi
                })
                .ok_or_else(|| {
                    invalid(format!(
                        "exact package requirement {}@{} is unbound",
                        requirement.package_id, requirement.exact_version
                    ))
                })?;
            Ok(PackageBinding {
                key: PackageRequirementKey {
                    caller_package_build_id: caller.package_build_id.clone(),
                    package_requirement_alias: requirement.alias.clone(),
                },
                package: package_artifact_ref(package)?,
            })
        })
        .collect()
}

fn service_selectors(input: &GeneratedServiceDeploymentInput<'_>) -> Vec<ServiceSelectorBinding> {
    input
        .package_closure
        .iter()
        .chain(std::iter::once(input.implementation))
        .flat_map(|caller| {
            caller
                .service_requirements
                .iter()
                .map(move |requirement| (caller, requirement))
        })
        .map(|(caller, requirement)| ServiceSelectorBinding {
            key: ServiceRequirementKey {
                caller_package_build_id: caller.package_build_id.clone(),
                service_requirement_slot: requirement.service_binding_slot,
            },
            contract: ServiceContractRef {
                service_id: requirement.contract_requirement.service_id.clone(),
                contract_version: requirement.contract_requirement.contract_version.clone(),
                service_protocol_identity: requirement
                    .contract_requirement
                    .expected_protocol_identity
                    .clone(),
            },
        })
        .collect()
}

fn runtime_capability_bindings(
    input: &GeneratedServiceDeploymentInput<'_>,
) -> Vec<RuntimeCapabilityBinding> {
    input
        .package_closure
        .iter()
        .chain(std::iter::once(input.implementation))
        .flat_map(|package| &package.runtime_requirements.runtime_capabilities)
        .map(|requirement| {
            (
                requirement.capability.clone(),
                requirement.required_version.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .map(|(capability, version)| RuntimeCapabilityBinding {
            capability,
            version,
        })
        .collect()
}

fn keyed_values(
    field: &'static str,
    value: &serde_json::Value,
) -> Result<BTreeMap<String, serde_json::Value>, GeneratedServiceDeploymentError> {
    keyed_typed(field, value)
}

fn keyed_strings(
    field: &'static str,
    value: &serde_json::Value,
) -> Result<BTreeMap<String, String>, GeneratedServiceDeploymentError> {
    keyed_typed(field, value)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StateAuthoring {
    kind: StateBindingKind,
    namespace: String,
}

fn state_bindings(
    value: &serde_json::Value,
) -> Result<Vec<StateBinding>, GeneratedServiceDeploymentError> {
    let values: BTreeMap<String, StateAuthoring> = keyed_typed("state", value)?;
    Ok(values
        .into_iter()
        .map(|(requirement_key, binding)| StateBinding {
            requirement_key,
            kind: binding.kind,
            namespace: binding.namespace,
        })
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceAuthoring {
    capability: String,
    resource_ref: String,
}

fn resource_bindings(
    value: &serde_json::Value,
) -> Result<Vec<ResourceBinding>, GeneratedServiceDeploymentError> {
    let values: BTreeMap<String, ResourceAuthoring> = keyed_typed("resources", value)?;
    Ok(values
        .into_iter()
        .map(|(requirement_key, binding)| ResourceBinding {
            requirement_key,
            capability: binding.capability,
            resource_ref: binding.resource_ref,
        })
        .collect())
}

fn keyed_typed<T: for<'de> Deserialize<'de>>(
    field: &'static str,
    value: &serde_json::Value,
) -> Result<BTreeMap<String, T>, GeneratedServiceDeploymentError> {
    if value.is_null() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_value(value.clone()).map_err(|error| {
        GeneratedServiceDeploymentError::InvalidProfile {
            field,
            message: error.to_string(),
        }
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QuotaAuthoring {
    cpu_millis: u32,
    memory_bytes: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LifecycleAuthoring {
    max_concurrency: u32,
    #[serde(default)]
    idle_timeout_ms: Option<u64>,
}

fn deployment_policy(
    profile: &ServiceConfigProfileAuthoring,
) -> Result<DeploymentPolicy, GeneratedServiceDeploymentError> {
    let timeout_ms: u64 = profile_field("timeout", &profile.timeout)?;
    let quota: QuotaAuthoring = profile_field("quota", &profile.quota)?;
    let lifecycle: LifecycleAuthoring = profile_field("lifecycle", &profile.lifecycle)?;
    let principal: String = profile_field("principal", &profile.principal)?;
    Ok(DeploymentPolicy {
        timeout_ms,
        resources: ResourcePolicy {
            cpu_millis: quota.cpu_millis,
            memory_bytes: quota.memory_bytes,
        },
        activation: ActivationPolicy {
            max_concurrency: lifecycle.max_concurrency,
            idle_timeout_ms: lifecycle.idle_timeout_ms,
        },
        principal,
    })
}

fn profile_field<T: for<'de> Deserialize<'de>>(
    field: &'static str,
    value: &serde_json::Value,
) -> Result<T, GeneratedServiceDeploymentError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        GeneratedServiceDeploymentError::InvalidProfile {
            field,
            message: error.to_string(),
        }
    })
}

fn generated_revision(
    input: &GeneratedServiceDeploymentInput<'_>,
) -> Result<DeploymentRevision, GeneratedServiceDeploymentError> {
    let bytes = skiff_canonical_json::canonical_json_bytes(&(
        &input.service.id,
        input.profile_name,
        &input.implementation.package_build_id,
        input.profile,
        input.service,
    ))
    .map_err(|error| invalid(error.to_string()))?;
    Ok(DeploymentRevision::new(format!(
        "sha256:{}",
        hex::encode(Sha256::digest(bytes))
    )))
}

fn invalid(message: impl Into<String>) -> GeneratedServiceDeploymentError {
    GeneratedServiceDeploymentError::InvalidInput(message.into())
}
