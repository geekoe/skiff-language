use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::type_ref_abi_key;
use skiff_artifact_model::{
    CallableSemanticFacts, CallableTargetFact, ExecutableKind, FileIrUnit, FunctionTypeParamIr,
    InterfaceInstantiationRef, LiteralIr, ServiceSymbolRef, TypeRefIr,
};
use skiff_compiler_projection_input::{
    ConfigRequirementAccessProjection, ConfigRequirementDependencyStepProjection,
    ConfigRequirementProjection, ConfigRequirementProvenanceProjection,
    ConfigRequirementPublicationProjection, ConfigRequirementScopeProjection,
    ConfigRequirementSetProjection, ConfigRequirementsSeed, ConfigSourcePositionProjection,
    ConfigSourceSpanProjection, EntryFunctionSignature, EntryParamSpec, EntryTypeSpec,
    ExportBindingProjection, ExportCallableProjection, ExportPublicInstanceInterfaceProjection,
    ExportPublicInstanceMethodProjection, ExportPublicInstanceProjection, ExportSchemaProjection,
    ExportSymbolProjection, PackageEntrypointProjectionFacts, ProjectionAbiDeclarationIds,
    ProjectionCallableEffectFacts, ProjectionDeclarationKey, ProjectionEntrypointAbiIndex,
    ProjectionExecutableKey, ProjectionInput, ProjectionLoweringFacts,
    ProjectionSourceDeclarationKind, ProjectionSourceFacts, ProjectionSourceFactsParts,
    ProjectionSourceMetadata, ProjectionSourceSymbolKey, ProjectionSyntheticEntrypointExecutable,
    ProjectionSyntheticEntrypointExecutableKind, ProjectionSyntheticEntrypointIndex,
    ProjectionSyntheticEntrypointModule, PublicCallableKindProjection, PublicCallableProjection,
    PublicInstanceInterfaceProjection, PublicInstanceProjection, PublicModuleExportProjection,
    PublicSymbolKindProjection, PublicSymbolProjection, PublicTypeKindProjection,
    PublicTypeProjection, PublicationApiProjectionSeed,
};
use skiff_compiler_source::{
    api::{PublicCallableKind, PublicSymbolKind, PublicTypeKind},
    entity::{
        abi::{abi_alias_id_from_anchor, abi_interface_id_from_anchor, abi_type_id_from_anchor},
        SourceDeclarationKind,
    },
    ConfigRequirement, ConfigRequirementAccess, ConfigRequirementScope, ConfigRequirementSet,
    ConfigSourceSpan, ExpressionOwnerKey, PackageSourceModel, PublicationApiSeed,
    ResolvedCallTarget, SourceInterfaceConformanceKey, SourceSymbolKey,
};

use crate::{package_callable_signatures, CompiledPackage, ProjectionInputBuildError};

pub fn build_projection_input(
    compiled: &CompiledPackage,
) -> Result<ProjectionInput, ProjectionInputBuildError> {
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
    let export_bindings = export_bindings_projection(model, compiled.file_ir_units())?;
    let source = ProjectionSourceFacts::new(ProjectionSourceFactsParts {
        publication_api_seed: publication_api_seed_projection(model.publication_api().seed()),
        export_bindings,
        config_requirements: config_requirements_seed(model),
        abi_ids: abi_declaration_ids(model, compiled.file_ir_units()),
        callable_effects: callable_effect_facts(model, compiled.file_ir_units()),
        callable_semantic_facts: callable_semantic_facts(model, compiled.file_ir_units()),
    });
    let lowering = ProjectionLoweringFacts::new(
        entrypoint_abi_index_from_file_ir_units(compiled.file_ir_units()),
        synthetic_entrypoint_index_projection(
            compiled.lowered().synthetic_operations().entrypoints(),
        ),
        compiled.service_db_metadata().to_vec(),
        compiled.service_actor_metadata().to_vec(),
        PackageEntrypointProjectionFacts::default(),
    );
    let callable_signatures = package_callable_signatures::build_package_callable_signatures(
        model,
        compiled.file_ir_units(),
        model.policy().package_id(),
    )?;
    Ok(ProjectionInput::new(
        file_ir_units,
        source_metadata,
        source,
        lowering,
        callable_signatures,
    ))
}

fn callable_semantic_facts(
    model: &PackageSourceModel,
    file_ir_units: &[FileIrUnit],
) -> BTreeMap<ProjectionExecutableKey, CallableSemanticFacts> {
    let units_by_module = file_ir_units
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    model
        .callable_effects()
        .operations()
        .iter()
        .map(|(source_key, effects)| {
            let unit = units_by_module[source_key.module_path()];
            let declaration = &unit.declarations.executables[source_key.symbol()];
            let provenance = model
                .callable_provenance()
                .operations()
                .get(source_key)
                .cloned()
                .expect("callable provenance must share the effect owner set");
            let resolved_call_targets = model
                .resolved_call_targets()
                .iter()
                .filter(|(expression, _)| expression_matches_source_owner(expression, source_key))
                .filter_map(|(expression, target)| {
                    callable_target_fact(target).map(|target| (expression.preorder_index(), target))
                })
                .collect();
            (
                ProjectionExecutableKey::new(
                    source_key.module_path(),
                    declaration.executable_index,
                ),
                CallableSemanticFacts {
                    effects: effects.clone(),
                    provenance,
                    resolved_call_targets,
                },
            )
        })
        .collect()
}

fn expression_matches_source_owner(
    expression: &skiff_compiler_source::ExpressionKey,
    source_key: &SourceSymbolKey,
) -> bool {
    if expression.module_path() != source_key.module_path() {
        return false;
    }
    match expression.owner() {
        ExpressionOwnerKey::Function(function) => function == source_key.symbol(),
        ExpressionOwnerKey::ImplMethod { type_name, method } => {
            skiff_compiler_source::semantic::impl_method_declaration_name(type_name, method)
                == source_key.symbol()
        }
        ExpressionOwnerKey::Const(_)
        | ExpressionOwnerKey::Test(_)
        | ExpressionOwnerKey::DbIndexWhere { .. } => false,
    }
}

fn callable_target_fact(target: &ResolvedCallTarget) -> Option<CallableTargetFact> {
    match target {
        ResolvedCallTarget::DependencyPackageFunction {
            package_callable_id,
            ..
        } => Some(CallableTargetFact::PackageDirect {
            package_callable_id: package_callable_id.to_string(),
        }),
        ResolvedCallTarget::ContractOperation {
            contract_operation_id,
            ..
        } => Some(CallableTargetFact::ContractOperation {
            operation_id: contract_operation_id.clone(),
        }),
        ResolvedCallTarget::Unknown { .. } => Some(CallableTargetFact::Unknown),
        ResolvedCallTarget::LocalFunction { .. }
        | ResolvedCallTarget::LocalImplMethod { .. }
        | ResolvedCallTarget::ActorMethod { .. }
        | ResolvedCallTarget::NativeFunction { .. }
        | ResolvedCallTarget::ReceiverBuiltin { .. } => None,
    }
}

fn callable_effect_facts(
    model: &PackageSourceModel,
    file_ir_units: &[FileIrUnit],
) -> ProjectionCallableEffectFacts {
    let units_by_module = file_ir_units
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    let operations = model
        .callable_effects()
        .operations()
        .iter()
        .map(|(source_key, summary)| {
            let unit = units_by_module
                .get(source_key.module_path())
                .unwrap_or_else(|| {
                    panic!(
                        "callable effect source module {} is missing from File IR",
                        source_key.module_path()
                    )
                });
            let declaration = unit
                .declarations
                .executables
                .get(source_key.symbol())
                .unwrap_or_else(|| {
                    panic!(
                        "callable effect source {}.{} is missing from File IR declarations",
                        source_key.module_path(),
                        source_key.symbol()
                    )
                });
            (
                ProjectionExecutableKey::new(
                    source_key.module_path(),
                    declaration.executable_index,
                ),
                summary.clone(),
            )
        })
        .collect();
    ProjectionCallableEffectFacts::new(operations)
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
    model: &PackageSourceModel,
    file_ir_units: &[FileIrUnit],
) -> Result<ExportBindingProjection, ProjectionInputBuildError> {
    let bindings = model.export_bindings();
    let file_units_by_module = file_ir_units
        .iter()
        .map(|unit| (unit.module_path.as_str(), unit))
        .collect::<BTreeMap<_, _>>();
    Ok(ExportBindingProjection::new(
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
                let receiver = package_callable_signatures::resolve_public_instance_receiver_symbol(
                    &file_units_by_module,
                    &value.source_module,
                    &value.source_symbol,
                )
                .ok_or_else(|| ProjectionInputBuildError::MissingPublicInstanceReceiver {
                    public_path: value.public_path.clone(),
                    source_module: value.source_module.clone(),
                    source_symbol: value.source_symbol.clone(),
                })?;
                let interfaces = value
                    .interfaces
                    .iter()
                    .map(|interface| {
                        let conformance_key = SourceInterfaceConformanceKey {
                            receiver: SourceSymbolKey::new(
                                &receiver.module_path,
                                &receiver.symbol,
                            ),
                            interface: SourceSymbolKey::new(
                                &interface.source_module,
                                &interface.source_symbol,
                            ),
                        };
                        let conformance = model
                            .interface_signatures()
                            .conformance(&conformance_key)
                            .ok_or_else(|| {
                                ProjectionInputBuildError::MissingValidatedPublicInstanceConformance {
                                    public_path: value.public_path.clone(),
                                    receiver_module: receiver.module_path.clone(),
                                    receiver_symbol: receiver.symbol.clone(),
                                    interface_module: interface.source_module.clone(),
                                    interface_symbol: interface.source_symbol.clone(),
                                }
                            })?;
                        Ok(ExportPublicInstanceInterfaceProjection {
                            interface: source_symbol_key_projection(&conformance.key.interface),
                            methods: conformance
                                .methods
                                .iter()
                                .map(|(method, validated)| {
                                    ExportPublicInstanceMethodProjection {
                                        method: method.clone(),
                                        executable: source_symbol_key_projection(
                                            &validated.executable,
                                        ),
                                    }
                                })
                                .collect(),
                        })
                    })
                    .collect::<Result<Vec<_>, ProjectionInputBuildError>>()?;
                Ok((
                    key.clone(),
                    ExportPublicInstanceProjection {
                        public_path: value.public_path.clone(),
                        source_module: value.source_module.clone(),
                        source_symbol: value.source_symbol.clone(),
                        receiver: ProjectionSourceSymbolKey::new(
                            receiver.module_path,
                            receiver.symbol,
                        ),
                        interfaces,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, ProjectionInputBuildError>>()?,
        bindings
            .module_exports()
            .iter()
            .map(public_module_export_projection)
            .collect(),
    ))
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

fn config_requirements_seed(model: &PackageSourceModel) -> ConfigRequirementsSeed {
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

fn entrypoint_abi_index_from_file_ir_units(
    file_ir_units: &[FileIrUnit],
) -> ProjectionEntrypointAbiIndex {
    let publication_type_names = file_ir_publication_type_names(file_ir_units);
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
                                entry_function_signature_from_executable(
                                    unit,
                                    name,
                                    executable,
                                    &publication_type_names,
                                ),
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
                                            executable.may_suspend(),
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
    publication_type_names: &BTreeMap<(String, u32), String>,
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
            .map(|param| {
                let ir = projection_visible_type_ref(&param.ty, publication_type_names);
                EntryParamSpec {
                    name: param.name.clone(),
                    ty: EntryTypeSpec {
                        name: type_ref_ir_source_text_with_local_types(&ir, &|type_index| {
                            local_type_names.get(&type_index).cloned()
                        }),
                        ir,
                        local_type_names: local_type_names.clone(),
                    },
                }
            })
            .collect(),
        return_type: {
            let ir = projection_visible_type_ref(&executable.return_type, publication_type_names);
            EntryTypeSpec {
                name: type_ref_ir_source_text_with_local_types(&ir, &|type_index| {
                    local_type_names.get(&type_index).cloned()
                }),
                ir,
                local_type_names: local_type_names.clone(),
            }
        },
        local_type_names,
        may_suspend: executable.may_suspend,
    }
}

fn file_ir_publication_type_names(file_ir_units: &[FileIrUnit]) -> BTreeMap<(String, u32), String> {
    file_ir_units
        .iter()
        .flat_map(|unit| {
            unit.type_table
                .iter()
                .enumerate()
                .map(|(index, ty)| ((unit.module_path.clone(), index as u32), ty.name.clone()))
        })
        .collect()
}

fn projection_visible_type_ref(
    ty: &TypeRefIr,
    publication_type_names: &BTreeMap<(String, u32), String>,
) -> TypeRefIr {
    match ty {
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => publication_type_names
            .get(&(module_path.clone(), *type_index))
            .map(|symbol| TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: module_path.clone(),
                    symbol: symbol.clone(),
                },
            })
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| projection_visible_type_ref(arg, publication_type_names))
                .collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        projection_visible_type_ref(ty, publication_type_names),
                    )
                })
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| projection_visible_type_ref(item, publication_type_names))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(projection_visible_type_ref(inner, publication_type_names)),
        },
        TypeRefIr::AnyInterface { interface } => {
            let interface_abi_id = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map(|identity| {
                    type_ref_abi_key(&projection_visible_type_ref(
                        &identity,
                        publication_type_names,
                    ))
                })
                .unwrap_or_else(|_| interface.interface_abi_id.clone());
            TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id,
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| projection_visible_type_ref(arg, publication_type_names))
                        .collect(),
                },
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: projection_visible_type_ref(&param.ty, publication_type_names),
                })
                .collect(),
            return_type: Box::new(projection_visible_type_ref(
                return_type,
                publication_type_names,
            )),
        },
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
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
        TypeRefIr::Builtin { name, args } if args.is_empty() => named_type(name),
        TypeRefIr::Builtin { name, args } => format!(
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
        TypeRefIr::PublicationType { module_path, .. } => {
            named_type(&format!("root.{module_path}"))
        }
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

fn entry_function_signature_projection(
    signature: skiff_compiler_lowering::EntryFunctionSignature,
    may_suspend: bool,
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
        may_suspend,
    }
}

fn entry_type_spec_projection(spec: skiff_compiler_lowering::EntryTypeSpec) -> EntryTypeSpec {
    EntryTypeSpec {
        name: spec.name,
        ir: spec.ir,
        local_type_names: spec.local_type_names,
    }
}

fn abi_declaration_ids(
    model: &PackageSourceModel,
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
    model: &PackageSourceModel,
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
