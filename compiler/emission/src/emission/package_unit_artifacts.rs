use serde::Serialize;
use skiff_artifact_model::{FileIrRef, PackageUnit, PACKAGE_UNIT_SCHEMA_VERSION};
use skiff_compiler_core::id::PublicationId;
use skiff_compiler_core::json_utils::value_sha256;

use crate::emission::artifact::{PublishedFileIrArtifact, PublishedJsonArtifact};
use crate::emission::artifact_assembly::{PackageVersionIndexModel, PublishedPackageArtifacts};
use crate::emission::identity::assign_package_unit_identities;
use crate::error::Result;
use crate::projection::package_unit_artifacts::ProjectedPackageIrArtifacts;

pub struct PublishedPackageIrArtifacts {
    pub package_unit: PublishedJsonArtifact,
    pub unit: PackageUnit,
    pub file_ir_units: Vec<PublishedFileIrArtifact>,
}

pub fn publish_package_ir_artifacts(
    package: &PublishedPackageArtifacts,
    projected: &ProjectedPackageIrArtifacts,
) -> Result<PublishedPackageIrArtifacts> {
    let mut unit = projected.unit.clone();
    attach_published_file_paths_to_package_unit(&mut unit.files, &package.file_ir_units);
    assign_package_unit_identities(&mut unit)?;
    let package_unit = package_unit_artifact(&unit);

    Ok(PublishedPackageIrArtifacts {
        package_unit,
        unit,
        file_ir_units: package.file_ir_units.clone(),
    })
}

fn attach_published_file_paths_to_package_unit(
    refs: &mut [FileIrRef],
    artifacts: &[PublishedFileIrArtifact],
) {
    let by_identity = artifacts
        .iter()
        .map(|artifact| (artifact.identity.as_str(), artifact))
        .collect::<std::collections::BTreeMap<_, _>>();
    for file_ref in refs {
        if let Some(artifact) = by_identity.get(file_ref.file_ir_identity.as_str()) {
            file_ref.artifact_path = Some(artifact.path.clone());
            file_ref.source_ast_hash = Some(artifact.unit.source_ast_hash.clone());
        }
    }
}

fn package_unit_artifact(unit: &PackageUnit) -> PublishedJsonArtifact {
    let value = serde_json::to_value(unit).expect("PackageUnit must serialize");
    let hash = value_sha256(&value);
    let package_path = PublicationId::parse(&unit.package_id)
        .expect("package id was validated before artifact projection")
        .artifact_path();
    let path = format!("units/packages/{package_path}/{hash}.json");
    PublishedJsonArtifact {
        value,
        identity: unit.build_identity.clone(),
        hash,
        path,
    }
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
