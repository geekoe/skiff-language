use skiff_artifact_model::{
    PackageArtifact, PackageBuildId, PackageCallableLinkFact, PackageLocalAbiIdentity,
    PublicationResourceRef,
};

use crate::{
    framing::{canonical_ir_bytes, framed_identity, sha256_hex},
    ArtifactIdentityError, Result, PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER, PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
    PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER,
};

use super::{
    implementation_links::{
        OperationTargetIdentityProjection, PackageImplementationLinksIdentityProjection,
    },
    CallableLinkIdentityProjection, FileIrOwnerIdentityProjection,
    PackageArtifactBuildIdentityProjection, PackageArtifactLocalAbiIdentityProjection,
    ResourceIdentityProjection,
};

pub(super) fn local_abi_projection(
    artifact: &PackageArtifact,
) -> PackageArtifactLocalAbiIdentityProjection {
    PackageArtifactLocalAbiIdentityProjection {
        schema: PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_SCHEMA_MARKER,
        package_id: artifact.package_id.clone(),
        public_symbols: artifact.package_local_abi.public_symbols.clone(),
    }
}

pub(super) fn local_abi_identity_from_validated(
    artifact: &PackageArtifact,
) -> Result<PackageLocalAbiIdentity> {
    local_abi_identity_from_projection(&local_abi_projection(artifact))
}

pub(super) fn local_abi_identity_from_projection(
    projection: &PackageArtifactLocalAbiIdentityProjection,
) -> Result<PackageLocalAbiIdentity> {
    let bytes = canonical_ir_bytes(
        projection,
        ArtifactIdentityError::SerializePackageArtifactLocalAbiIdentity,
    )?;
    Ok(PackageLocalAbiIdentity::new(framed_identity(
        PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

pub(super) fn build_projection_from_validated(
    artifact: &PackageArtifact,
    local_abi_identity: PackageLocalAbiIdentity,
) -> Result<PackageArtifactBuildIdentityProjection> {
    let mut files = artifact
        .files
        .iter()
        .map(FileIrOwnerIdentityProjection::from_ref)
        .collect::<Vec<_>>();
    files.sort_by(|left, right| {
        (left.file_ir_identity.as_str(), left.module_path.as_str())
            .cmp(&(right.file_ir_identity.as_str(), right.module_path.as_str()))
    });

    let mut static_resources = artifact
        .static_resources
        .iter()
        .map(ResourceIdentityProjection::from_ref)
        .collect::<Vec<_>>();
    static_resources.sort_by(|left, right| {
        (left.path.as_str(), left.sha256.as_str())
            .cmp(&(right.path.as_str(), right.sha256.as_str()))
    });

    let mut package_requirements = artifact.package_requirements.clone();
    package_requirements.sort_by(|left, right| {
        (left.alias.as_str(), left.package_id.as_str())
            .cmp(&(right.alias.as_str(), right.package_id.as_str()))
    });
    let mut contract_requirements = artifact.contract_requirements.clone();
    contract_requirements.sort_by(|left, right| {
        (
            left.alias.as_str(),
            left.service_id.as_str(),
            &left.expected_protocol_identity,
        )
            .cmp(&(
                right.alias.as_str(),
                right.service_id.as_str(),
                &right.expected_protocol_identity,
            ))
    });
    let mut service_requirements = artifact.service_requirements.clone();
    service_requirements.sort_by_key(|requirement| requirement.service_binding_slot);
    let mut service_call_refs = artifact.service_call_refs.clone();
    service_call_refs.sort_by(|left, right| {
        (
            left.service_requirement_slot,
            left.contract_operation_id.as_str(),
        )
            .cmp(&(
                right.service_requirement_slot,
                right.contract_operation_id.as_str(),
            ))
    });
    let mut runtime_requirements = artifact.runtime_requirements.clone();
    runtime_requirements
        .config
        .sort_by(|left, right| left.path.cmp(&right.path));
    runtime_requirements
        .resources
        .sort_by(|left, right| left.key.cmp(&right.key));
    runtime_requirements
        .runtime_capabilities
        .sort_by(|left, right| left.capability.cmp(&right.capability));

    Ok(PackageArtifactBuildIdentityProjection {
        schema: PACKAGE_ARTIFACT_BUILD_IDENTITY_SCHEMA_MARKER,
        package_id: artifact.package_id.clone(),
        local_abi_identity,
        implementation_symbols: artifact.package_local_abi.implementation_symbols.clone(),
        package_schema_index: artifact.package_schema_index.clone(),
        package_schema_type_records: artifact.package_schema_type_records.clone(),
        files,
        static_resources,
        implementation_links: PackageImplementationLinksIdentityProjection::from_links(
            &artifact.implementation_links,
        )?,
        callable_links: artifact
            .callable_links
            .iter()
            .map(|(key, link)| (key.clone(), CallableLinkIdentityProjection::from_fact(link)))
            .collect(),
        package_requirements: crate::identity_labels::without_human_version_labels(
            &package_requirements,
            ArtifactIdentityError::SerializePackageArtifactBuildIdentity,
        )?,
        contract_requirements: crate::identity_labels::without_human_version_labels(
            &contract_requirements,
            ArtifactIdentityError::SerializePackageArtifactBuildIdentity,
        )?,
        service_requirements: crate::identity_labels::without_human_version_labels(
            &service_requirements,
            ArtifactIdentityError::SerializePackageArtifactBuildIdentity,
        )?,
        runtime_requirements,
        callable_semantic_facts: artifact.callable_semantic_facts.clone(),
        boundary_projections: artifact.boundary_projections.clone(),
        service_call_refs,
    })
}

pub(super) fn build_identity_from_projection(
    projection: &PackageArtifactBuildIdentityProjection,
) -> Result<PackageBuildId> {
    let bytes = canonical_ir_bytes(
        projection,
        ArtifactIdentityError::SerializePackageArtifactBuildIdentity,
    )?;
    Ok(PackageBuildId::new(framed_identity(
        PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX,
        &sha256_hex(&bytes),
    )))
}

impl ResourceIdentityProjection {
    fn from_ref(resource: &PublicationResourceRef) -> Self {
        Self {
            path: resource.path.clone(),
            sha256: resource.sha256.clone(),
            byte_len: resource.byte_len,
            content_type: resource.content_type.clone(),
        }
    }
}

impl CallableLinkIdentityProjection {
    fn from_fact(link: &PackageCallableLinkFact) -> Self {
        Self {
            callable_id: link.callable_id.clone(),
            target: OperationTargetIdentityProjection::from_ref(&link.target),
        }
    }
}
