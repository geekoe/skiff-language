use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use skiff_artifact_identity::canonical_json_value;
use skiff_artifact_model::{
    CallTargetIr, DbIndexIr, DbMetadataIndexIr, DbMetadataIr, ExecutableKind, ExprIr, FileIrRef,
    FileIrUnit, MetadataValue, OperationCallableKind, OperationTargetRef, PackageOperationTarget,
    PackageRefIr, PackageTestAssembly, PackageTestFileIrRef, PackageTestFileLinkScope,
    PackageTestPackageUnitRef, PackageUnit, RecoverableArtifactMetadata, ServiceUnit,
    SpawnTargetIr, SpawnTargetKindIr, TypeRefIr,
};
use skiff_compiler_projection::recoverable_boundary::{
    recoverable_metadata_for_service_artifacts, RecoverableInputs, RecoverablePackageTypeSource,
};
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_linked_program::{
    ExecutableAddr, FileAddr, LinkedFileUnit, LinkedProgramImage, RuntimeProgramIdentity, UnitAddr,
};
use skiff_runtime_linker::package_handler_target;
use skiff_runtime_loader::ArtifactGraphCache;

mod builder;
mod dispatch_selection;
mod executable_graph;

pub use self::builder::PackageTestRuntimeBuilder;
#[cfg(test)]
pub(crate) use self::dispatch_selection::validate_package_test_dispatch_from_artifact_roots;
#[allow(unused_imports)]
pub use self::dispatch_selection::ValidatedPackageTestDispatch;
#[cfg(test)]
use self::dispatch_selection::{identity_hash, publication_storage_segment};
pub use self::dispatch_selection::{
    load_package_test_build_artifact_from_artifact_roots,
    load_package_test_dispatch_artifact_from_artifact_roots, PackageTestBuildArtifact,
    PackageTestBuildSelection, PackageTestDispatchArtifact, PackageTestDispatchSelection,
};
use self::dispatch_selection::{select_package_test_entrypoint, ValidatedPackageTestBuild};
#[cfg(test)]
use self::executable_graph::validate_package_test_executable_graph;

const PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX: &str =
    "skiff-package-implementation-links-v1:sha256";
const SPAWN_SUBMIT_METADATA_KEY: &str = "spawnSubmit";
const SPAWN_FUNCTION_TARGET_PREFIX: &str = "function:";

#[derive(Debug, Clone)]
pub struct LoadedPackageTestRuntimeProgram {
    pub dispatch: PackageTestDispatchArtifact,
    pub production_unit: Arc<PackageUnit>,
    pub synthetic_service_unit: Arc<ServiceUnit>,
    pub identity: RuntimeProgramIdentity,
    pub image: Arc<LinkedProgramImage>,
    pub activation: Arc<RuntimeActivation>,
    pub executable_addr: ExecutableAddr,
}

#[derive(Debug, Clone)]
pub struct PackageTestRuntimeTemplate {
    pub(crate) validated: ValidatedPackageTestBuild,
    pub(crate) assembly: PackageTestAssembly,
    pub(crate) production_unit: Arc<PackageUnit>,
    pub(crate) synthetic_service_unit: Arc<ServiceUnit>,
    pub(crate) identity: RuntimeProgramIdentity,
    pub(crate) image: Arc<LinkedProgramImage>,
    pub(crate) activation: Arc<RuntimeActivation>,
    pub(crate) entrypoints: BTreeMap<String, PackageTestRuntimeEntrypointTemplate>,
}

#[derive(Debug, Clone)]
pub(crate) struct PackageTestRuntimeEntrypointTemplate {
    pub(crate) entrypoint: skiff_artifact_model::PackageTestEntrypoint,
    pub(crate) executable_addr: ExecutableAddr,
}

impl PackageTestRuntimeTemplate {
    pub fn load(
        &self,
        selection: &PackageTestDispatchSelection,
    ) -> anyhow::Result<LoadedPackageTestRuntimeProgram> {
        let entrypoint = select_package_test_entrypoint(&self.assembly, selection)?;
        let entrypoint_template =
            self.entrypoints
                .get(&entrypoint.entrypoint_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "package test entrypoint {} was not prepared in runtime template {}",
                        entrypoint.entrypoint_id,
                        self.assembly.test_build_identity
                    )
                })?;
        Ok(LoadedPackageTestRuntimeProgram {
            dispatch: PackageTestDispatchArtifact {
                validated: ValidatedPackageTestDispatch {
                    artifact_root: self.validated.artifact_root.clone(),
                    assembly_path: self.validated.assembly_path.clone(),
                    entrypoint_id: selection.entrypoint_id.clone(),
                },
                assembly: self.assembly.clone(),
                entrypoint: entrypoint_template.entrypoint.clone(),
            },
            production_unit: Arc::clone(&self.production_unit),
            synthetic_service_unit: Arc::clone(&self.synthetic_service_unit),
            identity: self.identity.clone(),
            image: Arc::clone(&self.image),
            activation: Arc::clone(&self.activation),
            executable_addr: entrypoint_template.executable_addr.clone(),
        })
    }

    pub fn estimated_size_bytes(&self) -> usize {
        self.metadata_estimated_size_bytes()
            .saturating_add(self.shared_runtime_estimated_size_bytes())
    }

    pub fn shared_runtime_estimated_size_bytes(&self) -> usize {
        runtime_program_identity_estimated_size(&self.identity)
            .saturating_add(serialized_estimated_size(self.production_unit.as_ref()))
            .saturating_add(serialized_estimated_size(
                self.synthetic_service_unit.as_ref(),
            ))
            .saturating_add(linked_program_image_estimated_size(self.image.as_ref()))
            .saturating_add(runtime_activation_estimated_size(self.activation.as_ref()))
    }

    fn metadata_estimated_size_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(serialized_estimated_size(&self.assembly))
            .saturating_add(self.validated.artifact_root.as_os_str().len())
            .saturating_add(self.validated.assembly_path.as_os_str().len())
            .saturating_add(
                self.entrypoints
                    .iter()
                    .map(|(entrypoint_id, entrypoint)| {
                        entrypoint_id.len().saturating_add(
                            package_test_entrypoint_template_estimated_size(entrypoint),
                        )
                    })
                    .sum::<usize>(),
            )
    }

    #[cfg(test)]
    pub fn entrypoint_count(&self) -> usize {
        self.entrypoints.len()
    }
}

pub fn load_package_test_runtime_program_from_artifact_roots(
    artifact_roots: &[PathBuf],
    selection: &PackageTestDispatchSelection,
    artifact_cache: ArtifactGraphCache<'_>,
) -> anyhow::Result<LoadedPackageTestRuntimeProgram> {
    PackageTestRuntimeBuilder::new(artifact_roots, artifact_cache).load(selection)
}

fn value_sha256(value: &serde_json::Value) -> anyhow::Result<String> {
    let canonical = canonical_json_value(value);
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| anyhow::anyhow!("failed to serialize artifact JSON: {error}"))?;
    Ok(hex::encode(Sha256::digest(&bytes)))
}

fn serialized_estimated_size<T: Serialize>(value: &T) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or_else(|_| std::mem::size_of_val(value))
}

fn runtime_program_identity_estimated_size(identity: &RuntimeProgramIdentity) -> usize {
    std::mem::size_of::<RuntimeProgramIdentity>()
        .saturating_add(identity.dynamic_build_id.len())
        .saturating_add(identity.linked_image_identity.len())
}

fn package_test_entrypoint_template_estimated_size(
    entrypoint: &PackageTestRuntimeEntrypointTemplate,
) -> usize {
    std::mem::size_of::<PackageTestRuntimeEntrypointTemplate>()
        .saturating_add(serialized_estimated_size(&entrypoint.entrypoint))
        .saturating_add(executable_addr_estimated_size(&entrypoint.executable_addr))
}

fn linked_program_image_estimated_size(image: &LinkedProgramImage) -> usize {
    std::mem::size_of::<LinkedProgramImage>()
        .saturating_add(image.service_files.len() * std::mem::size_of::<Arc<LinkedFileUnit>>())
        .saturating_add(
            image
                .service_files
                .iter()
                .map(|file| serialized_estimated_size(file.as_ref()))
                .sum::<usize>(),
        )
        .saturating_add(
            image.packages.len()
                * std::mem::size_of::<Arc<skiff_runtime_linked_program::PackageUnit>>(),
        )
        .saturating_add(
            image
                .packages
                .iter()
                .map(|package| serialized_estimated_size(package.as_ref()))
                .sum::<usize>(),
        )
        .saturating_add(
            image
                .package_files
                .iter()
                .map(|files| {
                    files
                        .len()
                        .saturating_mul(std::mem::size_of::<Arc<LinkedFileUnit>>())
                        .saturating_add(
                            files
                                .iter()
                                .map(|file| serialized_estimated_size(file.as_ref()))
                                .sum::<usize>(),
                        )
                })
                .sum::<usize>(),
        )
        .saturating_add(string_map_estimated_size(&image.routes))
        .saturating_add(string_map_estimated_size(&image.spawn_routes))
        .saturating_add(string_map_estimated_size(&image.operations))
        .saturating_add(string_map_estimated_size(&image.operation_receivers))
        .saturating_add(serialized_estimated_size(&image.link_overlay))
        .saturating_add(serialized_estimated_size(&image.types))
}

fn runtime_activation_estimated_size(activation: &RuntimeActivation) -> usize {
    std::mem::size_of::<RuntimeActivation>()
        .saturating_add(activation.service.id.len())
        .saturating_add(
            activation
                .service
                .display_name
                .as_deref()
                .map(str::len)
                .unwrap_or(0),
        )
        .saturating_add(serialized_estimated_size(&activation.service.metadata))
        .saturating_add(activation.version.len())
        .saturating_add(serialized_estimated_size(&activation.package_configs))
        .saturating_add(serialized_estimated_size(&activation.service_dependencies))
        .saturating_add(serialized_estimated_size(&activation.timeout))
        .saturating_add(serialized_estimated_size(
            &activation.operation_route_bindings,
        ))
        .saturating_add(serialized_estimated_size(&activation.db))
        .saturating_add(serialized_estimated_size(&activation.actors))
        .saturating_add(serialized_estimated_size(&activation.gateway))
}

fn string_map_estimated_size<T: Serialize>(map: &std::collections::HashMap<String, T>) -> usize {
    std::mem::size_of_val(map)
        .saturating_add(map.keys().map(String::len).sum::<usize>())
        .saturating_add(map.values().map(serialized_estimated_size).sum::<usize>())
}

fn executable_addr_estimated_size(addr: &ExecutableAddr) -> usize {
    std::mem::size_of::<ExecutableAddr>()
        .saturating_add(unit_addr_estimated_size(&addr.unit))
        .saturating_add(file_addr_estimated_size(&addr.file))
}

fn unit_addr_estimated_size(addr: &UnitAddr) -> usize {
    match addr {
        UnitAddr::Service => std::mem::size_of::<UnitAddr>(),
        UnitAddr::Package(_) => std::mem::size_of::<UnitAddr>(),
    }
}

fn file_addr_estimated_size(addr: &FileAddr) -> usize {
    match addr {
        FileAddr::LoadedFileIndex(_) => std::mem::size_of::<FileAddr>(),
        FileAddr::FileIrIdentity(identity) => {
            std::mem::size_of::<FileAddr>().saturating_add(identity.len())
        }
    }
}

fn validate_loaded_package_test_link_policy(
    assembly: &PackageTestAssembly,
    production_unit: &skiff_artifact_model::PackageUnit,
    dependency_units: &[Arc<skiff_artifact_model::PackageUnit>],
) -> anyhow::Result<()> {
    validate_loaded_package_unit_ref(
        production_unit,
        &assembly.production_package_unit,
        "productionPackageUnit",
    )?;
    validate_loaded_current_package_production_scope(assembly, production_unit)?;

    if dependency_units.len() != assembly.dependency_package_units.len() {
        anyhow::bail!(
            "loaded dependency package count {} does not match dependencyPackageUnits count {}",
            dependency_units.len(),
            assembly.dependency_package_units.len()
        );
    }
    if assembly.link_policy.dependency_public_scopes.len()
        != assembly.dependency_package_units.len()
    {
        anyhow::bail!(
            "linkPolicy.dependencyPublicScopes count {} must match dependencyPackageUnits count {}",
            assembly.link_policy.dependency_public_scopes.len(),
            assembly.dependency_package_units.len()
        );
    }
    for (index, ((reference, scope), unit)) in assembly
        .dependency_package_units
        .iter()
        .zip(&assembly.link_policy.dependency_public_scopes)
        .zip(dependency_units)
        .enumerate()
    {
        validate_loaded_package_unit_ref(
            unit.as_ref(),
            reference,
            &format!("dependencyPackageUnits[{index}]"),
        )?;
        validate_loaded_dependency_public_scope(index, reference, scope, unit.as_ref())?;
    }

    Ok(())
}

fn validate_loaded_current_package_production_scope(
    assembly: &PackageTestAssembly,
    unit: &skiff_artifact_model::PackageUnit,
) -> anyhow::Result<()> {
    let scope = &assembly.link_policy.current_package_production;
    let files_digest = canonical_digest(&unit.files, "productionPackageUnit.files")?;
    if scope.files_digest != files_digest {
        anyhow::bail!(
            "linkPolicy.currentPackageProduction.filesDigest {} does not match loaded production files digest {}",
            scope.files_digest,
            files_digest
        );
    }
    let implementation_links_digest = canonical_digest(
        &unit.implementation_links,
        "productionPackageUnit.implementationLinks",
    )?;
    if scope.implementation_links_digest != implementation_links_digest {
        anyhow::bail!(
            "linkPolicy.currentPackageProduction.implementationLinksDigest {} does not match loaded production implementation links digest {}",
            scope.implementation_links_digest,
            implementation_links_digest
        );
    }
    Ok(())
}

fn validate_loaded_dependency_public_scope(
    index: usize,
    reference: &PackageTestPackageUnitRef,
    scope: &skiff_artifact_model::PackageDependencyPublicLinkScope,
    unit: &skiff_artifact_model::PackageUnit,
) -> anyhow::Result<()> {
    if scope.package_id != reference.package_id
        || scope.version != reference.version
        || scope.build_identity != reference.build_identity
        || scope.public_abi_identity != reference.public_abi_identity
    {
        anyhow::bail!(
            "linkPolicy.dependencyPublicScopes[{index}] must match dependencyPackageUnits[{index}] identity fields"
        );
    }
    if scope.allow_private {
        anyhow::bail!("linkPolicy.dependencyPublicScopes[{index}].allowPrivate must be false");
    }

    let public_export_digest = canonical_digest(
        &unit.publication_abi,
        &format!("dependencyPackageUnits[{index}].publicationAbi"),
    )?;
    if scope.public_export_digest != public_export_digest {
        anyhow::bail!(
            "linkPolicy.dependencyPublicScopes[{index}].publicExportDigest {} does not match loaded dependency public export digest {}",
            scope.public_export_digest,
            public_export_digest
        );
    }
    let implementation_links_identity = package_implementation_links_identity(unit)?;
    if scope.implementation_links_digest != implementation_links_identity {
        anyhow::bail!(
            "linkPolicy.dependencyPublicScopes[{index}].implementationLinksDigest {} does not match loaded dependency implementation links identity {}",
            scope.implementation_links_digest,
            implementation_links_identity
        );
    }
    Ok(())
}

fn validate_loaded_test_file_scopes(
    assembly: &PackageTestAssembly,
    test_files: &[Arc<FileIrUnit>],
) -> anyhow::Result<()> {
    if test_files.len() != assembly.test_files.len() {
        anyhow::bail!(
            "loaded package-test testFiles count {} does not match assembly testFiles count {}",
            test_files.len(),
            assembly.test_files.len()
        );
    }
    for (index, ((scope, reference), file)) in assembly
        .link_policy
        .test_file_scopes
        .iter()
        .zip(&assembly.test_files)
        .zip(test_files)
        .enumerate()
    {
        if file.file_ir_identity != reference.file_ir_identity {
            anyhow::bail!(
                "package-test testFiles[{index}] loaded fileIrIdentity {} does not match reference {}",
                file.file_ir_identity,
                reference.file_ir_identity
            );
        }
        if file.module_path != reference.module_path {
            anyhow::bail!(
                "package-test testFiles[{index}] loaded modulePath {} does not match reference {}",
                file.module_path,
                reference.module_path
            );
        }
        let expected_digest = package_test_allowed_local_link_digest(scope, reference, file)?;
        if scope.allowed_local_link_digest != expected_digest {
            anyhow::bail!(
                "linkPolicy.testFileScopes[{index}].allowedLocalLinkDigest {} does not match loaded test file local target digest {}",
                scope.allowed_local_link_digest,
                expected_digest
            );
        }
    }
    Ok(())
}

fn package_test_db_metadata(
    production_unit: &PackageUnit,
    production_files: &[Arc<FileIrUnit>],
) -> Vec<DbMetadataIr> {
    production_files
        .iter()
        .flat_map(|file| {
            file.declarations.db.values().map(move |db| DbMetadataIr {
                module_path: file.module_path.clone(),
                source_role: "package".to_string(),
                package_id: Some(production_unit.package_id.clone()),
                package_version: Some(production_unit.version.clone()),
                file_ir_identity: Some(file.file_ir_identity.clone()),
                kind: db.kind,
                ty: db.type_ref.clone(),
                type_name: db.type_name.clone(),
                collection_name: db.collection_name.clone(),
                key: Some(db.key.clone()),
                fields: db.fields.clone(),
                retention: db.retention.clone(),
                leases: db.leases.clone(),
                indexes: db.indexes.iter().map(package_test_db_index).collect(),
            })
        })
        .collect()
}

fn package_test_protocol_identity(assembly: &PackageTestAssembly) -> String {
    let hash = value_sha256(&json!({
        "schemaVersion": "skiff-package-test-runtime-protocol-v1",
        "packageId": assembly.package_id,
        "packageVersion": assembly.package_version,
    }))
    .expect("package-test protocol identity JSON should hash");
    format!("skiff-protocol-v1:sha256:{hash}")
}

fn package_test_db_index(index: &DbIndexIr) -> DbMetadataIndexIr {
    DbMetadataIndexIr {
        name: index.name.clone(),
        unique: index.unique,
        fields: index.fields.clone(),
        where_expr: index.where_expr.clone(),
    }
}

fn package_test_recoverable_metadata(
    service_id: &str,
    production_unit: &PackageUnit,
    production_files: &[Arc<FileIrUnit>],
    test_files: &[Arc<FileIrUnit>],
    dependency_units: &[Arc<PackageUnit>],
    dependency_files: &[Vec<Arc<FileIrUnit>>],
    db_metadata: &[DbMetadataIr],
    spawn_targets: &[SpawnTargetIr],
) -> anyhow::Result<RecoverableArtifactMetadata> {
    let file_ir_units = production_files
        .iter()
        .chain(test_files)
        .map(|file| file.as_ref().clone())
        .collect::<Vec<_>>();
    let mut package_sources = Vec::with_capacity(dependency_units.len() + 1);
    package_sources.push(RecoverablePackageTypeSource {
        package_id: production_unit.package_id.clone(),
        dependency_refs: Vec::new(),
        unit: production_unit.clone(),
        file_ir_units: production_files
            .iter()
            .map(|file| file.as_ref().clone())
            .collect(),
    });
    package_sources.extend(
        dependency_units
            .iter()
            .zip(dependency_files)
            .map(|(unit, files)| RecoverablePackageTypeSource {
                package_id: unit.package_id.clone(),
                dependency_refs: production_unit
                    .dependencies
                    .iter()
                    .filter(|dependency| dependency.id == unit.package_id)
                    .map(|dependency| dependency.alias.clone())
                    .collect(),
                unit: unit.as_ref().clone(),
                file_ir_units: files.iter().map(|file| file.as_ref().clone()).collect(),
            }),
    );

    recoverable_metadata_for_service_artifacts(
        service_id,
        &file_ir_units,
        db_metadata,
        spawn_targets,
        RecoverableInputs {
            package_sources: &package_sources,
            ..RecoverableInputs::default()
        },
    )
    .map_err(|error| {
        anyhow::anyhow!("failed to project package-test recoverable metadata: {error}")
    })
}

fn package_test_spawn_targets(
    production_unit: &PackageUnit,
    production_files: &[Arc<FileIrUnit>],
    dependency_units: &[Arc<PackageUnit>],
    dependency_files: &[Vec<Arc<FileIrUnit>>],
    service_protocol_identity: &str,
) -> anyhow::Result<Vec<SpawnTargetIr>> {
    let mut targets = BTreeMap::<String, SpawnTargetIr>::new();
    for file in production_files {
        for executable in &file.executables {
            for expr in &executable.body.expressions {
                let ExprIr::Call { call } = expr else {
                    continue;
                };
                let Some(metadata) = call.metadata.get(SPAWN_SUBMIT_METADATA_KEY) else {
                    continue;
                };
                if !spawn_submit_is_function(metadata)? {
                    continue;
                }
                let Some(target) = package_test_spawn_target_for_call(
                    production_unit,
                    production_files,
                    dependency_units,
                    dependency_files,
                    file,
                    call,
                    service_protocol_identity,
                )?
                else {
                    continue;
                };
                targets
                    .entry(target.target_identity.clone())
                    .or_insert(target);
            }
        }
    }
    Ok(targets.into_values().collect())
}

fn spawn_submit_is_function(metadata: &MetadataValue) -> anyhow::Result<bool> {
    let MetadataValue::Object(object) = metadata else {
        anyhow::bail!("spawnSubmit metadata must be an object");
    };
    let Some(MetadataValue::String(target_kind)) = object.get("targetKind") else {
        anyhow::bail!("spawnSubmit metadata targetKind must be a string");
    };
    if target_kind != "function" {
        anyhow::bail!("spawn target kind {target_kind} is unsupported");
    }
    Ok(true)
}

fn package_test_spawn_target_for_call(
    production_unit: &PackageUnit,
    production_files: &[Arc<FileIrUnit>],
    dependency_units: &[Arc<PackageUnit>],
    dependency_files: &[Vec<Arc<FileIrUnit>>],
    file: &FileIrUnit,
    call: &skiff_artifact_model::CallIr,
    service_protocol_identity: &str,
) -> anyhow::Result<Option<SpawnTargetIr>> {
    match &call.target {
        CallTargetIr::LocalExecutable { executable_index } => {
            let (declaration_name, declaration) =
                executable_declaration_for_index(file, *executable_index).ok_or_else(|| {
                    anyhow::anyhow!(
                        "spawn target executable index {executable_index} is not declared in package-test production module {}",
                        file.module_path
                    )
                })?;
            let target_identity = format!("{SPAWN_FUNCTION_TARGET_PREFIX}{}", declaration.symbol);
            Ok(Some(function_spawn_target_from_declaration(
                file,
                declaration_name,
                declaration.executable_index,
                target_identity,
                service_protocol_identity,
            )?))
        }
        CallTargetIr::PublicationExecutable {
            module_path,
            executable_index,
        } => {
            let (target_file, declaration_name, declaration) =
                package_test_publication_executable_declaration_for_index(
                    production_files,
                    module_path,
                    *executable_index,
                )?;
            let target_identity = format!("{SPAWN_FUNCTION_TARGET_PREFIX}{}", declaration.symbol);
            Ok(Some(function_spawn_target_from_declaration(
                target_file,
                declaration_name,
                declaration.executable_index,
                target_identity,
                service_protocol_identity,
            )?))
        }
        CallTargetIr::ExternalServiceSymbol { symbol } => {
            let target_identity = format!("{SPAWN_FUNCTION_TARGET_PREFIX}{}", symbol.symbol_path());
            Ok(Some(package_test_service_function_spawn_target(
                production_files,
                &target_identity,
                service_protocol_identity,
            )?))
        }
        CallTargetIr::PackageSymbol {
            package_ref,
            operation,
        } => Ok(Some(package_test_package_operation_spawn_target(
            production_unit,
            dependency_units,
            dependency_files,
            package_ref,
            operation,
            service_protocol_identity,
        )?)),
        _ => Ok(None),
    }
}

fn package_test_publication_executable_declaration_for_index<'a>(
    production_files: &'a [Arc<FileIrUnit>],
    module_path: &str,
    executable_index: u32,
) -> anyhow::Result<(
    &'a FileIrUnit,
    &'a String,
    &'a skiff_artifact_model::ExecutableDeclarationIr,
)> {
    let mut matching_files = production_files
        .iter()
        .filter(|file| file.module_path == module_path);
    let Some(target_file) = matching_files.next() else {
        anyhow::bail!(
            "spawn target executable index {executable_index} references missing package-test production module {module_path}"
        );
    };
    if matching_files.next().is_some() {
        anyhow::bail!(
            "spawn target executable index {executable_index} references duplicate package-test production module {module_path}"
        );
    }
    let (declaration_name, declaration) =
        executable_declaration_for_index(target_file, executable_index).ok_or_else(|| {
            anyhow::anyhow!(
                "spawn target executable index {executable_index} is not declared in package-test production module {module_path}"
            )
        })?;
    Ok((target_file.as_ref(), declaration_name, declaration))
}

fn package_test_service_function_spawn_target(
    production_files: &[Arc<FileIrUnit>],
    target_identity: &str,
    service_protocol_identity: &str,
) -> anyhow::Result<SpawnTargetIr> {
    for file in production_files {
        for (declaration_name, declaration) in &file.declarations.executables {
            let executable = file
                .executables
                .get(declaration.executable_index as usize)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "spawn target {}.{} points to missing executable index {}",
                        file.module_path,
                        declaration_name,
                        declaration.executable_index
                    )
                })?;
            if executable.kind != ExecutableKind::Function {
                continue;
            }
            if format!("{SPAWN_FUNCTION_TARGET_PREFIX}{}", executable.symbol) != target_identity {
                continue;
            }
            return function_spawn_target_from_declaration(
                file,
                declaration_name,
                declaration.executable_index,
                target_identity.to_string(),
                service_protocol_identity,
            );
        }
    }
    anyhow::bail!("spawn target {target_identity} does not resolve to an owner production function")
}

fn package_test_package_operation_spawn_target(
    production_unit: &PackageUnit,
    dependency_units: &[Arc<PackageUnit>],
    dependency_files: &[Vec<Arc<FileIrUnit>>],
    package_ref: &PackageRefIr,
    operation: &skiff_artifact_model::OperationAbiRef,
    service_protocol_identity: &str,
) -> anyhow::Result<SpawnTargetIr> {
    let (package, files) = package_test_dependency_for_ref(
        production_unit,
        dependency_units,
        dependency_files,
        package_ref,
    )
    .ok_or_else(|| {
        anyhow::anyhow!(
            "spawn package target {} does not resolve to a package-test dependency",
            operation.public_path
        )
    })?;
    let Some(target) = package
        .implementation_links
        .operation_targets
        .get(&operation.operation_abi_id)
    else {
        anyhow::bail!(
            "spawn package target {} operationAbiId {} does not resolve to a package operation target",
            package.package_id,
            operation.operation_abi_id
        );
    };
    let PackageOperationTarget::LocalExecutable { target, .. } = target else {
        anyhow::bail!(
            "spawn package target {}.{} resolves to a receiver operation; spawn supports function targets only",
            package.package_id,
            operation.public_path
        );
    };
    let executable = package_test_operation_executable(package, files, target)?;
    let target_identity = package_handler_target(&package.package_id, &operation.public_path);
    Ok(SpawnTargetIr {
        target_identity: target_identity.clone(),
        kind: SpawnTargetKindIr::Function,
        executable_target: target.clone(),
        param_types: executable
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        return_type: spawn_function_return_type(&target_identity, &executable.return_type)?,
        service_protocol_identity: service_protocol_identity.to_string(),
    })
}

fn package_test_dependency_for_ref<'a>(
    production_unit: &'a PackageUnit,
    dependency_units: &'a [Arc<PackageUnit>],
    dependency_files: &'a [Vec<Arc<FileIrUnit>>],
    package_ref: &PackageRefIr,
) -> Option<(&'a PackageUnit, &'a [Arc<FileIrUnit>])> {
    let package_id = match package_ref {
        PackageRefIr::PackageId { package_id } => package_id.as_str(),
        PackageRefIr::Dependency { dependency_ref } => production_unit
            .dependencies
            .iter()
            .find(|dependency| {
                dependency.alias == *dependency_ref || dependency.id == *dependency_ref
            })
            .map(|dependency| dependency.id.as_str())
            .unwrap_or(dependency_ref.as_str()),
    };
    dependency_units
        .iter()
        .zip(dependency_files.iter())
        .find_map(|(unit, files)| {
            (unit.package_id == package_id).then_some((unit.as_ref(), files.as_slice()))
        })
}

fn package_test_operation_executable<'a>(
    package: &PackageUnit,
    files: &'a [Arc<FileIrUnit>],
    target: &OperationTargetRef,
) -> anyhow::Result<&'a skiff_artifact_model::ExecutableIr> {
    let file = files
        .iter()
        .find(|file| {
            file.file_ir_identity == target.file_ref.file_ir_identity
                && file.module_path == target.file_ref.module_path
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "package operation target file {} is missing from package {}",
                target.file_ref.file_ir_identity,
                package.package_id
            )
        })?;
    file.executables
        .get(target.executable_index as usize)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "package operation target {} executable index {} is missing",
                target.file_ref.file_ir_identity,
                target.executable_index
            )
        })
}

fn executable_declaration_for_index(
    file: &FileIrUnit,
    executable_index: u32,
) -> Option<(&String, &skiff_artifact_model::ExecutableDeclarationIr)> {
    file.declarations
        .executables
        .iter()
        .find(|(_, declaration)| declaration.executable_index == executable_index)
}

fn function_spawn_target_from_declaration(
    file: &FileIrUnit,
    declaration_name: &str,
    executable_index: u32,
    target_identity: String,
    service_protocol_identity: &str,
) -> anyhow::Result<SpawnTargetIr> {
    let executable = file
        .executables
        .get(executable_index as usize)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "spawn target {}.{} points to missing executable index {}",
                file.module_path,
                declaration_name,
                executable_index
            )
        })?;
    if executable.kind != ExecutableKind::Function || declaration_name.contains('.') {
        anyhow::bail!("spawn target {target_identity} must resolve to a function");
    }
    Ok(SpawnTargetIr {
        target_identity: target_identity.clone(),
        kind: SpawnTargetKindIr::Function,
        executable_target: OperationTargetRef {
            file_ref: FileIrRef::new(file.file_ir_identity.clone(), file.module_path.clone()),
            executable_index,
            callable_abi_id: format!("callable:{}.{}", file.module_path, declaration_name),
            callable_kind: OperationCallableKind::InternalFunction,
        },
        param_types: executable
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        return_type: spawn_function_return_type(&target_identity, &executable.return_type)?,
        service_protocol_identity: service_protocol_identity.to_string(),
    })
}

fn spawn_function_return_type(
    target_identity: &str,
    ty: &TypeRefIr,
) -> anyhow::Result<Option<TypeRefIr>> {
    match ty {
        TypeRefIr::Native { name, args }
            if args.is_empty() && (name == "void" || name == "null") =>
        {
            Ok(None)
        }
        other => {
            anyhow::bail!("spawn target {target_identity} must return void/null, found {other:?}")
        }
    }
}

fn validate_loaded_package_unit_ref(
    unit: &skiff_artifact_model::PackageUnit,
    reference: &PackageTestPackageUnitRef,
    label: &str,
) -> anyhow::Result<()> {
    if unit.package_id != reference.package_id {
        anyhow::bail!(
            "{label} loaded packageId {} does not match reference {}",
            unit.package_id,
            reference.package_id
        );
    }
    if unit.version != reference.version {
        anyhow::bail!(
            "{label} loaded version {} does not match reference {}",
            unit.version,
            reference.version
        );
    }
    if unit.build_identity != reference.build_identity {
        anyhow::bail!(
            "{label} loaded buildIdentity {} does not match reference {}",
            unit.build_identity,
            reference.build_identity
        );
    }
    if unit.abi_identity != reference.public_abi_identity {
        anyhow::bail!(
            "{label} loaded abiIdentity {} does not match reference publicAbiIdentity {}",
            unit.abi_identity,
            reference.public_abi_identity
        );
    }
    let implementation_links_identity = package_implementation_links_identity(unit)?;
    if implementation_links_identity != reference.implementation_links_identity {
        anyhow::bail!(
            "{label} loaded implementationLinksIdentity {} does not match reference implementationLinksIdentity {}",
            implementation_links_identity,
            reference.implementation_links_identity
        );
    }
    Ok(())
}

fn package_test_file_ir_ref_to_file_ref(reference: &PackageTestFileIrRef) -> FileIrRef {
    FileIrRef {
        file_ir_identity: reference.file_ir_identity.clone(),
        module_path: reference.module_path.clone(),
        artifact_path: Some(reference.file_ir_path.clone()),
        source_ast_hash: None,
    }
}

fn package_test_allowed_local_link_digest(
    scope: &PackageTestFileLinkScope,
    reference: &PackageTestFileIrRef,
    file: &FileIrUnit,
) -> anyhow::Result<String> {
    if reference.file_ir_identity != file.file_ir_identity {
        anyhow::bail!(
            "package-test local link digest file identity mismatch: reference {}, loaded {}",
            reference.file_ir_identity,
            file.file_ir_identity
        );
    }
    if reference.module_path != file.module_path {
        anyhow::bail!(
            "package-test local link digest module mismatch: reference {}, loaded {}",
            reference.module_path,
            file.module_path
        );
    }
    let mut entrypoint_local_ids = scope.entrypoint_local_ids.clone();
    entrypoint_local_ids.sort();
    entrypoint_local_ids.dedup();
    value_sha256(&json!({
        "fileIrIdentity": reference.file_ir_identity,
        "sourcePath": reference.source_path,
        "modulePath": reference.module_path,
        "entrypointLocalIds": entrypoint_local_ids,
        "localTargets": {
            "declarations": &file.declarations,
            "linkTargets": &file.link_targets,
            "typeCount": file.type_table.len(),
            "constCount": file.constants.len(),
            "executableCount": file.executables.len(),
        },
    }))
}

fn package_implementation_links_identity(
    unit: &skiff_artifact_model::PackageUnit,
) -> anyhow::Result<String> {
    Ok(format!(
        "{PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX}:{}",
        canonical_digest(
            &unit.implementation_links,
            "package implementation links identity"
        )?
    ))
}

fn canonical_digest<T: Serialize>(value: &T, label: &str) -> anyhow::Result<String> {
    let value = serde_json::to_value(value)
        .map_err(|error| anyhow::anyhow!("failed to serialize {label}: {error}"))?;
    value_sha256(&value)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::json;
    use skiff_artifact_identity::{
        assign_file_ir_identity, assign_package_unit_identities, derive_package_test_entrypoint_id,
        package_test_build_identity, package_test_entrypoint_local_id,
        PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
    };
    use skiff_artifact_model::{
        interface_instantiation_ref, CallIr as ArtifactCallIr, ConfigAndEffectMetadata,
        DbDeclarationIr, DbObjectFieldIr, DbObjectKeyIr, ExecutableBody as ArtifactExecutableBody,
        ExecutableDeclarationIr as ArtifactExecutableDeclarationIr, ExecutableExport,
        ExecutableIr as ArtifactExecutableIr, ExecutableKind as ArtifactExecutableKind,
        ExecutableSignatureIr, ExprIr as ArtifactExprIr,
        FunctionTypeParamIr as ArtifactFunctionTypeParamIr, InterfaceDeclIr, InterfaceOperationIr,
        MetadataValue as ArtifactMetadataValue, PackageDependencyPublicLinkScope,
        PackageProductionLinkScope, PackageTestAssemblyKind, PackageTestEntrypoint,
        PackageTestEntrypointKind, PackageTestExecutableRef, PackageTestFileLinkScope,
        PackageTestLinkPolicy, PackageTestRuntimeExpectedError, PackageUnit,
        ParamIr as ArtifactParamIr, ReceiverCallAbi, RecoverableStorageLane, ServiceSymbolRef,
        SlotLayout as ArtifactSlotLayout, TypeDeclIr, TypeDeclarationIr, TypeDescriptorIr,
        TypeRefIr,
    };
    use skiff_runtime_loader::{FileIrCache, PackageCache};

    use skiff_runtime_linked_program::{
        CallIr, ConstIr, ExprRefIr, LinkedBoxSourceIr, LinkedCallTarget, LinkedExecutable,
        LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedFunctionTypeParamIr,
        LinkedInterfaceInstantiationRef, LinkedInterfaceMethodSlotPlanIr,
        LinkedInterfaceMethodSlotSignatureIr, LinkedInterfaceMethodSlotTargetIr,
        LinkedInterfaceMethodTablePlanIr, LinkedTypeRef, LiteralIr,
    };

    use super::*;

    static TEMP_ROOT_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn package_test_dispatch_validates_dev_pointer_and_assembly_identity() {
        let (root, selection) = write_package_test_fixture(false);

        let loaded =
            validate_package_test_dispatch_from_artifact_roots(&[root.path_buf()], &selection)
                .expect("package test dispatch should validate");

        assert_eq!(loaded.artifact_root, root.path_buf());
        assert_eq!(loaded.entrypoint_id, selection.entrypoint_id);
        assert!(loaded
            .assembly_path
            .parent()
            .expect("assembly path should have parent")
            .ends_with("assemblies/package-tests/example~com~~pkg"));
    }

    #[test]
    fn package_test_dispatch_loads_entrypoint_metadata_from_assembly() {
        let mut entrypoint_metadata = ConfigAndEffectMetadata::default();
        entrypoint_metadata.config.insert(
            "first.secret".to_string(),
            ArtifactMetadataValue::Bool(true),
        );
        let expected_metadata = entrypoint_metadata.clone();
        let (root, selection) = write_package_test_fixture_with(false, move |assembly| {
            assembly.test_entrypoints[0].config_and_effect_metadata = entrypoint_metadata;
        });

        let loaded =
            load_package_test_dispatch_artifact_from_artifact_roots(&[root.path_buf()], &selection)
                .expect("package test dispatch should load entrypoint metadata");

        assert_eq!(
            loaded.entrypoint.config_and_effect_metadata,
            expected_metadata
        );
    }

    #[test]
    fn package_test_dispatch_rejects_legacy_top_level_assembly_pointer() {
        let (root, selection) = write_package_test_fixture(false);
        let hash = identity_hash(
            &selection.test_build_identity,
            PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
            "testBuildIdentity",
        )
        .expect("test build hash");
        let package_path =
            publication_storage_segment(&selection.package_id, "packageId").expect("package path");
        let assembly_relative = PathBuf::from("assemblies")
            .join("package-tests")
            .join(&package_path)
            .join(format!("{hash}.json"));
        let pointer_path = package_test_pointer_path(&root, &selection);
        fs::write(
            &pointer_path,
            serde_json::to_vec_pretty(&json!({
                "schemaVersion": "skiff-package-test-dev-pointer-v1",
                "packageId": selection.package_id,
                "packageVersion": selection.package_version,
                "testBuildIdentity": selection.test_build_identity,
                "assemblyPath": assembly_relative.to_string_lossy(),
                "assemblyIdentity": selection.test_build_identity
            }))
            .expect("legacy pointer json"),
        )
        .expect("write legacy pointer");

        let error =
            validate_package_test_dispatch_from_artifact_roots(&[root.path_buf()], &selection)
                .expect_err("legacy top-level assembly pointer must fail closed");

        let message = error.to_string();
        assert!(message.contains("assemblyPath") || message.contains("packageTestAssembly"));
    }

    #[test]
    fn package_test_dispatch_rejects_tampered_entrypoint_id() {
        let (root, selection) = write_package_test_fixture(true);

        let error =
            validate_package_test_dispatch_from_artifact_roots(&[root.path_buf()], &selection)
                .expect_err("tampered persisted entrypoint id must fail");

        assert!(error.to_string().contains("package test entrypoint"));
        assert!(error.to_string().contains("entrypointId"));
    }

    #[test]
    fn package_test_dispatch_rejects_tampered_entrypoint_local_id() {
        let (root, selection) = write_package_test_fixture_with(false, |assembly| {
            assembly.test_entrypoints[0].entrypoint_local_id =
                "not-a-package-test-entrypoint-local-id".to_string();
            assembly.link_policy.test_file_scopes[0].entrypoint_local_ids =
                vec![assembly.test_entrypoints[0].entrypoint_local_id.clone()];
        });

        let error =
            validate_package_test_dispatch_from_artifact_roots(&[root.path_buf()], &selection)
                .expect_err("tampered persisted entrypoint local id must fail");

        assert!(error.to_string().contains("entrypointLocalId"));
    }

    #[test]
    fn package_test_dispatch_rejects_entrypoint_source_module_mismatch() {
        let (root, selection) = write_package_test_fixture_with(false, |assembly| {
            assembly.test_entrypoints[0].source_path = "pkg.other.test.skiff".to_string();
        });

        let error =
            validate_package_test_dispatch_from_artifact_roots(&[root.path_buf()], &selection)
                .expect_err("entrypoint sourcePath mismatch must fail");

        assert!(error.to_string().contains("sourcePath"));

        let (root, selection) = write_package_test_fixture_with(false, |assembly| {
            assembly.test_entrypoints[0].module_path = "pkg.other_test".to_string();
        });

        let error =
            validate_package_test_dispatch_from_artifact_roots(&[root.path_buf()], &selection)
                .expect_err("entrypoint modulePath mismatch must fail");

        assert!(error.to_string().contains("modulePath"));
    }

    #[test]
    fn package_test_dispatch_accepts_multiple_owner_test_files() {
        let (root, selection) = write_package_test_fixture_with(false, |assembly| {
            let mut other = assembly.test_files[0].clone();
            other.file_ir_identity =
                "skiff-file-ir-v3:sha256:9999999999999999999999999999999999999999999999999999999999999999"
                    .to_string();
            other.file_ir_path =
                "units/files/9999999999999999999999999999999999999999999999999999999999999999.json"
                    .to_string();
            other.source_path = "pkg.other.test.skiff".to_string();
            other.module_path = "pkg.other_test".to_string();
            let other_entrypoint_local_id = package_test_entrypoint_local_id(
                &assembly.package_id,
                &assembly.package_version,
                &other.source_path,
                0,
                "runs other owner",
            )
            .expect("other entrypoint local id");
            assembly.test_files.push(other.clone());
            assembly.test_entrypoints.push(PackageTestEntrypoint {
                kind: PackageTestEntrypointKind::TestOnly,
                entrypoint_local_id: other_entrypoint_local_id.clone(),
                entrypoint_id: String::new(),
                display_name: "runs other owner".to_string(),
                source_path: other.source_path.clone(),
                module_path: other.module_path.clone(),
                owner_test_file: other.clone(),
                executable_ref: PackageTestExecutableRef {
                    file_ir_identity: other.file_ir_identity.clone(),
                    executable_index: 0,
                    executable_local_id: "entrypoint-0".to_string(),
                    symbol: Some("pkg.other_test.__skiff_package_test_0".to_string()),
                },
                default_run: true,
                config_and_effect_metadata: ConfigAndEffectMetadata::default(),
                runtime_expected_error: None,
            });
            assembly
                .link_policy
                .test_file_scopes
                .push(PackageTestFileLinkScope {
                    owner_test_file_identity: other.file_ir_identity.clone(),
                    source_path: other.source_path.clone(),
                    module_path: other.module_path.clone(),
                    allowed_local_link_digest: value_sha256(&json!({
                        "fixture": "pkg.other.test.skiff"
                    }))
                    .expect("test file scope digest"),
                    entrypoint_local_ids: vec![other_entrypoint_local_id],
                });
        });

        let loaded =
            validate_package_test_dispatch_from_artifact_roots(&[root.path_buf()], &selection)
                .expect("package-test assembly should allow multiple owner test files");

        assert_eq!(loaded.entrypoint_id, selection.entrypoint_id);
    }

    #[test]
    fn package_test_dispatch_rejects_illegal_activation_id() {
        let (root, mut selection) = write_package_test_fixture(false);
        selection.activation_id = "skiff-package-test-run-v1:%2f..%2f".to_string();

        let error =
            validate_package_test_dispatch_from_artifact_roots(&[root.path_buf()], &selection)
                .expect_err("illegal activation id must fail closed");

        assert!(error.to_string().contains("activationId"));
    }

    #[test]
    fn package_test_dispatch_rejects_entrypoint_not_authorized_by_link_policy() {
        let (root, selection) = write_package_test_fixture_with(false, |assembly| {
            assembly.link_policy.test_file_scopes[0]
                .entrypoint_local_ids
                .clear();
        });

        let error =
            validate_package_test_dispatch_from_artifact_roots(&[root.path_buf()], &selection)
                .expect_err("missing link policy entrypoint scope must fail");

        assert!(error.to_string().contains("linkPolicy.testFileScopes"));
        assert!(error.to_string().contains("local id"));
    }

    #[test]
    fn package_test_dispatch_fails_closed_without_artifact_roots() {
        let selection = PackageTestDispatchSelection {
            package_id: "example.com/pkg".to_string(),
            package_version: "1.0.0".to_string(),
            test_build_identity:
                "skiff-package-test-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            entrypoint_id:
                "skiff-package-test-entrypoint-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                    .to_string(),
            activation_id: "skiff-package-test-run-v1:example~com~~pkg:run:1".to_string(),
        };

        let error = validate_package_test_dispatch_from_artifact_roots(&[], &selection)
            .expect_err("missing artifact roots must fail closed");

        assert!(error
            .to_string()
            .contains("no artifact roots are configured for package-test dispatch"));
    }

    #[test]
    fn package_test_db_metadata_projects_owner_production_db_objects() {
        let production_unit = package_unit_fixture(
            "example.com/agent",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "3333333333333333333333333333333333333333333333333333333333333333",
        );
        let production_file = Arc::new(file_ir_with_db_object(
            "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "model",
            "AgentThread",
            "agentThread",
        ));
        let owner_test_file = Arc::new(file_ir_with_db_object(
            "skiff-file-ir-v3:sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "agent.test",
            "TestOnlyScratch",
            "testOnlyScratch",
        ));

        let metadata = package_test_db_metadata(&production_unit, &[production_file]);

        assert_eq!(metadata.len(), 1);
        assert_eq!(metadata[0].module_path, "model");
        assert_eq!(metadata[0].source_role, "package");
        assert_eq!(metadata[0].package_id.as_deref(), Some("example.com/agent"));
        assert_eq!(metadata[0].package_version.as_deref(), Some("1.0.0"));
        assert_eq!(metadata[0].type_name, "AgentThread");
        assert_eq!(metadata[0].collection_name, "agentThread");
        assert!(metadata
            .iter()
            .all(|entry| entry.file_ir_identity != Some(owner_test_file.file_ir_identity.clone())));
    }

    #[test]
    fn package_test_recoverable_metadata_projects_any_interface_db_lanes() {
        let production_unit = package_unit_fixture(
            "example.com/agent",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "3333333333333333333333333333333333333333333333333333333333333333",
        );
        let production_file = Arc::new(file_ir_with_recoverable_agent_run_db_object(
            "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "agent.run",
        ));
        let metadata = package_test_db_metadata(&production_unit, &[production_file.clone()]);

        let recoverable = package_test_recoverable_metadata(
            "__skiff.package-test/example.com/agent",
            &production_unit,
            &[production_file],
            &[],
            &[],
            &[],
            &metadata,
            &[],
        )
        .expect("package-test recoverable metadata should project");

        assert_eq!(
            recoverable.storage_lanes["db:AgentRun:field:runtimeBindings"].lane,
            RecoverableStorageLane::RecoverableEnvelope
        );
        assert_eq!(
            recoverable.storage_lanes["db:AgentRun:field:currentConfig"].lane,
            RecoverableStorageLane::RecoverableEnvelope
        );
        assert!(
            recoverable.boundary_plans["db:AgentRun:field:runtimeBindings"]
                .runtime_carrier_check_required
        );
    }

    #[test]
    fn package_test_recoverable_metadata_rejects_non_recoverable_db_function_fields() {
        let production_unit = package_unit_fixture(
            "example.com/agent",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "3333333333333333333333333333333333333333333333333333333333333333",
        );
        let mut production_file = file_ir_with_recoverable_agent_run_db_object(
            "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "agent.run",
        );
        let callback_ty = TypeRefIr::Function {
            params: vec![ArtifactFunctionTypeParamIr {
                name: "input".to_string(),
                ty: TypeRefIr::native("string"),
            }],
            return_type: Box::new(TypeRefIr::native("string")),
        };
        production_file
            .declarations
            .db
            .get_mut("AgentRun")
            .expect("AgentRun DB declaration")
            .fields
            .push(DbObjectFieldIr {
                name: "callback".to_string(),
                ty: callback_ty.clone(),
            });
        let TypeDescriptorIr::Record { fields } = &mut production_file.type_table[0].descriptor
        else {
            panic!("AgentRun type should be a record");
        };
        fields.insert("callback".to_string(), callback_ty);
        let production_file = Arc::new(production_file);
        let metadata = package_test_db_metadata(&production_unit, &[production_file.clone()]);

        let error = package_test_recoverable_metadata(
            "__skiff.package-test/example.com/agent",
            &production_unit,
            &[production_file],
            &[],
            &[],
            &[],
            &metadata,
            &[],
        )
        .expect_err("function DB fields must not enter recoverable package-test metadata");

        let message = error.to_string();
        assert!(message.contains("db field AgentRun.callback"));
        assert!(message.contains("callback function type"));
    }

    #[test]
    fn package_test_builder_loads_recoverable_db_metadata_for_any_interface_fields() {
        let root = unique_temp_dir();
        let (selection, _production_unit, _production_file, _test_file) =
            write_package_test_builder_recoverable_fixture(&root);
        let file_cache = FileIrCache::new();
        let package_cache = PackageCache::new();
        let loaded = PackageTestRuntimeBuilder::new(
            &[root.path_buf()],
            ArtifactGraphCache::new(&file_cache, &package_cache),
        )
        .load(&selection)
        .expect("package-test builder should load synthetic service");

        let service = loaded.synthetic_service_unit.as_ref();
        let agent_run = service
            .db
            .iter()
            .find(|entry| entry.type_name == "AgentRun")
            .expect("AgentRun DB metadata");
        assert!(agent_run
            .fields
            .iter()
            .any(|field| field.name == "runtimeBindings"));
        assert_eq!(
            service.recoverable_metadata.storage_lanes["db:AgentRun:field:runtimeBindings"].lane,
            RecoverableStorageLane::RecoverableEnvelope
        );
        assert_eq!(
            service.recoverable_metadata.storage_lanes["db:AgentRun:field:currentConfig"].lane,
            RecoverableStorageLane::RecoverableEnvelope
        );
        assert!(
            service.recoverable_metadata.boundary_plans["db:AgentRun:field:runtimeBindings"]
                .runtime_carrier_check_required
        );
    }

    #[test]
    fn package_test_spawn_targets_project_owner_production_function_routes() {
        let production_unit = package_unit_fixture(
            "example.com/agent",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "3333333333333333333333333333333333333333333333333333333333333333",
        );
        let wake_file = Arc::new(file_ir_with_spawn_call(
            "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "thread",
            "wakeThreadDrain",
            "runner",
            "runThreadDrain",
            "function:runner.runThreadDrain",
        ));
        let runner_file = Arc::new(file_ir_with_function(
            "skiff-file-ir-v3:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "runner",
            "runThreadDrain",
        ));

        let targets = package_test_spawn_targets(
            &production_unit,
            &[wake_file, runner_file],
            &[],
            &[],
            "package-test-protocol",
        )
        .expect("owner production spawn targets should project");

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].target_identity, "function:runner.runThreadDrain");
        assert_eq!(targets[0].kind, SpawnTargetKindIr::Function);
        assert_eq!(targets[0].executable_target.file_ref.module_path, "runner");
        assert_eq!(targets[0].executable_target.executable_index, 0);
        assert_eq!(
            targets[0].executable_target.callable_kind,
            OperationCallableKind::InternalFunction
        );
        assert_eq!(targets[0].return_type, None);
    }

    #[test]
    fn package_test_spawn_targets_project_owner_publication_executable_routes() {
        let production_unit = package_unit_fixture(
            "example.com/agent",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "3333333333333333333333333333333333333333333333333333333333333333",
        );
        let wake_file = Arc::new(file_ir_with_publication_spawn_call(
            "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "thread",
            "wakeThreadDrain",
            "runner",
            0,
            "function:runner.runThreadDrain",
        ));
        let runner_file = Arc::new(file_ir_with_function(
            "skiff-file-ir-v3:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "runner",
            "runThreadDrain",
        ));

        let targets = package_test_spawn_targets(
            &production_unit,
            &[wake_file, runner_file],
            &[],
            &[],
            "package-test-protocol",
        )
        .expect("owner publication executable spawn targets should project");

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].target_identity, "function:runner.runThreadDrain");
        assert_eq!(targets[0].kind, SpawnTargetKindIr::Function);
        assert_eq!(targets[0].executable_target.file_ref.module_path, "runner");
        assert_eq!(
            targets[0].executable_target.file_ref.file_ir_identity,
            "skiff-file-ir-v3:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        assert_eq!(targets[0].executable_target.executable_index, 0);
        assert_eq!(
            targets[0].executable_target.callable_abi_id,
            "callable:runner.runThreadDrain"
        );
        assert_eq!(
            targets[0].executable_target.callable_kind,
            OperationCallableKind::InternalFunction
        );
        assert_eq!(targets[0].return_type, None);
    }

    #[test]
    fn loaded_package_test_rejects_tampered_production_implementation_links_identity() {
        let (mut assembly, production_unit, dependency_units) = loaded_policy_fixture();
        assembly
            .production_package_unit
            .implementation_links_identity = format!(
            "{PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX}:0000000000000000000000000000000000000000000000000000000000000000"
        );

        let error = validate_loaded_package_test_link_policy(
            &assembly,
            &production_unit,
            &dependency_units,
        )
        .expect_err("tampered production implementation links identity must fail");

        assert!(error
            .to_string()
            .contains("productionPackageUnit loaded implementationLinksIdentity"));
    }

    #[test]
    fn loaded_package_test_rejects_tampered_dependency_implementation_links_identity() {
        let (mut assembly, production_unit, dependency_units) = loaded_policy_fixture();
        assembly.dependency_package_units[0].implementation_links_identity = format!(
            "{PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX}:0000000000000000000000000000000000000000000000000000000000000000"
        );

        let error = validate_loaded_package_test_link_policy(
            &assembly,
            &production_unit,
            &dependency_units,
        )
        .expect_err("tampered dependency implementation links identity must fail");

        assert!(error
            .to_string()
            .contains("dependencyPackageUnits[0] loaded implementationLinksIdentity"));
    }

    #[test]
    fn loaded_package_test_rejects_tampered_dependency_public_export_digest() {
        let (mut assembly, production_unit, dependency_units) = loaded_policy_fixture();
        assembly.link_policy.dependency_public_scopes[0].public_export_digest =
            "0000000000000000000000000000000000000000000000000000000000000000".to_string();

        let error = validate_loaded_package_test_link_policy(
            &assembly,
            &production_unit,
            &dependency_units,
        )
        .expect_err("tampered dependency public export digest must fail");

        assert!(error
            .to_string()
            .contains("dependencyPublicScopes[0].publicExportDigest"));
    }

    #[test]
    fn loaded_package_test_rejects_tampered_dependency_implementation_links_digest() {
        let (mut assembly, production_unit, dependency_units) = loaded_policy_fixture();
        assembly.link_policy.dependency_public_scopes[0].implementation_links_digest = format!(
            "{PACKAGE_IMPLEMENTATION_LINKS_IDENTITY_PREFIX}:0000000000000000000000000000000000000000000000000000000000000000"
        );

        let error = validate_loaded_package_test_link_policy(
            &assembly,
            &production_unit,
            &dependency_units,
        )
        .expect_err("tampered dependency implementation links digest must fail");

        assert!(error
            .to_string()
            .contains("dependencyPublicScopes[0].implementationLinksDigest"));
    }

    #[test]
    fn loaded_package_test_rejects_tampered_current_package_production_digest() {
        let (mut assembly, production_unit, dependency_units) = loaded_policy_fixture();
        assembly
            .link_policy
            .current_package_production
            .implementation_links_digest =
            "0000000000000000000000000000000000000000000000000000000000000000".to_string();

        let error = validate_loaded_package_test_link_policy(
            &assembly,
            &production_unit,
            &dependency_units,
        )
        .expect_err("tampered current package production digest must fail");

        assert!(error
            .to_string()
            .contains("currentPackageProduction.implementationLinksDigest"));
    }

    #[test]
    fn package_test_graph_allows_direct_dependency_public_executable() {
        let (dispatch, image, production_unit, entrypoint_addr) =
            package_test_graph_fixture(LinkedCallTarget::Executable {
                addr: ExecutableAddr::package(0, 0, 0),
            });

        validate_package_test_executable_graph(
            &dispatch,
            &image,
            &entrypoint_addr,
            &production_unit,
        )
        .expect("dependency public executable target should be allowed");
    }

    #[test]
    fn package_test_graph_rejects_direct_dependency_private_executable() {
        let (dispatch, image, production_unit, entrypoint_addr) =
            package_test_graph_fixture(LinkedCallTarget::Executable {
                addr: ExecutableAddr::package(0, 0, 1),
            });

        let error = validate_package_test_executable_graph(
            &dispatch,
            &image,
            &entrypoint_addr,
            &production_unit,
        )
        .expect_err("dependency private executable target must fail closed");

        assert!(error.to_string().contains("dependency private executable"));
    }

    #[test]
    fn package_test_graph_rejects_cross_test_file_helper() {
        let (dispatch, image, production_unit, entrypoint_addr) =
            package_test_graph_fixture(LinkedCallTarget::Executable {
                addr: ExecutableAddr::service(2, 0),
            });

        let error = validate_package_test_executable_graph(
            &dispatch,
            &image,
            &entrypoint_addr,
            &production_unit,
        )
        .expect_err("cross test file helper target must fail closed");

        assert!(error
            .to_string()
            .contains("neither current package production nor owner test file"));
    }

    #[test]
    fn package_test_graph_scans_interface_box_method_table_target() {
        let (dispatch, image, production_unit, entrypoint_addr) =
            package_test_graph_interface_box_fixture(empty_executable("owner_interface_impl"));

        validate_package_test_executable_graph(
            &dispatch,
            &image,
            &entrypoint_addr,
            &production_unit,
        )
        .expect("owner interface method table target should be scanned and allowed");
    }

    #[test]
    fn package_test_graph_rejects_hidden_edge_inside_interface_box_target() {
        let (dispatch, image, production_unit, entrypoint_addr) =
            package_test_graph_interface_box_fixture(executable_calling(
                "owner_interface_impl",
                LinkedCallTarget::Executable {
                    addr: ExecutableAddr::service(2, 0),
                },
            ));

        let error = validate_package_test_executable_graph(
            &dispatch,
            &image,
            &entrypoint_addr,
            &production_unit,
        )
        .expect_err("interface method table target body must be part of the package-test graph");

        assert!(error
            .to_string()
            .contains("neither current package production nor owner test file"));
    }

    fn write_package_test_fixture(
        tamper_entrypoint: bool,
    ) -> (TempRoot, PackageTestDispatchSelection) {
        write_package_test_fixture_with(tamper_entrypoint, |_| {})
    }

    fn write_package_test_fixture_with<F>(
        tamper_entrypoint: bool,
        mutate_before_identity: F,
    ) -> (TempRoot, PackageTestDispatchSelection)
    where
        F: FnOnce(&mut PackageTestAssembly),
    {
        let root = unique_temp_dir();
        let mut assembly = package_test_assembly_fixture();
        mutate_before_identity(&mut assembly);
        assembly.test_build_identity =
            package_test_build_identity(&assembly).expect("test build identity");
        let mut selected_entrypoint_id = String::new();
        for (index, entrypoint) in assembly.test_entrypoints.iter_mut().enumerate() {
            let entrypoint_id = derive_package_test_entrypoint_id(
                &assembly.test_build_identity,
                &entrypoint.entrypoint_local_id,
            )
            .expect("entrypoint id");
            if index == 0 {
                selected_entrypoint_id = entrypoint_id.clone();
                entrypoint.entrypoint_id = if tamper_entrypoint {
                    "skiff-package-test-entrypoint-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                        .to_string()
                } else {
                    entrypoint_id
                };
            } else {
                entrypoint.entrypoint_id = entrypoint_id;
            }
        }

        let hash = identity_hash(
            &assembly.test_build_identity,
            PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
            "testBuildIdentity",
        )
        .expect("test build hash");
        let package_path =
            publication_storage_segment(&assembly.package_id, "packageId").expect("package path");
        let assembly_relative = PathBuf::from("assemblies")
            .join("package-tests")
            .join(&package_path)
            .join(format!("{hash}.json"));
        let pointer_relative = PathBuf::from("dev")
            .join("package-tests")
            .join(&package_path)
            .join(format!("{hash}.json"));
        fs::create_dir_all(root.path().join(assembly_relative.parent().unwrap()))
            .expect("assembly dir");
        fs::create_dir_all(root.path().join(pointer_relative.parent().unwrap()))
            .expect("pointer dir");
        fs::write(
            root.path().join(&assembly_relative),
            serde_json::to_vec_pretty(&assembly).expect("assembly json"),
        )
        .expect("write assembly");
        fs::write(
            root.path().join(&pointer_relative),
            serde_json::to_vec_pretty(&json!({
                "schemaVersion": "skiff-package-test-dev-pointer-v1",
                "packageId": assembly.package_id,
                "packageVersion": assembly.package_version,
                "testBuildIdentity": assembly.test_build_identity,
                "packageTestAssembly": {
                    "assemblyPath": assembly_relative.to_string_lossy(),
                    "assemblyIdentity": assembly.test_build_identity
                }
            }))
            .expect("pointer json"),
        )
        .expect("write pointer");

        (
            root,
            PackageTestDispatchSelection {
                package_id: "example.com/pkg".to_string(),
                package_version: "1.0.0".to_string(),
                test_build_identity: assembly.test_build_identity,
                entrypoint_id: selected_entrypoint_id,
                activation_id: "skiff-package-test-run-v1:example~com~~pkg:run:1".to_string(),
            },
        )
    }

    fn write_package_test_builder_recoverable_fixture(
        root: &TempRoot,
    ) -> (
        PackageTestDispatchSelection,
        PackageUnit,
        FileIrUnit,
        FileIrUnit,
    ) {
        let mut production_file =
            file_ir_with_recoverable_agent_run_db_object(String::new(), "agent.run");
        assign_file_ir_identity(&mut production_file).expect("production file identity");
        let production_file_path = file_artifact_path(&production_file);

        let mut test_file =
            file_ir_with_function(String::new(), "agent.test", "__skiff_package_test_0");
        assign_file_ir_identity(&mut test_file).expect("test file identity");
        let test_file_path = file_artifact_path(&test_file);

        let mut production_unit =
            PackageUnit::empty("example.com/pkg", "1.0.0", String::new(), String::new());
        production_unit.files = vec![file_ref_for_artifact(
            &production_file,
            &production_file_path,
        )];
        assign_package_unit_identities(&mut production_unit).expect("production package identity");

        write_json_artifact(root, &production_file_path, &production_file);
        write_json_artifact(root, &test_file_path, &test_file);
        let production_unit_ref = package_unit_ref_fixture(&production_unit);
        write_json_artifact(root, &production_unit_ref.unit_path, &production_unit);

        let source_path = "agent.test.skiff";
        let entrypoint_local_id = package_test_entrypoint_local_id(
            &production_unit.package_id,
            &production_unit.version,
            source_path,
            0,
            "loads recoverable DB metadata",
        )
        .expect("entrypoint local id");
        let owner_test_file = PackageTestFileIrRef {
            file_ir_identity: test_file.file_ir_identity.clone(),
            file_ir_path: test_file_path.clone(),
            source_path: source_path.to_string(),
            module_path: test_file.module_path.clone(),
        };
        let test_scope = PackageTestFileLinkScope {
            owner_test_file_identity: owner_test_file.file_ir_identity.clone(),
            source_path: source_path.to_string(),
            module_path: owner_test_file.module_path.clone(),
            allowed_local_link_digest: String::new(),
            entrypoint_local_ids: vec![entrypoint_local_id.clone()],
        };
        let mut assembly = PackageTestAssembly {
            schema_version: "skiff-package-test-assembly-v1".to_string(),
            kind: PackageTestAssemblyKind::PackageTest,
            package_id: production_unit.package_id.clone(),
            package_version: production_unit.version.clone(),
            test_build_identity: String::new(),
            production_package_unit: production_unit_ref,
            test_files: vec![owner_test_file.clone()],
            dependency_package_units: Vec::new(),
            test_entrypoints: vec![PackageTestEntrypoint {
                kind: PackageTestEntrypointKind::TestOnly,
                entrypoint_local_id: entrypoint_local_id.clone(),
                entrypoint_id: String::new(),
                display_name: "loads recoverable DB metadata".to_string(),
                source_path: source_path.to_string(),
                module_path: owner_test_file.module_path.clone(),
                owner_test_file: owner_test_file.clone(),
                executable_ref: PackageTestExecutableRef {
                    file_ir_identity: owner_test_file.file_ir_identity.clone(),
                    executable_index: 0,
                    executable_local_id: "entrypoint-0".to_string(),
                    symbol: Some("agent.test.__skiff_package_test_0".to_string()),
                },
                default_run: true,
                config_and_effect_metadata: ConfigAndEffectMetadata::default(),
                runtime_expected_error: None,
            }],
            link_policy: PackageTestLinkPolicy {
                current_package_production: PackageProductionLinkScope {
                    package_id: production_unit.package_id.clone(),
                    version: production_unit.version.clone(),
                    build_identity: production_unit.build_identity.clone(),
                    files_digest: canonical_digest(
                        &production_unit.files,
                        "builder fixture production files",
                    )
                    .expect("production files digest"),
                    implementation_links_digest: canonical_digest(
                        &production_unit.implementation_links,
                        "builder fixture production implementation links",
                    )
                    .expect("production implementation links digest"),
                    allow_private: true,
                },
                test_file_scopes: vec![test_scope],
                dependency_public_scopes: Vec::new(),
            },
            config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            source_map: json!({}),
        };
        assembly.link_policy.test_file_scopes[0].allowed_local_link_digest =
            package_test_allowed_local_link_digest(
                &assembly.link_policy.test_file_scopes[0],
                &assembly.test_files[0],
                &test_file,
            )
            .expect("test file local link digest");
        assembly.test_build_identity =
            package_test_build_identity(&assembly).expect("test build identity");
        let entrypoint_id = derive_package_test_entrypoint_id(
            &assembly.test_build_identity,
            &assembly.test_entrypoints[0].entrypoint_local_id,
        )
        .expect("entrypoint id");
        assembly.test_entrypoints[0].entrypoint_id = entrypoint_id.clone();

        let assembly_path = write_package_test_assembly_and_pointer(root, &assembly);
        let _ = assembly_path;

        (
            PackageTestDispatchSelection {
                package_id: assembly.package_id.clone(),
                package_version: assembly.package_version.clone(),
                test_build_identity: assembly.test_build_identity.clone(),
                entrypoint_id,
                activation_id: "skiff-package-test-run-v1:example~com~~pkg:run:recoverable"
                    .to_string(),
            },
            production_unit,
            production_file,
            test_file,
        )
    }

    fn write_package_test_assembly_and_pointer(
        root: &TempRoot,
        assembly: &PackageTestAssembly,
    ) -> PathBuf {
        let hash = identity_hash(
            &assembly.test_build_identity,
            PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
            "testBuildIdentity",
        )
        .expect("test build hash");
        let package_path =
            publication_storage_segment(&assembly.package_id, "packageId").expect("package path");
        let assembly_relative = PathBuf::from("assemblies")
            .join("package-tests")
            .join(&package_path)
            .join(format!("{hash}.json"));
        let pointer_relative = PathBuf::from("dev")
            .join("package-tests")
            .join(&package_path)
            .join(format!("{hash}.json"));
        write_json_artifact(root, &assembly_relative, assembly);
        write_json_artifact(
            root,
            &pointer_relative,
            &json!({
                "schemaVersion": "skiff-package-test-dev-pointer-v1",
                "packageId": assembly.package_id,
                "packageVersion": assembly.package_version,
                "testBuildIdentity": assembly.test_build_identity,
                "packageTestAssembly": {
                    "assemblyPath": assembly_relative.to_string_lossy(),
                    "assemblyIdentity": assembly.test_build_identity
                }
            }),
        );
        assembly_relative
    }

    fn package_test_pointer_path(
        root: &TempRoot,
        selection: &PackageTestDispatchSelection,
    ) -> PathBuf {
        let hash = identity_hash(
            &selection.test_build_identity,
            PACKAGE_TEST_BUILD_IDENTITY_PREFIX,
            "testBuildIdentity",
        )
        .expect("test build hash");
        let package_path =
            publication_storage_segment(&selection.package_id, "packageId").expect("package path");
        root.path()
            .join("dev")
            .join("package-tests")
            .join(package_path)
            .join(format!("{hash}.json"))
    }

    fn package_test_assembly_fixture() -> PackageTestAssembly {
        let production_unit = package_unit_fixture(
            "example.com/pkg",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "3333333333333333333333333333333333333333333333333333333333333333",
        );
        let dependency_unit = package_unit_fixture(
            "example.com/dep",
            "4444444444444444444444444444444444444444444444444444444444444444",
            "5555555555555555555555555555555555555555555555555555555555555555",
        );
        let owner_test_file = PackageTestFileIrRef {
            file_ir_identity:
                "skiff-file-ir-v3:sha256:1111111111111111111111111111111111111111111111111111111111111111"
                    .to_string(),
            file_ir_path:
                "units/files/1111111111111111111111111111111111111111111111111111111111111111.json"
                    .to_string(),
            source_path: "pkg.test.skiff".to_string(),
            module_path: "pkg.test".to_string(),
        };
        let entrypoint_local_id = package_test_entrypoint_local_id(
            "example.com/pkg",
            "1.0.0",
            "pkg.test.skiff",
            0,
            "runs internal helper",
        )
        .expect("entrypoint local id");

        let mut assembly = PackageTestAssembly {
            schema_version: "skiff-package-test-assembly-v1".to_string(),
            kind: PackageTestAssemblyKind::PackageTest,
            package_id: "example.com/pkg".to_string(),
            package_version: "1.0.0".to_string(),
            test_build_identity:
                "skiff-package-test-build-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
            production_package_unit: package_unit_ref_fixture(&production_unit),
            test_files: vec![owner_test_file.clone()],
            dependency_package_units: vec![package_unit_ref_fixture(&dependency_unit)],
            test_entrypoints: vec![PackageTestEntrypoint {
                kind: PackageTestEntrypointKind::TestOnly,
                entrypoint_local_id: entrypoint_local_id.clone(),
                entrypoint_id:
                    "skiff-package-test-entrypoint-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                display_name: "runs internal helper".to_string(),
                source_path: "pkg.test.skiff".to_string(),
                module_path: "pkg.test".to_string(),
                owner_test_file: owner_test_file.clone(),
                executable_ref: PackageTestExecutableRef {
                    file_ir_identity: owner_test_file.file_ir_identity.clone(),
                    executable_index: 0,
                    executable_local_id: "entrypoint-0".to_string(),
                    symbol: Some("__skiff_package_test_0".to_string()),
                },
                default_run: true,
                config_and_effect_metadata: ConfigAndEffectMetadata::default(),
                runtime_expected_error: Some(PackageTestRuntimeExpectedError {
                    code: "ExpectedError".to_string(),
                    message_contains: None,
                }),
            }],
            link_policy: PackageTestLinkPolicy {
                current_package_production: PackageProductionLinkScope {
                    package_id: String::new(),
                    version: String::new(),
                    build_identity: String::new(),
                    files_digest: String::new(),
                    implementation_links_digest: String::new(),
                    allow_private: true,
                },
                test_file_scopes: vec![PackageTestFileLinkScope {
                    owner_test_file_identity: owner_test_file.file_ir_identity.clone(),
                    source_path: "pkg.test.skiff".to_string(),
                    module_path: "pkg.test".to_string(),
                    allowed_local_link_digest: value_sha256(&json!({
                        "fixture": "pkg.test.skiff"
                    }))
                    .expect("test file scope digest"),
                    entrypoint_local_ids: vec![entrypoint_local_id],
                }],
                dependency_public_scopes: vec![PackageDependencyPublicLinkScope {
                    package_id: String::new(),
                    version: String::new(),
                    build_identity: String::new(),
                    public_abi_identity: String::new(),
                    public_export_digest: String::new(),
                    implementation_links_digest: String::new(),
                    allow_private: false,
                }],
            },
            config_and_effect_metadata: ConfigAndEffectMetadata::default(),
            source_map: json!({}),
        };
        sync_loaded_policy_fixture_fields(&mut assembly, &production_unit, &dependency_unit);
        assembly
    }

    fn loaded_policy_fixture() -> (PackageTestAssembly, PackageUnit, Vec<Arc<PackageUnit>>) {
        let production_unit = package_unit_fixture(
            "example.com/pkg",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "3333333333333333333333333333333333333333333333333333333333333333",
        );
        let dependency_unit = package_unit_fixture(
            "example.com/dep",
            "4444444444444444444444444444444444444444444444444444444444444444",
            "5555555555555555555555555555555555555555555555555555555555555555",
        );
        let mut assembly = package_test_assembly_fixture();
        sync_loaded_policy_fixture_fields(&mut assembly, &production_unit, &dependency_unit);
        (assembly, production_unit, vec![Arc::new(dependency_unit)])
    }

    fn sync_loaded_policy_fixture_fields(
        assembly: &mut PackageTestAssembly,
        production_unit: &PackageUnit,
        dependency_unit: &PackageUnit,
    ) {
        assembly.production_package_unit = package_unit_ref_fixture(production_unit);
        assembly.dependency_package_units = vec![package_unit_ref_fixture(dependency_unit)];
        assembly.link_policy.current_package_production = PackageProductionLinkScope {
            package_id: production_unit.package_id.clone(),
            version: production_unit.version.clone(),
            build_identity: production_unit.build_identity.clone(),
            files_digest: canonical_digest(&production_unit.files, "test production files")
                .expect("production files digest"),
            implementation_links_digest: canonical_digest(
                &production_unit.implementation_links,
                "test production implementation links",
            )
            .expect("production implementation links digest"),
            allow_private: true,
        };
        assembly.link_policy.dependency_public_scopes = vec![PackageDependencyPublicLinkScope {
            package_id: dependency_unit.package_id.clone(),
            version: dependency_unit.version.clone(),
            build_identity: dependency_unit.build_identity.clone(),
            public_abi_identity: dependency_unit.abi_identity.clone(),
            public_export_digest: canonical_digest(
                &dependency_unit.publication_abi,
                "test dependency publication ABI",
            )
            .expect("dependency publication ABI digest"),
            implementation_links_digest: package_implementation_links_identity(dependency_unit)
                .expect("dependency implementation links identity"),
            allow_private: false,
        }];
    }

    fn package_unit_fixture(package_id: &str, build_hash: &str, abi_hash: &str) -> PackageUnit {
        PackageUnit::empty(
            package_id,
            "1.0.0",
            format!("skiff-package-build-v1:sha256:{build_hash}"),
            format!("skiff-package-abi-v1:sha256:{abi_hash}"),
        )
    }

    fn file_ir_with_db_object(
        file_ir_identity: impl Into<String>,
        module_path: impl Into<String>,
        type_name: &str,
        collection_name: &str,
    ) -> FileIrUnit {
        let module_path = module_path.into();
        let mut file = FileIrUnit::empty(&module_path, "source-ast:test");
        file.file_ir_identity = file_ir_identity.into();
        file.declarations.db.insert(
            type_name.to_string(),
            DbDeclarationIr {
                type_ref: TypeRefIr::DbObjectSymbol {
                    symbol: ServiceSymbolRef {
                        module_path,
                        symbol: type_name.to_string(),
                    },
                },
                type_name: type_name.to_string(),
                collection_name: collection_name.to_string(),
                kind: Default::default(),
                key: DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: TypeRefIr::native("string"),
                },
                fields: Vec::new(),
                retention: None,
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );
        file
    }

    fn file_ir_with_recoverable_agent_run_db_object(
        file_ir_identity: impl Into<String>,
        module_path: impl Into<String>,
    ) -> FileIrUnit {
        let module_path = module_path.into();
        let mut file = file_ir_with_function(file_ir_identity, &module_path, "prod");
        let event_receiver_symbol = TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: module_path.clone(),
                symbol: "AgentEventReceiver".to_string(),
            },
        };
        let tool_provider_symbol = TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: module_path.clone(),
                symbol: "ToolProvider".to_string(),
            },
        };
        let event_receiver_any = TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(event_receiver_symbol, Vec::new()),
        };
        let tool_provider_any = TypeRefIr::AnyInterface {
            interface: interface_instantiation_ref(tool_provider_symbol, Vec::new()),
        };
        let runtime_bindings_ty = TypeRefIr::Record {
            fields: BTreeMap::from([
                ("events".to_string(), event_receiver_any),
                (
                    "providers".to_string(),
                    TypeRefIr::Native {
                        name: "Array".to_string(),
                        args: vec![tool_provider_any],
                    },
                ),
            ]),
        };
        let current_config_ty = TypeRefIr::Record {
            fields: BTreeMap::from([("runtimeBindings".to_string(), runtime_bindings_ty.clone())]),
        };

        file.type_table.push(TypeDeclIr {
            name: "AgentRun".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([
                    ("id".to_string(), TypeRefIr::native("string")),
                    ("currentConfig".to_string(), current_config_ty.clone()),
                    ("runtimeBindings".to_string(), runtime_bindings_ty.clone()),
                ]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            "AgentRun".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "AgentRun".to_string(),
                source_span: None,
            },
        );
        file.declarations.interfaces.insert(
            "AgentEventReceiver".to_string(),
            simple_interface("AgentEventReceiver"),
        );
        file.declarations
            .interfaces
            .insert("ToolProvider".to_string(), simple_interface("ToolProvider"));
        let event_receiver_type_index = file.type_table.len() as u32;
        file.type_table.push(TypeDeclIr {
            name: "AgentEventReceiver".to_string(),
            descriptor: TypeDescriptorIr::Native {
                symbol: "AgentEventReceiver".to_string(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            "AgentEventReceiver".to_string(),
            TypeDeclarationIr {
                type_index: event_receiver_type_index,
                symbol: "AgentEventReceiver".to_string(),
                source_span: None,
            },
        );
        let tool_provider_type_index = file.type_table.len() as u32;
        file.type_table.push(TypeDeclIr {
            name: "ToolProvider".to_string(),
            descriptor: TypeDescriptorIr::Native {
                symbol: "ToolProvider".to_string(),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: Vec::new(),
            source_span: None,
        });
        file.declarations.types.insert(
            "ToolProvider".to_string(),
            TypeDeclarationIr {
                type_index: tool_provider_type_index,
                symbol: "ToolProvider".to_string(),
                source_span: None,
            },
        );
        file.declarations.db.insert(
            "AgentRun".to_string(),
            DbDeclarationIr {
                type_ref: TypeRefIr::DbObjectSymbol {
                    symbol: ServiceSymbolRef {
                        module_path,
                        symbol: "AgentRun".to_string(),
                    },
                },
                type_name: "AgentRun".to_string(),
                collection_name: "agentRun".to_string(),
                kind: Default::default(),
                key: DbObjectKeyIr {
                    name: "id".to_string(),
                    ty: TypeRefIr::native("string"),
                },
                fields: vec![
                    DbObjectFieldIr {
                        name: "id".to_string(),
                        ty: TypeRefIr::native("string"),
                    },
                    DbObjectFieldIr {
                        name: "currentConfig".to_string(),
                        ty: current_config_ty,
                    },
                    DbObjectFieldIr {
                        name: "runtimeBindings".to_string(),
                        ty: runtime_bindings_ty,
                    },
                ],
                retention: None,
                leases: Vec::new(),
                indexes: Vec::new(),
                source_span: None,
            },
        );
        file
    }

    fn simple_interface(name: &str) -> InterfaceDeclIr {
        InterfaceDeclIr {
            name: name.to_string(),
            type_params: Vec::new(),
            operations: vec![InterfaceOperationIr {
                name: "ping".to_string(),
                type_params: Vec::new(),
                params: Vec::new(),
                return_type: TypeRefIr::native("string"),
                is_native: false,
                is_provider: false,
                is_static: false,
                implicit_self: None,
            }],
            source_span: None,
        }
    }

    fn file_ir_with_spawn_call(
        file_ir_identity: impl Into<String>,
        module_path: impl Into<String>,
        declaration_name: &str,
        target_module_path: &str,
        target_symbol: &str,
        metadata_target: &str,
    ) -> FileIrUnit {
        let module_path = module_path.into();
        let symbol = format!("{module_path}.{declaration_name}");
        let mut file = FileIrUnit::empty(&module_path, "source-ast:test");
        file.file_ir_identity = file_ir_identity.into();
        file.declarations.executables.insert(
            declaration_name.to_string(),
            artifact_executable_declaration(&symbol, 0),
        );
        file.executables.push(artifact_function(
            &symbol,
            TypeRefIr::native("void"),
            ArtifactExecutableBody {
                expressions: vec![ArtifactExprIr::Call {
                    call: ArtifactCallIr {
                        target: CallTargetIr::ExternalServiceSymbol {
                            symbol: ServiceSymbolRef {
                                module_path: target_module_path.to_string(),
                                symbol: target_symbol.to_string(),
                            },
                        },
                        args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::from([(
                            SPAWN_SUBMIT_METADATA_KEY.to_string(),
                            ArtifactMetadataValue::Object(BTreeMap::from([
                                (
                                    "targetKind".to_string(),
                                    ArtifactMetadataValue::String("function".to_string()),
                                ),
                                (
                                    "target".to_string(),
                                    ArtifactMetadataValue::String(metadata_target.to_string()),
                                ),
                            ])),
                        )]),
                    },
                }],
                ..ArtifactExecutableBody::default()
            },
        ));
        file
    }

    fn file_ir_with_publication_spawn_call(
        file_ir_identity: impl Into<String>,
        module_path: impl Into<String>,
        declaration_name: &str,
        target_module_path: &str,
        target_executable_index: u32,
        metadata_target: &str,
    ) -> FileIrUnit {
        let module_path = module_path.into();
        let symbol = format!("{module_path}.{declaration_name}");
        let mut file = FileIrUnit::empty(&module_path, "source-ast:test");
        file.file_ir_identity = file_ir_identity.into();
        file.declarations.executables.insert(
            declaration_name.to_string(),
            artifact_executable_declaration(&symbol, 0),
        );
        file.executables.push(artifact_function(
            &symbol,
            TypeRefIr::native("void"),
            ArtifactExecutableBody {
                expressions: vec![ArtifactExprIr::Call {
                    call: ArtifactCallIr {
                        target: CallTargetIr::PublicationExecutable {
                            module_path: target_module_path.to_string(),
                            executable_index: target_executable_index,
                        },
                        args: Vec::new(),
                        type_args: BTreeMap::new(),
                        metadata: BTreeMap::from([(
                            SPAWN_SUBMIT_METADATA_KEY.to_string(),
                            ArtifactMetadataValue::Object(BTreeMap::from([
                                (
                                    "targetKind".to_string(),
                                    ArtifactMetadataValue::String("function".to_string()),
                                ),
                                (
                                    "target".to_string(),
                                    ArtifactMetadataValue::String(metadata_target.to_string()),
                                ),
                            ])),
                        )]),
                    },
                }],
                ..ArtifactExecutableBody::default()
            },
        ));
        file
    }

    fn file_ir_with_function(
        file_ir_identity: impl Into<String>,
        module_path: impl Into<String>,
        declaration_name: &str,
    ) -> FileIrUnit {
        let module_path = module_path.into();
        let symbol = format!("{module_path}.{declaration_name}");
        let mut file = FileIrUnit::empty(&module_path, "source-ast:test");
        file.file_ir_identity = file_ir_identity.into();
        file.declarations.executables.insert(
            declaration_name.to_string(),
            artifact_executable_declaration(&symbol, 0),
        );
        file.executables.push(artifact_function(
            &symbol,
            TypeRefIr::native("void"),
            ArtifactExecutableBody::default(),
        ));
        file
    }

    fn artifact_function(
        symbol: &str,
        return_type: TypeRefIr,
        body: ArtifactExecutableBody,
    ) -> ArtifactExecutableIr {
        ArtifactExecutableIr {
            kind: ArtifactExecutableKind::Function,
            symbol: symbol.to_string(),
            type_params: Vec::new(),
            params: vec![ArtifactParamIr {
                name: "threadId".to_string(),
                slot: 0,
                ty: TypeRefIr::native("string"),
            }],
            return_type,
            self_type: None,
            slots: ArtifactSlotLayout::default(),
            may_suspend: false,
            body,
            source_span: None,
        }
    }

    fn artifact_executable_declaration(
        symbol: &str,
        executable_index: u32,
    ) -> ArtifactExecutableDeclarationIr {
        ArtifactExecutableDeclarationIr {
            executable_index,
            symbol: symbol.to_string(),
            source_span: None,
        }
    }

    fn package_unit_ref_fixture(unit: &PackageUnit) -> PackageTestPackageUnitRef {
        let package_path =
            publication_storage_segment(&unit.package_id, "packageId").expect("package path");
        let build_hash = identity_hash(
            &unit.build_identity,
            "skiff-package-build-v1:sha256",
            "buildIdentity",
        )
        .expect("build identity hash");
        PackageTestPackageUnitRef {
            package_id: unit.package_id.clone(),
            version: unit.version.clone(),
            build_identity: unit.build_identity.clone(),
            unit_path: format!("units/packages/{package_path}/{build_hash}.json"),
            public_abi_identity: unit.abi_identity.clone(),
            implementation_links_identity: package_implementation_links_identity(unit)
                .expect("implementation links identity"),
        }
    }

    fn file_artifact_path(file: &FileIrUnit) -> String {
        let hash = identity_hash(
            &file.file_ir_identity,
            "skiff-file-ir-v3:sha256",
            "fileIrIdentity",
        )
        .expect("file identity hash");
        format!("units/files/{hash}.json")
    }

    fn file_ref_for_artifact(file: &FileIrUnit, artifact_path: &str) -> FileIrRef {
        FileIrRef {
            file_ir_identity: file.file_ir_identity.clone(),
            module_path: file.module_path.clone(),
            artifact_path: Some(artifact_path.to_string()),
            source_ast_hash: Some(file.source_ast_hash.clone()),
        }
    }

    fn write_json_artifact<T: Serialize>(
        root: &TempRoot,
        relative_path: impl AsRef<Path>,
        value: &T,
    ) {
        let path = root.path().join(relative_path.as_ref());
        fs::create_dir_all(path.parent().expect("artifact path parent")).expect("artifact dir");
        fs::write(
            &path,
            serde_json::to_vec_pretty(value).expect("artifact JSON should serialize"),
        )
        .expect("write artifact");
    }

    fn package_test_graph_fixture(
        entrypoint_target: LinkedCallTarget,
    ) -> (
        PackageTestDispatchArtifact,
        LinkedProgramImage,
        PackageUnit,
        ExecutableAddr,
    ) {
        let assembly = package_test_assembly_fixture();
        let entrypoint = assembly.test_entrypoints[0].clone();
        let owner_file_identity = entrypoint.owner_test_file.file_ir_identity.clone();
        let production_file_identity = "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let dependency_file_identity = "skiff-file-ir-v3:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let other_test_file_identity = "skiff-file-ir-v3:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

        let mut production_unit = package_unit_fixture(
            "example.com/pkg",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "3333333333333333333333333333333333333333333333333333333333333333",
        );
        production_unit.files = vec![FileIrRef::new(production_file_identity, "pkg.api")];

        let mut dependency_unit = package_unit_fixture(
            "example.com/dep",
            "4444444444444444444444444444444444444444444444444444444444444444",
            "5555555555555555555555555555555555555555555555555555555555555555",
        );
        dependency_unit.files = vec![FileIrRef::new(dependency_file_identity, "dep.api")];
        dependency_unit.implementation_links.functions.insert(
            "public".to_string(),
            ExecutableExport {
                file: dependency_unit.files[0].clone(),
                executable_index: 0,
                symbol: "public".to_string(),
                signature: empty_executable_signature(),
            },
        );

        let service_files = vec![
            linked_file(
                production_file_identity,
                "pkg.api",
                vec![empty_executable("prod")],
            ),
            linked_file(
                &owner_file_identity,
                "pkg.test",
                vec![executable_calling(
                    "__skiff_package_test_0",
                    entrypoint_target,
                )],
            ),
            linked_file(
                other_test_file_identity,
                "pkg.other_test",
                vec![empty_executable("other_test_helper")],
            ),
        ];
        let package_files = vec![vec![linked_file(
            dependency_file_identity,
            "dep.api",
            vec![empty_executable("public"), empty_executable("private")],
        )]];
        let image = LinkedProgramImage {
            service_files,
            packages: vec![Arc::new(dependency_unit)],
            package_files,
            routes: Default::default(),
            spawn_routes: Default::default(),
            operations: Default::default(),
            operation_receivers: Default::default(),
            link_overlay: Default::default(),
            types: Default::default(),
        };
        let entrypoint_addr = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::file_ir_identity(owner_file_identity),
            executable: 0,
        };
        let dispatch = PackageTestDispatchArtifact {
            validated: ValidatedPackageTestDispatch {
                artifact_root: PathBuf::new(),
                assembly_path: PathBuf::new(),
                entrypoint_id: entrypoint.entrypoint_id.clone(),
            },
            assembly,
            entrypoint,
        };
        (dispatch, image, production_unit, entrypoint_addr)
    }

    fn package_test_graph_interface_box_fixture(
        owner_interface_impl: LinkedExecutable,
    ) -> (
        PackageTestDispatchArtifact,
        LinkedProgramImage,
        PackageUnit,
        ExecutableAddr,
    ) {
        let assembly = package_test_assembly_fixture();
        let entrypoint = assembly.test_entrypoints[0].clone();
        let owner_file_identity = entrypoint.owner_test_file.file_ir_identity.clone();
        let production_file_identity = "skiff-file-ir-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let other_test_file_identity = "skiff-file-ir-v3:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

        let mut production_unit = package_unit_fixture(
            "example.com/pkg",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "3333333333333333333333333333333333333333333333333333333333333333",
        );
        production_unit.files = vec![FileIrRef::new(production_file_identity, "pkg.api")];

        let service_files = vec![
            linked_file(
                production_file_identity,
                "pkg.api",
                vec![empty_executable("prod")],
            ),
            linked_file(
                &owner_file_identity,
                "pkg.test",
                vec![
                    executable_boxing_interface("__skiff_package_test_0", 1),
                    owner_interface_impl,
                ],
            ),
            linked_file(
                other_test_file_identity,
                "pkg.other_test",
                vec![empty_executable("other_test_helper")],
            ),
        ];
        let image = LinkedProgramImage {
            service_files,
            packages: Vec::new(),
            package_files: Vec::new(),
            routes: Default::default(),
            spawn_routes: Default::default(),
            operations: Default::default(),
            operation_receivers: Default::default(),
            link_overlay: Default::default(),
            types: Default::default(),
        };
        let entrypoint_addr = ExecutableAddr {
            unit: UnitAddr::Service,
            file: FileAddr::file_ir_identity(owner_file_identity),
            executable: 0,
        };
        let dispatch = PackageTestDispatchArtifact {
            validated: ValidatedPackageTestDispatch {
                artifact_root: PathBuf::new(),
                assembly_path: PathBuf::new(),
                entrypoint_id: entrypoint.entrypoint_id.clone(),
            },
            assembly,
            entrypoint,
        };
        (dispatch, image, production_unit, entrypoint_addr)
    }

    fn linked_file(
        file_ir_identity: impl Into<String>,
        module_path: impl Into<String>,
        executables: Vec<LinkedExecutable>,
    ) -> Arc<LinkedFileUnit> {
        linked_file_with_constants(file_ir_identity, module_path, executables, Vec::new())
    }

    fn linked_file_with_constants(
        file_ir_identity: impl Into<String>,
        module_path: impl Into<String>,
        executables: Vec<LinkedExecutable>,
        constants: Vec<ConstIr>,
    ) -> Arc<LinkedFileUnit> {
        Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: file_ir_identity.into(),
            source_ast_hash: "source-ast:test".to_string(),
            module_path: module_path.into(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: Default::default(),
            declarations: Default::default(),
            link_targets: Default::default(),
            types: Vec::new(),
            constants,
            executables,
            external_refs: Default::default(),
        })
    }

    fn executable_calling(symbol: &str, target: LinkedCallTarget) -> LinkedExecutable {
        LinkedExecutable {
            body: LinkedExecutableBody {
                expressions: vec![LinkedExprIr::Call {
                    call: CallIr {
                        target,
                        args: Vec::new(),
                        type_args: Default::default(),
                        metadata: Default::default(),
                    },
                }],
                ..Default::default()
            },
            ..empty_executable(symbol)
        }
    }

    fn executable_boxing_interface(symbol: &str, target_executable_index: u32) -> LinkedExecutable {
        LinkedExecutable {
            body: LinkedExecutableBody {
                expressions: vec![
                    LinkedExprIr::Literal {
                        value: LiteralIr::Null,
                    },
                    LinkedExprIr::InterfaceBox {
                        value: ExprRefIr { expression: 0 },
                        interface: package_test_interface_ref(),
                        source: package_test_interface_box_source(target_executable_index),
                    },
                ],
                ..Default::default()
            },
            ..empty_executable(symbol)
        }
    }

    fn package_test_interface_box_source(target_executable_index: u32) -> LinkedBoxSourceIr {
        LinkedBoxSourceIr::Local {
            concrete_type: native_type("Provider"),
            method_table: LinkedInterfaceMethodTablePlanIr {
                interface: package_test_interface_ref(),
                concrete_type: native_type("Provider"),
                slots: vec![LinkedInterfaceMethodSlotPlanIr {
                    slot: 0,
                    method_name: "read".to_string(),
                    method_abi_id: "method:pkg.test.Reader.read".to_string(),
                    signature: LinkedInterfaceMethodSlotSignatureIr {
                        params: vec![LinkedFunctionTypeParamIr {
                            name: "self".to_string(),
                            ty: native_type("Provider"),
                        }],
                        return_type: native_type("String"),
                    },
                    target: LinkedInterfaceMethodSlotTargetIr {
                        executable_index: target_executable_index,
                        receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                    },
                }],
            },
        }
    }

    fn package_test_interface_ref() -> LinkedInterfaceInstantiationRef {
        LinkedInterfaceInstantiationRef {
            interface_abi_id: "iface:pkg.test.Reader".to_string(),
            canonical_type_args: Vec::new(),
        }
    }

    fn native_type(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn empty_executable(symbol: &str) -> LinkedExecutable {
        LinkedExecutable {
            kind: skiff_runtime_linked_program::ExecutableKind::Function,
            symbol: symbol.to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            self_type: None,
            slots: Default::default(),
            may_suspend: false,
            body: Default::default(),
        }
    }

    fn empty_executable_signature() -> ExecutableSignatureIr {
        ExecutableSignatureIr {
            params: Vec::new(),
            return_type: TypeRefIr::Native {
                name: "Void".to_string(),
                args: Vec::new(),
            },
            self_type: None,
            may_suspend: false,
        }
    }

    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn path(&self) -> &Path {
            &self.path
        }

        fn path_buf(&self) -> PathBuf {
            self.path.clone()
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn unique_temp_dir() -> TempRoot {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be monotonic enough")
            .as_nanos();
        let counter = TEMP_ROOT_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-runtime-package-test-loader-{}-{nanos}-{counter}",
            std::process::id(),
        ));
        fs::create_dir(&path).expect("temp dir");
        TempRoot { path }
    }
}
