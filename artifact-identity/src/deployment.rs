use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentGatewayEntry, DeploymentIngressBinding,
    DeploymentOperationBinding, DeploymentPolicy, DeploymentRevision, GatewayEntryKey,
    ResourceBinding, RuntimeCapabilityBinding, ServiceDeployment, ServiceDeploymentRef,
};

mod normalization;
mod validation;

pub(crate) use validation::{
    validate_contract_ref, validate_deployment_ref_shape, validate_package_ref,
};
pub use validation::{validate_service_deployment_input, validate_service_deployment_surface};

use crate::{
    framing::{canonical_ir_bytes, sha256_hex},
    ArtifactIdentityError, Result, DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
    DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER,
};

/// Complete canonical preimage of `DeploymentArtifactIdentity`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentArtifactIdentityProjection {
    schema: &'static str,
    contract: Value,
    deployment_revision: DeploymentRevision,
    implementation: Value,
    operation_bindings: Vec<DeploymentOperationBinding>,
    package_bindings: Value,
    service_selectors: Value,
    gateway_entries: BTreeMap<GatewayEntryKey, DeploymentGatewayEntry>,
    ingress: Vec<DeploymentIngressBinding>,
    resource_bindings: Vec<ResourceBinding>,
    runtime_capability_bindings: Vec<RuntimeCapabilityBinding>,
    policy: DeploymentPolicy,
}

/// Return the canonical identity preimage. Diagnostic text and the declared identity are excluded.
pub fn service_deployment_identity_projection(
    deployment: &ServiceDeployment,
) -> Result<DeploymentArtifactIdentityProjection> {
    validate_service_deployment_surface(deployment)?;
    let mut projection = DeploymentArtifactIdentityProjection {
        schema: DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER,
        contract: crate::identity_labels::without_human_version_labels(
            &deployment.contract,
            ArtifactIdentityError::SerializeDeploymentArtifactIdentity,
        )?,
        deployment_revision: deployment.deployment_revision.clone(),
        implementation: crate::identity_labels::without_human_version_labels(
            &deployment.implementation,
            ArtifactIdentityError::SerializeDeploymentArtifactIdentity,
        )?,
        operation_bindings: deployment.operation_bindings.clone(),
        package_bindings: crate::identity_labels::without_human_version_labels(
            &deployment.package_bindings,
            ArtifactIdentityError::SerializeDeploymentArtifactIdentity,
        )?,
        service_selectors: crate::identity_labels::without_human_version_labels(
            &deployment.service_selectors,
            ArtifactIdentityError::SerializeDeploymentArtifactIdentity,
        )?,
        gateway_entries: deployment.gateway_entries.clone(),
        ingress: deployment.ingress.clone(),
        resource_bindings: deployment.resource_bindings.clone(),
        runtime_capability_bindings: deployment.runtime_capability_bindings.clone(),
        policy: deployment.policy.clone(),
    };
    normalization::normalize_projection(&mut projection);
    Ok(projection)
}

pub fn service_deployment_identity(
    deployment: &ServiceDeployment,
) -> Result<DeploymentArtifactIdentity> {
    let projection = service_deployment_identity_projection(deployment)?;
    let bytes = canonical_ir_bytes(
        &projection,
        ArtifactIdentityError::SerializeDeploymentArtifactIdentity,
    )?;
    Ok(DeploymentArtifactIdentity::new(crate::framed_identity(
        DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

pub fn assign_service_deployment_identity(
    deployment: &mut ServiceDeployment,
) -> Result<DeploymentArtifactIdentity> {
    let identity = service_deployment_identity(deployment)?;
    deployment.deployment_artifact_identity = identity.clone();
    validate_service_deployment_identity(deployment)?;
    Ok(identity)
}

pub fn validate_service_deployment_identity(deployment: &ServiceDeployment) -> Result<()> {
    let computed = service_deployment_identity(deployment)?;
    if deployment.deployment_artifact_identity != computed {
        return Err(ArtifactIdentityError::DeploymentArtifactIdentityMismatch {
            declared: deployment.deployment_artifact_identity.to_string(),
            computed: computed.to_string(),
        });
    }
    Ok(())
}

pub fn service_deployment_ref(deployment: &ServiceDeployment) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.contract.service_id.clone(),
        contract_version: deployment.contract.contract_version.clone(),
        deployment_revision: deployment.deployment_revision.clone(),
        deployment_artifact_identity: deployment.deployment_artifact_identity.clone(),
    }
}

pub fn validate_service_deployment_ref(
    reference: &ServiceDeploymentRef,
    deployment: &ServiceDeployment,
) -> Result<()> {
    validate_service_deployment_identity(deployment)?;
    let expected = service_deployment_ref(deployment);
    if reference != &expected {
        return Err(ArtifactIdentityError::ServiceDeploymentRefMismatch {
            message: format!("declared {reference:?}, expected {expected:?}"),
        });
    }
    Ok(())
}
