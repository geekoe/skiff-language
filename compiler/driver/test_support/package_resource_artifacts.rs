use std::collections::BTreeMap;

use skiff_compiler_compiled::{
    projection_input::{build_package_projection_inputs, build_projection_input},
    CompiledPublication, PackagePublication,
};
use skiff_compiler_projection_input::PublicationResourceProjectionInput;

use crate::{
    input::{PackageManifestKey, Publication, ResolvedPackage},
    shared::publication_error::PublicationError,
};

use super::{
    internal_manifest_map, package_api_entries_for_test, projected_package_dependencies_for_test,
    TestPackageDependencyPublications, TestPackageManifest, TestPackageUnitArtifact,
    TestResolvedPackage,
};

pub(super) fn package_unit_from_compiled_package_for_test(
    publication: &Publication,
    compiled: &CompiledPublication,
) -> Result<TestPackageUnitArtifact, PublicationError> {
    let prelude_projection = crate::shared::prelude_registry::projection_prelude_context();
    let projection_context = skiff_compiler_projection::PackageProjectionContext::new(
        skiff_compiler_projection::context::PackageProjectionContextInput {
            package_id: publication.manifest.id.as_str(),
            version: publication.manifest.version.as_str(),
            dependencies: projected_package_dependencies_for_test(
                &publication.manifest.dependencies,
            ),
            api_entries: package_api_entries_for_test(&publication.manifest),
            api_source: publication.manifest.api.source.as_ref().map(|source| {
                skiff_compiler_projection::context::PackageApiSourceProjection {
                    relative_path: source.relative_path.clone(),
                    content_hash: source.content_hash.clone(),
                }
            }),
            package_root: &publication.source_tree.root,
            prelude: &prelude_projection,
        },
    );
    let projection_input = build_projection_input(compiled).with_resources(
        crate::pipeline::publication_resource_projection_inputs(publication),
    );
    let projection_view = projection_input.view();
    let package_projection =
        skiff_compiler_projection::project_package(projection_view, projection_context)?;
    let unit_artifacts =
        skiff_compiler_projection::package_unit_artifacts::project_package_ir_artifacts(
            skiff_compiler_projection::package_unit_artifacts::PackageIrProjectionSource {
                package_id: publication.manifest.id.as_str(),
                version: publication.manifest.version.as_str(),
                exports: &package_projection.exports,
                abi_identity_projection: &package_projection.abi_identity_projection,
                config_projection: &package_projection.config_projection,
                callable_effects: package_projection.input.source().callable_effects(),
                resources: projection_view.resources(),
                file_ir_units: package_projection
                    .input
                    .file_ir_units()
                    .iter()
                    .cloned()
                    .map(
                        skiff_compiler_projection::package_unit_artifacts::PackageFileIrProjection::from_unit,
                    )
                    .collect(),
            },
            &projected_package_dependencies_for_test(&publication.manifest.dependencies),
        )?;
    let production_files =
        crate::emission::file_ir_artifacts::published_file_ir_artifacts_from_projection_input(
            projection_view,
        )?;
    let resource_blobs =
        crate::emission::resources::publish_resource_artifacts(&unit_artifacts.resources)?;
    let materialized = crate::emission::package_unit_artifacts::materialize_package_unit_artifact(
        &unit_artifacts.unit,
        &production_files,
        &resource_blobs,
    )?;
    Ok(TestPackageUnitArtifact {
        package_id: materialized.unit.package_id.clone(),
        package_version: materialized.unit.version.clone(),
        package_dependencies: materialized.unit.dependencies.clone(),
        production_files,
        resource_blobs: materialized.resource_blobs,
        unit: materialized.unit,
    })
}

pub fn compile_package_dependency_publications_for_test(
    current: &TestPackageManifest,
    dependency_packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<TestPackageDependencyPublications, PublicationError> {
    let package_publications =
        package_publications_for_test(current, dependency_packages, available)?;
    let resource_inputs =
        package_resource_inputs_for_publications(&package_publications, available)?;
    Ok(TestPackageDependencyPublications {
        package_publications,
        resource_inputs,
    })
}

fn package_publications_for_test(
    current: &TestPackageManifest,
    dependency_packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<Vec<PackagePublication>, PublicationError> {
    let internal_available = internal_manifest_map(available);
    let packages = dependency_packages
        .iter()
        .filter(|package| package.manifest.id != current.id)
        .map(|package| ResolvedPackage {
            manifest: package.manifest.clone().into_internal(),
            config: package.config.clone(),
        })
        .collect::<Vec<_>>();
    let package_jobs = crate::input::package_job::build_package_jobs(packages)?;
    crate::pipeline::compile_package_jobs(package_jobs, &internal_available)
}

pub fn package_unit_artifacts_for_test(
    packages: &[TestResolvedPackage],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<Vec<TestPackageUnitArtifact>, PublicationError> {
    if packages.is_empty() {
        return Ok(Vec::new());
    }
    let internal_available = internal_manifest_map(available);
    let packages = packages
        .iter()
        .map(|package| ResolvedPackage {
            manifest: package.manifest.clone().into_internal(),
            config: package.config.clone(),
        })
        .collect::<Vec<_>>();
    let package_jobs = crate::input::package_job::build_package_jobs(packages)?;
    let publications = crate::pipeline::compile_package_jobs(package_jobs, &internal_available)?;
    let resource_inputs = package_resource_inputs_for_publications(&publications, available)?;
    package_unit_artifacts_from_package_publications(&publications, &resource_inputs)
}

pub fn package_unit_artifacts_from_dependency_publications_for_test(
    dependency_publications: &TestPackageDependencyPublications,
) -> Result<Vec<TestPackageUnitArtifact>, PublicationError> {
    package_unit_artifacts_from_package_publications(
        dependency_publications.as_slice(),
        &dependency_publications.resource_inputs,
    )
}

fn package_unit_artifacts_from_package_publications(
    publications: &[PackagePublication],
    resource_inputs: &BTreeMap<(String, String), Vec<PublicationResourceProjectionInput>>,
) -> Result<Vec<TestPackageUnitArtifact>, PublicationError> {
    if publications.is_empty() {
        return Ok(Vec::new());
    }
    let projection_inputs = build_package_projection_inputs(publications)
        .into_iter()
        .map(|input| {
            let key = (input.id().to_string(), input.version().to_string());
            let resources = resource_inputs.get(&key).cloned().ok_or_else(|| {
                PublicationError::ContractValidation {
                    message: format!(
                        "compiled package {}@{} has no resource projection entry",
                        key.0, key.1
                    ),
                }
            })?;
            Ok(input.with_resources(resources))
        })
        .collect::<Result<Vec<_>, PublicationError>>()?;
    let prelude_projection = crate::shared::prelude_registry::projection_prelude_context();
    let package_projections = skiff_compiler_projection::project_package_publications(
        &projection_inputs,
        &prelude_projection,
    )?;
    let package_artifacts =
        crate::emission::package_artifacts::build_package_artifacts(&package_projections)?;
    let projections_by_id = package_projections
        .iter()
        .map(|projection| (projection.manifest().id().to_string(), projection))
        .collect::<BTreeMap<_, _>>();
    package_artifacts
        .into_iter()
        .map(|artifact| {
            let projection = projections_by_id
                .get(&artifact.package_id)
                .expect("package artifact must have matching package projection");
            let unit_artifacts =
                crate::emission::package_unit_artifacts::publish_package_ir_artifacts(
                    &artifact,
                    &projection.package_ir,
                )?;
            Ok(TestPackageUnitArtifact {
                package_id: artifact.package_id,
                package_version: artifact.version,
                package_dependencies: unit_artifacts.unit.dependencies.clone(),
                production_files: unit_artifacts.file_ir_units,
                resource_blobs: unit_artifacts.resource_blobs,
                unit: unit_artifacts.unit,
            })
        })
        .collect()
}

fn package_resource_inputs_for_publications(
    publications: &[PackagePublication],
    available: &BTreeMap<PackageManifestKey, TestPackageManifest>,
) -> Result<BTreeMap<(String, String), Vec<PublicationResourceProjectionInput>>, PublicationError> {
    publications
        .iter()
        .map(|publication| {
            let key = (
                publication.id().to_string(),
                publication.version().to_string(),
            );
            let manifest =
                available
                    .get(&key)
                    .ok_or_else(|| PublicationError::ContractValidation {
                        message: format!(
                            "compiled package {}@{} has no available test manifest",
                            publication.id(),
                            publication.version()
                        ),
                    })?;
            let root =
                manifest
                    .path
                    .parent()
                    .ok_or_else(|| PublicationError::ContractValidation {
                        message: format!(
                            "package {}@{} manifest path {} has no parent",
                            manifest.id,
                            manifest.version,
                            manifest.path.display()
                        ),
                    })?;
            let resources =
                skiff_compiler_input::read_publication_resources(root, &manifest.resources)?;
            Ok((key, crate::pipeline::resource_projection_inputs(&resources)))
        })
        .collect()
}
