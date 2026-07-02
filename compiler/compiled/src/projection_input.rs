use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    ExecutableKind, FileIrUnit, InterfaceInstantiationRef, LiteralIr, ServiceSymbolRef, TypeRefIr,
};
use skiff_compiler_projection_input::{
    ConfigRequirementAccessProjection, ConfigRequirementDependencyStepProjection,
    ConfigRequirementProjection, ConfigRequirementProvenanceProjection,
    ConfigRequirementPublicationProjection, ConfigRequirementScopeProjection,
    ConfigRequirementSetProjection, ConfigRequirementsSeed, ConfigSourcePositionProjection,
    ConfigSourceSpanProjection, EntryFunctionSignature, EntryParamSpec, EntryTypeSpec,
    ExportBindingProjection, ExportCallableProjection, ExportPublicInstanceInterfaceProjection,
    ExportPublicInstanceProjection, ExportSchemaProjection, ExportSymbolProjection, PackageAbiType,
    PackageAbiTypeDescriptor, PackageApiEntryProjectionInfo, PackageApiSourceProjectionInfo,
    PackageDependencyProjectionInfo, PackageEntrypointFunctionProjection,
    PackageEntrypointProjectionFacts, PackageProjectionInput, PackageProjectionInputParts,
    PackagePublicationProjectionInfo, PackagePublicationProjectionProvenance,
    ProjectionAbiDeclarationIds, ProjectionDeclarationKey, ProjectionEntrypointAbiIndex,
    ProjectionInput, ProjectionLoweringFacts, ProjectionSourceDeclarationKind,
    ProjectionSourceFacts, ProjectionSourceFactsParts, ProjectionSourceMetadata,
    ProjectionSourceSymbolKey, ProjectionSyntheticEntrypointExecutable,
    ProjectionSyntheticEntrypointExecutableKind, ProjectionSyntheticEntrypointIndex,
    ProjectionSyntheticEntrypointModule, PublicCallableKindProjection, PublicCallableProjection,
    PublicInstanceInterfaceProjection, PublicInstanceProjection, PublicModuleExportProjection,
    PublicSymbolKindProjection, PublicSymbolProjection, PublicTypeKindProjection,
    PublicTypeProjection, PublicationApiProjectionSeed, ServiceDependencyProjectionFacts,
    ServiceHttpIngressProjection, ServiceHttpRouteIngressProjection,
    ServiceIngressHandlerProjection, ServiceIngressProjection, ServiceWebSocketIngressProjection,
};
use skiff_compiler_source::{
    api::{PublicCallableKind, PublicSymbolKind, PublicTypeKind},
    entity::{
        abi::{abi_alias_id_from_anchor, abi_interface_id_from_anchor, abi_type_id_from_anchor},
        SourceDeclarationKind,
    },
    service_ingress::ServiceHttpIngress,
    ConfigRequirement, ConfigRequirementAccess, ConfigRequirementScope, ConfigRequirementSet,
    ConfigSourceSpan, PublicationApiSeed, ServiceHttpRouteIngress, ServiceIngressHandler,
    ServiceIngressModel, ServiceWebSocketIngress, SourceCompileModel, SourceSymbolKey,
};

use crate::{CompiledPublication, PackagePublication};

pub fn build_projection_input(compiled: &CompiledPublication) -> ProjectionInput {
    build_projection_input_with_package_id(compiled, None)
}

fn build_projection_input_with_package_id(
    compiled: &CompiledPublication,
    package_id: Option<&str>,
) -> ProjectionInput {
    let model = compiled.compile_model();
    let file_ir_units = compiled.file_ir_units().to_vec();
    let source_metadata = compiled
        .source_metadata()
        .iter()
        .map(|source| ProjectionSourceMetadata {
            source_path: source.source_path.clone(),
            module_path: source.module_path.clone(),
            role: source.role,
            source_ast_hash: source.source_ast_hash.clone(),
        })
        .collect::<Vec<_>>();
    let source = ProjectionSourceFacts::new(ProjectionSourceFactsParts {
        publication_api_seed: publication_api_seed_projection(model.publication_api().seed()),
        export_bindings: export_bindings_projection(model, compiled.file_ir_units()),
        config_requirements: config_requirements_seed(model),
        abi_ids: abi_declaration_ids(model, compiled.file_ir_units()),
        service_ingress: model.service_ingress().map(service_ingress_projection),
        service_dependencies: service_dependency_projection_facts(model),
    });
    let lowering = ProjectionLoweringFacts::new(
        entrypoint_abi_index_from_file_ir_units(compiled.file_ir_units()),
        synthetic_entrypoint_index_projection(
            compiled.lowered().synthetic_operations().entrypoints(),
        ),
        compiled.service_db_metadata().to_vec(),
        compiled.service_actor_metadata().to_vec(),
        package_entrypoint_projection_facts(compiled, package_id),
    );
    ProjectionInput::new(file_ir_units, source_metadata, source, lowering)
}

pub fn build_package_projection_input(package: &PackagePublication) -> PackageProjectionInput {
    PackageProjectionInput::new(PackageProjectionInputParts {
        info: package_publication_info_projection(package),
        compiled: build_projection_input_with_package_id(package.compiled(), Some(package.id())),
        dependency_config: package.config().clone(),
    })
}

pub fn build_package_projection_inputs(
    packages: &[PackagePublication],
) -> Vec<PackageProjectionInput> {
    packages
        .iter()
        .map(build_package_projection_input)
        .collect()
}

fn package_publication_info_projection(
    package: &PackagePublication,
) -> PackagePublicationProjectionInfo {
    let manifest = package.manifest();
    PackagePublicationProjectionInfo::new(
        manifest.id().to_string(),
        manifest.version().to_string(),
        manifest
            .dependencies()
            .iter()
            .map(package_dependency_projection)
            .collect(),
        manifest
            .api_entries()
            .iter()
            .map(|entry| {
                PackageApiEntryProjectionInfo::new(
                    entry.path().to_string(),
                    entry.module().to_string(),
                )
            })
            .collect(),
        manifest.api_source().map(|source| {
            PackageApiSourceProjectionInfo::new(
                source.relative_path().to_path_buf(),
                source.content_hash().to_string(),
            )
        }),
        manifest.source_root().to_path_buf(),
        PackagePublicationProjectionProvenance::new(manifest.provenance().synthetic()),
    )
}

fn package_dependency_projection(
    dependency: &crate::PackageDependencyInfo,
) -> PackageDependencyProjectionInfo {
    PackageDependencyProjectionInfo::new(
        dependency.id().to_string(),
        dependency.version().to_string(),
        dependency.alias().map(str::to_string),
        dependency.config().clone(),
        dependency.collection_name_mapping().clone(),
    )
}

fn publication_api_seed_projection(seed: &PublicationApiSeed) -> PublicationApiProjectionSeed {
    PublicationApiProjectionSeed {
        public_modules: seed.public_modules.clone(),
        public_symbols: seed
            .public_symbols
            .iter()
            .map(|(key, value)| (key.clone(), public_symbol_projection(value)))
            .collect(),
        public_callables: seed
            .public_callables
            .iter()
            .map(|(key, value)| (key.clone(), public_callable_projection(value)))
            .collect(),
        public_schema_types: seed
            .public_schema_types
            .iter()
            .map(|(key, value)| (key.clone(), public_type_projection(value)))
            .collect(),
        public_instances: seed
            .public_instances
            .iter()
            .map(|(key, value)| (key.clone(), public_instance_projection(value)))
            .collect(),
        module_exports: seed
            .module_exports
            .iter()
            .map(public_module_export_projection)
            .collect(),
        publication_schema_symbols: seed
            .publication_schema_symbols
            .iter()
            .map(|(key, value)| (source_symbol_key_projection(key), value.clone()))
            .collect(),
        publication_callable_symbols: seed
            .publication_callable_symbols
            .iter()
            .map(source_symbol_key_projection)
            .collect(),
        publication_public_instance_symbols: seed
            .publication_public_instance_symbols
            .iter()
            .map(source_symbol_key_projection)
            .collect(),
    }
}

fn export_bindings_projection(
    model: &SourceCompileModel,
    file_ir_units: &[FileIrUnit],
) -> ExportBindingProjection {
    let bindings = model.export_bindings();
    let file_units_by_module = file_ir_units
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    ExportBindingProjection::new(
        bindings
            .public_symbols()
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    ExportSymbolProjection {
                        public_path: value.public_path.clone(),
                        source_module: value.source_module.clone(),
                        source_symbol: value.source_symbol.clone(),
                        kind: public_symbol_kind_projection(value.kind),
                    },
                )
            })
            .collect(),
        bindings
            .public_callables()
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    ExportCallableProjection {
                        public_path: value.public_path.clone(),
                        source_module: value.source_module.clone(),
                        source_symbol: value.source_symbol.clone(),
                        kind: public_callable_kind_projection(value.kind),
                    },
                )
            })
            .collect(),
        bindings
            .public_schema_types()
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    ExportSchemaProjection {
                        public_path: value.public_path.clone(),
                        source_module: value.source_module.clone(),
                        source_symbol: value.source_symbol.clone(),
                        kind: public_type_kind_projection(value.kind),
                    },
                )
            })
            .collect(),
        bindings
            .public_instances()
            .iter()
            .map(|(key, value)| {
                let receiver = package_public_instance_receiver_symbol_for_adapter(
                    &file_units_by_module,
                    &value.source_module,
                    &value.source_symbol,
                );
                (
                    key.clone(),
                    ExportPublicInstanceProjection {
                        public_path: value.public_path.clone(),
                        source_module: value.source_module.clone(),
                        source_symbol: value.source_symbol.clone(),
                        interfaces: value
                            .interfaces
                            .iter()
                            .map(|interface| {
                                let conformance = receiver.as_ref().and_then(|receiver| {
                                    model.type_resolution().source_interface_conformance(
                                        &SourceSymbolKey::new(
                                            &receiver.module_path,
                                            &receiver.symbol,
                                        ),
                                        &ServiceSymbolRef {
                                            module_path: interface.source_module.clone(),
                                            symbol: interface.source_symbol.clone(),
                                        },
                                    )
                                });
                                let package_interface = receiver.as_ref().and_then(|receiver| {
                                    public_instance_listed_package_interface_for_adapter(
                                        model,
                                        &file_units_by_module,
                                        receiver,
                                        interface,
                                    )
                                });
                                ExportPublicInstanceInterfaceProjection {
                                    source_module: interface.source_module.clone(),
                                    source_symbol: interface.source_symbol.clone(),
                                    implements_interface: conformance.is_some(),
                                    canonical_type_args: conformance
                                        .map(|conformance| conformance.interface_args.to_vec())
                                        .unwrap_or_default(),
                                    package_interface_identity: package_interface
                                        .as_ref()
                                        .map(|fact| fact.0.clone()),
                                    package_interface_methods: package_interface
                                        .as_ref()
                                        .map(|fact| fact.1.clone())
                                        .unwrap_or_default(),
                                    receiver_implements_package_interface: package_interface
                                        .is_some(),
                                }
                            })
                            .collect(),
                    },
                )
            })
            .collect(),
        bindings
            .module_exports()
            .iter()
            .map(public_module_export_projection)
            .collect(),
    )
}

fn public_symbol_projection(
    symbol: &skiff_compiler_source::api::PublicSymbol,
) -> PublicSymbolProjection {
    PublicSymbolProjection {
        public_path: symbol.public_path.clone(),
        source_module: symbol.source_module.clone(),
        source_symbol: symbol.source_symbol.clone(),
        kind: public_symbol_kind_projection(symbol.kind),
    }
}

fn package_public_instance_receiver_symbol_for_adapter(
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    const_module: &str,
    const_symbol: &str,
) -> Option<ServiceSymbolRef> {
    let unit = file_units_by_module.get(const_module).copied()?;
    let const_decl = unit.declarations.constants.get(const_symbol)?;
    let constant = unit.constants.get(const_decl.const_index as usize)?;
    package_nominal_service_symbol_for_adapter(file_units_by_module, const_module, &constant.ty)
}

fn package_nominal_service_symbol_for_adapter(
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
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
            let unit = file_units_by_module
                .get(symbol.module_path.as_str())
                .copied()?;
            unit.declarations.types.get(&symbol.symbol)?;
            Some(symbol.clone())
        }
        _ => None,
    }
}

fn public_instance_listed_package_interface_for_adapter(
    model: &SourceCompileModel,
    file_units_by_module: &BTreeMap<&str, &FileIrUnit>,
    receiver: &ServiceSymbolRef,
    listed_interface: &skiff_compiler_source::ExportPublicInstanceInterfaceBinding,
) -> Option<(
    TypeRefIr,
    Vec<skiff_artifact_model::InterfaceMethodSignature>,
)> {
    let selector_path = format!(
        "{}.{}",
        listed_interface.source_module, listed_interface.source_symbol
    );
    let listed_package_interface = model
        .type_resolution()
        .resolve_package_interface(&selector_path)?;
    let receiver_decl = file_units_by_module
        .get(receiver.module_path.as_str())
        .and_then(|unit| unit.declarations.types.get(&receiver.symbol))
        .and_then(|declaration| {
            file_units_by_module
                .get(receiver.module_path.as_str())?
                .type_table
                .get(declaration.type_index as usize)
        })?;
    let receiver_implements_listed = receiver_decl.implements.iter().any(|implemented_ty| {
        model
            .type_resolution()
            .package_interface_for_type_ref(implemented_ty)
            .is_some_and(|implemented| {
                package_interface_identities_match(
                    &implemented.identity,
                    &listed_package_interface.identity,
                ) || public_instance_package_interface_matches_selector(
                    &implemented.identity,
                    &selector_path,
                )
            })
    }) || {
        let receiver_key = SourceSymbolKey::new(&receiver.module_path, &receiver.symbol);
        model
            .type_resolution()
            .source_interface_conformance_matching(&receiver_key, |implemented_identity| {
                package_interface_identities_match(
                    implemented_identity,
                    &listed_package_interface.identity,
                ) || public_instance_package_interface_matches_selector(
                    implemented_identity,
                    &selector_path,
                )
            })
            .is_some()
    };
    receiver_implements_listed.then_some((
        listed_package_interface.identity,
        listed_package_interface.methods,
    ))
}

fn package_interface_identities_match(left: &TypeRefIr, right: &TypeRefIr) -> bool {
    let (
        TypeRefIr::PackageSymbol {
            symbol: left_symbol,
        },
        TypeRefIr::PackageSymbol {
            symbol: right_symbol,
        },
    ) = (left, right)
    else {
        return false;
    };
    left_symbol.package == right_symbol.package
        && left_symbol.symbol_path == right_symbol.symbol_path
}

fn public_instance_package_interface_matches_selector(
    identity: &TypeRefIr,
    selector: &str,
) -> bool {
    let TypeRefIr::PackageSymbol { symbol } = identity else {
        return false;
    };
    symbol.symbol_path == selector
        || symbol
            .symbol_path
            .strip_prefix("root.")
            .is_some_and(|stripped| stripped == selector)
}

fn public_callable_projection(
    callable: &skiff_compiler_source::api::PublicCallable,
) -> PublicCallableProjection {
    PublicCallableProjection {
        public_path: callable.public_path.clone(),
        source_module: callable.source_module.clone(),
        source_symbol: callable.source_symbol.clone(),
        kind: public_callable_kind_projection(callable.kind),
    }
}

fn public_type_projection(ty: &skiff_compiler_source::api::PublicType) -> PublicTypeProjection {
    PublicTypeProjection {
        public_path: ty.public_path.clone(),
        source_module: ty.source_module.clone(),
        source_symbol: ty.source_symbol.clone(),
        kind: public_type_kind_projection(ty.kind),
    }
}

fn public_instance_projection(
    instance: &skiff_compiler_source::api::PublicInstance,
) -> PublicInstanceProjection {
    PublicInstanceProjection {
        public_path: instance.public_path.clone(),
        source_module: instance.source_module.clone(),
        source_symbol: instance.source_symbol.clone(),
        interfaces: instance
            .interfaces
            .iter()
            .map(|interface| PublicInstanceInterfaceProjection {
                source_module: interface.source_module.clone(),
                source_symbol: interface.source_symbol.clone(),
            })
            .collect(),
    }
}

fn public_module_export_projection(
    export: &skiff_compiler_source::api::PublicModuleExport,
) -> PublicModuleExportProjection {
    PublicModuleExportProjection {
        public_path: export.public_path.clone(),
        source_module: export.source_module.clone(),
    }
}

fn public_symbol_kind_projection(kind: PublicSymbolKind) -> PublicSymbolKindProjection {
    match kind {
        PublicSymbolKind::Type => PublicSymbolKindProjection::Type,
        PublicSymbolKind::Alias => PublicSymbolKindProjection::Alias,
        PublicSymbolKind::Interface => PublicSymbolKindProjection::Interface,
        PublicSymbolKind::Function => PublicSymbolKindProjection::Function,
        PublicSymbolKind::Const => PublicSymbolKindProjection::Const,
    }
}

fn public_callable_kind_projection(kind: PublicCallableKind) -> PublicCallableKindProjection {
    match kind {
        PublicCallableKind::Function => PublicCallableKindProjection::Function,
        PublicCallableKind::Method => PublicCallableKindProjection::Method,
    }
}

fn public_type_kind_projection(kind: PublicTypeKind) -> PublicTypeKindProjection {
    match kind {
        PublicTypeKind::Type => PublicTypeKindProjection::Type,
        PublicTypeKind::Alias => PublicTypeKindProjection::Alias,
        PublicTypeKind::Interface => PublicTypeKindProjection::Interface,
    }
}

fn source_symbol_key_projection(key: &SourceSymbolKey) -> ProjectionSourceSymbolKey {
    ProjectionSourceSymbolKey::new(key.module_path(), key.symbol())
}

fn config_requirements_seed(model: &SourceCompileModel) -> ConfigRequirementsSeed {
    ConfigRequirementsSeed::new(
        config_requirement_set_projection(&model.legacy_config_projection_requirements()),
        config_requirement_set_projection(model.own_config_requirements()),
        config_requirement_set_projection(model.dependency_config_requirements()),
        config_requirement_set_projection(model.effective_config_requirements()),
    )
}

fn config_requirement_set_projection(set: &ConfigRequirementSet) -> ConfigRequirementSetProjection {
    ConfigRequirementSetProjection::new(
        set.requirements()
            .iter()
            .map(config_requirement_projection)
            .collect(),
    )
}

fn config_requirement_projection(requirement: &ConfigRequirement) -> ConfigRequirementProjection {
    ConfigRequirementProjection {
        scope: config_requirement_scope_projection(requirement.scope()),
        path: requirement.path().to_string(),
        access: config_requirement_access_projection(requirement.access()),
        provenances: requirement
            .provenances()
            .iter()
            .map(|provenance| ConfigRequirementProvenanceProjection {
                source_path: provenance.source_path().to_string(),
                source_span: provenance.source_span().map(config_source_span_projection),
                declaring_publication: provenance.declaring_publication().map(|publication| {
                    ConfigRequirementPublicationProjection {
                        id: publication.id().to_string(),
                        version: publication.version().to_string(),
                    }
                }),
                dependency_path: provenance
                    .dependency_path()
                    .iter()
                    .map(|step| ConfigRequirementDependencyStepProjection {
                        id: step.id().to_string(),
                        version: step.version().to_string(),
                        alias: step.alias().map(str::to_string),
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn config_requirement_scope_projection(
    scope: &ConfigRequirementScope,
) -> ConfigRequirementScopeProjection {
    match scope {
        ConfigRequirementScope::Service => ConfigRequirementScopeProjection::Service,
        ConfigRequirementScope::Package { package_id } => {
            ConfigRequirementScopeProjection::Package {
                package_id: package_id.clone(),
            }
        }
    }
}

fn config_requirement_access_projection(
    access: &ConfigRequirementAccess,
) -> ConfigRequirementAccessProjection {
    match access {
        ConfigRequirementAccess::Require { ty } => {
            ConfigRequirementAccessProjection::Require { ty: ty.clone() }
        }
        ConfigRequirementAccess::Optional { ty } => {
            ConfigRequirementAccessProjection::Optional { ty: ty.clone() }
        }
        ConfigRequirementAccess::Has => ConfigRequirementAccessProjection::Has,
    }
}

fn config_source_span_projection(span: ConfigSourceSpan) -> ConfigSourceSpanProjection {
    ConfigSourceSpanProjection {
        start: ConfigSourcePositionProjection {
            line: span.start.line,
            column: span.start.column,
            offset: span.start.offset,
        },
        end: ConfigSourcePositionProjection {
            line: span.end.line,
            column: span.end.column,
            offset: span.end.offset,
        },
    }
}

fn service_ingress_projection(ingress: &ServiceIngressModel) -> ServiceIngressProjection {
    ServiceIngressProjection {
        package_aliases: ingress.package_aliases.clone(),
        http: ingress.http.as_ref().map(service_http_ingress_projection),
        websocket: ingress
            .websocket
            .as_ref()
            .map(service_websocket_ingress_projection),
    }
}

fn service_http_ingress_projection(ingress: &ServiceHttpIngress) -> ServiceHttpIngressProjection {
    ServiceHttpIngressProjection {
        entry_target: ingress.entry_target.clone(),
        guard: ingress
            .guard
            .as_ref()
            .map(service_ingress_handler_projection),
        pre: ingress.pre.as_ref().map(service_ingress_handler_projection),
        routes: ingress
            .routes
            .iter()
            .map(service_http_route_ingress_projection)
            .collect(),
    }
}

fn service_http_route_ingress_projection(
    route: &ServiceHttpRouteIngress,
) -> ServiceHttpRouteIngressProjection {
    ServiceHttpRouteIngressProjection {
        method: route.method.clone(),
        path: route.path.clone(),
        handler: service_ingress_handler_projection(&route.handler),
    }
}

fn service_websocket_ingress_projection(
    ingress: &ServiceWebSocketIngress,
) -> ServiceWebSocketIngressProjection {
    ServiceWebSocketIngressProjection {
        target: ingress.target.clone(),
        connect: ingress
            .connect
            .as_ref()
            .map(service_ingress_handler_projection),
        receive: ingress
            .receive
            .as_ref()
            .map(service_ingress_handler_projection),
    }
}

fn service_ingress_handler_projection(
    handler: &ServiceIngressHandler,
) -> ServiceIngressHandlerProjection {
    match handler {
        ServiceIngressHandler::ServiceFunction {
            source,
            module_path,
            symbol,
        } => ServiceIngressHandlerProjection::ServiceFunction {
            source: source.clone(),
            module_path: module_path.clone(),
            symbol: symbol.clone(),
        },
        ServiceIngressHandler::PackageFunction {
            source,
            package_id,
            alias,
            symbol_path,
        } => ServiceIngressHandlerProjection::PackageFunction {
            source: source.clone(),
            package_id: package_id.clone(),
            alias: alias.clone(),
            symbol_path: symbol_path.clone(),
        },
    }
}

fn service_dependency_projection_facts(
    model: &SourceCompileModel,
) -> ServiceDependencyProjectionFacts {
    let dependencies = model.dependencies().service_dependencies();
    ServiceDependencyProjectionFacts::new(
        dependencies.constraints().to_vec(),
        dependencies
            .dependency_lock()
            .iter()
            .map(|entry| {
                serde_json::to_value(entry).expect("service dependency lock entry should serialize")
            })
            .collect(),
    )
}

fn entrypoint_abi_index_from_file_ir_units(
    file_ir_units: &[FileIrUnit],
) -> ProjectionEntrypointAbiIndex {
    ProjectionEntrypointAbiIndex::new(
        file_ir_units
            .iter()
            .map(|unit| {
                (
                    unit.module_path.clone(),
                    unit.declarations
                        .executables
                        .iter()
                        .filter_map(|(name, declaration)| {
                            let executable = unit
                                .executables
                                .get(declaration.executable_index as usize)?;
                            Some((
                                name.clone(),
                                entry_function_signature_from_executable(unit, name, executable),
                            ))
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

fn synthetic_entrypoint_index_projection(
    entrypoints: &skiff_compiler_lowering::SyntheticEntrypointIndex,
) -> ProjectionSyntheticEntrypointIndex {
    ProjectionSyntheticEntrypointIndex::new(
        entrypoints
            .modules()
            .map(|(module_path, module)| {
                (
                    module_path.to_string(),
                    ProjectionSyntheticEntrypointModule::new(
                        module.types().map(str::to_string).collect(),
                        module
                            .executables()
                            .map(|(name, executable)| {
                                (
                                    name.to_string(),
                                    ProjectionSyntheticEntrypointExecutable::new(
                                        synthetic_entrypoint_executable_kind_projection(
                                            executable.kind(),
                                        ),
                                        entry_function_signature_projection(
                                            executable.signature().clone(),
                                        ),
                                    ),
                                )
                            })
                            .collect(),
                    ),
                )
            })
            .collect(),
    )
}

fn synthetic_entrypoint_executable_kind_projection(
    kind: skiff_compiler_lowering::SyntheticEntrypointExecutableKind,
) -> ProjectionSyntheticEntrypointExecutableKind {
    match kind {
        skiff_compiler_lowering::SyntheticEntrypointExecutableKind::Function => {
            ProjectionSyntheticEntrypointExecutableKind::Function
        }
        skiff_compiler_lowering::SyntheticEntrypointExecutableKind::ImplMethod => {
            ProjectionSyntheticEntrypointExecutableKind::ImplMethod
        }
    }
}

fn entry_function_signature_from_executable(
    unit: &FileIrUnit,
    name: &str,
    executable: &skiff_artifact_model::ExecutableIr,
) -> EntryFunctionSignature {
    let params = if executable.kind == ExecutableKind::ImplMethod
        && executable
            .params
            .first()
            .is_some_and(|param| param.name == "self")
    {
        executable.params.get(1..).unwrap_or(&[])
    } else {
        executable.params.as_slice()
    };
    let local_type_names = file_ir_local_type_names(unit);
    EntryFunctionSignature {
        name: name.to_string(),
        params: params
            .iter()
            .map(|param| EntryParamSpec {
                name: param.name.clone(),
                ty: EntryTypeSpec {
                    name: type_ref_ir_source_text_with_local_types(&param.ty, &|type_index| {
                        local_type_names.get(&type_index).cloned()
                    }),
                    ir: param.ty.clone(),
                    local_type_names: local_type_names.clone(),
                },
            })
            .collect(),
        return_type: EntryTypeSpec {
            name: type_ref_ir_source_text_with_local_types(
                &executable.return_type,
                &|type_index| local_type_names.get(&type_index).cloned(),
            ),
            ir: executable.return_type.clone(),
            local_type_names: local_type_names.clone(),
        },
        local_type_names,
    }
}

fn type_ref_ir_source_text_with_local_types(
    ty: &TypeRefIr,
    local_type_name: &impl Fn(u32) -> Option<String>,
) -> String {
    type_ref_ir_source_text_with_named_types(ty, local_type_name, &|name| name.to_string())
}

fn type_ref_ir_source_text_with_named_types(
    ty: &TypeRefIr,
    local_type_name: &impl Fn(u32) -> Option<String>,
    named_type: &impl Fn(&str) -> String,
) -> String {
    match ty {
        TypeRefIr::Native { name, args } if args.is_empty() => named_type(name),
        TypeRefIr::Native { name, args } => format!(
            "{}<{}>",
            named_type(name),
            args.iter()
                .map(|arg| {
                    type_ref_ir_source_text_with_named_types(arg, local_type_name, named_type)
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::LocalType { type_index } => named_type(
            &local_type_name(*type_index)
                .unwrap_or_else(|| format!("__invalid_local_type_{type_index}")),
        ),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            let name = if symbol.module_path.is_empty() {
                symbol.symbol.clone()
            } else if symbol.module_path.starts_with("std.") {
                symbol.symbol_path()
            } else {
                format!("root.{}", symbol.symbol_path())
            };
            named_type(&name)
        }
        TypeRefIr::PackageSymbol { symbol } => named_type(&symbol.symbol_path),
        TypeRefIr::Record { fields } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(name, ty)| {
                    format!(
                        "{name}: {}",
                        type_ref_ir_source_text_with_named_types(ty, local_type_name, named_type)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::Union { items } => items
            .iter()
            .map(|item| type_ref_ir_source_text_with_named_types(item, local_type_name, named_type))
            .collect::<Vec<_>>()
            .join(" | "),
        TypeRefIr::Nullable { inner } => format!(
            "{}?",
            type_ref_ir_source_text_with_named_types(inner, local_type_name, named_type)
        ),
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null => "null".to_string(),
            LiteralIr::Bool { value } => value.to_string(),
            LiteralIr::Number { value } => value.to_string(),
            LiteralIr::String { value } => {
                serde_json::to_string(value).expect("string literal should serialize")
            }
        },
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::AnyInterface { interface } => {
            any_interface_source_text(interface, local_type_name, named_type)
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => format!(
            "function({}) -> {}",
            params
                .iter()
                .map(|param| {
                    format!(
                        "{}: {}",
                        param.name,
                        type_ref_ir_source_text_with_named_types(
                            &param.ty,
                            local_type_name,
                            named_type
                        )
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
            type_ref_ir_source_text_with_named_types(return_type, local_type_name, named_type)
        ),
    }
}

fn any_interface_source_text(
    interface: &InterfaceInstantiationRef,
    local_type_name: &impl Fn(u32) -> Option<String>,
    named_type: &impl Fn(&str) -> String,
) -> String {
    let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .map_or_else(
            |_| interface.interface_abi_id.clone(),
            |ty| type_ref_ir_source_text_with_named_types(&ty, local_type_name, named_type),
        );
    if interface.canonical_type_args.is_empty() {
        format!("any {interface_name}")
    } else {
        format!(
            "any {interface_name}<{}>",
            interface
                .canonical_type_args
                .iter()
                .map(|arg| {
                    type_ref_ir_source_text_with_named_types(arg, local_type_name, named_type)
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn file_ir_local_type_names(unit: &FileIrUnit) -> BTreeMap<u32, String> {
    unit.type_table
        .iter()
        .enumerate()
        .map(|(index, ty)| (index as u32, ty.name.clone()))
        .collect()
}

fn package_entrypoint_projection_facts(
    compiled: &CompiledPublication,
    package_id: Option<&str>,
) -> PackageEntrypointProjectionFacts {
    let Some(package_id) = package_id else {
        return PackageEntrypointProjectionFacts::default();
    };
    let model = compiled.compile_model();
    let symbol_paths = package_entrypoint_symbol_paths(model, package_id);
    let functions: BTreeMap<_, _> = symbol_paths
        .iter()
        .filter_map(|symbol_path| {
            let result = skiff_compiler_lowering::package_entrypoint_function_signature(
                model,
                compiled.lowered().entrypoint_abi(),
                package_id,
                symbol_path,
            )
            .ok()??;
            Some((
                symbol_path.clone(),
                PackageEntrypointFunctionProjection {
                    source_module: result.0,
                    source_symbol: result.1,
                    signature: entry_function_signature_projection(result.2),
                },
            ))
        })
        .collect();
    let mut modules = model
        .export_bindings()
        .public_schema_types()
        .values()
        .map(|public_type| public_type.source_module.clone())
        .collect::<BTreeSet<_>>();
    modules.extend(
        functions
            .values()
            .map(|function| function.source_module.clone()),
    );
    let schema_type_names_by_module = modules
        .iter()
        .map(|module| {
            (
                module.clone(),
                skiff_compiler_lowering::package_public_schema_type_names_for_module(model, module),
            )
        })
        .collect();
    let schema_abi_types_by_module = modules
        .into_iter()
        .filter_map(|module| {
            skiff_compiler_lowering::package_public_schema_abi_types_for_module(model, &module)
                .ok()
                .map(|types| {
                    (
                        module,
                        types
                            .into_iter()
                            .map(package_abi_type_projection)
                            .collect::<Vec<_>>(),
                    )
                })
        })
        .collect();
    PackageEntrypointProjectionFacts::new(
        functions,
        schema_type_names_by_module,
        schema_abi_types_by_module,
    )
}

fn package_entrypoint_symbol_paths(
    model: &SourceCompileModel,
    package_id: &str,
) -> BTreeSet<String> {
    model
        .export_bindings()
        .public_callables()
        .values()
        .flat_map(|callable| {
            let mut paths = Vec::new();
            paths.push(callable.public_path.clone());
            if let Some((export_path, symbol)) = callable.public_path.rsplit_once('.') {
                let public_path = package_public_path(package_id, export_path);
                paths.push(format!("{public_path}.{symbol}"));
            }
            paths
        })
        .collect()
}

fn package_public_path(package_id: &str, export_path: &str) -> String {
    if export_path.is_empty() {
        package_id.to_string()
    } else if package_id.is_empty() {
        export_path.to_string()
    } else {
        format!("{package_id}.{export_path}")
    }
}

fn entry_function_signature_projection(
    signature: skiff_compiler_lowering::EntryFunctionSignature,
) -> EntryFunctionSignature {
    EntryFunctionSignature {
        name: signature.name,
        params: signature
            .params
            .into_iter()
            .map(|param| EntryParamSpec {
                name: param.name,
                ty: entry_type_spec_projection(param.ty),
            })
            .collect(),
        return_type: entry_type_spec_projection(signature.return_type),
        local_type_names: signature.local_type_names,
    }
}

fn entry_type_spec_projection(spec: skiff_compiler_lowering::EntryTypeSpec) -> EntryTypeSpec {
    EntryTypeSpec {
        name: spec.name,
        ir: spec.ir,
        local_type_names: spec.local_type_names,
    }
}

fn package_abi_type_projection(ty: skiff_compiler_lowering::PackageAbiType) -> PackageAbiType {
    PackageAbiType {
        name: ty.name,
        descriptor: match ty.descriptor {
            skiff_compiler_lowering::PackageAbiTypeDescriptor::Alias { target } => {
                PackageAbiTypeDescriptor::Alias { target }
            }
            skiff_compiler_lowering::PackageAbiTypeDescriptor::Union { variants } => {
                PackageAbiTypeDescriptor::Union { variants }
            }
            skiff_compiler_lowering::PackageAbiTypeDescriptor::Record { fields } => {
                PackageAbiTypeDescriptor::Record { fields }
            }
            skiff_compiler_lowering::PackageAbiTypeDescriptor::External => {
                PackageAbiTypeDescriptor::External
            }
        },
        discriminator: ty.discriminator,
        local_type_names: ty.local_type_names,
    }
}

fn abi_declaration_ids(
    model: &SourceCompileModel,
    file_ir_units: &[FileIrUnit],
) -> BTreeMap<ProjectionDeclarationKey, ProjectionAbiDeclarationIds> {
    let candidates = abi_candidate_keys(model, file_ir_units);
    candidates
        .into_iter()
        .filter_map(|(source_key, projection_kind, source_kind)| {
            let anchor = model
                .declaration_anchors()
                .anchors()
                .iter()
                .find(|anchor| {
                    anchor.matches_source_key(
                        source_key.module_path(),
                        source_key.symbol(),
                        source_kind,
                    )
                })?;
            Some((
                ProjectionDeclarationKey::new(&source_key, projection_kind),
                ProjectionAbiDeclarationIds {
                    type_id: (projection_kind == ProjectionSourceDeclarationKind::Type)
                        .then(|| abi_type_id_from_anchor(anchor, &[])),
                    alias_id: (projection_kind == ProjectionSourceDeclarationKind::Alias)
                        .then(|| abi_alias_id_from_anchor(anchor)),
                    interface_id: (projection_kind == ProjectionSourceDeclarationKind::Interface)
                        .then(|| abi_interface_id_from_anchor(anchor, &[])),
                },
            ))
        })
        .collect()
}

fn abi_candidate_keys(
    model: &SourceCompileModel,
    file_ir_units: &[FileIrUnit],
) -> BTreeSet<(
    ProjectionSourceSymbolKey,
    ProjectionSourceDeclarationKind,
    SourceDeclarationKind,
)> {
    let mut candidates = BTreeSet::new();
    for unit in file_ir_units {
        for name in unit.declarations.types.keys() {
            for (projection, source) in [
                (
                    ProjectionSourceDeclarationKind::Type,
                    SourceDeclarationKind::Type,
                ),
                (
                    ProjectionSourceDeclarationKind::Alias,
                    SourceDeclarationKind::Alias,
                ),
                (
                    ProjectionSourceDeclarationKind::Interface,
                    SourceDeclarationKind::Interface,
                ),
            ] {
                candidates.insert((
                    ProjectionSourceSymbolKey::new(&unit.module_path, name),
                    projection,
                    source,
                ));
            }
        }
        for name in unit.declarations.interfaces.keys() {
            candidates.insert((
                ProjectionSourceSymbolKey::new(&unit.module_path, name),
                ProjectionSourceDeclarationKind::Interface,
                SourceDeclarationKind::Interface,
            ));
        }
    }
    for binding in model.export_bindings().public_schema_types().values() {
        let kind = match binding.kind {
            PublicTypeKind::Type => (
                ProjectionSourceDeclarationKind::Type,
                SourceDeclarationKind::Type,
            ),
            PublicTypeKind::Alias => (
                ProjectionSourceDeclarationKind::Alias,
                SourceDeclarationKind::Alias,
            ),
            PublicTypeKind::Interface => (
                ProjectionSourceDeclarationKind::Interface,
                SourceDeclarationKind::Interface,
            ),
        };
        candidates.insert((
            ProjectionSourceSymbolKey::new(&binding.source_module, &binding.source_symbol),
            kind.0,
            kind.1,
        ));
    }
    candidates
}
