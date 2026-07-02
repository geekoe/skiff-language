use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    canonical_interface_method_abi_id, interface_instantiation_ref,
    CanonicalPublicCallableSignature, ExecutableIr, ExecutableSignatureIr, FileIrUnit,
    InterfaceInstantiationRef, OperationAbiRef, PackageRefIr, PackageSymbolRef, PublicationAbiUnit,
    PublicationOperationAbi, PublicationOperationKind, ServiceSymbolRef,
    SourceCallOperationIndexEntry, TypeDeclIr, TypeRefIr,
};

use crate::{
    shared::{
        ast::TypeDecl,
        id::SKIFF_STD_PUBLICATION_ID,
        operation_abi_identity::{
            public_function_operation_abi_id, public_instance_method_operation_abi_id,
        },
        package_interface_methods::{
            instantiate_interface_method_signatures, normalize_package_interface_type_ref,
            package_interface_method_signatures, InterfaceMethodSignature, PackageTypeSymbolIndex,
        },
        publication_error::PublicationError,
    },
    SourceCompilePackageFacts,
};

use super::{
    api::PublicSymbolKind, semantic::impl_method_declaration_name, ExportCallableBinding,
    ExportPublicInstanceBinding, TypeResolutionContext,
};

#[derive(Debug)]
pub struct DependencyPackageOperationFacts {
    package_id: String,
    version: String,
    source_call_operations: Vec<SourceCallOperationIndexEntry>,
}

struct PackagePublicInstanceInterfaceProjection {
    instantiation: InterfaceInstantiationRef,
    methods: Vec<InterfaceMethodSignature>,
}

impl DependencyPackageOperationFacts {
    pub fn from_package_facts(
        package_facts: Option<&[SourceCompilePackageFacts<'_>]>,
    ) -> Result<Vec<Self>, PublicationError> {
        package_facts
            .unwrap_or_default()
            .iter()
            .map(Self::from_package_fact)
            .collect()
    }

    fn from_package_fact(
        package: &SourceCompilePackageFacts<'_>,
    ) -> Result<Self, PublicationError> {
        let publication_abi = package_dependency_publication_abi(package)?;
        Ok(Self {
            package_id: package.id().to_string(),
            version: package.version().to_string(),
            source_call_operations: publication_abi.source_call_operation_index,
        })
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn source_call_operations(&self) -> &[SourceCallOperationIndexEntry] {
        &self.source_call_operations
    }
}

fn package_dependency_publication_abi(
    package: &SourceCompilePackageFacts<'_>,
) -> Result<PublicationAbiUnit, PublicationError> {
    let package_id = package.id();
    let version = package.version();
    let mut publication_abi = PublicationAbiUnit::empty(package_id, version, "");
    let compiled = package.compile_model();
    let type_symbols = package_dependency_type_symbol_index(package)?;
    for binding in compiled.export_bindings().public_callables().values() {
        let public_path = package_scoped_public_path(package_id, &binding.public_path);
        let executable = package_public_callable_executable(package, binding)?;
        // An INSTANCE impl method of an exported type (explicit `self` receiver)
        // is part of the contract but is not a standalone `PublicFunction`
        // operation -- it is dispatched through an instance as an impl-method call.
        // Keep the dependency's operation surface in agreement with the package's
        // own publication ABI (which routes these into `impl_methods`, not the
        // operation surface) so the runtime package linker never sees a
        // `PublicFunction` operation backed by a receiver `ImplMethod`. Static
        // methods carry no receiver and remain real function-compatible
        // operations (e.g. an HTTP route handler), so gate on the explicit `self`
        // receiver, not on `ExecutableKind` (static methods also lower to
        // `ImplMethod`).
        let has_self_receiver = executable.self_type.is_some()
            || executable
                .params
                .first()
                .is_some_and(|param| param.name == "self");
        if has_self_receiver {
            continue;
        }
        let public_signature = CanonicalPublicCallableSignature::from(ExecutableSignatureIr {
            params: executable.params.clone(),
            return_type: executable.return_type.clone(),
            self_type: executable.self_type.clone(),
            may_suspend: executable.may_suspend,
        });
        let operation = OperationAbiRef {
            operation_abi_id: public_function_operation_abi_id(
                &public_path,
                &public_signature,
                &[],
                &BTreeMap::new(),
            ),
            kind: PublicationOperationKind::PublicFunction,
            public_path: public_path.clone(),
            public_instance_key: None,
            interface: None,
            method_abi_id: None,
            display_name: public_path.clone(),
        };
        publication_abi.operation_abi.push(PublicationOperationAbi {
            operation: operation.clone(),
            public_signature,
            schema_closure: Vec::new(),
            stream_effect_throw_config: BTreeMap::new(),
        });
        push_publication_source_call_operation(
            &mut publication_abi,
            package_id,
            public_path.clone(),
            operation.clone(),
        )?;
        publication_abi.operation_exports.push(operation);
    }
    for public_instance in compiled.export_bindings().public_instances().values() {
        let public_instance_key =
            package_scoped_public_path(package_id, &public_instance.public_path);
        let projected_instance = package_dependency_public_instance(
            package,
            &type_symbols,
            &public_instance_key,
            public_instance,
        )?;
        for operation in &projected_instance.method_operations {
            let public_signature =
                package_dependency_public_instance_operation_signature(package, operation)?;
            publication_abi.operation_abi.push(PublicationOperationAbi {
                operation: operation.clone(),
                public_signature,
                schema_closure: Vec::new(),
                stream_effect_throw_config: BTreeMap::new(),
            });
            push_publication_source_call_operation(
                &mut publication_abi,
                package_id,
                operation.public_path.clone(),
                operation.clone(),
            )?;
            publication_abi.operation_exports.push(operation.clone());
        }
        publication_abi.public_instances.push(projected_instance);
    }
    Ok(publication_abi)
}

fn push_publication_source_call_operation(
    publication_abi: &mut PublicationAbiUnit,
    package_id: &str,
    source_call_path: String,
    operation: OperationAbiRef,
) -> Result<(), PublicationError> {
    if let Some(existing) = publication_abi
        .source_call_operation_index
        .iter()
        .find(|entry| entry.source_call_path == source_call_path)
    {
        return Err(PublicationError::ContractValidation {
            message: format!(
                "package {package_id} publication ABI sourceCallOperationIndex duplicate sourceCallPath `{source_call_path}` for operations {} and {}",
                existing.operation.operation_abi_id, operation.operation_abi_id
            ),
        });
    }
    publication_abi
        .source_call_operation_index
        .push(SourceCallOperationIndexEntry {
            source_call_path,
            operation,
        });
    Ok(())
}

fn package_dependency_type_symbol_index(
    package: &SourceCompilePackageFacts<'_>,
) -> Result<PackageTypeSymbolIndex, PublicationError> {
    let package_id = package.id();
    let compiled = package.compile_model();
    let file_units_by_module = package
        .file_ir_units()
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let mut index = PackageTypeSymbolIndex::default();
    for dependency in package.dependencies() {
        index.insert_dependency(dependency.effective_alias(), dependency.id.as_str());
        index.insert_dependency(dependency.id.as_str(), dependency.id.as_str());
    }
    for binding in compiled.export_bindings().public_symbols().values() {
        if !matches!(
            binding.kind,
            PublicSymbolKind::Type | PublicSymbolKind::Alias | PublicSymbolKind::Interface
        ) {
            continue;
        }
        let Some(unit) = file_units_by_module
            .get(binding.source_module.as_str())
            .copied()
        else {
            continue;
        };
        let Some(type_decl) = unit.declarations.types.get(&binding.source_symbol) else {
            continue;
        };
        index.insert_type(
            binding.source_module.clone(),
            type_decl.type_index,
            binding.source_symbol.clone(),
            package_scoped_public_path(package_id, &binding.public_path),
        );
    }
    Ok(index)
}

fn package_dependency_public_instance(
    package: &SourceCompilePackageFacts<'_>,
    type_symbols: &PackageTypeSymbolIndex,
    public_instance_key: &str,
    public_instance: &ExportPublicInstanceBinding,
) -> Result<skiff_artifact_model::PublicationPublicInstanceExport, PublicationError> {
    let receiver = package_dependency_public_instance_receiver(package, public_instance)?;
    let interfaces = package_dependency_public_instance_interfaces(
        package,
        type_symbols,
        public_instance_key,
        &receiver,
        public_instance,
    )?;
    let mut source_call_method_index = Vec::new();
    let mut method_operations = Vec::new();
    let mut method_names = BTreeSet::new();
    for interface in &interfaces {
        for method in &interface.methods {
            if !method_names.insert(method.name.clone()) {
                return Err(PublicationError::ContractValidation {
                    message: format!(
                        "package {} public instance {} derives conflicting operation `{}` from multiple interfaces",
                        package.id(),
                        public_instance_key,
                        method.name
                    ),
                });
            }
            let executable = package_dependency_public_instance_method_executable(
                package,
                public_instance_key,
                &receiver,
                &method.name,
            )?;
            let public_signature =
                public_signature_from_receiver_executable(executable_signature_ir(executable));
            let interface_signature = public_signature_from_interface_method(method);
            if public_signature != interface_signature {
                return Err(PublicationError::ContractValidation {
                    message: format!(
                        "package {} public instance {} receiver {}.{} method {} signature does not match listed interface method",
                        package.id(),
                        public_instance_key,
                        receiver.module_path,
                        receiver.symbol,
                        method.name
                    ),
                });
            }
            let operation = package_dependency_public_instance_operation_ref(
                public_instance_key,
                &interface.instantiation,
                &method.name,
                &interface_signature,
            );
            source_call_method_index.push(skiff_artifact_model::SourceCallMethodIndexEntry {
                method_name: method.name.clone(),
                operation: operation.clone(),
            });
            method_operations.push(operation);
        }
    }
    Ok(skiff_artifact_model::PublicationPublicInstanceExport {
        public_instance_key: public_instance_key.to_string(),
        interfaces: interfaces
            .iter()
            .map(|interface| interface.instantiation.clone())
            .collect(),
        source_call_method_index,
        method_operations,
    })
}

fn package_dependency_public_instance_receiver(
    package: &SourceCompilePackageFacts<'_>,
    public_instance: &ExportPublicInstanceBinding,
) -> Result<ServiceSymbolRef, PublicationError> {
    let unit = package
        .file_ir_units()
        .iter()
        .find(|unit| unit.module_path == public_instance.source_module)
        .ok_or_else(|| PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {} const selector points to missing module {}",
                package.id(),
                public_instance.public_path,
                public_instance.source_module
            ),
        })?;
    let const_decl = unit
        .declarations
        .constants
        .get(&public_instance.source_symbol)
        .ok_or_else(|| PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {} const selector points to missing const {}.{}",
                package.id(),
                public_instance.public_path,
                public_instance.source_module,
                public_instance.source_symbol
            ),
        })?;
    let constant = unit
        .constants
        .get(const_decl.const_index as usize)
        .ok_or_else(|| PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {} const selector points to missing const index {}",
                package.id(),
                public_instance.public_path,
                const_decl.const_index
            ),
        })?;
    package_dependency_nominal_type_ref(package, &unit.module_path, &constant.ty).ok_or_else(|| {
        PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {} const must have an explicit nominal receiver type",
                package.id(),
                public_instance.public_path
            ),
        }
    })
}

fn package_dependency_public_instance_interfaces(
    package: &SourceCompilePackageFacts<'_>,
    type_symbols: &PackageTypeSymbolIndex,
    public_instance_key: &str,
    receiver: &ServiceSymbolRef,
    public_instance: &ExportPublicInstanceBinding,
) -> Result<Vec<PackagePublicInstanceInterfaceProjection>, PublicationError> {
    let receiver_decl =
        package_dependency_type_decl(package, &receiver.module_path, &receiver.symbol).ok_or_else(
            || PublicationError::ContractValidation {
                message: format!(
                    "package {} public instance {} receiver type {}.{} is missing",
                    package.id(),
                    public_instance_key,
                    receiver.module_path,
                    receiver.symbol
                ),
            },
        )?;
    let receiver_source_decl =
        package_dependency_source_type_decl(package, &receiver.module_path, &receiver.symbol)
            .ok_or_else(|| PublicationError::ContractValidation {
                message: format!(
                    "package {} public instance {} receiver source type {}.{} is missing",
                    package.id(),
                    public_instance_key,
                    receiver.module_path,
                    receiver.symbol
                ),
            })?;
    let mut seen = BTreeSet::new();
    let mut interfaces = Vec::new();
    for interface in &public_instance.interfaces {
        let interface_unit = package
            .file_ir_units()
            .iter()
            .find(|unit| unit.module_path == interface.source_module)
            .ok_or_else(|| PublicationError::ContractValidation {
                message: format!(
                    "package {} public instance {} interface selector points to missing module {}",
                    package.id(),
                    public_instance_key,
                    interface.source_module
                ),
            })?;
        let interface_decl = interface_unit
            .declarations
            .interfaces
            .get(&interface.source_symbol)
            .ok_or_else(|| PublicationError::ContractValidation {
                message: format!(
                    "package {} public instance {} interface selector {}.{} must resolve to an interface",
                    package.id(),
                    public_instance_key,
                    interface.source_module,
                    interface.source_symbol
                ),
            })?;
        if !receiver_decl.implements.iter().any(|implemented| {
            package_dependency_type_ref_matches_interface(
                package,
                &receiver.module_path,
                implemented,
                &interface.source_module,
                &interface.source_symbol,
            )
        }) {
            return Err(PublicationError::ContractValidation {
                message: format!(
                    "package {} public instance {} receiver {}.{} does not explicitly implement listed interface {}.{}",
                    package.id(),
                    public_instance_key,
                    receiver.module_path,
                    receiver.symbol,
                    interface.source_module,
                    interface.source_symbol
                ),
            });
        }
        let interface_ty = package_dependency_public_interface_type_ref(
            package,
            &interface.source_module,
            &interface.source_symbol,
        );
        let instantiation = package_dependency_public_interface_instantiation_ref(
            package,
            type_symbols,
            receiver_source_decl,
            &receiver.module_path,
            &interface_ty,
            &interface.source_module,
            &interface.source_symbol,
            public_instance_key,
        )?;
        let interface_key =
            serde_json::to_string(&instantiation).expect("interface ref must serialize");
        if !seen.insert(interface_key) {
            return Err(PublicationError::ContractValidation {
                message: format!(
                    "package {} public instance {} duplicate interface selector {}.{}",
                    package.id(),
                    public_instance_key,
                    interface.source_module,
                    interface.source_symbol
                ),
            });
        }
        let methods = package_interface_method_signatures(
            package.id(),
            type_symbols,
            &interface_unit.module_path,
            interface_decl,
        )
        .map_err(|message| PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {}: {message}",
                package.id(),
                public_instance_key
            ),
        })?;
        let methods = instantiate_interface_method_signatures(
            methods,
            &interface_decl.type_params,
            &instantiation.canonical_type_args,
        )
        .map_err(|error| PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {} interface {}.{} expects {} type arguments but got {}",
                package.id(),
                public_instance_key,
                interface.source_module,
                interface.source_symbol,
                error.expected_type_args,
                error.actual_type_args
            ),
        })?;
        interfaces.push(PackagePublicInstanceInterfaceProjection {
            instantiation,
            methods,
        });
    }
    Ok(interfaces)
}

fn package_dependency_public_interface_instantiation_ref(
    package: &SourceCompilePackageFacts<'_>,
    type_symbols: &PackageTypeSymbolIndex,
    receiver_source_decl: &TypeDecl,
    receiver_module_path: &str,
    interface_ty: &TypeRefIr,
    interface_module: &str,
    interface_symbol: &str,
    public_instance_key: &str,
) -> Result<InterfaceInstantiationRef, PublicationError> {
    let type_resolution = package.compile_model().type_resolution();
    let context = TypeResolutionContext::with_type_params(
        receiver_module_path,
        receiver_source_decl.type_params.iter().cloned().collect(),
    );
    for implemented in &receiver_source_decl.implements {
        let Some((interface_identity, type_args)) = type_resolution
            .resolve_interface_instantiation_parts_text(&implemented.name, &context)
            .map_err(|message| PublicationError::ContractValidation {
                message: format!(
                    "package {} public instance {} receiver {}.{} implements entry `{}` failed to resolve: {message}",
                    package.id(),
                    public_instance_key,
                    receiver_module_path,
                    receiver_source_decl.name,
                    implemented.name
                ),
            })?
        else {
            continue;
        };
        if !package_dependency_type_ref_matches_interface(
            package,
            receiver_module_path,
            &interface_identity,
            interface_module,
            interface_symbol,
        ) {
            continue;
        }
        let context_name = format!(
            "{}.{} implements {}",
            receiver_module_path, receiver_source_decl.name, implemented.name
        );
        let canonical_type_args = type_args
            .iter()
            .map(|arg| {
                normalize_package_interface_type_ref(
                    package.id(),
                    type_symbols,
                    receiver_module_path,
                    arg,
                    &context_name,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|message| PublicationError::ContractValidation {
                message: format!(
                    "package {} public instance {} receiver {}.{} interface {} type argument failed to normalize: {message}",
                    package.id(),
                    public_instance_key,
                    receiver_module_path,
                    receiver_source_decl.name,
                    implemented.name
                ),
            })?;
        return Ok(interface_instantiation_ref(
            interface_ty.clone(),
            canonical_type_args,
        ));
    }

    Err(PublicationError::ContractValidation {
        message: format!(
            "package {} public instance {} receiver {}.{} does not explicitly implement listed interface {}.{}",
            package.id(),
            public_instance_key,
            receiver_module_path,
            receiver_source_decl.name,
            interface_module,
            interface_symbol
        ),
    })
}

fn package_dependency_public_instance_method_executable<'a>(
    package: &'a SourceCompilePackageFacts<'a>,
    public_instance_key: &str,
    receiver: &ServiceSymbolRef,
    method_name: &str,
) -> Result<&'a ExecutableIr, PublicationError> {
    let unit = package
        .file_ir_units()
        .iter()
        .find(|unit| unit.module_path == receiver.module_path)
        .ok_or_else(|| PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {} receiver module {} is missing",
                package.id(),
                public_instance_key,
                receiver.module_path
            ),
        })?;
    let target_symbol = impl_method_declaration_name(&receiver.symbol, method_name);
    let executable_index = package_dependency_impl_method_executable_index(unit, &target_symbol)
        .ok_or_else(|| PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {} receiver {}.{} is missing implementation method {}",
                package.id(),
                public_instance_key,
                receiver.module_path,
                receiver.symbol,
                method_name
            ),
        })?;
    unit.executables
        .get(executable_index as usize)
        .ok_or_else(|| PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {} receiver {}.{} method {} points to missing executable index {}",
                package.id(),
                public_instance_key,
                receiver.module_path,
                receiver.symbol,
                method_name,
                executable_index
            ),
        })
}

fn package_dependency_impl_method_executable_index(
    unit: &FileIrUnit,
    target_symbol: &str,
) -> Option<u32> {
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

fn package_dependency_public_instance_operation_signature(
    package: &SourceCompilePackageFacts<'_>,
    operation: &OperationAbiRef,
) -> Result<CanonicalPublicCallableSignature, PublicationError> {
    let public_instance_key = operation.public_instance_key.as_deref().ok_or_else(|| {
        PublicationError::ContractValidation {
            message: format!(
                "package {} public instance operation {} is missing publicInstanceKey",
                package.id(),
                operation.operation_abi_id
            ),
        }
    })?;
    let method_name = operation
        .public_path
        .strip_prefix(&format!("{public_instance_key}."))
        .unwrap_or(operation.public_path.as_str());
    let public_instance = package
        .compile_model()
        .export_bindings()
        .public_instances()
        .values()
        .find(|instance| {
            package_scoped_public_path(package.id(), &instance.public_path) == public_instance_key
        })
        .ok_or_else(|| PublicationError::ContractValidation {
            message: format!(
                "package {} public instance {} is missing from export bindings",
                package.id(),
                public_instance_key
            ),
        })?;
    let receiver = package_dependency_public_instance_receiver(package, public_instance)?;
    let executable = package_dependency_public_instance_method_executable(
        package,
        public_instance_key,
        &receiver,
        method_name,
    )?;
    Ok(public_signature_from_receiver_executable(
        executable_signature_ir(executable),
    ))
}

fn package_dependency_public_instance_operation_ref(
    public_instance_key: &str,
    interface: &InterfaceInstantiationRef,
    method_name: &str,
    public_signature: &CanonicalPublicCallableSignature,
) -> OperationAbiRef {
    let public_path = format!("{public_instance_key}.{method_name}");
    let method_abi_id = canonical_interface_method_abi_id(interface, method_name);
    OperationAbiRef {
        operation_abi_id: public_instance_method_operation_abi_id(
            &public_path,
            public_instance_key,
            interface,
            &method_abi_id,
            public_signature,
            &[],
            &BTreeMap::new(),
        ),
        kind: PublicationOperationKind::PublicInstanceMethod,
        public_path: public_path.clone(),
        public_instance_key: Some(public_instance_key.to_string()),
        interface: Some(interface.clone()),
        method_abi_id: Some(method_abi_id),
        display_name: public_path,
    }
}

fn package_dependency_public_interface_type_ref(
    package: &SourceCompilePackageFacts<'_>,
    module: &str,
    symbol: &str,
) -> TypeRefIr {
    let symbol_path = package
        .compile_model()
        .export_bindings()
        .public_symbols()
        .values()
        .find_map(|binding| {
            (binding.source_module == module
                && binding.source_symbol == symbol
                && matches!(
                    binding.kind,
                    PublicSymbolKind::Type | PublicSymbolKind::Alias | PublicSymbolKind::Interface
                ))
            .then(|| package_scoped_public_path(package.id(), &binding.public_path))
        })
        .unwrap_or_else(|| format!("{module}.{symbol}"));
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: package.id().to_string(),
            },
            symbol_path,
            abi_expectation: None,
        },
    }
}

fn package_dependency_nominal_type_ref(
    package: &SourceCompilePackageFacts<'_>,
    module_path: &str,
    ty: &TypeRefIr,
) -> Option<ServiceSymbolRef> {
    match ty {
        TypeRefIr::LocalType { type_index } => {
            let unit = package
                .file_ir_units()
                .iter()
                .find(|unit| unit.module_path == module_path)?;
            let decl = unit.type_table.get(*type_index as usize)?;
            Some(ServiceSymbolRef {
                module_path: module_path.to_string(),
                symbol: decl.name.clone(),
            })
        }
        TypeRefIr::ServiceSymbol { symbol } => {
            package_dependency_type_decl(package, &symbol.module_path, &symbol.symbol)?;
            Some(symbol.clone())
        }
        _ => None,
    }
}

fn package_dependency_type_decl<'a>(
    package: &'a SourceCompilePackageFacts<'a>,
    module_path: &str,
    name: &str,
) -> Option<&'a TypeDeclIr> {
    let unit = package
        .file_ir_units()
        .iter()
        .find(|unit| unit.module_path == module_path)?;
    let type_index = unit.declarations.types.get(name)?.type_index;
    unit.type_table.get(type_index as usize)
}

fn package_dependency_source_type_decl<'a>(
    package: &'a SourceCompilePackageFacts<'a>,
    module_path: &str,
    name: &str,
) -> Option<&'a TypeDecl> {
    package
        .compile_model()
        .sources()
        .parsed_sources()
        .iter()
        .find(|source| source.source().module_path == module_path)?
        .ast()
        .types
        .iter()
        .find(|ty| ty.name == name)
}

fn package_dependency_type_ref_matches_interface(
    package: &SourceCompilePackageFacts<'_>,
    context_module: &str,
    ty: &TypeRefIr,
    interface_module: &str,
    interface_symbol: &str,
) -> bool {
    match ty {
        TypeRefIr::LocalType { type_index } => package
            .file_ir_units()
            .iter()
            .find(|unit| unit.module_path == context_module)
            .and_then(|unit| unit.type_table.get(*type_index as usize))
            .is_some_and(|decl| {
                decl.name == interface_symbol && context_module == interface_module
            }),
        TypeRefIr::ServiceSymbol { symbol } => {
            symbol.module_path == interface_module && symbol.symbol == interface_symbol
        }
        _ => false,
    }
}

fn public_signature_from_interface_method(
    method: &InterfaceMethodSignature,
) -> CanonicalPublicCallableSignature {
    CanonicalPublicCallableSignature {
        params: method.params.clone(),
        return_type: method.return_type.clone(),
        may_suspend: matches!(method.return_type, TypeRefIr::Native { ref name, .. } if name == "Stream"),
    }
}

fn public_signature_from_receiver_executable(
    signature: ExecutableSignatureIr,
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

fn executable_signature_ir(executable: &ExecutableIr) -> ExecutableSignatureIr {
    ExecutableSignatureIr {
        params: executable.params.clone(),
        return_type: executable.return_type.clone(),
        self_type: executable.self_type.clone(),
        may_suspend: executable.may_suspend,
    }
}

fn package_public_callable_executable<'a>(
    package: &'a SourceCompilePackageFacts<'a>,
    binding: &ExportCallableBinding,
) -> Result<&'a ExecutableIr, PublicationError> {
    let Some(unit) = package
        .file_ir_units()
        .iter()
        .find(|unit| unit.module_path == binding.source_module)
    else {
        return Err(PublicationError::ContractValidation {
            message: format!(
                "package {} public operation {} points to missing module {}",
                package.id(),
                binding.public_path,
                binding.source_module
            ),
        });
    };
    let Some(declaration) = unit.declarations.executables.get(&binding.source_symbol) else {
        return Err(PublicationError::ContractValidation {
            message: format!(
                "package {} public operation {} points to missing executable {}.{}",
                package.id(),
                binding.public_path,
                binding.source_module,
                binding.source_symbol
            ),
        });
    };
    unit.executables
        .get(declaration.executable_index as usize)
        .ok_or_else(|| PublicationError::ContractValidation {
            message: format!(
                "package {} public operation {} points to missing executable index {}",
                package.id(),
                binding.public_path,
                declaration.executable_index
            ),
        })
}

fn package_scoped_public_path(package_id: &str, public_path: &str) -> String {
    if package_id == SKIFF_STD_PUBLICATION_ID && !public_path.starts_with("std.") {
        format!("std.{public_path}")
    } else {
        public_path.to_string()
    }
}
