//! Source-free service deployment projection.
//!
//! The trust boundary consumes exact callable identities from typed input and
//! emits only typed, canonical artifact references and callable identities.

mod error;
mod operations;
mod package_closure;
mod requirements;

pub mod actor_routing;

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::ValidatedPackageArtifact;
use skiff_artifact_model::{
    validate_package_boundary_projections, DeploymentArtifactIdentity, PackageArtifact,
    PackageSchemaTypeId, PackageSchemaTypeRecord, ServiceContract, ServiceDeployment,
    ServiceDeploymentInput, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};

pub use error::{ProjectionError, ProjectionResult};

/// Project one deployment from exact, already-typed contract and package artifacts.
pub fn project_service_deployment(
    input: ServiceDeploymentInput,
    contract: &ServiceContract,
    package_artifacts: &[PackageArtifact],
    package_schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> ProjectionResult<ServiceDeployment> {
    validate_contract_ref(&input, contract)?;
    validate_package_schema_records(contract, package_schema_records)?;
    let closure = package_closure::PackageClosure::resolve(&input, package_artifacts)?;
    for artifact in closure.artifacts() {
        validate_package_boundary_projections(artifact).map_err(|source| {
            ProjectionError::InvalidPackageBoundaryProjections {
                build_id: artifact.package_build_id.clone(),
                source,
            }
        })?;
    }
    project_service_deployment_after_package_validation(input, contract, closure)
}

/// Projects from opaque, exact PackageArtifact admissions.
///
/// The raw slice is retained only to preserve the existing projection model;
/// every element must exactly match the corresponding private admission.
pub fn project_service_deployment_with_validated_packages(
    input: ServiceDeploymentInput,
    contract: &ServiceContract,
    package_artifacts: &[PackageArtifact],
    package_schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
    validated_packages: &[ValidatedPackageArtifact],
) -> ProjectionResult<ServiceDeployment> {
    validate_exact_package_admissions(package_artifacts, validated_packages)?;
    validate_contract_ref(&input, contract)?;
    validate_package_schema_records(contract, package_schema_records)?;
    let closure =
        package_closure::PackageClosure::resolve_after_validation(&input, package_artifacts)?;
    project_service_deployment_after_package_validation(input, contract, closure)
}

fn project_service_deployment_after_package_validation(
    input: ServiceDeploymentInput,
    contract: &ServiceContract,
    closure: package_closure::PackageClosure<'_>,
) -> ProjectionResult<ServiceDeployment> {
    let operations =
        operations::project_operation_bindings(&input, contract, closure.implementation(&input))?;
    requirements::validate_requirement_bindings(&input, &closure, &operations.selected)?;
    skiff_artifact_identity::validate_service_deployment_input(&input).map_err(|error| {
        ProjectionError::InvalidTypedArtifact {
            artifact: "ServiceDeploymentInput",
            identity_error: Box::new(error),
        }
    })?;

    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: input.contract,
        deployment_revision: input.deployment_revision,
        deployment_artifact_identity: DeploymentArtifactIdentity::new("unassigned"),
        implementation: input.implementation,
        operation_bindings: operations.bindings,
        package_bindings: input.package_bindings,
        service_selectors: input.service_selectors,
        gateway_entries: input.gateway_entries,
        ingress: input.ingress,
        diagnostic_text: input.diagnostic_text,
    };
    normalize_deployment(&mut deployment);
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)?;
    skiff_artifact_identity::validate_service_deployment_identity(&deployment)?;
    Ok(deployment)
}

fn validate_exact_package_admissions(
    package_artifacts: &[PackageArtifact],
    validated_packages: &[ValidatedPackageArtifact],
) -> ProjectionResult<()> {
    if package_artifacts.len() != validated_packages.len()
        || package_artifacts
            .iter()
            .zip(validated_packages)
            .any(|(artifact, validated)| !validated.exactly_matches(artifact))
    {
        return Err(ProjectionError::ValidatedPackageAdmissionMismatch);
    }
    Ok(())
}

fn validate_package_schema_records(
    contract: &ServiceContract,
    records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> ProjectionResult<()> {
    skiff_artifact_identity::validate_package_schema_records(records).map_err(
        |identity_error| ProjectionError::InvalidTypedArtifact {
            artifact: "PackageSchemaTypeRecord closure",
            identity_error: Box::new(identity_error),
        },
    )?;

    let mut required = BTreeMap::new();
    for requirement in &contract.package_type_requirements {
        for type_id in &requirement.required_type_ids {
            required.insert(type_id.clone(), requirement.package_id.as_str());
        }
    }
    let actual = records.keys().cloned().collect::<BTreeSet<_>>();
    let expected = required.keys().cloned().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(ProjectionError::PackageSchemaClosureMismatch {
            missing: expected.difference(&actual).cloned().collect(),
            extra: actual.difference(&expected).cloned().collect(),
        });
    }
    for (type_id, record) in records {
        let expected_owner = required[type_id];
        if record.package_id != expected_owner {
            return Err(ProjectionError::PackageSchemaOwnerMismatch {
                type_id: type_id.clone(),
                expected: expected_owner.to_string(),
                actual: record.package_id.clone(),
            });
        }
    }
    Ok(())
}

fn validate_contract_ref(
    input: &ServiceDeploymentInput,
    contract: &ServiceContract,
) -> ProjectionResult<()> {
    skiff_artifact_identity::validate_service_contract_identities(contract).map_err(|error| {
        ProjectionError::InvalidTypedArtifact {
            artifact: "ServiceContract",
            identity_error: Box::new(error),
        }
    })?;
    for (field, expected, actual) in [
        (
            "serviceId",
            contract.service_id.as_str(),
            input.contract.service_id.as_str(),
        ),
        (
            "contractVersion",
            contract.contract_version.as_str(),
            input.contract.contract_version.as_str(),
        ),
        (
            "serviceProtocolIdentity",
            contract.service_protocol_identity.as_str(),
            input.contract.service_protocol_identity.as_str(),
        ),
    ] {
        if expected != actual {
            return Err(ProjectionError::ContractReferenceMismatch {
                field,
                expected: expected.to_string(),
                actual: actual.to_string(),
            });
        }
    }
    Ok(())
}

fn normalize_deployment(deployment: &mut ServiceDeployment) {
    deployment
        .operation_bindings
        .sort_by(|left, right| left.contract_operation_id.cmp(&right.contract_operation_id));
    deployment
        .package_bindings
        .sort_by(|left, right| left.key.cmp(&right.key));
    deployment
        .service_selectors
        .sort_by(|left, right| left.key.cmp(&right.key));
    deployment
        .ingress
        .sort_by(|left, right| left.selector.cmp(&right.selector));
}

#[cfg(test)]
mod tests;
