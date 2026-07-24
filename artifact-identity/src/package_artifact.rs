use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use skiff_artifact_model::{
    BoundaryCallableProjection, CallableSemanticFacts, PackageArtifact, PackageArtifactRef,
    PackageBuildId, PackageCallableId, PackageLocalAbiIdentity, PackageLocalAbiSymbol,
    PackageRuntimeRequirements, PackageSchemaIndexRef, PackageSchemaTypeId,
    PackageSchemaTypeRecordRef, ServiceCallRef,
};

use crate::{
    package::projection::{
        implementation_links::{
            OperationTargetIdentityProjection, PackageImplementationLinksIdentityProjection,
        },
        FileIrOwnerIdentityProjection,
    },
    ArtifactIdentityError, Result,
};

mod projection;
mod validation;

/// Complete canonical preimage of a package-local public ABI identity.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageArtifactLocalAbiIdentityProjection {
    schema: &'static str,
    package_id: String,
    public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
}

/// Complete canonical preimage of a package artifact build identity.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageArtifactBuildIdentityProjection {
    schema: &'static str,
    package_id: String,
    local_abi_identity: PackageLocalAbiIdentity,
    package_schema_index: PackageSchemaIndexRef,
    package_schema_type_records: BTreeMap<PackageSchemaTypeId, PackageSchemaTypeRecordRef>,
    files: Vec<FileIrOwnerIdentityProjection>,
    static_resources: Vec<ResourceIdentityProjection>,
    implementation_links: PackageImplementationLinksIdentityProjection,
    callable_links: BTreeMap<PackageCallableId, CallableLinkIdentityProjection>,
    package_requirements: Value,
    contract_requirements: Value,
    service_requirements: Value,
    runtime_requirements: PackageRuntimeRequirements,
    callable_semantic_facts: BTreeMap<PackageCallableId, CallableSemanticFacts>,
    boundary_projections: BTreeMap<PackageCallableId, BoundaryCallableProjection>,
    service_call_refs: Vec<ServiceCallRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceIdentityProjection {
    path: String,
    sha256: String,
    byte_len: u64,
    content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CallableLinkIdentityProjection {
    callable_id: PackageCallableId,
    target: OperationTargetIdentityProjection,
}

pub fn package_artifact_local_abi_identity_projection(
    artifact: &PackageArtifact,
) -> Result<PackageArtifactLocalAbiIdentityProjection> {
    validation::validate_package_artifact_surface(artifact)?;
    Ok(projection::local_abi_projection(artifact))
}

pub fn package_artifact_local_abi_identity(
    artifact: &PackageArtifact,
) -> Result<PackageLocalAbiIdentity> {
    let projection = package_artifact_local_abi_identity_projection(artifact)?;
    projection::local_abi_identity_from_projection(&projection)
}

pub fn package_artifact_build_identity_projection(
    artifact: &PackageArtifact,
) -> Result<PackageArtifactBuildIdentityProjection> {
    validation::validate_package_artifact_surface(artifact)?;
    let local_abi_identity = projection::local_abi_identity_from_validated(artifact)?;
    projection::build_projection_from_validated(artifact, local_abi_identity)
}

pub fn package_artifact_build_identity(artifact: &PackageArtifact) -> Result<PackageBuildId> {
    let projection = package_artifact_build_identity_projection(artifact)?;
    projection::build_identity_from_projection(&projection)
}

pub fn assign_package_artifact_identities(
    artifact: &mut PackageArtifact,
) -> Result<(PackageBuildId, PackageLocalAbiIdentity)> {
    validation::validate_package_artifact_surface(artifact)?;
    let local_abi_identity = projection::local_abi_identity_from_validated(artifact)?;
    artifact.package_local_abi.local_abi_identity = local_abi_identity.clone();
    let build_projection =
        projection::build_projection_from_validated(artifact, local_abi_identity.clone())?;
    let build_identity = projection::build_identity_from_projection(&build_projection)?;
    artifact.package_build_id = build_identity.clone();
    validate_package_artifact_identities(artifact)?;
    Ok((build_identity, local_abi_identity))
}

pub fn validate_package_artifact_identities(artifact: &PackageArtifact) -> Result<()> {
    validation::validate_package_artifact_surface(artifact)?;
    let computed_local = projection::local_abi_identity_from_validated(artifact)?;
    if artifact.package_local_abi.local_abi_identity != computed_local {
        return Err(
            ArtifactIdentityError::PackageArtifactLocalAbiIdentityMismatch {
                declared: artifact.package_local_abi.local_abi_identity.to_string(),
                computed: computed_local.to_string(),
            },
        );
    }
    let build_projection =
        projection::build_projection_from_validated(artifact, computed_local.clone())?;
    let computed_build = projection::build_identity_from_projection(&build_projection)?;
    if artifact.package_build_id != computed_build {
        return Err(
            ArtifactIdentityError::PackageArtifactBuildIdentityMismatch {
                declared: artifact.package_build_id.to_string(),
                computed: computed_build.to_string(),
            },
        );
    }
    Ok(())
}

pub fn package_artifact_ref(artifact: &PackageArtifact) -> Result<PackageArtifactRef> {
    validate_package_artifact_identities(artifact)?;
    Ok(PackageArtifactRef {
        package_id: artifact.package_id.clone(),
        package_version: artifact.package_version.clone(),
        package_build_id: artifact.package_build_id.clone(),
        package_local_abi_identity: artifact.package_local_abi.local_abi_identity.clone(),
    })
}

fn invalid_artifact<T>(message: impl Into<String>) -> Result<T> {
    Err(ArtifactIdentityError::InvalidPackageArtifact {
        message: message.into(),
    })
}
