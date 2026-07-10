use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use skiff_artifact_model::ServiceUnit;
use skiff_runtime_activation::build_runtime_activation_for_image;
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkedProgramImageResolverExt, UnitAddr,
};
use skiff_runtime_linker::link_runtime_program_image;
use skiff_runtime_loader::{
    ArtifactGraph, ArtifactGraphCache, ArtifactGraphIdentities, ArtifactGraphLoader,
};

use super::{
    executable_graph::validate_package_test_executable_graph,
    load_package_test_build_artifact_from_artifact_roots, package_test_db_metadata,
    package_test_file_ir_ref_to_file_ref, package_test_protocol_identity,
    package_test_recoverable_metadata, package_test_spawn_targets,
    validate_loaded_package_test_link_policy, validate_loaded_test_file_scopes,
    LoadedPackageTestRuntimeProgram, PackageTestBuildArtifact, PackageTestBuildSelection,
    PackageTestDispatchArtifact, PackageTestDispatchSelection,
    PackageTestRuntimeEntrypointTemplate, PackageTestRuntimeTemplate, ValidatedPackageTestDispatch,
};

pub struct PackageTestRuntimeBuilder<'a> {
    artifact_roots: &'a [PathBuf],
    artifact_cache: ArtifactGraphCache<'a>,
}

impl<'a> PackageTestRuntimeBuilder<'a> {
    pub fn new(artifact_roots: &'a [PathBuf], artifact_cache: ArtifactGraphCache<'a>) -> Self {
        Self {
            artifact_roots,
            artifact_cache,
        }
    }

    pub fn load(
        &self,
        selection: &PackageTestDispatchSelection,
    ) -> anyhow::Result<LoadedPackageTestRuntimeProgram> {
        let template = self.load_template(&selection.build_selection())?;
        template.load(selection)
    }

    pub fn load_template(
        &self,
        selection: &PackageTestBuildSelection,
    ) -> anyhow::Result<PackageTestRuntimeTemplate> {
        let build =
            load_package_test_build_artifact_from_artifact_roots(self.artifact_roots, selection)?;
        let artifact_loader =
            ArtifactGraphLoader::new(&build.validated.artifact_root, self.artifact_cache);
        let assembly = &build.assembly;

        let production_unit = artifact_loader
            .load_package_unit_at_path(Path::new(&assembly.production_package_unit.unit_path))?;
        let production_files = artifact_loader.load_file_refs(
            &production_unit.files,
            &format!(
                "package-test production package {}@{} files",
                production_unit.package_id, production_unit.version
            ),
        )?;

        let dependency_units = assembly
            .dependency_package_units
            .iter()
            .enumerate()
            .map(|(index, reference)| {
                let unit =
                    artifact_loader.load_package_unit_at_path(Path::new(&reference.unit_path))?;
                if unit.package_id != reference.package_id {
                    anyhow::bail!(
                        "dependencyPackageUnits[{index}] loaded packageId {} does not match reference {}",
                        unit.package_id,
                        reference.package_id
                    );
                }
                Ok(unit)
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        validate_loaded_package_test_link_policy(
            assembly,
            production_unit.as_ref(),
            &dependency_units,
        )?;
        let dependency_files = dependency_units
            .iter()
            .map(|unit| {
                artifact_loader.load_file_refs(
                    &unit.files,
                    &format!(
                        "package-test dependency package {}@{} files",
                        unit.package_id, unit.version
                    ),
                )
            })
            .collect::<anyhow::Result<Vec<_>>>()?;

        let test_file_refs = assembly
            .test_files
            .iter()
            .map(package_test_file_ir_ref_to_file_ref)
            .collect::<Vec<_>>();
        let test_files =
            artifact_loader.load_file_refs(&test_file_refs, "package-test testFiles")?;
        validate_loaded_test_file_scopes(assembly, &test_files)?;

        let mut synthetic_service = ServiceUnit::empty(
            format!("__skiff.package-test/{}", assembly.package_id),
            assembly.package_version.clone(),
            package_test_protocol_identity(assembly),
        );
        synthetic_service.files = production_unit.files.clone();
        synthetic_service.files.extend(test_file_refs);
        synthetic_service.package_dependencies = production_unit.dependencies.clone();
        synthetic_service.db =
            package_test_db_metadata(production_unit.as_ref(), &production_files);
        synthetic_service.spawn_targets = package_test_spawn_targets(
            production_unit.as_ref(),
            &production_files,
            &dependency_units,
            &dependency_files,
            &synthetic_service.protocol_identity,
        )?;
        synthetic_service.recoverable_metadata = package_test_recoverable_metadata(
            &synthetic_service.service.id,
            production_unit.as_ref(),
            &production_files,
            &test_files,
            &dependency_units,
            &dependency_files,
            &synthetic_service.db,
            &synthetic_service.spawn_targets,
        )?;
        let synthetic_service = Arc::new(synthetic_service);

        let mut service_files = production_files;
        service_files.extend(test_files);
        let identities = ArtifactGraphIdentities::from_loaded_units(
            &service_files,
            &dependency_units,
            &dependency_files,
        );
        let graph = ArtifactGraph {
            service_unit: synthetic_service.clone(),
            service_files,
            service_resources: Default::default(),
            package_units: dependency_units,
            package_files: dependency_files,
            package_resources: Vec::new(),
            identities,
        };
        let image_build = link_runtime_program_image(graph).map_err(|error| {
            anyhow::anyhow!("failed to link package-test runtime program: {error}")
        })?;
        let identity = image_build.identity;
        let image = Arc::new(image_build.image);
        let activation = Arc::new(
            build_runtime_activation_for_image(image.as_ref(), image_build.activation_facts)
                .map_err(|error| {
                    anyhow::anyhow!("failed to build package-test runtime activation: {error}")
                })?,
        );
        let entrypoints =
            package_test_entrypoint_templates(&build, image.as_ref(), production_unit.as_ref())?;

        Ok(PackageTestRuntimeTemplate {
            validated: build.validated,
            assembly: build.assembly,
            production_unit,
            synthetic_service_unit: synthetic_service,
            identity,
            image,
            activation,
            entrypoints,
        })
    }
}

fn package_test_entrypoint_templates(
    build: &PackageTestBuildArtifact,
    image: &skiff_runtime_linked_program::LinkedProgramImage,
    production_unit: &skiff_artifact_model::PackageUnit,
) -> anyhow::Result<BTreeMap<String, PackageTestRuntimeEntrypointTemplate>> {
    let mut templates = BTreeMap::new();
    for entrypoint in &build.assembly.test_entrypoints {
        let executable_addr = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::file_ir_identity(&entrypoint.executable_ref.file_ir_identity),
            executable: entrypoint.executable_ref.executable_index as usize,
        };
        let resolved = image
            .resolve_executable(&executable_addr)
            .map_err(|error| {
                anyhow::anyhow!(
                    "package test entrypoint {} references invalid executable: {error}",
                    entrypoint.entrypoint_id
                )
            })?;
        if let Some(expected_symbol) = entrypoint.executable_ref.symbol.as_deref() {
            if resolved.executable.symbol != expected_symbol {
                anyhow::bail!(
                    "package test entrypoint {} executable symbol mismatch: expected {}, got {}",
                    entrypoint.entrypoint_id,
                    expected_symbol,
                    resolved.executable.symbol
                );
            }
        }
        let dispatch = PackageTestDispatchArtifact {
            validated: ValidatedPackageTestDispatch {
                artifact_root: build.validated.artifact_root.clone(),
                assembly_path: build.validated.assembly_path.clone(),
                entrypoint_id: entrypoint.entrypoint_id.clone(),
            },
            assembly: build.assembly.clone(),
            entrypoint: entrypoint.clone(),
        };
        validate_package_test_executable_graph(
            &dispatch,
            image,
            &executable_addr,
            production_unit,
        )?;
        templates.insert(
            entrypoint.entrypoint_id.clone(),
            PackageTestRuntimeEntrypointTemplate {
                entrypoint: entrypoint.clone(),
                executable_addr,
            },
        );
    }
    Ok(templates)
}
