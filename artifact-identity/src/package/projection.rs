mod implementation_links;
mod recoverable;

use serde::Serialize;
use serde_json::{Map, Value};
use skiff_artifact_model::{
    AbiIdentityFacts, CallableEffectFacts, ConfigMetadataFacts, FileIrRef,
    PackageDependencyConstraint, PackageUnit, PublicationResourceRef, RecoverableArtifactMetadata,
};

use self::implementation_links::PackageImplementationLinksIdentityProjection;
use self::recoverable::canonical_recoverable_metadata;
use crate::framing::canonical_ir_bytes;
use crate::{
    ArtifactIdentityError, Result, PACKAGE_BUILD_IDENTITY_SCHEMA_MARKER,
    PACKAGE_LOCAL_ABI_IDENTITY_SCHEMA_MARKER,
};

/// Canonical package local ABI preimage.
///
/// It deliberately contains only the package coordinate, validated public
/// surface identity and nominal ABI facts. Implementation and deployment facts
/// cannot be added accidentally by serializing `PackageUnit` wholesale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageLocalAbiIdentityProjection {
    schema: &'static str,
    package_id: String,
    package_version: String,
    public_surface_identity: String,
    abi_identity_facts: AbiIdentityFacts,
}

impl PackageLocalAbiIdentityProjection {
    pub(super) fn from_unit(unit: &PackageUnit) -> Self {
        Self {
            schema: PACKAGE_LOCAL_ABI_IDENTITY_SCHEMA_MARKER,
            package_id: unit.package_id.clone(),
            package_version: unit.version.clone(),
            public_surface_identity: unit.publication_abi.abi_identity.clone(),
            abi_identity_facts: unit.abi_identity_projection.clone(),
        }
    }
}

/// Canonical package build preimage.
///
/// File IR storage paths and ref-level provenance never appear here. Their
/// semantic content is represented by the already assigned opaque File IR
/// identity, while module ownership remains explicit for linking.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageBuildIdentityProjection {
    schema: &'static str,
    local_abi_identity: String,
    file_ir_units: Vec<FileIrOwnerIdentityProjection>,
    resources: Vec<ResourceIdentityProjection>,
    implementation_links: PackageImplementationLinksIdentityProjection,
    package_dependencies: Vec<PackageDependencyIdentityProjection>,
    config_requirements: ConfigMetadataFacts,
    recoverable_metadata: RecoverableArtifactMetadata,
    callable_effects: CallableEffectFacts,
}

impl PackageBuildIdentityProjection {
    pub(super) fn from_unit(unit: &PackageUnit, local_abi_identity: String) -> Result<Self> {
        Ok(Self {
            schema: PACKAGE_BUILD_IDENTITY_SCHEMA_MARKER,
            local_abi_identity,
            file_ir_units: canonical_sort(
                unit.files
                    .iter()
                    .map(FileIrOwnerIdentityProjection::from_ref)
                    .collect(),
            )?,
            resources: canonical_sort(
                unit.resources
                    .iter()
                    .map(ResourceIdentityProjection::from_ref)
                    .collect(),
            )?,
            implementation_links: PackageImplementationLinksIdentityProjection::from_links(
                &unit.implementation_links,
            )?,
            package_dependencies: canonical_sort(
                unit.dependencies
                    .iter()
                    .map(PackageDependencyIdentityProjection::from_constraint)
                    .collect(),
            )?,
            config_requirements: unit.config_and_effect_metadata.config.clone(),
            recoverable_metadata: canonical_recoverable_metadata(&unit.recoverable_metadata),
            callable_effects: unit.config_and_effect_metadata.effects.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct FileIrOwnerIdentityProjection {
    file_ir_identity: String,
    module_path: String,
}

impl FileIrOwnerIdentityProjection {
    pub(super) fn from_ref(file: &FileIrRef) -> Self {
        Self {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceIdentityProjection {
    path: String,
    sha256: String,
    byte_len: u64,
    content_type: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageDependencyIdentityProjection {
    id: String,
    version: String,
    alias: String,
    config: Value,
}

impl PackageDependencyIdentityProjection {
    fn from_constraint(dependency: &PackageDependencyConstraint) -> Self {
        Self {
            id: dependency.id.clone(),
            version: dependency.version.clone(),
            alias: dependency.alias.clone(),
            config: if dependency.config.is_null() {
                Value::Object(Map::new())
            } else {
                dependency.config.clone()
            },
        }
    }
}

pub(super) fn canonical_sort<T: Serialize>(values: Vec<T>) -> Result<Vec<T>> {
    let mut keyed = Vec::with_capacity(values.len());
    for value in values {
        let key = canonical_ir_bytes(&value, ArtifactIdentityError::SerializePackageBuildIdentity)?;
        keyed.push((key, value));
    }
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(keyed.into_iter().map(|(_, value)| value).collect())
}
