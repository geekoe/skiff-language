use serde::{Deserialize, Serialize};
use skiff_artifact_model::AbiIdentityFacts;

use super::package_artifact_metadata::PackageDependencyEntry;
use crate::emission::artifact::{
    PublishedFileIrArtifact, PublishedJsonArtifact, ARTIFACT_INDEX_SCHEMA_VERSION,
    PACKAGE_ASSEMBLY_KIND,
};
use crate::{
    projection::source_map::PublicationSourceMap,
    projection::{
        package_exports::PackageExports, ConfigActivation, ConfigProjection,
        ConfigRequirementsProjection, ConfigShape, ConfigUseEntry,
    },
};

#[derive(Debug)]
pub struct PublishedPackageArtifacts {
    pub package_id: String,
    pub version: String,
    pub exports: PackageExports,
    pub abi_identity_projection: AbiIdentityFacts,
    pub file_ir_units: Vec<PublishedFileIrArtifact>,
    pub assembly: PublishedJsonArtifact,
    /// Strongly-typed package version index. `version_index.value` is emitted
    /// from this same model and is only retained for final artifact write-out.
    pub version_index_model: PackageVersionIndexModel,
    pub version_index: PublishedJsonArtifact,
    /// Strongly-typed config projection for this package, captured at assembly
    /// build time; the same values are serialized into `assembly`.
    pub config_projection: ConfigProjection,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageAssemblyPackageObject<'a> {
    pub id: &'a str,
    pub version: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assembly_identity: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackageVersionIndexPackageObject {
    pub(crate) id: String,
    pub(crate) version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageAssemblyFileRef {
    pub source_path: String,
    pub module_path: String,
    pub file_ir_identity: String,
    pub file_ir_path: String,
}

impl PackageAssemblyFileRef {
    pub fn from_published_file_ir_artifact(artifact: &PublishedFileIrArtifact) -> Self {
        Self {
            source_path: artifact.source_path.clone(),
            module_path: artifact.module_path.clone(),
            file_ir_identity: artifact.identity.clone(),
            file_ir_path: artifact.path.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PackageVersionIndexAssemblyPointer {
    pub(crate) assembly_identity: String,
    pub(crate) assembly_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageVersionIndexModel {
    pub(crate) schema_version: &'static str,
    pub(crate) kind: &'static str,
    pub(crate) package: PackageVersionIndexPackageObject,
    pub(crate) assembly: PackageVersionIndexAssemblyPointer,
    pub(crate) exports: PackageExports,
    pub(crate) abi_identity_projection: AbiIdentityFacts,
    pub(crate) files: Vec<PackageAssemblyFileRef>,
    pub(crate) dependencies: Vec<PackageDependencyEntry>,
    pub(crate) config_shape: ConfigShape,
    pub(crate) config_uses: Vec<ConfigUseEntry>,
    pub(crate) config_activation: ConfigActivation,
    pub(crate) config_requirements: ConfigRequirementsProjection,
    pub(crate) source_map: PublicationSourceMap,
}

impl PackageVersionIndexModel {
    pub(crate) fn new(
        package_id: impl Into<String>,
        version: impl Into<String>,
        assembly_identity: impl Into<String>,
        assembly_path: impl Into<String>,
        exports: PackageExports,
        abi_identity_projection: AbiIdentityFacts,
        files: Vec<PackageAssemblyFileRef>,
        dependencies: Vec<PackageDependencyEntry>,
        config_projection: &ConfigProjection,
        source_map: PublicationSourceMap,
    ) -> Self {
        Self {
            schema_version: ARTIFACT_INDEX_SCHEMA_VERSION,
            kind: PACKAGE_ASSEMBLY_KIND,
            package: PackageVersionIndexPackageObject {
                id: package_id.into(),
                version: version.into(),
            },
            assembly: PackageVersionIndexAssemblyPointer {
                assembly_identity: assembly_identity.into(),
                assembly_path: assembly_path.into(),
            },
            exports,
            abi_identity_projection,
            files,
            dependencies,
            config_shape: config_projection.shape.clone(),
            config_uses: config_projection.uses.clone(),
            config_activation: config_projection.activation.clone(),
            config_requirements: config_projection.requirements.clone(),
            source_map,
        }
    }
}
