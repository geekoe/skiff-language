use std::collections::BTreeMap;

use crate::error::{CompileError, Result};
use skiff_compiler_core::file_ir_identity::file_ir_identity;

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

use super::identity::{assign_package_unit_identities, public_function_operation_abi_id};

use super::interface_methods::{package_interface_method_signatures, PackageTypeSymbolIndex};
use super::publication_abi::{
    public_signature_from_receiver_executable_signature, publication_public_instance_export,
    push_publication_operation_abi,
};

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
    let mut links = PackageImplementationLinks::from_exports(exports);
    for operation in &publication_abi.operation_exports {
        match operation.kind {
            PublicationOperationKind::PublicFunction => {
                if let Some(export) = exports.functions.get(&operation.public_path) {
                    links.operation_targets.insert(
                        operation.operation_abi_id.clone(),
                        PackageOperationTarget::LocalExecutable {
                            operation: operation.clone(),
                            target: export.operation_target_ref(
                                format!("callable:{}", export.symbol),
                                OperationCallableKind::PublicFunction,
                            ),
                        },
                    );
                }
            }
            PublicationOperationKind::PublicInstanceMethod => {
                if let Some(public_operation) =
                    package_public_instance_operation(exports, &operation.operation_abi_id)
                {
                    links.operation_targets.insert(
                        operation.operation_abi_id.clone(),
                        PackageOperationTarget::LocalConstReceiverExecutable {
                            operation: operation.clone(),
                            target: public_operation.receiver_executable.clone(),
                        },
                    );
                }
            }
        }
    }
    links
}

pub fn package_publication_abi(
    package_id: &str,
    version: &str,
    exports: &PackageExportIndex,
) -> Result<PublicationAbiUnit> {
    let mut publication_abi = PublicationAbiUnit::empty(package_id, version, "");
    let public_instance_signatures = package_public_instance_signature_index(exports);
    for (public_path, export) in &exports.types {
        publication_abi
            .schema_closure
            .push(package_publication_schema_type(public_path, export));
    }
    for (public_path, export) in &exports.functions {
        let public_signature = CanonicalPublicCallableSignature::from(export.signature.clone());
        let operation = package_public_function_operation(public_path, &public_signature);
        push_publication_operation_abi(
            &mut publication_abi,
            public_path.clone(),
            operation,
            public_signature,
        )?;
    }
    for public_instance in &exports.public_instances {
        let projected_instance =
            package_publication_public_instance(public_instance, &public_instance_signatures)?;
        for operation in &public_instance.operations {
            let public_signature = package_public_instance_public_signature(
                public_instance,
                operation,
                &public_instance_signatures,
            )?;
            let operation_ref = operation.operation.clone();
            push_publication_operation_abi(
                &mut publication_abi,
                operation_ref.public_path.clone(),
                operation_ref,
                public_signature,
            )?;
        }
        publication_abi.public_instances.push(projected_instance);
    }
    Ok(publication_abi)
}

fn package_public_function_operation(
    public_path: &str,
    public_signature: &CanonicalPublicCallableSignature,
) -> OperationAbiRef {
    OperationAbiRef {
        operation_abi_id: public_function_operation_abi_id(
            public_path,
            public_signature,
            &[],
            &BTreeMap::new(),
        ),
        kind: PublicationOperationKind::PublicFunction,
        public_path: public_path.to_string(),
        public_instance_key: None,
        interface: None,
        method_abi_id: None,
        display_name: public_path.to_string(),
    }
}

fn package_publication_public_instance(
    public_instance: &PublicInstanceExport,
    signatures: &BTreeMap<(String, u32), ExecutableSignatureIr>,
) -> Result<PublicationPublicInstanceExport> {
    let declared_interfaces = public_instance
        .implemented_interfaces
        .iter()
        .map(interface_instantiation_ref_for_type_ref)
        .collect::<Vec<_>>();
    for operation in &public_instance.operations {
        validate_package_public_instance_operation(
            public_instance,
            operation,
            &declared_interfaces,
        )?;
        package_public_instance_public_signature(public_instance, operation, signatures)?;
    }
    publication_public_instance_export(
        public_instance.name.clone(),
        public_instance
            .operations
            .iter()
            .map(|operation| (operation, operation.operation.clone())),
        Some(format!(
            "package public instance `{}`",
            public_instance.name
        )),
    )
}

fn validate_package_public_instance_operation(
    public_instance: &PublicInstanceExport,
    operation: &PublicInstanceOperation,
    declared_interfaces: &[InterfaceInstantiationRef],
) -> Result<()> {
    if operation.operation.kind != PublicationOperationKind::PublicInstanceMethod {
        return Err(CompileError::Semantic(format!(
            "package public instance `{}` operation `{}` must use PublicInstanceMethod kind",
            public_instance.name, operation.operation.display_name
        )));
    }
    if operation.operation.public_instance_key.as_deref() != Some(public_instance.name.as_str()) {
        return Err(CompileError::Semantic(format!(
            "package public instance `{}` operation `{}` must carry matching publicInstanceKey",
            public_instance.name, operation.operation.display_name
        )));
    }
    let Some(interface) = operation.operation.interface.as_ref() else {
        return Err(CompileError::Semantic(format!(
            "package public instance `{}` operation `{}` must carry interface instantiation",
            public_instance.name, operation.operation.display_name
        )));
    };
    if !declared_interfaces
        .iter()
        .any(|candidate| candidate.interface_abi_id == interface.interface_abi_id)
    {
        return Err(CompileError::Semantic(format!(
            "package public instance `{}` operation `{}` interface is not exposed by the instance",
            public_instance.name, operation.operation.display_name
        )));
    }
    if operation.operation.method_abi_id.as_deref()
        != Some(operation.receiver_executable.method_abi_id.as_str())
    {
        return Err(CompileError::Semantic(format!(
            "package public instance `{}` operation `{}` methodAbiId does not match receiver executable",
            public_instance.name, operation.operation.display_name
        )));
    }
    if operation.receiver_executable.receiver_call_abi != ReceiverCallAbi::ExplicitSelfFirst {
        return Err(CompileError::Semantic(format!(
            "package public instance `{}` operation `{}` must use ExplicitSelfFirst receiver ABI",
            public_instance.name, operation.operation.display_name
        )));
    }
    match operation
        .receiver_executable
        .executable_target
        .callable_kind
    {
        OperationCallableKind::ImplMethod | OperationCallableKind::ReceiverMethod => Ok(()),
        other => Err(CompileError::Semantic(format!(
            "package public instance `{}` operation `{}` target must be a receiver method, got {:?}",
            public_instance.name, operation.operation.display_name, other
        ))),
    }
}

fn package_public_instance_public_signature(
    public_instance: &PublicInstanceExport,
    operation: &PublicInstanceOperation,
    signatures: &BTreeMap<(String, u32), ExecutableSignatureIr>,
) -> Result<CanonicalPublicCallableSignature> {
    let target = &operation.receiver_executable.executable_target;
    let signature = signatures
        .get(&(target.file_ref.module_path.clone(), target.executable_index))
        .cloned()
        .ok_or_else(|| {
            CompileError::Semantic(format!(
                "package public instance `{}` operation `{}` target file `{}` executable index {} is missing from package impl method exports",
                public_instance.name,
                operation.operation.display_name,
                target.file_ref.module_path,
                target.executable_index
            ))
        })?;
    Ok(public_signature_from_receiver_executable_signature(
        signature,
    ))
}

fn package_public_instance_signature_index(
    exports: &PackageExportIndex,
) -> BTreeMap<(String, u32), ExecutableSignatureIr> {
    exports
        .impl_methods
        .values()
        .map(|export| {
            (
                (export.file.module_path.clone(), export.executable_index),
                export.signature.clone(),
            )
        })
        .collect()
}

fn package_public_instance_operation<'a>(
    exports: &'a PackageExportIndex,
    operation_abi_id: &str,
) -> Option<&'a PublicInstanceOperation> {
    exports
        .public_instances
        .iter()
        .flat_map(|public_instance| &public_instance.operations)
        .find(|operation| operation.operation.operation_abi_id == operation_abi_id)
}

fn package_publication_schema_type(
    public_path: &str,
    export: &TypeExport,
) -> PublicationSchemaType {
    PublicationSchemaType {
        abi_type_id: format!("type:{public_path}"),
        nameability: PublicationSchemaTypeNameability::PublicNameable,
        ty: package_type_descriptor_type_ref(public_path, export.descriptor.as_ref()),
        descriptor: export.descriptor.clone(),
    }
}

fn package_type_descriptor_type_ref(
    public_path: &str,
    descriptor: Option<&TypeDescriptorIr>,
) -> TypeRefIr {
    match descriptor {
        Some(TypeDescriptorIr::Record { fields }) => TypeRefIr::Record {
            fields: fields.clone(),
        },
        Some(TypeDescriptorIr::Alias { target }) => target.clone(),
        Some(TypeDescriptorIr::Union { variants }) => TypeRefIr::Union {
            items: variants.clone(),
        },
        Some(TypeDescriptorIr::Native { symbol }) => TypeRefIr::native(symbol.clone()),
        None => TypeRefIr::native(public_path.to_string()),
    }
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
