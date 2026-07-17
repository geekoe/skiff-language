use serde::Serialize;
use skiff_artifact_identity::validate_package_unit_identities;
use skiff_artifact_model::{PackageUnit, PACKAGE_UNIT_SCHEMA_VERSION};
use skiff_compiler_core::id::PublicationId;
use skiff_compiler_core::json_utils::value_sha256;

use crate::emission::artifact::{
    PublishedFileIrArtifact, PublishedJsonArtifact, PublishedResourceArtifact,
};
use crate::emission::artifact_assembly::{PackageVersionIndexModel, PublishedPackageArtifacts};
use crate::emission::resources::{normalized_resource_artifacts, publish_resource_artifacts};
use crate::error::EmissionError;
use crate::error::Result;
use crate::projection::package_unit_artifacts::ProjectedPackageIrArtifacts;

pub struct PublishedPackageIrArtifacts {
    pub package_unit: PublishedJsonArtifact,
    pub unit: PackageUnit,
    pub file_ir_units: Vec<PublishedFileIrArtifact>,
    pub resource_blobs: Vec<PublishedResourceArtifact>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaterializedPackageUnitArtifact {
    pub unit: PackageUnit,
    pub artifact: PublishedJsonArtifact,
    pub resource_blobs: Vec<PublishedResourceArtifact>,
}

pub fn publish_package_ir_artifacts(
    package: &PublishedPackageArtifacts,
    projected: &ProjectedPackageIrArtifacts,
) -> Result<PublishedPackageIrArtifacts> {
    let resource_blobs = publish_resource_artifacts(&projected.resources)?;
    let materialized = materialize_package_unit_artifact(
        &projected.unit,
        &package.file_ir_units,
        &resource_blobs,
    )?;

    Ok(PublishedPackageIrArtifacts {
        package_unit: materialized.artifact,
        unit: materialized.unit,
        file_ir_units: package.file_ir_units.clone(),
        resource_blobs: materialized.resource_blobs,
    })
}

pub fn materialize_package_unit_artifact(
    projected: &PackageUnit,
    files: &[PublishedFileIrArtifact],
    resource_blobs: &[PublishedResourceArtifact],
) -> Result<MaterializedPackageUnitArtifact> {
    let mut unit = projected.clone();
    let resource_blobs = normalized_resource_artifacts(resource_blobs)?;
    super::package_assets::attach_published_file_paths("package unit", &mut unit.files, files)?;
    super::package_assets::attach_resource_paths(
        "package unit",
        &mut unit.resources,
        &resource_blobs,
    )?;
    validate_package_unit_identities(&unit)?;
    let artifact = package_unit_artifact(&unit)?;
    Ok(MaterializedPackageUnitArtifact {
        unit,
        artifact,
        resource_blobs,
    })
}

fn package_unit_artifact(unit: &PackageUnit) -> Result<PublishedJsonArtifact> {
    let value = serde_json::to_value(unit).expect("PackageUnit must serialize");
    let hash = value_sha256(&value);
    let package_path = PublicationId::parse(&unit.package_id)
        .map_err(|error| EmissionError::ContractValidation {
            message: format!("package unit id {} is invalid: {error}", unit.package_id),
        })?
        .artifact_path();
    let path = format!("units/packages/{package_path}/{hash}.json");
    Ok(PublishedJsonArtifact {
        value,
        identity: unit.build_identity.clone(),
        hash,
        path,
    })
}

pub fn package_index_with_package_unit(
    package: &PublishedPackageArtifacts,
    unit: &PackageUnit,
    package_unit: &PublishedJsonArtifact,
) -> PublishedJsonArtifact {
    let value = serde_json::to_value(PackageVersionIndexWithPackageUnit {
        index: &package.version_index_model,
        package_unit: PackageUnitPointer::new(unit, package_unit),
    })
    .expect("package version index with package unit must serialize");
    let hash = value_sha256(&value);
    PublishedJsonArtifact {
        value,
        identity: package.version_index.identity.clone(),
        hash,
        path: package.version_index.path.clone(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageVersionIndexWithPackageUnit<'a> {
    #[serde(flatten)]
    index: &'a PackageVersionIndexModel,
    package_unit: PackageUnitPointer<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageUnitPointer<'a> {
    schema_version: &'static str,
    package_id: &'a str,
    version: &'a str,
    build_identity: &'a str,
    abi_identity: &'a str,
    unit_hash: &'a str,
    unit_path: &'a str,
}

impl<'a> PackageUnitPointer<'a> {
    fn new(unit: &'a PackageUnit, artifact: &'a PublishedJsonArtifact) -> Self {
        Self {
            schema_version: PACKAGE_UNIT_SCHEMA_VERSION,
            package_id: unit.package_id.as_str(),
            version: unit.version.as_str(),
            build_identity: artifact.identity.as_str(),
            abi_identity: unit.abi_identity.as_str(),
            unit_hash: artifact.hash.as_str(),
            unit_path: artifact.path.as_str(),
        }
    }
}

#[cfg(test)]
mod tests;
