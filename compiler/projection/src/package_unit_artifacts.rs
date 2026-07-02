use std::collections::{BTreeMap as StdBTreeMap, BTreeSet};

use crate::context::ProjectedPackageDependency;
use crate::error::ProjectionError;
use crate::package_exports::PackageExports;
use crate::typed_artifacts::{
    assign_package_unit_identities, canonical_interface_method_abi_id,
    interface_methods::{package_interface_method_signatures, PackageTypeSymbolIndex},
    package_implementation_links, package_publication_abi, public_instance_method_operation_abi_id,
    ConfigAndEffectMetadata, ConstExport, ExecutableExport, InterfaceMethodSignature,
    OperationCallableKind, OperationConstReceiverRef, OperationTargetRef,
    PackageDependencyConstraint, PackageExportIndex, PackageUnit, PublicInstanceExport,
    PublicInstanceOperation, ReceiverCallAbi, RecoverableArtifactMetadata, TypeExport,
};
use crate::ConfigProjection;
use skiff_artifact_model::{
    interface_instantiation_ref, type_ref_abi_key, CanonicalPublicCallableSignature, ConstIr,
    ExecutableIr, ExecutableSignatureIr, FileIrRef, FileIrUnit, InterfaceInstantiationRef,
    LocalReceiverExecutableRef, MetadataValue, OperationAbiRef, PackageRefIr, PackageSymbolRef,
    PublicationOperationKind, ServiceSymbolRef, TypeDescriptorIr, TypeRefIr,
    PACKAGE_UNIT_SCHEMA_VERSION,
};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_core::naming::impl_method_declaration_name;
use skiff_compiler_core::package_interface_methods::instantiate_interface_method_signatures;

#[derive(Debug, Clone)]
pub struct PackageFileIrProjection {
    pub unit: FileIrUnit,
    pub identity: String,
    pub source_ast_hash: String,
    pub module_path: String,
}

impl PackageFileIrProjection {
    pub fn from_unit(unit: FileIrUnit) -> Self {
        Self {
            identity: unit.file_ir_identity.clone(),
            source_ast_hash: unit.source_ast_hash.clone(),
            module_path: unit.module_path.clone(),
            unit,
        }
    }
}

pub struct PackageIrProjectionSource<'a> {
    pub package_id: &'a str,
    pub version: &'a str,
    pub exports: &'a PackageExports,
    pub abi_identity_projection: &'a skiff_artifact_model::AbiIdentityFacts,
    pub config_projection: &'a ConfigProjection,
    pub file_ir_units: Vec<PackageFileIrProjection>,
}

#[derive(Debug, Clone)]
pub struct ProjectedPackageIrArtifacts {
    pub unit: PackageUnit,
    pub config_projection: ConfigProjection,
    pub file_ir_units: Vec<PackageFileIrProjection>,
}

pub fn project_package_ir_artifacts(
    mut package: PackageIrProjectionSource<'_>,
    dependencies: &[ProjectedPackageDependency],
) -> Result<ProjectedPackageIrArtifacts, ProjectionError> {
    let file_refs = file_ir_refs_for_projected(&package.file_ir_units);
    let exports = package_unit_export_index(&package, &package.file_ir_units, dependencies)?;
    let dependency_constraints = package_unit_dependency_constraints(
        dependencies,
        &package.file_ir_units,
        package.package_id,
    );
    let config_and_effect_metadata =
        config_and_effect_metadata_from_config_projection(package.config_projection);
    let publication_abi = package_publication_abi(package.package_id, package.version, &exports)
        .map_err(|error| ProjectionError::ContractValidation {
            message: error.to_string(),
        })?;
    let implementation_links = package_implementation_links(&exports, &publication_abi);
    let mut unit = PackageUnit {
        schema_version: PACKAGE_UNIT_SCHEMA_VERSION.to_string(),
        package_id: package.package_id.to_string(),
        version: package.version.to_string(),
        build_identity: String::new(),
        abi_identity: String::new(),
        abi_identity_projection: package.abi_identity_projection.clone(),
        publication_abi,
        files: file_refs,
        implementation_links,
        dependencies: dependency_constraints,
        recoverable_metadata: RecoverableArtifactMetadata::default(),
        config_and_effect_metadata,
    };
    assign_package_unit_identities(&mut unit);

    Ok(ProjectedPackageIrArtifacts {
        unit,
        config_projection: package.config_projection.clone(),
        file_ir_units: std::mem::take(&mut package.file_ir_units),
    })
}

pub fn file_ir_refs_for_projected(artifacts: &[PackageFileIrProjection]) -> Vec<FileIrRef> {
    artifacts
        .iter()
        .map(|artifact| FileIrRef {
            file_ir_identity: artifact.identity.clone(),
            module_path: artifact.module_path.clone(),
            artifact_path: None,
            source_ast_hash: Some(artifact.source_ast_hash.clone()),
        })
        .collect()
}

pub fn config_and_effect_metadata_from_config_projection(
    config_projection: &ConfigProjection,
) -> ConfigAndEffectMetadata {
    // The assembly serializes these typed values verbatim, so projecting them
    // straight into MetadataValue is byte-equivalent to re-reading the
    // serialized assembly (which is what this used to do).
    let mut config = StdBTreeMap::new();
    config.insert(
        "shape".to_string(),
        MetadataValue::from_serializable(&config_projection.shape),
    );
    config.insert(
        "uses".to_string(),
        MetadataValue::from_serializable(&config_projection.uses),
    );
    config.insert(
        "activation".to_string(),
        MetadataValue::from_serializable(&config_projection.activation),
    );
    config.insert(
        "requirements".to_string(),
        MetadataValue::from_serializable(&config_projection.requirements),
    );
    ConfigAndEffectMetadata {
        config,
        effects: StdBTreeMap::new(),
    }
}

fn package_unit_export_index(
    package: &PackageIrProjectionSource<'_>,
    file_ir_units: &[PackageFileIrProjection],
    dependencies: &[ProjectedPackageDependency],
) -> Result<PackageExportIndex, ProjectionError> {
    let files_by_module = file_ir_refs_for_projected(file_ir_units)
        .into_iter()
        .map(|file_ref| (file_ref.module_path.clone(), file_ref))
        .collect::<StdBTreeMap<_, _>>();
    let file_units_by_module = file_ir_units
        .iter()
        .map(|artifact| (artifact.module_path.as_str(), &artifact.unit))
        .collect::<StdBTreeMap<_, _>>();
    let type_symbols = package_type_symbol_index(package, &file_units_by_module, dependencies)?;
    let mut exports = PackageExportIndex::default();

    for (public_symbol, export) in &package.exports.symbols {
        let package_symbol = package_scoped_export_symbol(package, public_symbol);
        let module = export.module.as_str();
        let symbol = export.symbol.as_str();
        let file_ref = files_by_module.get(module).cloned().ok_or_else(|| {
            package_export_error(
                package,
                public_symbol,
                format!("manifest export points to missing module {module}"),
            )
        })?;
        let file_unit = file_units_by_module.get(module).copied().ok_or_else(|| {
            package_export_error(
                package,
                public_symbol,
                format!("manifest export points to missing File IR unit for module {module}"),
            )
        })?;
        if let Some(type_index) = type_link_target_index(file_unit, symbol) {
            let ty = type_export_decl(package, public_symbol, module, file_unit, type_index)?;
            let interface_methods = file_unit
                .declarations
                .interfaces
                .get(&ty.name)
                .map(|interface| {
                    package_interface_method_signatures(
                        &package.package_id,
                        &type_symbols,
                        module,
                        interface,
                    )
                    .map_err(|message| package_export_error(package, public_symbol, message))
                })
                .transpose()?
                .unwrap_or_default();
            exports.types.insert(
                package_symbol.clone(),
                TypeExport {
                    file: file_ref,
                    type_index,
                    symbol: package_symbol.clone(),
                    descriptor: Some(ty.descriptor.clone()),
                    type_params: ty.type_params.clone(),
                    interface_methods,
                },
            );
            continue;
        }
        if let Some(const_index) = const_link_target_index(file_unit, symbol) {
            let constant =
                const_export_decl(package, public_symbol, module, file_unit, const_index)?;
            exports.constants.insert(
                package_symbol.clone(),
                ConstExport {
                    file: file_ref,
                    const_index,
                    symbol: package_symbol.clone(),
                    ty: constant.ty.clone(),
                },
            );
            continue;
        }
        if let Some(executable_index) = executable_link_target_index(file_unit, symbol) {
            let executable = executable_export_decl(
                package,
                public_symbol,
                module,
                file_unit,
                executable_index,
            )?;
            let export = ExecutableExport {
                file: file_ref,
                executable_index,
                symbol: executable.symbol.clone(),
                signature: executable_signature(executable),
            };
            // An exported public type carries its impl methods into the public
            // contract (api.rs `insert_public_callable(..Method..)`). An INSTANCE
            // method (explicit `self` receiver) is reached through an instance and
            // dispatched as an impl-method call — it is NOT a standalone
            // `PublicFunction` operation. Route those into `impl_methods` (their
            // dedicated link home, which the runtime resolves for method
            // dispatch) instead of `functions`; otherwise `package_publication_abi`
            // would emit each as a `PublicFunction` operation whose executable is
            // an `ImplMethod`, which the runtime package linker rejects on load
            // ("expected non-receiver executable kind"). Static methods carry no
            // receiver, so they lower to `ImplMethod` kind but remain real
            // function-compatible operations (e.g. an HTTP route handler) — gate
            // on the explicit `self` receiver, not on `ExecutableKind`.
            if executable_has_self_receiver(executable) {
                exports.impl_methods.insert(package_symbol.clone(), export);
            } else {
                exports.functions.insert(package_symbol.clone(), export);
            }
            continue;
        }

        return Err(package_export_error(
            package,
            public_symbol,
            format!("manifest export points to missing symbol {symbol} in module {module}"),
        ));
    }

    project_package_public_instances(
        package,
        &files_by_module,
        &file_units_by_module,
        &type_symbols,
        &mut exports,
    )?;

    Ok(exports)
}

fn project_package_public_instances(
    package: &PackageIrProjectionSource<'_>,
    files_by_module: &StdBTreeMap<String, FileIrRef>,
    file_units_by_module: &StdBTreeMap<&str, &FileIrUnit>,
    type_symbols: &PackageTypeSymbolIndex,
    exports: &mut PackageExportIndex,
) -> Result<(), ProjectionError> {
    let mut seen_instances = StdBTreeMap::<String, String>::new();
    for public_instance in &package.exports.public_instances {
        let public_path = package_scoped_export_symbol(package, &public_instance.public_path);
        let source = format!(
            "{}.{}",
            public_instance.module, public_instance.const_symbol
        );
        if let Some(existing) = seen_instances.insert(public_path.clone(), source.clone()) {
            return Err(package_public_instance_error(
                package,
                &public_path,
                format!("duplicate public instance exported by both {existing} and {source}"),
            ));
        }
        let receiver_unit = file_units_by_module
            .get(public_instance.module.as_str())
            .copied()
            .ok_or_else(|| {
                package_public_instance_error(
                    package,
                    &public_path,
                    format!(
                        "const selector points to missing module {}",
                        public_instance.module
                    ),
                )
            })?;
        let receiver_file = files_by_module
            .get(&public_instance.module)
            .cloned()
            .ok_or_else(|| {
                package_public_instance_error(
                    package,
                    &public_path,
                    format!(
                        "const selector points to missing File IR ref for module {}",
                        public_instance.module
                    ),
                )
            })?;
        let const_decl = receiver_unit
            .declarations
            .constants
            .get(&public_instance.const_symbol)
            .ok_or_else(|| {
                package_public_instance_error(
                    package,
                    &public_path,
                    format!(
                        "const selector points to missing const {}.{}",
                        public_instance.module, public_instance.const_symbol
                    ),
                )
            })?;
        let constant = receiver_unit
            .constants
            .get(const_decl.const_index as usize)
            .ok_or_else(|| {
                package_public_instance_error(
                    package,
                    &public_path,
                    format!(
                        "const selector {}.{} points to missing const index {}",
                        public_instance.module,
                        public_instance.const_symbol,
                        const_decl.const_index
                    ),
                )
            })?;
        let receiver_const = OperationConstReceiverRef {
            file_ref: receiver_file.clone(),
            const_index: const_decl.const_index,
            const_abi_id: format!(
                "const:{}.{}",
                public_instance.module, public_instance.const_symbol
            ),
            const_type_abi_id: type_ref_abi_key(&constant.ty),
        };
        let receiver = package_public_instance_receiver(
            package,
            file_units_by_module,
            &public_path,
            &public_instance.module,
            &constant.ty,
        )?;
        let implemented_interfaces = package_public_instance_interfaces(
            package,
            file_units_by_module,
            type_symbols,
            &public_path,
            &receiver,
            &public_instance.interfaces,
        )?;
        let operations = package_public_instance_operations(
            package,
            files_by_module,
            &public_path,
            &receiver,
            &receiver_const,
            &implemented_interfaces,
            exports,
        )?;
        exports.public_instances.push(PublicInstanceExport {
            name: public_path,
            module_path: receiver_unit.module_path.clone(),
            declared_receiver_type: receiver_type_ref(&receiver.symbol),
            implemented_interfaces: implemented_interfaces
                .iter()
                .map(|interface| interface.ty.clone())
                .collect(),
            operations,
        });
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct PackagePublicInstanceReceiver<'a> {
    symbol: ServiceSymbolRef,
    unit: &'a FileIrUnit,
    decl: &'a skiff_artifact_model::TypeDeclIr,
}

#[derive(Debug, Clone)]
struct PackagePublicInstanceInterface {
    ty: TypeRefIr,
    instantiation: InterfaceInstantiationRef,
    methods: Vec<InterfaceMethodSignature>,
}

fn package_public_instance_receiver<'a>(
    package: &PackageIrProjectionSource<'_>,
    file_units_by_module: &'a StdBTreeMap<&str, &FileIrUnit>,
    public_path: &str,
    const_module: &str,
    ty: &TypeRefIr,
) -> Result<PackagePublicInstanceReceiver<'a>, ProjectionError> {
    let symbol = package_nominal_service_symbol(file_units_by_module, const_module, ty)
        .ok_or_else(|| {
            package_public_instance_error(
                package,
                public_path,
                "const must have an explicit nominal receiver type",
            )
        })?;
    let (unit, _type_index, decl) = package_type_decl_by_module_local_name(
        file_units_by_module,
        &symbol.module_path,
        &symbol.symbol,
    )
    .ok_or_else(|| {
        package_public_instance_error(
            package,
            public_path,
            format!(
                "receiver type {}.{} does not resolve to a package type",
                symbol.module_path, symbol.symbol
            ),
        )
    })?;
    if matches!(decl.descriptor, TypeDescriptorIr::Alias { .. })
        || unit.declarations.interfaces.contains_key(&symbol.symbol)
    {
        return Err(package_public_instance_error(
            package,
            public_path,
            format!(
                "receiver type {}.{} must be a concrete nominal type, not an alias or interface",
                symbol.module_path, symbol.symbol
            ),
        ));
    }
    Ok(PackagePublicInstanceReceiver { symbol, unit, decl })
}

fn package_public_instance_interfaces(
    package: &PackageIrProjectionSource<'_>,
    file_units_by_module: &StdBTreeMap<&str, &FileIrUnit>,
    type_symbols: &PackageTypeSymbolIndex,
    public_path: &str,
    receiver: &PackagePublicInstanceReceiver<'_>,
    interfaces: &[crate::package_exports::PackageExportPublicInstanceInterface],
) -> Result<Vec<PackagePublicInstanceInterface>, ProjectionError> {
    if interfaces.is_empty() {
        return Err(package_public_instance_error(
            package,
            public_path,
            "interfaces must not be empty",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut projected = Vec::new();
    for interface in interfaces {
        let (interface_unit, _type_index, _type_decl) = package_type_decl_by_module_local_name(
            file_units_by_module,
            &interface.module,
            &interface.symbol,
        )
        .ok_or_else(|| {
            package_public_instance_error(
                package,
                public_path,
                format!(
                    "interface selector points to missing type {}.{}",
                    interface.module, interface.symbol
                ),
            )
        })?;
        let interface_decl = interface_unit
            .declarations
            .interfaces
            .get(&interface.symbol)
            .ok_or_else(|| {
                package_public_instance_error(
                    package,
                    public_path,
                    format!(
                        "interface selector {}.{} must resolve to an interface",
                        interface.module, interface.symbol
                    ),
                )
            })?;
        if !receiver.decl.implements.iter().any(|implemented| {
            package_type_ref_matches_interface_selector(
                file_units_by_module,
                &receiver.symbol.module_path,
                implemented,
                &interface.module,
                &interface.symbol,
            )
        }) {
            return Err(package_public_instance_error(
                package,
                public_path,
                format!(
                    "receiver {}.{} does not explicitly implement listed interface {}.{}",
                    receiver.symbol.module_path,
                    receiver.symbol.symbol,
                    interface.module,
                    interface.symbol
                ),
            ));
        }
        let interface_ty =
            package_public_interface_type_ref(package, &interface.module, &interface.symbol);
        let interface_ref = interface_instantiation_ref(
            interface_ty.clone(),
            interface.canonical_type_args.clone(),
        );
        let interface_key =
            serde_json::to_string(&interface_ref).expect("interface ref must serialize");
        if !seen.insert(interface_key) {
            return Err(package_public_instance_error(
                package,
                public_path,
                format!(
                    "duplicate interface selector {}.{}",
                    interface.module, interface.symbol
                ),
            ));
        }
        let methods = package_interface_method_signatures(
            &package.package_id,
            type_symbols,
            &interface_unit.module_path,
            interface_decl,
        )
        .map_err(|message| package_public_instance_error(package, public_path, message))?;
        let methods = instantiate_interface_method_signatures(
            methods,
            &interface_decl.type_params,
            &interface_ref.canonical_type_args,
        )
        .map_err(|error| {
            package_public_instance_error(
                package,
                public_path,
                format!(
                    "interface {}.{} expects {} type arguments but got {}",
                    interface.module,
                    interface.symbol,
                    error.expected_type_args,
                    error.actual_type_args
                ),
            )
        })?;
        projected.push(PackagePublicInstanceInterface {
            ty: interface_ty,
            instantiation: interface_ref,
            methods,
        });
    }
    Ok(projected)
}

fn package_public_instance_operations(
    package: &PackageIrProjectionSource<'_>,
    files_by_module: &StdBTreeMap<String, FileIrRef>,
    public_path: &str,
    receiver: &PackagePublicInstanceReceiver<'_>,
    receiver_const: &OperationConstReceiverRef,
    interfaces: &[PackagePublicInstanceInterface],
    exports: &mut PackageExportIndex,
) -> Result<Vec<PublicInstanceOperation>, ProjectionError> {
    let mut operations = Vec::new();
    let mut method_names = BTreeSet::new();
    for interface in interfaces {
        for method in &interface.methods {
            if !method_names.insert(method.name.clone()) {
                return Err(package_public_instance_error(
                    package,
                    public_path,
                    format!(
                        "derives conflicting operation `{}` from multiple interfaces",
                        method.name
                    ),
                ));
            }
            let target_symbol = impl_method_declaration_name(&receiver.symbol.symbol, &method.name);
            let executable_index = impl_method_executable_index(receiver.unit, &target_symbol)
                .ok_or_else(|| {
                    package_public_instance_error(
                        package,
                        public_path,
                        format!(
                            "receiver {}.{} is missing implementation method {}",
                            receiver.symbol.module_path, receiver.symbol.symbol, method.name
                        ),
                    )
                })?;
            let executable = receiver
                .unit
                .executables
                .get(executable_index as usize)
                .ok_or_else(|| {
                    package_public_instance_error(
                        package,
                        public_path,
                        format!(
                            "receiver {}.{} method {} points to missing executable index {}",
                            receiver.symbol.module_path,
                            receiver.symbol.symbol,
                            method.name,
                            executable_index
                        ),
                    )
                })?;
            let executable_signature = executable_signature(executable);
            let public_signature =
                public_signature_from_receiver_executable_signature(&executable_signature);
            let interface_signature = public_callable_signature_from_interface_method(method);
            if public_signature != interface_signature {
                return Err(package_public_instance_error(
                    package,
                    public_path,
                    format!(
                        "receiver {}.{} method {} signature does not match listed interface method",
                        receiver.symbol.module_path, receiver.symbol.symbol, method.name
                    ),
                ));
            }
            let operation = package_public_instance_operation_ref(
                public_path,
                &interface.instantiation,
                &method.name,
                &interface_signature,
            );
            let target_file = files_by_module
                .get(&receiver.unit.module_path)
                .cloned()
                .ok_or_else(|| {
                    package_public_instance_error(
                        package,
                        public_path,
                        format!(
                            "receiver module {} has no File IR ref",
                            receiver.unit.module_path
                        ),
                    )
                })?;
            let executable_export = ExecutableExport {
                file: target_file.clone(),
                executable_index,
                symbol: target_symbol.clone(),
                signature: executable_signature,
            };
            exports
                .impl_methods
                .entry(target_symbol.clone())
                .or_insert(executable_export);
            let method_abi_id = operation
                .method_abi_id
                .clone()
                .unwrap_or_else(|| operation.operation_abi_id.clone());
            operations.push(PublicInstanceOperation {
                operation,
                receiver_executable: LocalReceiverExecutableRef {
                    receiver: receiver_const.clone(),
                    executable_target: OperationTargetRef {
                        file_ref: target_file,
                        executable_index,
                        callable_abi_id: format!(
                            "callable:{}.{}",
                            receiver.unit.module_path, target_symbol
                        ),
                        callable_kind: OperationCallableKind::ImplMethod,
                    },
                    method_abi_id,
                    receiver_call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                },
            });
        }
    }
    Ok(operations)
}

fn impl_method_executable_index(unit: &FileIrUnit, target_symbol: &str) -> Option<u32> {
    unit.link_targets
        .executables
        .get(target_symbol)
        .map(|target| target.executable_index)
        .or_else(|| {
            unit.declarations
                .executables
                .get(target_symbol)
                .map(|target| target.executable_index)
        })
}

fn package_public_instance_operation_ref(
    public_instance_key: &str,
    interface: &InterfaceInstantiationRef,
    method_name: &str,
    public_signature: &CanonicalPublicCallableSignature,
) -> OperationAbiRef {
    let public_path = format!("{public_instance_key}.{method_name}");
    let method_abi_id = canonical_interface_method_abi_id(&interface, method_name);
    OperationAbiRef {
        operation_abi_id: public_instance_method_operation_abi_id(
            &public_path,
            public_instance_key,
            interface,
            &method_abi_id,
            public_signature,
            &[],
            &StdBTreeMap::new(),
        ),
        kind: PublicationOperationKind::PublicInstanceMethod,
        public_path: public_path.clone(),
        public_instance_key: Some(public_instance_key.to_string()),
        interface: Some(interface.clone()),
        method_abi_id: Some(method_abi_id),
        display_name: public_path,
    }
}

fn public_callable_signature_from_interface_method(
    method: &InterfaceMethodSignature,
) -> CanonicalPublicCallableSignature {
    CanonicalPublicCallableSignature {
        params: method.params.clone(),
        return_type: method.return_type.clone(),
        may_suspend: is_stream_type_ref(&method.return_type),
    }
}

fn is_stream_type_ref(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, .. } if name == "Stream")
}

fn public_signature_from_receiver_executable_signature(
    signature: &ExecutableSignatureIr,
) -> CanonicalPublicCallableSignature {
    let mut public_signature = CanonicalPublicCallableSignature::from(signature.clone());
    if let Some(self_type) = &signature.self_type {
        if public_signature
            .params
            .first()
            .is_some_and(|param| &param.ty == self_type)
        {
            public_signature.params.remove(0);
        }
    }
    public_signature
}

fn package_public_interface_type_ref(
    package: &PackageIrProjectionSource<'_>,
    module: &str,
    symbol: &str,
) -> TypeRefIr {
    let symbol_path = package
        .exports
        .symbols
        .iter()
        .find_map(|(public_path, export)| {
            (export.module == module && export.symbol == symbol)
                .then(|| package_scoped_export_symbol(package, public_path))
        })
        .unwrap_or_else(|| format!("{module}.{symbol}"));
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package.package_id.to_string(),
            },
            symbol_path,
            abi_expectation: None,
        },
    }
}

fn package_nominal_service_symbol(
    file_units_by_module: &StdBTreeMap<&str, &FileIrUnit>,
    module_path: &str,
    ty: &TypeRefIr,
) -> Option<ServiceSymbolRef> {
    match ty {
        TypeRefIr::LocalType { type_index } => {
            let unit = file_units_by_module.get(module_path).copied()?;
            let decl = unit.type_table.get(*type_index as usize)?;
            Some(ServiceSymbolRef {
                module_path: module_path.to_string(),
                symbol: decl.name.clone(),
            })
        }
        TypeRefIr::ServiceSymbol { symbol } => {
            package_type_decl_by_module_local_name(
                file_units_by_module,
                &symbol.module_path,
                &symbol.symbol,
            )?;
            Some(symbol.clone())
        }
        TypeRefIr::Native { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => None,
    }
}

fn package_type_decl_by_module_local_name<'a>(
    file_units_by_module: &'a StdBTreeMap<&str, &FileIrUnit>,
    module_path: &str,
    name: &str,
) -> Option<(&'a FileIrUnit, u32, &'a skiff_artifact_model::TypeDeclIr)> {
    let unit = file_units_by_module.get(module_path).copied()?;
    let type_index = unit.declarations.types.get(name)?.type_index;
    let decl = unit.type_table.get(type_index as usize)?;
    Some((unit, type_index, decl))
}

fn package_type_ref_matches_interface_selector(
    file_units_by_module: &StdBTreeMap<&str, &FileIrUnit>,
    context_module: &str,
    ty: &TypeRefIr,
    interface_module: &str,
    interface_symbol: &str,
) -> bool {
    match ty {
        TypeRefIr::LocalType { type_index } => file_units_by_module
            .get(context_module)
            .and_then(|unit| unit.type_table.get(*type_index as usize))
            .is_some_and(|decl| {
                decl.name == interface_symbol && context_module == interface_module
            }),
        TypeRefIr::ServiceSymbol { symbol } => {
            symbol.module_path == interface_module && symbol.symbol == interface_symbol
        }
        TypeRefIr::Native { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Record { .. }
        | TypeRefIr::Union { .. }
        | TypeRefIr::Nullable { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. }
        | TypeRefIr::AnyInterface { .. }
        | TypeRefIr::Function { .. } => false,
    }
}

fn receiver_type_ref(receiver: &ServiceSymbolRef) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: receiver.clone(),
    }
}

fn package_public_instance_error(
    package: &PackageIrProjectionSource<'_>,
    public_instance: &str,
    message: impl Into<String>,
) -> ProjectionError {
    ProjectionError::ContractValidation {
        message: format!(
            "package {} public instance {}: {}",
            package.package_id,
            public_instance,
            message.into()
        ),
    }
}

fn package_type_symbol_index(
    package: &PackageIrProjectionSource<'_>,
    file_units_by_module: &StdBTreeMap<&str, &FileIrUnit>,
    dependencies: &[ProjectedPackageDependency],
) -> Result<PackageTypeSymbolIndex, ProjectionError> {
    let mut index = PackageTypeSymbolIndex::default();
    for dependency in dependencies {
        index.insert_dependency(dependency.effective_alias(), dependency.id.as_str());
        index.insert_dependency(dependency.id.as_str(), dependency.id.as_str());
    }
    for (public_symbol, export) in &package.exports.symbols {
        let module = export.module.as_str();
        let symbol = export.symbol.as_str();
        let Some(file_unit) = file_units_by_module.get(module).copied() else {
            continue;
        };
        let Some(type_index) = type_link_target_index(file_unit, symbol) else {
            continue;
        };
        let Some(type_decl) = file_unit.type_table.get(type_index as usize) else {
            return Err(package_export_error(
                package,
                public_symbol,
                format!(
                    "type export index {type_index} is out of bounds for module {module} type table"
                ),
            ));
        };
        index.insert_type(
            module.to_string(),
            type_index,
            type_decl.name.clone(),
            package_scoped_export_symbol(package, public_symbol),
        );
    }
    Ok(index)
}

fn package_export_error(
    package: &PackageIrProjectionSource<'_>,
    public_symbol: &str,
    message: impl Into<String>,
) -> ProjectionError {
    ProjectionError::ContractValidation {
        message: format!(
            "package {} export {}: {}",
            package.package_id,
            public_symbol,
            message.into()
        ),
    }
}

fn package_scoped_export_symbol(
    package: &PackageIrProjectionSource<'_>,
    public_symbol: &str,
) -> String {
    if package.package_id == SKIFF_STD_PUBLICATION_ID && !public_symbol.starts_with("std.") {
        format!("std.{public_symbol}")
    } else {
        public_symbol.to_string()
    }
}

fn type_link_target_index(unit: &FileIrUnit, symbol: &str) -> Option<u32> {
    unit.link_targets
        .types
        .get(symbol)
        .map(|target| target.type_index)
}

fn executable_link_target_index(unit: &FileIrUnit, symbol: &str) -> Option<u32> {
    unit.link_targets
        .executables
        .get(symbol)
        .map(|target| target.executable_index)
}

fn const_link_target_index(unit: &FileIrUnit, symbol: &str) -> Option<u32> {
    unit.link_targets
        .constants
        .get(symbol)
        .map(|target| target.const_index)
}

fn type_export_decl<'a>(
    package: &PackageIrProjectionSource<'_>,
    public_symbol: &str,
    module: &str,
    file_unit: &'a FileIrUnit,
    type_index: u32,
) -> Result<&'a skiff_artifact_model::TypeDeclIr, ProjectionError> {
    file_unit.type_table.get(type_index as usize).ok_or_else(|| {
        package_export_error(
            package,
            public_symbol,
            format!("type export index {type_index} is out of bounds for module {module} type table"),
        )
    })
}

fn const_export_decl<'a>(
    package: &PackageIrProjectionSource<'_>,
    public_symbol: &str,
    module: &str,
    file_unit: &'a FileIrUnit,
    const_index: u32,
) -> Result<&'a ConstIr, ProjectionError> {
    file_unit
        .constants
        .get(const_index as usize)
        .ok_or_else(|| {
            package_export_error(
                package,
                public_symbol,
                format!("const export index {const_index} is out of bounds for module {module}"),
            )
        })
}

fn executable_export_decl<'a>(
    package: &PackageIrProjectionSource<'_>,
    public_symbol: &str,
    module: &str,
    file_unit: &'a FileIrUnit,
    executable_index: u32,
) -> Result<&'a ExecutableIr, ProjectionError> {
    file_unit
        .executables
        .get(executable_index as usize)
        .ok_or_else(|| {
            package_export_error(
                package,
                public_symbol,
                format!(
                    "executable export index {executable_index} is out of bounds for module {module}"
                ),
            )
        })
}

/// Whether an executable is an instance receiver method (carries an explicit
/// `self` receiver), as opposed to a free function or a static method. Used to
/// keep instance methods out of the package's `PublicFunction` operation surface
/// while leaving static methods (function-compatible callables) as operations.
fn executable_has_self_receiver(executable: &ExecutableIr) -> bool {
    executable.self_type.is_some()
        || executable
            .params
            .first()
            .is_some_and(|param| param.name == "self")
}

/// Project the public signature of a package executable directly from its typed
/// declaration. `ExecutableIr` already carries the params/return/self type as
/// `TypeRefIr`s, so no JSON reparsing is needed.
fn executable_signature(executable: &ExecutableIr) -> ExecutableSignatureIr {
    ExecutableSignatureIr {
        params: executable.params.clone(),
        return_type: executable.return_type.clone(),
        self_type: executable.self_type.clone(),
        may_suspend: executable.may_suspend,
    }
}

pub fn package_unit_dependency_constraints(
    dependencies: &[ProjectedPackageDependency],
    file_ir_units: &[PackageFileIrProjection],
    package_id: &str,
) -> Vec<PackageDependencyConstraint> {
    let mut constraints = dependencies
        .iter()
        .map(package_dependency_constraint)
        .collect::<Vec<_>>();
    if package_id != SKIFF_STD_PUBLICATION_ID
        && file_ir_units_reference_std_package(file_ir_units.iter().map(|artifact| &artifact.unit))
        && !constraints
            .iter()
            .any(|dependency| dependency.id == SKIFF_STD_PUBLICATION_ID)
    {
        constraints.push(std_package_dependency_constraint());
    }
    constraints
}

pub fn package_dependency_constraint(
    dependency: &ProjectedPackageDependency,
) -> PackageDependencyConstraint {
    let alias = dependency.effective_alias().to_string();
    PackageDependencyConstraint {
        id: dependency.id.clone(),
        version: dependency.version.clone(),
        alias,
        config: dependency.config.clone(),
    }
}

pub fn std_package_dependency_constraint() -> PackageDependencyConstraint {
    PackageDependencyConstraint {
        id: SKIFF_STD_PUBLICATION_ID.to_string(),
        version: "1.0.0".to_string(),
        alias: "std".to_string(),
        config: crate::context::empty_dependency_config(),
    }
}

pub fn file_ir_units_reference_std_package<'a>(
    file_ir_units: impl IntoIterator<Item = &'a FileIrUnit>,
) -> bool {
    file_ir_units
        .into_iter()
        .any(|file| file_unit_references_package(file, SKIFF_STD_PUBLICATION_ID))
}

fn file_unit_references_package(file: &FileIrUnit, package_id: &str) -> bool {
    file.external_refs
        .package_symbols
        .iter()
        .any(|symbol| package_symbol_references_package(symbol, package_id))
        || file.type_table.iter().any(|ty| {
            type_descriptor_references_package(&ty.descriptor, package_id)
                || ty
                    .implements
                    .iter()
                    .any(|implemented| type_ref_references_package(implemented, package_id))
        })
        || file.executables.iter().any(|executable| {
            executable
                .params
                .iter()
                .any(|param| type_ref_references_package(&param.ty, package_id))
                || type_ref_references_package(&executable.return_type, package_id)
                || executable
                    .self_type
                    .as_ref()
                    .is_some_and(|ty| type_ref_references_package(ty, package_id))
        })
}

fn type_descriptor_references_package(descriptor: &TypeDescriptorIr, package_id: &str) -> bool {
    match descriptor {
        TypeDescriptorIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_references_package(field, package_id)),
        TypeDescriptorIr::Alias { target } => type_ref_references_package(target, package_id),
        TypeDescriptorIr::Union { variants } => variants
            .iter()
            .any(|variant| type_ref_references_package(variant, package_id)),
        TypeDescriptorIr::Native { .. } => false,
    }
}

fn type_ref_references_package(ty: &TypeRefIr, package_id: &str) -> bool {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => {
            package_symbol_references_package(symbol, package_id)
        }
        TypeRefIr::Native { args, .. } => args
            .iter()
            .any(|arg| type_ref_references_package(arg, package_id)),
        TypeRefIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_references_package(field, package_id)),
        TypeRefIr::Union { items } => items
            .iter()
            .any(|item| type_ref_references_package(item, package_id)),
        TypeRefIr::Nullable { inner } => type_ref_references_package(inner, package_id),
        TypeRefIr::AnyInterface { interface } => {
            interface_instantiation_references_package(interface, package_id)
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| type_ref_references_package(&param.ty, package_id))
                || type_ref_references_package(return_type, package_id)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

fn interface_instantiation_references_package(
    interface: &InterfaceInstantiationRef,
    package_id: &str,
) -> bool {
    serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .ok()
        .is_some_and(|ty| type_ref_references_package(&ty, package_id))
        || interface
            .canonical_type_args
            .iter()
            .any(|arg| type_ref_references_package(arg, package_id))
}

fn package_symbol_references_package(symbol: &PackageSymbolRef, package_id: &str) -> bool {
    match &symbol.package {
        PackageRefIr::PackageId {
            package_id: candidate,
        } => candidate == package_id,
        PackageRefIr::Dependency { .. } => false,
    }
}
