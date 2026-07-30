use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};
use skiff_artifact_identity::{service_contract_ref, ValidatedPackageArtifact};
use skiff_artifact_model::{
    DeploymentDiagnosticText, DeploymentRevision, HttpGatewayDocumentAuthoring, PackageArtifact,
    PackageBinding, PackageRequirementKey, PackageSchemaTypeId, PackageSchemaTypeRecord,
    ServiceContractRef, ServiceDeployment, ServiceDeploymentInput, ServiceDeploymentOperationInput,
    ServiceManifestAuthoring, ServiceRequirementKey, ServiceSelectorBinding,
    WebSocketGatewayDocumentAuthoring, SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION,
};
use skiff_compiler_contract::ServiceApiProjection;
use skiff_deployment::projection::{
    project_service_deployment_with_validated_packages, ProjectionError,
};
use thiserror::Error;

use crate::http_gateway_projection::{
    project_http_gateway_after_package_validation, HttpGatewayProjectionError, ProjectedHttpGateway,
};
use crate::websocket_gateway_projection::{
    project_websocket_gateway_after_package_validation, ProjectedWebSocketGateway,
    WebSocketGatewayProjectionError,
};

/// Exact typed inputs used to generate one deployment. There is deliberately no
/// `deployment.yml` or manually-authored operation map in this seam.
pub struct GeneratedServiceDeploymentInput<'a> {
    pub service: &'a ServiceManifestAuthoring,
    pub http: Option<&'a HttpGatewayDocumentAuthoring>,
    pub websocket: Option<&'a WebSocketGatewayDocumentAuthoring>,
    pub service_api: &'a ServiceApiProjection,
    pub implementation: &'a PackageArtifact,
    pub package_closure: &'a [PackageArtifact],
    pub package_schema_records: &'a BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
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
    #[error("generated deployment artifact identity is invalid: {message}")]
    Identity { message: String },
    #[error("generated deployment projection failed: {message}")]
    Projection { message: String },
    #[error(transparent)]
    HttpGateway(#[from] HttpGatewayProjectionError),
    #[error(transparent)]
    WebSocketGateway(#[from] WebSocketGatewayProjectionError),
}

pub fn generate_service_deployment(
    input: GeneratedServiceDeploymentInput<'_>,
) -> Result<ServiceDeployment, GeneratedServiceDeploymentError> {
    let implementation =
        ValidatedPackageArtifact::admit_clone(input.implementation).map_err(identity_error)?;
    let package_closure = input
        .package_closure
        .iter()
        .map(ValidatedPackageArtifact::admit_clone)
        .collect::<Result<Vec<_>, _>>()
        .map_err(identity_error)?;
    generate_service_deployment_with_validated_packages(input, &implementation, &package_closure)
}

pub fn generate_service_deployment_with_validated_packages(
    input: GeneratedServiceDeploymentInput<'_>,
    implementation: &ValidatedPackageArtifact,
    package_closure: &[ValidatedPackageArtifact],
) -> Result<ServiceDeployment, GeneratedServiceDeploymentError> {
    validate_exact_package_admissions(&input, implementation, package_closure)?;
    validate_exact_api(&input)?;
    if input.http.is_some() || input.websocket.is_some() {
        skiff_artifact_identity::validate_package_schema_records(input.package_schema_records)
            .map_err(identity_error)?;
    }
    let ProjectedHttpGateway {
        mut gateway_entries,
        mut ingress,
    } = project_http_gateway_after_package_validation(
        input.http,
        input.implementation,
        input.package_closure,
        input.package_schema_records,
    )?;
    let ProjectedWebSocketGateway {
        gateway_entries: websocket_entries,
        ingress: websocket_ingress,
    } = project_websocket_gateway_after_package_validation(
        input.websocket,
        input.implementation,
        input.package_closure,
        input.package_schema_records,
    )?;
    for (key, entry) in websocket_entries {
        if gateway_entries.insert(key.clone(), entry).is_some() {
            return Err(GeneratedServiceDeploymentError::InvalidManifest {
                field: "websocket",
                message: format!(
                    "gateway entry key {key} is declared by both http.yml and websocket.yml"
                ),
            });
        }
    }
    ingress.extend(websocket_ingress);
    let typed = ServiceDeploymentInput {
        schema_version: SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION.to_string(),
        contract: service_contract_ref(&input.service_api.contract).map_err(identity_error)?,
        deployment_revision: generated_revision(&input)?,
        implementation: implementation.reference().clone(),
        operation_bindings: operation_bindings(&input)?,
        package_bindings: package_bindings(&input, implementation, package_closure)?,
        service_selectors: service_selectors(&input),
        gateway_entries,
        ingress,
        diagnostic_text: DeploymentDiagnosticText {
            display_name: format!(
                "{}@{}",
                input.service.id, input.implementation.package_version
            ),
            notes: BTreeMap::new(),
        },
    };
    let mut artifacts = input.package_closure.to_vec();
    let mut validated_artifacts = package_closure.to_vec();
    if !artifacts
        .iter()
        .any(|candidate| candidate.package_build_id == input.implementation.package_build_id)
    {
        artifacts.push(input.implementation.clone());
        validated_artifacts.push(implementation.clone());
    }
    let contract_schema_records =
        contract_package_schema_records(&input, input.package_schema_records)?;
    Ok(project_service_deployment_with_validated_packages(
        typed,
        &input.service_api.contract,
        &artifacts,
        &contract_schema_records,
        &validated_artifacts,
    )
    .map_err(projection_error)?)
}

fn validate_exact_package_admissions(
    input: &GeneratedServiceDeploymentInput<'_>,
    implementation: &ValidatedPackageArtifact,
    package_closure: &[ValidatedPackageArtifact],
) -> Result<(), GeneratedServiceDeploymentError> {
    if !implementation.exactly_matches(input.implementation) {
        return Err(invalid(
            "validated implementation PackageArtifact does not match the generated deployment input",
        ));
    }
    if package_closure.len() != input.package_closure.len()
        || package_closure
            .iter()
            .zip(input.package_closure)
            .any(|(validated, artifact)| !validated.exactly_matches(artifact))
    {
        return Err(invalid(
            "validated package closure does not exactly match the generated deployment input",
        ));
    }
    Ok(())
}

fn contract_package_schema_records(
    input: &GeneratedServiceDeploymentInput<'_>,
    available: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> Result<BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>, GeneratedServiceDeploymentError>
{
    input
        .service_api
        .contract
        .package_type_requirements
        .iter()
        .flat_map(|requirement| {
            requirement
                .required_type_ids
                .iter()
                .map(move |type_id| (&requirement.package_id, type_id))
        })
        .map(|(expected_owner, type_id)| {
            let record = available.get(type_id).ok_or_else(|| {
                invalid(format!(
                    "service contract requires unavailable Package schema record {type_id}"
                ))
            })?;
            if &record.package_id != expected_owner {
                return Err(invalid(format!(
                    "service contract Package schema record {type_id} expected owner {expected_owner}, got {}",
                    record.package_id
                )));
            }
            Ok((type_id.clone(), record.clone()))
        })
        .collect()
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
    let manifest_service_calls = canonical_service_calls(input.service)?;
    if manifest_service_calls != input.service_api.service_calls {
        return Err(invalid(
            "service manifest serviceCalls do not match the typed service API selection",
        ));
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
            Ok(ServiceDeploymentOperationInput {
                contract_operation_id: operation_id.clone(),
                package_callable_id: callable_id.clone(),
            })
        })
        .collect()
}

fn package_bindings(
    input: &GeneratedServiceDeploymentInput<'_>,
    implementation: &ValidatedPackageArtifact,
    package_closure: &[ValidatedPackageArtifact],
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
                        && requirement
                            .expected_package_build
                            .as_ref()
                            .is_none_or(|expected| expected == &candidate.package_build_id)
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
                package: validated_package_reference(
                    package,
                    input,
                    implementation,
                    package_closure,
                )?
                .clone(),
            })
        })
        .collect()
}

fn validated_package_reference<'a>(
    package: &PackageArtifact,
    input: &GeneratedServiceDeploymentInput<'_>,
    implementation: &'a ValidatedPackageArtifact,
    package_closure: &'a [ValidatedPackageArtifact],
) -> Result<&'a skiff_artifact_model::PackageArtifactRef, GeneratedServiceDeploymentError> {
    if package.package_build_id == input.implementation.package_build_id {
        if implementation.exactly_matches(package) {
            return Ok(implementation.reference());
        }
        return Err(invalid(
            "validated implementation PackageArtifact content changed after admission",
        ));
    }
    package_closure
        .iter()
        .find(|validated| {
            validated.reference().package_build_id == package.package_build_id
                && validated.exactly_matches(package)
        })
        .map(ValidatedPackageArtifact::reference)
        .ok_or_else(|| {
            invalid(format!(
                "package {} has no exact validated admission",
                package.package_build_id
            ))
        })
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

fn generated_revision(
    input: &GeneratedServiceDeploymentInput<'_>,
) -> Result<DeploymentRevision, GeneratedServiceDeploymentError> {
    let mut service = input.service.clone();
    service.service_calls = input.service_api.service_calls.clone();
    let bytes = skiff_canonical_json::canonical_json_bytes(&(
        &service.id,
        &input.implementation.package_build_id,
        &service,
        input.http,
        input.websocket,
    ))
    .map_err(|error| invalid(error.to_string()))?;
    Ok(DeploymentRevision::new(format!(
        "sha256-{}",
        hex::encode(Sha256::digest(bytes))
    )))
}

fn canonical_service_calls(
    service: &ServiceManifestAuthoring,
) -> Result<Vec<String>, GeneratedServiceDeploymentError> {
    let mut paths = service.service_calls.clone();
    paths.sort();
    if paths.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(invalid(
            "serviceCalls contains a duplicate path after canonical sorting",
        ));
    }
    Ok(paths)
}

fn invalid(message: impl Into<String>) -> GeneratedServiceDeploymentError {
    GeneratedServiceDeploymentError::InvalidInput(message.into())
}

fn identity_error(
    error: skiff_artifact_identity::ArtifactIdentityError,
) -> GeneratedServiceDeploymentError {
    GeneratedServiceDeploymentError::Identity {
        message: error.to_string(),
    }
}

fn projection_error(error: ProjectionError) -> GeneratedServiceDeploymentError {
    GeneratedServiceDeploymentError::Projection {
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_service_deployment_authoring_accepts_path_only_websocket() {
        let websocket = serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(
            r#"
path: /chat
"#,
        )
        .unwrap();
        assert_eq!(websocket.path, "/chat");
        assert!(websocket.connect.is_none());
        assert!(websocket.json_rpc.is_empty());
    }

    #[test]
    fn generated_service_deployment_rejects_legacy_websocket_operation_ingress() {
        let error = serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(
            r#"
routes:
  - path: /chat
    operation: receive
"#,
        )
        .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("unknown field `routes`"), "{message}");
    }
}
