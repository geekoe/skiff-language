use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Map, Value};
use skiff_compiler_core::artifact::{
    ConfigAndEffectMetadata, FileIrRef, PackageDependencyConstraint,
    PackageDependencyPublicLinkScope, PackageImplementationLinks, PackageProductionLinkScope,
    PackageTestAssembly, PackageTestAssemblyKind, PackageTestEntrypoint, PackageTestEntrypointKind,
    PackageTestExecutableRef, PackageTestFileIrRef, PackageTestFileLinkScope,
    PackageTestLinkPolicy, PackageTestPackageUnitRef, PackageUnit,
    PACKAGE_TEST_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_compiler_core::id::PublicationId;
use skiff_compiler_core::json_utils::value_sha256;
use thiserror::Error;

use crate::emission::artifact::PublishedFileIrArtifact;
use crate::emission::identity::{
    derive_package_test_entrypoint_id, file_ir_identity, package_abi_identity,
    package_build_identity, package_test_build_hash, package_test_build_identity,
    package_test_entrypoint_local_id, publication_abi_identity,
    validate_package_test_assembly_identity, ArtifactIdentityError, FILE_IR_IDENTITY_PREFIX,
    PACKAGE_BUILD_IDENTITY_PREFIX,
};
use crate::projection::typed_artifacts::build_package_unit;

#[derive(Debug, Clone)]
pub struct PackageTestArtifactBuildInput {
    pub package_id: String,
    pub package_version: String,
    pub package_dependencies: Vec<PackageDependencyConstraint>,
    pub production_package_unit: Option<PackageUnit>,
    pub production_config_and_effect_metadata: ConfigAndEffectMetadata,
    pub package_test_config_and_effect_metadata: ConfigAndEffectMetadata,
    pub production_files: Vec<PublishedFileIrArtifact>,
    pub dependency_packages: Vec<PackageTestDependencyPackageInput>,
    pub test_files: Vec<PackageTestFileIrArtifact>,
    pub entrypoints: Vec<PackageTestEntrypointInput>,
}

#[derive(Debug, Clone)]
pub struct PackageTestDependencyPackageInput {
    pub package_id: String,
    pub package_version: String,
    pub package_dependencies: Vec<PackageDependencyConstraint>,
    pub production_files: Vec<PublishedFileIrArtifact>,
    pub package_unit: Option<PackageUnit>,
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
    pub unit: PackageUnit,
    pub value: Value,
    pub unit_path: String,
    pub reference: PackageTestPackageUnitRef,
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
    let production_unit = if let Some(unit) = input.production_package_unit.clone() {
        package_unit_artifact_from_unit(
            package_id.as_str(),
            &input.package_version,
            production_files.clone(),
            unit,
        )?
    } else {
        package_unit_artifact(
            package_id.as_str(),
            &input.package_version,
            &production_files,
            &input.package_dependencies,
            &input.production_config_and_effect_metadata,
        )?
    };
    validate_unique_dependency_aliases(&production_unit.unit)?;
    let package_path = package_id.artifact_path();
    let empty_dependency_metadata = ConfigAndEffectMetadata::default();
    let dependency_units = input
        .dependency_packages
        .iter()
        .map(|dependency| {
            if let Some(unit) = dependency.package_unit.clone() {
                package_unit_artifact_from_unit(
                    &dependency.package_id,
                    &dependency.package_version,
                    dependency.production_files.clone(),
                    unit,
                )
            } else {
                let files = published_files(dependency.production_files.clone())?;
                package_unit_artifact(
                    &dependency.package_id,
                    &dependency.package_version,
                    &files,
                    &dependency.package_dependencies,
                    &empty_dependency_metadata,
                )
            }
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

fn published_files(
    files: Vec<PublishedFileIrArtifact>,
) -> Result<Vec<PublishedFileIrArtifact>, PackageTestArtifactBuildError> {
    files
        .into_iter()
        .map(|file| {
            let identity = file_ir_identity(&file.unit)?;
            if file.unit.file_ir_identity != identity || file.identity != identity {
                return Err(PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "production file {} identity metadata does not match its File IR payload",
                        file.source_path
                    ),
                });
            }
            if file.path.trim().is_empty() {
                return Err(PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "production file {} artifact path is empty",
                        file.source_path
                    ),
                });
            }
            Ok(file)
        })
        .collect()
}

fn published_test_files(
    files: Vec<PackageTestFileIrArtifact>,
) -> Result<Vec<PublishedFileIrArtifact>, PackageTestArtifactBuildError> {
    files
        .into_iter()
        .map(|file| {
            let mut unit = file.file_ir;
            let identity = file_ir_identity(&unit)?;
            unit.file_ir_identity = identity.clone();
            let hash = file_ir_identity_hash(&identity)?;
            Ok(PublishedFileIrArtifact {
                identity,
                unit,
                hash: hash.clone(),
                path: format!("units/files/{hash}.json"),
                source_path: file.source_path,
                module_path: file.module_path,
                role: "package-test".to_string(),
            })
        })
        .collect()
}

fn package_unit_artifact(
    package_id: &str,
    version: &str,
    files: &[PublishedFileIrArtifact],
    dependencies: &[PackageDependencyConstraint],
    config_and_effect_metadata: &ConfigAndEffectMetadata,
) -> Result<PublishedPackageTestPackageUnitArtifact, PackageTestArtifactBuildError> {
    let unit = production_package_unit(
        package_id,
        version,
        files,
        dependencies,
        config_and_effect_metadata,
    )?;
    let value = package_unit_json_value(&unit);
    let unit_hash = package_unit_build_hash(&unit)?;
    let package_path = PublicationId::parse(package_id)
        .map_err(|error| PackageTestArtifactBuildError::InvalidInput {
            message: format!("package id {package_id} is invalid: {error}"),
        })?
        .artifact_path();
    let unit_path = package_unit_path(&package_path, &unit_hash);
    let reference = PackageTestPackageUnitRef {
        package_id: package_id.to_string(),
        version: version.to_string(),
        build_identity: unit.build_identity.clone(),
        unit_path: unit_path.clone(),
        public_abi_identity: unit.abi_identity.clone(),
        implementation_links_identity: implementation_links_identity(&unit.implementation_links),
    };
    Ok(PublishedPackageTestPackageUnitArtifact {
        files: files.to_vec(),
        unit,
        value,
        unit_path,
        reference,
    })
}

fn package_unit_artifact_from_unit(
    package_id: &str,
    version: &str,
    files: Vec<PublishedFileIrArtifact>,
    mut unit: PackageUnit,
) -> Result<PublishedPackageTestPackageUnitArtifact, PackageTestArtifactBuildError> {
    if unit.package_id != package_id {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "dependency package unit id {} does not match input package id {package_id}",
                unit.package_id
            ),
        });
    }
    if unit.version != version {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "dependency package unit version {} does not match input package version {version}",
                unit.version
            ),
        });
    }
    unit.files = files.iter().map(file_ref_for_published).collect();
    normalize_package_dependency_configs(&mut unit);
    unit.build_identity = package_build_identity(&unit)?;
    let value = package_unit_json_value(&unit);
    let unit_hash = package_unit_build_hash(&unit)?;
    let package_path = PublicationId::parse(package_id)
        .map_err(|error| PackageTestArtifactBuildError::InvalidInput {
            message: format!("package id {package_id} is invalid: {error}"),
        })?
        .artifact_path();
    let unit_path = package_unit_path(&package_path, &unit_hash);
    let reference = PackageTestPackageUnitRef {
        package_id: package_id.to_string(),
        version: version.to_string(),
        build_identity: unit.build_identity.clone(),
        unit_path: unit_path.clone(),
        public_abi_identity: unit.abi_identity.clone(),
        implementation_links_identity: implementation_links_identity(&unit.implementation_links),
    };
    Ok(PublishedPackageTestPackageUnitArtifact {
        files,
        unit,
        value,
        unit_path,
        reference,
    })
}

#[derive(Clone)]
struct DependencySlotRecord {
    unit: PublishedPackageTestPackageUnitArtifact,
    public_scope: PackageDependencyPublicLinkScope,
}

fn normalize_dependency_slot_records(
    dependency_units: Vec<PublishedPackageTestPackageUnitArtifact>,
) -> Result<Vec<DependencySlotRecord>, PackageTestArtifactBuildError> {
    let mut records = dependency_units
        .into_iter()
        .map(|unit| DependencySlotRecord {
            public_scope: dependency_public_link_scope(&unit.unit),
            unit,
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        dependency_slot_sort_key(&left.unit.reference)
            .cmp(&dependency_slot_sort_key(&right.unit.reference))
    });

    let mut package_slots = BTreeMap::<String, (String, String)>::new();
    let mut slot_records = BTreeMap::<
        (String, String, String),
        (PackageTestPackageUnitRef, PackageDependencyPublicLinkScope),
    >::new();
    let mut normalized = Vec::new();
    for record in records {
        let reference = record.unit.reference.clone();
        let slot_key = dependency_slot_sort_key(&reference);
        if let Some((seen_version, seen_build_identity)) =
            package_slots.get(reference.package_id.as_str())
        {
            if seen_version != &reference.version
                || seen_build_identity != &reference.build_identity
            {
                return Err(PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "dependency package {} resolves to multiple package slots: {}@{} and {}@{}",
                        reference.package_id,
                        reference.version,
                        reference.build_identity,
                        seen_version,
                        seen_build_identity
                    ),
                });
            }
        } else {
            package_slots.insert(
                reference.package_id.clone(),
                (reference.version.clone(), reference.build_identity.clone()),
            );
        }

        if let Some((seen_ref, seen_scope)) = slot_records.get(&slot_key) {
            if seen_ref == &reference && seen_scope == &record.public_scope {
                continue;
            }
            if seen_ref == &reference {
                return Err(PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "dependency package {}@{} build {} has conflicting public scopes",
                        reference.package_id, reference.version, reference.build_identity
                    ),
                });
            }
            return Err(PackageTestArtifactBuildError::InvalidInput {
                message: format!(
                    "dependency package {}@{} build {} has conflicting package unit refs",
                    reference.package_id, reference.version, reference.build_identity
                ),
            });
        }
        slot_records.insert(slot_key, (reference, record.public_scope.clone()));
        normalized.push(record);
    }
    Ok(normalized)
}

fn dependency_slot_sort_key(reference: &PackageTestPackageUnitRef) -> (String, String, String) {
    (
        reference.package_id.clone(),
        reference.version.clone(),
        reference.build_identity.clone(),
    )
}

fn dependency_public_link_scope(unit: &PackageUnit) -> PackageDependencyPublicLinkScope {
    PackageDependencyPublicLinkScope {
        package_id: unit.package_id.clone(),
        version: unit.version.clone(),
        build_identity: unit.build_identity.clone(),
        public_abi_identity: unit.abi_identity.clone(),
        public_export_digest: value_sha256(
            &serde_json::to_value(&unit.publication_abi).expect("publication ABI must serialize"),
        ),
        implementation_links_digest: implementation_links_identity(&unit.implementation_links),
        allow_private: false,
    }
}

fn validate_unique_dependency_aliases(
    package_unit: &PackageUnit,
) -> Result<(), PackageTestArtifactBuildError> {
    let mut aliases = BTreeMap::<String, ()>::new();
    for dependency in &package_unit.dependencies {
        if aliases.insert(dependency.alias.clone(), ()).is_some() {
            return Err(PackageTestArtifactBuildError::InvalidInput {
                message: format!(
                    "package {} dependency alias {} is declared more than once",
                    package_unit.package_id, dependency.alias
                ),
            });
        }
    }
    Ok(())
}

fn package_unit_build_hash(unit: &PackageUnit) -> Result<String, PackageTestArtifactBuildError> {
    let expected_prefix = format!("{PACKAGE_BUILD_IDENTITY_PREFIX}:");
    unit.build_identity
        .strip_prefix(&expected_prefix)
        .map(str::to_string)
        .ok_or_else(|| PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "package unit buildIdentity {} does not use {PACKAGE_BUILD_IDENTITY_PREFIX}",
                unit.build_identity,
            ),
        })
}

fn package_unit_json_value(unit: &PackageUnit) -> Value {
    let mut value = serde_json::to_value(unit).expect("PackageUnit must serialize");
    materialize_empty_dependency_configs(&mut value);
    value
}

fn materialize_empty_dependency_configs(value: &mut Value) {
    let Some(dependencies) = value
        .get_mut("dependencies")
        .and_then(|dependencies| dependencies.as_array_mut())
    else {
        return;
    };
    for dependency in dependencies {
        let Some(object) = dependency.as_object_mut() else {
            continue;
        };
        if object.get("config").map_or(true, Value::is_null) {
            object.insert("config".to_string(), json!({}));
        }
    }
}

fn file_ir_identity_hash(identity: &str) -> Result<String, PackageTestArtifactBuildError> {
    let expected_prefix = format!("{FILE_IR_IDENTITY_PREFIX}:");
    identity
        .strip_prefix(&expected_prefix)
        .map(str::to_string)
        .ok_or_else(|| PackageTestArtifactBuildError::InvalidInput {
            message: format!("fileIrIdentity {identity} does not use {FILE_IR_IDENTITY_PREFIX}"),
        })
}

fn production_package_unit(
    package_id: &str,
    version: &str,
    files: &[PublishedFileIrArtifact],
    dependencies: &[PackageDependencyConstraint],
    config_and_effect_metadata: &ConfigAndEffectMetadata,
) -> Result<PackageUnit, PackageTestArtifactBuildError> {
    let file_units = files
        .iter()
        .map(|file| file.unit.clone())
        .collect::<Vec<_>>();
    let mut unit = build_package_unit(
        package_id.to_string(),
        version.to_string(),
        file_units,
        dependencies.to_vec(),
        config_and_effect_metadata.clone(),
    )
    .map_err(|error| PackageTestArtifactBuildError::InvalidInput {
        message: format!("failed to build package unit {package_id}@{version}: {error}"),
    })?;
    unit.files = files.iter().map(file_ref_for_published).collect();
    unit.publication_abi.publication_id = unit.package_id.clone();
    unit.publication_abi.version = unit.version.clone();
    unit.publication_abi.abi_identity = publication_abi_identity(&unit.publication_abi)?;
    unit.abi_identity = package_abi_identity(&unit)?;
    normalize_package_dependency_configs(&mut unit);
    unit.build_identity = package_build_identity(&unit)?;
    Ok(unit)
}

fn normalize_package_dependency_configs(unit: &mut PackageUnit) {
    for dependency in &mut unit.dependencies {
        if dependency.config.is_null() {
            dependency.config = Value::Object(Map::new());
        }
    }
}

fn file_ref_for_published(artifact: &PublishedFileIrArtifact) -> FileIrRef {
    FileIrRef {
        file_ir_identity: artifact.identity.clone(),
        module_path: artifact.module_path.clone(),
        artifact_path: Some(artifact.path.clone()),
        source_ast_hash: Some(artifact.unit.source_ast_hash.clone()),
    }
}

fn package_unit_path(package_path: &str, unit_hash: &str) -> String {
    format!("units/packages/{package_path}/{unit_hash}.json")
}

fn package_test_file_ref(file: &PublishedFileIrArtifact) -> PackageTestFileIrRef {
    PackageTestFileIrRef {
        file_ir_identity: file.identity.clone(),
        file_ir_path: file.path.clone(),
        source_path: file.source_path.clone(),
        module_path: file.module_path.clone(),
    }
}

fn test_files_by_source_path(
    files: &[PackageTestFileIrRef],
) -> Result<BTreeMap<String, PackageTestFileIrRef>, PackageTestArtifactBuildError> {
    let mut by_source_path = BTreeMap::new();
    for file in files {
        if by_source_path
            .insert(file.source_path.clone(), file.clone())
            .is_some()
        {
            return Err(PackageTestArtifactBuildError::InvalidInput {
                message: format!(
                    "duplicate package test file source path {}",
                    file.source_path
                ),
            });
        }
    }
    Ok(by_source_path)
}

fn test_entrypoints(
    input: &PackageTestArtifactBuildInput,
    test_files: &[PublishedFileIrArtifact],
    files_by_source_path: &BTreeMap<String, PackageTestFileIrRef>,
) -> Result<Vec<PackageTestEntrypoint>, PackageTestArtifactBuildError> {
    if input.entrypoints.is_empty() {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: "entrypoints must not be empty".to_string(),
        });
    }
    let files_by_identity = test_files
        .iter()
        .map(|file| (file.identity.as_str(), file))
        .collect::<BTreeMap<_, _>>();
    input
        .entrypoints
        .iter()
        .map(|entrypoint| {
            let owner = files_by_source_path
                .get(&entrypoint.source_path)
                .cloned()
                .ok_or_else(|| PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "entrypoint source_path {} does not match any test file",
                        entrypoint.source_path
                    ),
                })?;
            if entrypoint.module_path != owner.module_path {
                return Err(PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "entrypoint module_path {} does not match owner test file module_path {} for {}",
                        entrypoint.module_path, owner.module_path, entrypoint.source_path
                    ),
                });
            }
            let owner_file = files_by_identity
                .get(owner.file_ir_identity.as_str())
                .copied()
                .ok_or_else(|| PackageTestArtifactBuildError::InvalidInput {
                    message: format!(
                        "entrypoint owner file identity {} has no published test file",
                        owner.file_ir_identity
                    ),
                })?;
            validate_entrypoint_executable_ref(entrypoint, &owner, &owner_file.unit)?;
            let entrypoint_local_id = package_test_entrypoint_local_id(
                &input.package_id,
                &input.package_version,
                &entrypoint.source_path,
                entrypoint.test_ordinal,
                &entrypoint.display_name,
            )?;
            let package_entrypoint = PackageTestEntrypoint {
                kind: PackageTestEntrypointKind::TestOnly,
                entrypoint_local_id,
                entrypoint_id: String::new(),
                display_name: entrypoint.display_name.clone(),
                source_path: entrypoint.source_path.clone(),
                module_path: entrypoint.module_path.clone(),
                owner_test_file: owner.clone(),
                executable_ref: PackageTestExecutableRef {
                    file_ir_identity: owner.file_ir_identity,
                    executable_index: entrypoint.executable_index,
                    executable_local_id: entrypoint.executable_local_id.clone(),
                    symbol: entrypoint.symbol.clone(),
                },
                default_run: entrypoint.default_run,
                config_and_effect_metadata: entrypoint.config_and_effect_metadata.clone(),
                runtime_expected_error: None,
            };
            validate_entrypoint_owner_contract(&package_entrypoint)?;
            Ok(package_entrypoint)
        })
        .collect()
}

fn validate_entrypoint_executable_ref(
    entrypoint: &PackageTestEntrypointInput,
    owner: &PackageTestFileIrRef,
    file_ir: &skiff_compiler_core::artifact::FileIrUnit,
) -> Result<(), PackageTestArtifactBuildError> {
    if file_ir.file_ir_identity != owner.file_ir_identity {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "owner test file {} identity {} does not match File IR identity {}",
                owner.source_path, owner.file_ir_identity, file_ir.file_ir_identity
            ),
        });
    }
    if file_ir.module_path != owner.module_path {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "owner test file {} module_path {} does not match File IR module_path {}",
                owner.source_path, owner.module_path, file_ir.module_path
            ),
        });
    }
    let executable = file_ir
        .executables
        .get(entrypoint.executable_index as usize)
        .ok_or_else(|| PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "entrypoint {} executable index {} does not exist in owner test file {}",
                entrypoint.display_name, entrypoint.executable_index, owner.source_path
            ),
        })?;
    if let Some(symbol) = &entrypoint.symbol {
        if executable.symbol != *symbol {
            return Err(PackageTestArtifactBuildError::InvalidInput {
                message: format!(
                    "entrypoint {} symbol {} does not match executable symbol {} in owner test file {}",
                    entrypoint.display_name, symbol, executable.symbol, owner.source_path
                ),
            });
        }
    }
    if let Some(declaration) = file_ir
        .declarations
        .executables
        .get(&entrypoint.executable_local_id)
    {
        if declaration.executable_index != entrypoint.executable_index {
            return Err(PackageTestArtifactBuildError::InvalidInput {
                message: format!(
                    "entrypoint {} executable_local_id {} points to index {}, not {}",
                    entrypoint.display_name,
                    entrypoint.executable_local_id,
                    declaration.executable_index,
                    entrypoint.executable_index
                ),
            });
        }
    } else if executable.symbol != entrypoint.executable_local_id {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "entrypoint {} executable_local_id {} does not match executable symbol {} in owner test file {}",
                entrypoint.display_name,
                entrypoint.executable_local_id,
                executable.symbol,
                owner.source_path
            ),
        });
    }
    Ok(())
}

fn validate_entrypoint_owner_contract(
    entrypoint: &PackageTestEntrypoint,
) -> Result<(), PackageTestArtifactBuildError> {
    if entrypoint.source_path != entrypoint.owner_test_file.source_path {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "entrypoint source_path {} does not match owner test file source_path {}",
                entrypoint.source_path, entrypoint.owner_test_file.source_path
            ),
        });
    }
    if entrypoint.module_path != entrypoint.owner_test_file.module_path {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "entrypoint module_path {} does not match owner test file module_path {}",
                entrypoint.module_path, entrypoint.owner_test_file.module_path
            ),
        });
    }
    if entrypoint.executable_ref.file_ir_identity != entrypoint.owner_test_file.file_ir_identity {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: format!(
                "entrypoint executable file identity {} does not match owner test file identity {}",
                entrypoint.executable_ref.file_ir_identity,
                entrypoint.owner_test_file.file_ir_identity
            ),
        });
    }
    Ok(())
}

fn owner_test_file_identities_for_assembly(
    entrypoints: &[PackageTestEntrypoint],
) -> Result<BTreeSet<&str>, PackageTestArtifactBuildError> {
    if entrypoints.is_empty() {
        return Err(PackageTestArtifactBuildError::InvalidInput {
            message: "entrypoints must not be empty".to_string(),
        });
    }
    let mut identities = BTreeSet::new();
    for entrypoint in entrypoints {
        identities.insert(entrypoint.owner_test_file.file_ir_identity.as_str());
    }
    Ok(identities)
}

fn link_policy(
    production_ref: &PackageTestPackageUnitRef,
    package_unit: &PackageUnit,
    test_file_artifacts: &[PublishedFileIrArtifact],
    test_file_refs: &[PackageTestFileIrRef],
    entrypoints: &[PackageTestEntrypoint],
    dependency_public_scopes: &[PackageDependencyPublicLinkScope],
) -> PackageTestLinkPolicy {
    let mut entrypoint_ids_by_file = BTreeMap::<String, Vec<String>>::new();
    for entrypoint in entrypoints {
        entrypoint_ids_by_file
            .entry(entrypoint.owner_test_file.file_ir_identity.clone())
            .or_default()
            .push(entrypoint.entrypoint_local_id.clone());
    }
    for ids in entrypoint_ids_by_file.values_mut() {
        ids.sort();
        ids.dedup();
    }

    PackageTestLinkPolicy {
        current_package_production: PackageProductionLinkScope {
            package_id: production_ref.package_id.clone(),
            version: production_ref.version.clone(),
            build_identity: production_ref.build_identity.clone(),
            files_digest: value_sha256(
                &serde_json::to_value(&package_unit.files).expect("files must serialize"),
            ),
            implementation_links_digest: value_sha256(
                &serde_json::to_value(&package_unit.implementation_links)
                    .expect("implementation links must serialize"),
            ),
            allow_private: true,
        },
        test_file_scopes: test_file_refs
            .iter()
            .zip(test_file_artifacts)
            .map(|(file_ref, file)| {
                let entrypoint_local_ids = entrypoint_ids_by_file
                    .get(&file_ref.file_ir_identity)
                    .cloned()
                    .unwrap_or_default();
                PackageTestFileLinkScope {
                    owner_test_file_identity: file_ref.file_ir_identity.clone(),
                    source_path: file_ref.source_path.clone(),
                    module_path: file_ref.module_path.clone(),
                    allowed_local_link_digest: package_test_allowed_local_link_digest(
                        file_ref,
                        &file.unit,
                        &entrypoint_local_ids,
                    ),
                    entrypoint_local_ids,
                }
            })
            .collect(),
        dependency_public_scopes: dependency_public_scopes.to_vec(),
    }
}

fn package_test_allowed_local_link_digest(
    file_ref: &PackageTestFileIrRef,
    file: &skiff_compiler_core::artifact::FileIrUnit,
    entrypoint_local_ids: &[String],
) -> String {
    let mut entrypoint_local_ids = entrypoint_local_ids.to_vec();
    entrypoint_local_ids.sort();
    entrypoint_local_ids.dedup();
    value_sha256(&json!({
        "fileIrIdentity": file_ref.file_ir_identity,
        "sourcePath": file_ref.source_path,
        "modulePath": file_ref.module_path,
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

fn implementation_links_identity(links: &PackageImplementationLinks) -> String {
    format!(
        "skiff-package-implementation-links-v1:sha256:{}",
        value_sha256(&serde_json::to_value(links).expect("implementation links must serialize"))
    )
}

fn source_map(test_files: &[PackageTestFileIrRef]) -> Value {
    json!({
        "sources": test_files
            .iter()
            .map(|file| {
                json!({
                    "sourcePath": file.source_path,
                    "modulePath": file.module_path,
                    "fileIrIdentity": file.file_ir_identity,
                })
            })
            .collect::<Vec<_>>()
    })
}
