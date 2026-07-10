use std::collections::BTreeMap;

use crate::error::{CompileError, Result};
use skiff_compiler_core::file_ir_identity::file_ir_identity;
use skiff_compiler_core::package_publication_abi as core_package_publication_abi;

pub use skiff_artifact_model::package_unit::InterfaceMethodSignature;
#[allow(unused_imports)]
pub use skiff_artifact_model::{
    canonical_interface_method_abi_id, interface_instantiation_ref_for_type_ref,
    CanonicalPublicCallableSignature, ConfigAndEffectMetadata, ConstExport, EffectMetadata,
    ExecutableExport, ExecutableIr, ExecutableKind, ExecutableSignatureIr, FileIrRef, FileIrUnit,
    InterfaceInstantiationRef, OperationAbiRef, OperationCallableKind, PackageAbiExpectation,
    PackageDependencyConstraint, PackageExportIndex, PackageImplementationLinks,
    PackageOperationTarget, PackageUnit, PackageUsedSymbol, PackageUsedSymbolKind,
    PublicInstanceExport, PublicInstanceOperation, PublicationAbiUnit, PublicationOperationKind,
    PublicationPublicInstanceExport, PublicationSchemaType, PublicationSchemaTypeNameability,
    ReceiverCallAbi, RecoverableArtifactMetadata, TypeDescriptorIr, TypeExport, TypeRefIr,
    PACKAGE_UNIT_SCHEMA_VERSION,
};

use super::identity::assign_package_unit_identities;

use super::interface_methods::{package_interface_method_signatures, PackageTypeSymbolIndex};
use super::publication_abi::publication_abi_build_error;

pub fn build_package_unit(
    package_id: impl Into<String>,
    version: impl Into<String>,
    file_units: Vec<FileIrUnit>,
    dependencies: Vec<PackageDependencyConstraint>,
    config_and_effect_metadata: ConfigAndEffectMetadata,
) -> Result<PackageUnit> {
    let package_id = package_id.into();
    let version = version.into();
    let files = file_units.iter().map(file_ref_for_unit).collect::<Vec<_>>();
    let exports =
        package_export_index_from_file_units(&package_id, &dependencies, &file_units, &files)?;
    build_package_unit_from_refs(
        package_id,
        version,
        files,
        exports,
        dependencies,
        config_and_effect_metadata,
    )
}

pub fn build_package_unit_from_refs(
    package_id: impl Into<String>,
    version: impl Into<String>,
    files: Vec<FileIrRef>,
    exports: PackageExportIndex,
    dependencies: Vec<PackageDependencyConstraint>,
    config_and_effect_metadata: ConfigAndEffectMetadata,
) -> Result<PackageUnit> {
    let package_id = package_id.into();
    let version = version.into();
    let publication_abi = package_publication_abi(&package_id, &version, &exports)?;
    let implementation_links = package_implementation_links(&exports, &publication_abi);
    let mut unit = PackageUnit {
        schema_version: PACKAGE_UNIT_SCHEMA_VERSION.to_string(),
        package_id: package_id.clone(),
        version: version.clone(),
        build_identity: String::new(),
        abi_identity: String::new(),
        abi_identity_projection: Default::default(),
        publication_abi,
        files,
        resources: Vec::new(),
        implementation_links,
        dependencies,
        recoverable_metadata: RecoverableArtifactMetadata::default(),
        config_and_effect_metadata,
    };
    assign_package_unit_identities(&mut unit);
    Ok(unit)
}

pub fn package_implementation_links(
    exports: &PackageExportIndex,
    publication_abi: &PublicationAbiUnit,
) -> PackageImplementationLinks {
    core_package_publication_abi::package_implementation_links(exports, publication_abi)
}

pub fn package_publication_abi(
    package_id: &str,
    version: &str,
    exports: &PackageExportIndex,
) -> Result<PublicationAbiUnit> {
    core_package_publication_abi::package_publication_abi(package_id, version, exports)
        .map_err(publication_abi_build_error)
}

pub fn package_export_index_from_file_units(
    package_id: &str,
    dependencies: &[PackageDependencyConstraint],
    file_units: &[FileIrUnit],
    files: &[FileIrRef],
) -> Result<PackageExportIndex> {
    if file_units.len() != files.len() {
        return Err(CompileError::Semantic(format!(
            "package unit builder received {} file units but {} file refs",
            file_units.len(),
            files.len()
        )));
    }

    let mut index = PackageExportIndex::default();
    let type_symbols = package_type_symbol_index_from_file_units(file_units, dependencies)?;
    for (unit, file) in file_units.iter().zip(files) {
        for (export_key, exported) in &unit.link_targets.types {
            let type_decl = unit
                .type_table
                .get(exported.type_index as usize)
                .ok_or_else(|| {
                    invalid_export_index("type", export_key, exported.type_index, &unit.module_path)
                })?;
            let interface_methods = unit
                .declarations
                .interfaces
                .get(&type_decl.name)
                .map(|interface| {
                    package_interface_method_signatures(
                        package_id,
                        &type_symbols,
                        &unit.module_path,
                        interface,
                    )
                    .map_err(CompileError::Semantic)
                })
                .transpose()?
                .unwrap_or_default();
            insert_unique(
                &mut index.types,
                export_key.clone(),
                TypeExport {
                    file: file.clone(),
                    type_index: exported.type_index,
                    symbol: export_key.clone(),
                    descriptor: Some(type_decl.descriptor.clone()),
                    type_params: type_decl.type_params.clone(),
                    interface_methods,
                },
                "type",
            )?;
        }

        for (export_key, exported) in &unit.link_targets.constants {
            let constant = unit
                .constants
                .get(exported.const_index as usize)
                .ok_or_else(|| {
                    invalid_export_index(
                        "const",
                        export_key,
                        exported.const_index,
                        &unit.module_path,
                    )
                })?;
            insert_unique(
                &mut index.constants,
                export_key.clone(),
                ConstExport {
                    file: file.clone(),
                    const_index: exported.const_index,
                    symbol: export_key.clone(),
                    ty: constant.ty.clone(),
                },
                "const",
            )?;
        }

        for (export_key, exported) in &unit.link_targets.executables {
            let executable = unit
                .executables
                .get(exported.executable_index as usize)
                .ok_or_else(|| {
                    invalid_export_index(
                        "executable",
                        export_key,
                        exported.executable_index,
                        &unit.module_path,
                    )
                })?;
            let export = ExecutableExport {
                file: file.clone(),
                executable_index: exported.executable_index,
                symbol: export_key.clone(),
                signature: executable_signature(executable),
            };
            match executable.kind {
                ExecutableKind::Function => {
                    insert_unique(&mut index.functions, export_key.clone(), export, "function")?;
                }
                ExecutableKind::ImplMethod => {
                    insert_unique(
                        &mut index.impl_methods,
                        export_key.clone(),
                        export,
                        "impl method",
                    )?;
                }
            }
        }
    }
    Ok(index)
}

fn package_type_symbol_index_from_file_units(
    file_units: &[FileIrUnit],
    dependencies: &[PackageDependencyConstraint],
) -> Result<PackageTypeSymbolIndex> {
    let mut index = PackageTypeSymbolIndex::default();
    for dependency in dependencies {
        index.insert_dependency(dependency.alias.as_str(), dependency.id.as_str());
        index.insert_dependency(dependency.id.as_str(), dependency.id.as_str());
    }
    for unit in file_units {
        for (export_key, exported) in &unit.link_targets.types {
            let type_decl = unit
                .type_table
                .get(exported.type_index as usize)
                .ok_or_else(|| {
                    invalid_export_index("type", export_key, exported.type_index, &unit.module_path)
                })?;
            index.insert_type(
                unit.module_path.clone(),
                exported.type_index,
                type_decl.name.clone(),
                export_key.clone(),
            );
        }
    }
    Ok(index)
}

fn file_ref_for_unit(unit: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: file_ir_identity(unit),
        module_path: unit.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(unit.source_ast_hash.clone()),
    }
}

fn executable_signature(executable: &ExecutableIr) -> ExecutableSignatureIr {
    ExecutableSignatureIr {
        params: executable.params.clone(),
        return_type: executable.return_type.clone(),
        self_type: executable.self_type.clone(),
        may_suspend: executable.may_suspend,
    }
}

fn insert_unique<T>(
    exports: &mut BTreeMap<String, T>,
    export_key: String,
    value: T,
    kind: &str,
) -> Result<()> {
    if exports.contains_key(&export_key) {
        return Err(CompileError::Semantic(format!(
            "duplicate package {kind} export `{export_key}`"
        )));
    }
    exports.insert(export_key, value);
    Ok(())
}

fn invalid_export_index(
    kind: &str,
    export_key: &str,
    index: u32,
    module_path: &str,
) -> CompileError {
    CompileError::Semantic(format!(
        "invalid {kind} export `{export_key}` in `{module_path}`: index {index} is out of bounds"
    ))
}
