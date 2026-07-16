use std::collections::{BTreeMap, BTreeSet};

use skiff_compiler_core::artifact::{
    PackageTestEntrypoint, PackageTestEntrypointKind, PackageTestExecutableRef,
    PackageTestFileIrRef,
};

use super::{
    PackageTestArtifactBuildError, PackageTestArtifactBuildInput, PackageTestEntrypointInput,
    PublishedFileIrArtifact,
};
use crate::emission::identity::package_test_entrypoint_local_id;

pub(super) fn test_entrypoints(
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

pub(super) fn owner_test_file_identities_for_assembly(
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
