//! Source-free service deployment projection.
//!
//! The trust boundary resolves human public paths once, then emits only typed,
//! canonical artifact references and callable identities.

mod eligibility;
mod error;
mod operations;
mod package_closure;
mod requirements;

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    websocket_ingress_context, DeploymentArtifactIdentity, IngressProtocol, PackageArtifact,
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
    validate_ingress_contracts(&input, contract, package_schema_records)?;
    let closure = package_closure::PackageClosure::resolve(&input, package_artifacts)?;
    let operations =
        operations::project_operation_bindings(&input, contract, closure.implementation(&input))?;
    requirements::validate_requirement_bindings(&input, &closure, &operations.selected)?;
    skiff_artifact_identity::validate_service_deployment_input(&input).map_err(|error| {
        ProjectionError::InvalidTypedArtifact {
            artifact: "ServiceDeploymentInput",
            identity_error: error,
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
        ingress: input.ingress,
        config_literals: input.config_literals,
        secret_refs: input.secret_refs,
        state_bindings: input.state_bindings,
        resource_bindings: input.resource_bindings,
        runtime_capability_bindings: input.runtime_capability_bindings,
        policy: input.policy,
        diagnostic_text: input.diagnostic_text,
    };
    normalize_deployment(&mut deployment);
    skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)?;
    skiff_artifact_identity::validate_service_deployment_identity(&deployment)?;
    Ok(deployment)
}

fn validate_ingress_contracts(
    input: &ServiceDeploymentInput,
    contract: &ServiceContract,
    package_schema_records: &BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecord>,
) -> ProjectionResult<()> {
    for binding in &input.ingress {
        if matches!(binding.selector.protocol, IngressProtocol::WebSocket) {
            websocket_ingress_context(
                contract,
                &binding.contract_operation_id,
                package_schema_records,
            )
            .map_err(|error| ProjectionError::InvalidWebSocketIngressContract {
                operation_id: binding.contract_operation_id.clone(),
                message: error.to_string(),
            })?;
        }
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
            identity_error,
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
            identity_error: error,
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
    deployment
        .config_literals
        .sort_by(|left, right| left.path.cmp(&right.path));
    deployment
        .secret_refs
        .sort_by(|left, right| left.path.cmp(&right.path));
    deployment
        .state_bindings
        .sort_by(|left, right| left.requirement_key.cmp(&right.requirement_key));
    deployment
        .resource_bindings
        .sort_by(|left, right| left.requirement_key.cmp(&right.requirement_key));
    deployment
        .runtime_capability_bindings
        .sort_by(|left, right| {
            left.capability
                .cmp(&right.capability)
                .then_with(|| left.version.cmp(&right.version))
        });
}

#[cfg(test)]
mod tests;
