mod entrypoints;
mod files;
mod link_policy;
mod package_units;

use std::collections::BTreeSet;

use serde_json::Value;
use skiff_compiler_core::artifact::{
    ConfigAndEffectMetadata, PackageTestAssembly, PackageTestAssemblyKind, PackageUnit,
    PACKAGE_TEST_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_compiler_core::id::PublicationId;
use thiserror::Error;

use self::entrypoints::{owner_test_file_identities_for_assembly, test_entrypoints};
use self::files::{
    package_test_file_ref, published_files, published_test_files, test_files_by_source_path,
};
use self::link_policy::{link_policy, source_map};
use self::package_units::{
    materialize_projected_package_unit_for_test, normalize_dependency_slot_records,
    validate_unique_dependency_aliases,
};
use crate::emission::artifact::{PublishedFileIrArtifact, PublishedResourceArtifact};
use crate::emission::identity::{
    derive_package_test_entrypoint_id, package_test_build_hash, package_test_build_identity,
    validate_package_test_assembly_identity, ArtifactIdentityError,
};

#[derive(Debug, Clone)]
pub struct PackageTestArtifactBuildInput {
    pub package_id: String,
    pub package_version: String,
    pub production_package_unit: PackageUnit,
    pub package_test_config_and_effect_metadata: ConfigAndEffectMetadata,
    pub production_files: Vec<PublishedFileIrArtifact>,
    pub production_resource_blobs: Vec<PublishedResourceArtifact>,
    pub dependency_packages: Vec<PackageTestDependencyPackageInput>,
    pub test_files: Vec<PackageTestFileIrArtifact>,
    pub entrypoints: Vec<PackageTestEntrypointInput>,
}

#[derive(Debug, Clone)]
pub struct PackageTestDependencyPackageInput {
    pub package_id: String,
    pub package_version: String,
    pub production_files: Vec<PublishedFileIrArtifact>,
    pub production_resource_blobs: Vec<PublishedResourceArtifact>,
    pub package_unit: PackageUnit,
}

#[derive(Debug, Clone)]
pub struct PackageTestFileIrArtifact {
    pub source_path: String,
    pub module_path: String,
    pub file_ir: skiff_compiler_core::artifact::FileIrUnit,
    pub explicit_const_type_annotations: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct PackageTestEntrypointInput {
    pub display_name: String,
    pub source_path: String,
    pub module_path: String,
    pub test_ordinal: u32,
    pub executable_index: u32,
    pub executable_local_id: String,
    pub symbol: Option<String>,
    pub default_run: bool,
    pub config_and_effect_metadata: ConfigAndEffectMetadata,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PackageTestArtifactBuildOutput {
    pub package_id: String,
    pub package_version: String,
    pub package_artifact_path: String,
    pub test_build_identity: String,
    pub test_build_hash: String,
    pub production_files: Vec<PublishedFileIrArtifact>,
    pub production_package_unit: PublishedPackageTestPackageUnitArtifact,
    pub dependency_package_units: Vec<PublishedPackageTestPackageUnitArtifact>,
    pub test_files: Vec<PublishedFileIrArtifact>,
    pub assembly: PublishedPackageTestAssemblyArtifact,
    pub entrypoints: Vec<PackageTestEntrypointSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedPackageTestPackageUnitArtifact {
    pub files: Vec<PublishedFileIrArtifact>,
    pub resource_blobs: Vec<PublishedResourceArtifact>,
    pub unit: PackageUnit,
    pub value: Value,
    pub unit_path: String,
    pub reference: skiff_compiler_core::artifact::PackageTestPackageUnitRef,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PublishedPackageTestAssemblyArtifact {
    pub assembly: PackageTestAssembly,
    pub value: Value,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageTestEntrypointSummary {
    pub display_name: String,
    pub entrypoint_local_id: String,
    pub entrypoint_id: String,
}

#[derive(Debug, Error)]
pub enum PackageTestArtifactBuildError {
    #[error("invalid package test artifact input: {message}")]
    InvalidInput { message: String },
    #[error("failed to compute package test identity: {0}")]
    Identity(#[from] ArtifactIdentityError),
}

pub fn build_package_test_artifacts(
    input: PackageTestArtifactBuildInput,
) -> Result<PackageTestArtifactBuildOutput, PackageTestArtifactBuildError> {
    let package_id = PublicationId::parse(&input.package_id).map_err(|error| {
        PackageTestArtifactBuildError::InvalidInput {
            message: format!("package id {} is invalid: {error}", input.package_id),
        }
    })?;
    if input.package_version.trim().is_empty() {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: "package_version must not be empty".to_string(),
        });
    }
    if input.production_files.is_empty() {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: "production_files must not be empty".to_string(),
        });
    }

    let production_files = published_files(input.production_files.clone())?;
    let test_files = published_test_files(input.test_files.clone())?;
    let production_unit = materialize_projected_package_unit_for_test(
        package_id.as_str(),
        &input.package_version,
        production_files.clone(),
        input.production_resource_blobs.clone(),
        input.production_package_unit.clone(),
    )?;
    validate_unique_dependency_aliases(&production_unit.unit)?;
    let package_path = package_id.artifact_path();
    let dependency_units = input
        .dependency_packages
        .iter()
        .map(|dependency| {
            materialize_projected_package_unit_for_test(
                &dependency.package_id,
                &dependency.package_version,
                published_files(dependency.production_files.clone())?,
                dependency.production_resource_blobs.clone(),
                dependency.package_unit.clone(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let dependency_slots = normalize_dependency_slot_records(dependency_units)?;
    let all_test_file_refs = test_files
        .iter()
        .map(package_test_file_ref)
        .collect::<Vec<_>>();
    let entrypoints_without_dispatch_ids = test_entrypoints(
        &input,
        &test_files,
        &test_files_by_source_path(&all_test_file_refs)?,
    )?;
    let owner_test_file_identities =
        owner_test_file_identities_for_assembly(&entrypoints_without_dispatch_ids)?;
    let test_files = test_files
        .into_iter()
        .filter(|file| owner_test_file_identities.contains(file.identity.as_str()))
        .collect::<Vec<_>>();
    let test_file_refs = all_test_file_refs
        .into_iter()
        .filter(|file| owner_test_file_identities.contains(file.file_ir_identity.as_str()))
        .collect::<Vec<_>>();
    let dependency_units = dependency_slots
        .iter()
        .map(|slot| slot.unit.clone())
        .collect::<Vec<_>>();
    let dependency_public_scopes = dependency_slots
        .iter()
        .map(|slot| slot.public_scope.clone())
        .collect::<Vec<_>>();
    let link_policy = link_policy(
        &production_unit.reference,
        &production_unit.unit,
        &test_files,
        &test_file_refs,
        &entrypoints_without_dispatch_ids,
        &dependency_public_scopes,
    );
    let mut assembly = PackageTestAssembly {
        schema_version: PACKAGE_TEST_ASSEMBLY_SCHEMA_VERSION.to_string(),
        kind: PackageTestAssemblyKind::PackageTest,
        package_id: input.package_id.clone(),
        package_version: input.package_version.clone(),
        test_build_identity: String::new(),
        production_package_unit: production_unit.reference.clone(),
        test_files: test_file_refs.clone(),
        dependency_package_units: dependency_units
            .iter()
            .map(|dependency| dependency.reference.clone())
            .collect(),
        test_entrypoints: entrypoints_without_dispatch_ids,
        link_policy,
        config_and_effect_metadata: input.package_test_config_and_effect_metadata.clone(),
        source_map: source_map(&test_file_refs),
    };
    assembly.test_build_identity = package_test_build_identity(&assembly)?;
    for entrypoint in &mut assembly.test_entrypoints {
        entrypoint.entrypoint_id = derive_package_test_entrypoint_id(
            &assembly.test_build_identity,
            &entrypoint.entrypoint_local_id,
        )?;
    }
    validate_package_test_assembly_identity(&assembly)?;

    let test_build_hash = package_test_build_hash(&assembly)?;
    let assembly_path = format!("assemblies/package-tests/{package_path}/{test_build_hash}.json");
    let assembly_value =
        serde_json::to_value(&assembly).expect("PackageTestAssembly must serialize");
    let entrypoints = assembly
        .test_entrypoints
        .iter()
        .map(|entrypoint| PackageTestEntrypointSummary {
            display_name: entrypoint.display_name.clone(),
            entrypoint_local_id: entrypoint.entrypoint_local_id.clone(),
            entrypoint_id: entrypoint.entrypoint_id.clone(),
        })
        .collect();

    Ok(PackageTestArtifactBuildOutput {
        package_id: input.package_id,
        package_version: input.package_version,
        package_artifact_path: package_path,
        test_build_identity: assembly.test_build_identity.clone(),
        test_build_hash,
        production_files,
        production_package_unit: production_unit,
        dependency_package_units: dependency_units,
        test_files,
        assembly: PublishedPackageTestAssemblyArtifact {
            assembly,
            value: assembly_value,
            path: assembly_path,
        },
        entrypoints,
    })
}
