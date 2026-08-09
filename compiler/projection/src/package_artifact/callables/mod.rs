mod implementation_manifests;
mod normalization;
mod signatures;
mod surface;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallableProjection, CallableSemanticFacts, ConstExport, ExecutableExport,
    ExecutableKind, ExecutableSignatureIr, FileIrRef, FileIrUnit, FunctionTypeParamIr,
    InterfaceMethodSignature, OperationCallableKind, OperationTargetRef,
    PackageActorImplementation, PackageCallableId, PackageCallableLinkFact,
    PackageCallableParameter, PackageCallableSignature, PackageExecutableCoordinate,
    PackageImplementationLinks, PackageLocalAbiSymbol, PackageLocalInterfaceConformance,
    PackageRequirement, PackageRuntimeRequirements, PackageTypeRef, ParamModeIr, TypeExport,
};
use skiff_compiler_core::{
    canonical_implementation_callable_source_path, implementation_package_callable_id,
    ImplementationCallableKind,
};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionLocalInterfaceConformanceFacts,
    ProjectionPackageCallableSignatureFacts, ResolvedPackageSchema,
};

use crate::{
    error::ProjectionError,
    package_artifact::{api_exports::PackageExports, export_links::ProjectedPackageExportLinks},
};

use super::{actor::project_actor_abi, boundary::project_boundary_callable_with_package_schemas};

pub(super) struct ProjectedPackageCallableSurface {
    pub public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    pub implementation_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    pub implementation_links: PackageImplementationLinks,
    pub callable_links: BTreeMap<PackageCallableId, PackageCallableLinkFact>,
    pub actor_implementations: Vec<PackageActorImplementation>,
    pub local_interface_conformances: Vec<PackageLocalInterfaceConformance>,
    pub semantic_facts: BTreeMap<PackageCallableId, CallableSemanticFacts>,
    pub boundary_projections: BTreeMap<PackageCallableId, BoundaryCallableProjection>,
}

struct ProjectedImplementationSymbols {
    symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    callables: BTreeMap<PackageExecutableCoordinate, PackageCallableId>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_package_callable_surface(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &ProjectedPackageExportLinks,
    file_ir_units: &[FileIrUnit],
    semantic_facts_by_executable: &BTreeMap<ProjectionExecutableKey, CallableSemanticFacts>,
    local_interface_conformances: &ProjectionLocalInterfaceConformanceFacts,
    package_requirements: &[PackageRequirement],
    signatures: &ProjectionPackageCallableSignatureFacts,
    runtime_requirements: &PackageRuntimeRequirements,
    package_schema_refs: &BTreeMap<(String, String), skiff_artifact_model::ContractTypeRef>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<ProjectedPackageCallableSurface, ProjectionError> {
    let mut local_surface =
        surface::project_local_surface(package_id, api_exports, exports, signatures)?;
    let mut callable_links = BTreeMap::new();
    let mut semantic_facts = BTreeMap::new();
    let mut boundary_projections = BTreeMap::new();
    for mut callable in local_surface.callables {
        normalization::normalize_public_signature(
            &callable.owner_module,
            &mut callable.signature,
            file_ir_units,
            package_schema_refs,
            resolved_package_schemas,
        )
        .map_err(|message| {
            projection_error(
                package_id,
                format!(
                    "public callable {} signature from module {}: {message}",
                    callable.public_path, callable.owner_module
                ),
            )
        })?;
        surface::insert_public_symbol(
            &mut local_surface.public_symbols,
            callable.public_path.clone(),
            PackageLocalAbiSymbol::Callable {
                callable_id: callable.callable_id.clone(),
                signature: callable.signature.clone(),
            },
        )?;
        let executable_key =
            ProjectionExecutableKey::new(callable.owner_module.clone(), callable.executable_index);
        let facts = semantic_facts_by_executable
            .get(&executable_key)
            .cloned()
            .ok_or_else(|| {
                projection_error(
                    package_id,
                    format!(
                        "public callable {} target {}#{} has no typed semantic facts",
                        callable.public_path, callable.owner_module, callable.executable_index
                    ),
                )
            })?;
        let facts = normalization::normalize_semantic_facts(facts);
        let projection = project_boundary_callable_with_package_schemas(
            &callable.owner_module,
            &callable.signature,
            &facts,
            runtime_requirements,
            file_ir_units,
            package_schema_refs,
            resolved_package_schemas,
            Some(callable.executable_index),
        )?;
        insert_callable_entry(
            &mut callable_links,
            callable.callable_id.clone(),
            PackageCallableLinkFact {
                callable_id: callable.callable_id.clone(),
                target: callable.target,
            },
            package_id,
            "callable link",
        )?;
        insert_callable_entry(
            &mut semantic_facts,
            callable.callable_id.clone(),
            facts,
            package_id,
            "callable semantic facts",
        )?;
        insert_callable_entry(
            &mut boundary_projections,
            callable.callable_id,
            projection,
            package_id,
            "boundary projection",
        )?;
    }
    let implementation = project_implementation_symbols(
        package_id,
        file_ir_units,
        semantic_facts_by_executable,
        &mut local_surface.implementation_links,
        &mut callable_links,
        &mut semantic_facts,
    )?;
    let manifests = implementation_manifests::project_implementation_manifests(
        package_id,
        file_ir_units,
        local_interface_conformances,
        package_requirements,
        &implementation.callables,
    )?;
    Ok(ProjectedPackageCallableSurface {
        public_symbols: local_surface.public_symbols,
        implementation_symbols: implementation.symbols,
        implementation_links: local_surface.implementation_links,
        callable_links,
        actor_implementations: manifests.actor_implementations,
        local_interface_conformances: manifests.local_interface_conformances,
        semantic_facts,
        boundary_projections,
    })
}

fn project_implementation_symbols(
    package_id: &str,
    units: &[FileIrUnit],
    facts_by_executable: &BTreeMap<ProjectionExecutableKey, CallableSemanticFacts>,
    implementation_links: &mut PackageImplementationLinks,
    callable_links: &mut BTreeMap<PackageCallableId, PackageCallableLinkFact>,
    semantic_facts: &mut BTreeMap<PackageCallableId, CallableSemanticFacts>,
) -> Result<ProjectedImplementationSymbols, ProjectionError> {
    let mut symbols = BTreeMap::new();
    let mut implementation_callables = BTreeMap::new();
    for unit in units {
        project_implementation_types(package_id, unit, units, &mut symbols, implementation_links)?;
        project_implementation_constants(
            package_id,
            unit,
            units,
            &mut symbols,
            implementation_links,
        )?;
        for declaration in unit.declarations.executables.values() {
            let Some(executable) = unit.executables.get(declaration.executable_index as usize)
            else {
                return Err(projection_error(
                    package_id,
                    format!(
                        "implementation symbol {}.{} targets missing executable #{}",
                        unit.module_path, declaration.symbol, declaration.executable_index
                    ),
                ));
            };
            let (callable_kind, identity_kind, self_type, explicit_parameters) =
                match executable.kind {
                    ExecutableKind::Function => (
                        OperationCallableKind::InternalFunction,
                        ImplementationCallableKind::Function,
                        None,
                        executable.params.as_slice(),
                    ),
                    ExecutableKind::ImplMethod => {
                        let explicit_self = executable
                            .params
                            .first()
                            .filter(|parameter| parameter.name == "self");
                        let self_type = match (executable.self_type.as_ref(), explicit_self) {
                            (Some(_), Some(_)) => {
                                return Err(projection_error(
                                    package_id,
                                    format!(
                                        "implementation method {} declares two receivers",
                                        declaration.symbol
                                    ),
                                ));
                            }
                            (Some(self_type), None) => self_type,
                            (None, Some(self_parameter)) => &self_parameter.ty,
                            (None, None) => {
                                return Err(projection_error(
                                    package_id,
                                    format!(
                                        "implementation method {} has no exact receiver type",
                                        declaration.symbol
                                    ),
                                ));
                            }
                        };
                        let explicit_parameters =
                            &executable.params[usize::from(explicit_self.is_some())..];
                        if explicit_parameters
                            .iter()
                            .any(|parameter| parameter.name == "self")
                        {
                            return Err(projection_error(
                                package_id,
                                format!(
                                    "implementation method {} has a non-leading receiver",
                                    declaration.symbol
                                ),
                            ));
                        }
                        (
                            OperationCallableKind::ImplMethod,
                            ImplementationCallableKind::ImplMethod,
                            Some(self_type),
                            explicit_parameters,
                        )
                    }
                };
            let (source_path, callable_id) = project_implementation_callable_identity(
                package_id,
                &unit.module_path,
                &declaration.symbol,
                identity_kind,
            )?;
            let mut parameters = Vec::new();
            if let Some(self_type) = self_type {
                parameters.push(PackageCallableParameter {
                    name: "self".to_string(),
                    ty: PackageTypeRef::Local {
                        local_type: normalization::normalize_implementation_type(
                            package_id,
                            &unit.module_path,
                            self_type,
                            units,
                        )
                        .map_err(|message| {
                            projection_error(
                                package_id,
                                format!(
                                    "implementation callable {source_path} receiver: {message}"
                                ),
                            )
                        })?,
                    },
                    mode: ParamModeIr::Value,
                });
            }
            parameters.extend(
                explicit_parameters
                    .iter()
                    .map(|parameter| {
                        Ok(PackageCallableParameter {
                            name: parameter.name.clone(),
                            ty: PackageTypeRef::Local {
                                local_type: normalization::normalize_implementation_type(
                                    package_id,
                                    &unit.module_path,
                                    &parameter.ty,
                                    units,
                                )
                                .map_err(|message| {
                                    projection_error(
                                        package_id,
                                        format!(
                                            "implementation callable {source_path} parameter {}: {message}",
                                            parameter.name
                                        ),
                                    )
                                })?,
                            },
                            mode: parameter.mode,
                        })
                    })
                    .collect::<Result<Vec<_>, ProjectionError>>()?,
            );
            let signature = PackageCallableSignature {
                type_params: executable.type_params.clone(),
                parameters,
                return_type: PackageTypeRef::Local {
                    local_type: normalization::normalize_implementation_type(
                        package_id,
                        &unit.module_path,
                        &executable.return_type,
                        units,
                    )
                    .map_err(|message| {
                        projection_error(
                            package_id,
                            format!("implementation callable {source_path} return type: {message}"),
                        )
                    })?,
                },
                may_suspend: executable.may_suspend,
            };
            if callable_kind == OperationCallableKind::ImplMethod {
                let implementation_export = ExecutableExport {
                    file: FileIrRef {
                        file_ir_identity: unit.file_ir_identity.clone(),
                        module_path: unit.module_path.clone(),
                        artifact_path: None,
                        source_ast_hash: Some(unit.source_ast_hash.clone()),
                    },
                    executable_index: declaration.executable_index,
                    symbol: declaration.symbol.clone(),
                    signature: ExecutableSignatureIr {
                        params: executable.params.clone(),
                        return_type: executable.return_type.clone(),
                        self_type: executable.self_type.clone(),
                        may_suspend: executable.may_suspend,
                    },
                };
                match implementation_links.impl_methods.get(&source_path) {
                    Some(existing) if existing != &implementation_export => {
                        return Err(projection_error(
                            package_id,
                            format!(
                                "implementation method source path {source_path} has conflicting execution links"
                            ),
                        ));
                    }
                    Some(_) => {}
                    None if implementation_links.impl_methods.values().any(|existing| {
                        existing.file == implementation_export.file
                            && existing.executable_index == implementation_export.executable_index
                    }) => {}
                    None => {
                        implementation_links
                            .impl_methods
                            .insert(source_path.clone(), implementation_export);
                    }
                }
            }
            if symbols
                .insert(
                    source_path.clone(),
                    PackageLocalAbiSymbol::Callable {
                        callable_id: callable_id.clone(),
                        signature,
                    },
                )
                .is_some()
            {
                return Err(projection_error(
                    package_id,
                    format!("duplicate implementation source path {source_path}"),
                ));
            }
            let target = OperationTargetRef {
                file_ref: FileIrRef {
                    file_ir_identity: unit.file_ir_identity.clone(),
                    module_path: unit.module_path.clone(),
                    artifact_path: None,
                    source_ast_hash: Some(unit.source_ast_hash.clone()),
                },
                executable_index: declaration.executable_index,
                callable_abi_id: callable_id.to_string(),
                callable_kind,
            };
            let coordinate = PackageExecutableCoordinate {
                file_ir_identity: unit.file_ir_identity.clone(),
                module_path: unit.module_path.clone(),
                executable_index: declaration.executable_index,
            };
            if implementation_callables
                .insert(coordinate.clone(), callable_id.clone())
                .is_some()
            {
                return Err(projection_error(
                    package_id,
                    format!(
                        "implementation executable coordinate {coordinate:?} has more than one canonical callable"
                    ),
                ));
            }
            insert_callable_entry(
                callable_links,
                callable_id.clone(),
                PackageCallableLinkFact {
                    callable_id: callable_id.clone(),
                    target,
                },
                package_id,
                "implementation callable link",
            )?;
            let key = ProjectionExecutableKey::new(
                unit.module_path.clone(),
                declaration.executable_index,
            );
            let facts = facts_by_executable.get(&key).cloned().ok_or_else(|| {
                projection_error(
                    package_id,
                    format!("implementation callable {source_path} has no semantic facts"),
                )
            })?;
            insert_callable_entry(
                semantic_facts,
                callable_id,
                normalization::normalize_semantic_facts(facts),
                package_id,
                "implementation callable semantic facts",
            )?;
        }
    }
    Ok(ProjectedImplementationSymbols {
        symbols,
        callables: implementation_callables,
    })
}

fn project_implementation_types(
    package_id: &str,
    unit: &FileIrUnit,
    units: &[FileIrUnit],
    symbols: &mut BTreeMap<String, PackageLocalAbiSymbol>,
    links: &mut PackageImplementationLinks,
) -> Result<(), ProjectionError> {
    for (name, declaration) in &unit.declarations.types {
        let ty = unit
            .type_table
            .get(declaration.type_index as usize)
            .ok_or_else(|| {
                projection_error(
                    package_id,
                    format!(
                        "implementation type {}.{} targets missing type #{}",
                        unit.module_path, name, declaration.type_index
                    ),
                )
            })?;
        let source_path = format!("{}.{}", unit.module_path, name);
        let actor = unit
            .actor_declarations
            .iter()
            .find(|declaration| declaration.abi.actor_name == *name)
            .map(|declaration| {
                project_actor_abi(declaration, |ty| {
                    normalization::normalize_implementation_type(
                        package_id,
                        &unit.module_path,
                        ty,
                        units,
                    )
                    .map_err(|message| {
                        projection_error(
                            package_id,
                            format!("implementation actor {source_path} type: {message}"),
                        )
                    })
                })
            })
            .transpose()?;
        let descriptor = normalization::normalize_implementation_descriptor(
            package_id,
            &unit.module_path,
            &ty.descriptor,
            units,
        )
        .map_err(|message| {
            projection_error(
                package_id,
                format!("implementation type {source_path}: {message}"),
            )
        })?;
        let interface = unit.declarations.interfaces.get(name);
        let interface_methods = interface
            .map(|interface| {
                interface
                    .operations
                    .iter()
                    .map(|method| {
                        Ok(InterfaceMethodSignature {
                            name: method.name.clone(),
                            type_params: method.type_params.clone(),
                            params: method
                                .params
                                .iter()
                                .map(|parameter| {
                                    Ok(FunctionTypeParamIr {
                                        name: parameter.name.clone(),
                                        ty: normalization::normalize_implementation_type(
                                            package_id,
                                            &unit.module_path,
                                            &parameter.ty,
                                            units,
                                        )
                                        .map_err(|message| {
                                            projection_error(
                                                package_id,
                                                format!(
                                                    "implementation interface {source_path} method {} parameter {}: {message}",
                                                    method.name, parameter.name
                                                ),
                                            )
                                        })?,
                                    })
                                })
                                .collect::<Result<Vec<_>, ProjectionError>>()?,
                            return_type: normalization::normalize_implementation_type(
                                package_id,
                                &unit.module_path,
                                &method.return_type,
                                units,
                            )
                            .map_err(|message| {
                                projection_error(
                                    package_id,
                                    format!(
                                        "implementation interface {source_path} method {} return type: {message}",
                                        method.name
                                    ),
                                )
                            })?,
                            is_native: method.is_native,
                            is_provider: method.is_provider,
                            is_static: method.is_static,
                            implicit_self: method
                                .implicit_self
                                .as_ref()
                                .map(|ty| {
                                    normalization::normalize_implementation_type(
                                        package_id,
                                        &unit.module_path,
                                        ty,
                                        units,
                                    )
                                    .map_err(|message| {
                                        projection_error(
                                            package_id,
                                            format!(
                                                "implementation interface {source_path} method {} receiver: {message}",
                                                method.name
                                            ),
                                        )
                                    })
                                })
                                .transpose()?,
                        })
                    })
                    .collect::<Result<Vec<_>, ProjectionError>>()
            })
            .transpose()?
            .unwrap_or_default();
        insert_implementation_symbol(
            symbols,
            source_path.clone(),
            PackageLocalAbiSymbol::Type {
                local_type_id: format!("type:{package_id}:top-level:{source_path}"),
                descriptor: descriptor.clone(),
                is_alias: ty.source_span.as_ref().is_some_and(|source_span| {
                    unit.source_map.spans.iter().any(|span| {
                        span.kind == "alias"
                            && span.name.as_deref() == Some(ty.name.as_str())
                            && span.span == *source_span
                    })
                }),
                is_interface: interface.is_some(),
                type_params: ty.type_params.clone(),
                interface_methods: interface_methods.clone(),
                actor: actor.clone(),
            },
            package_id,
        )?;
        let link = TypeExport {
            file: implementation_file_ref(unit),
            type_index: declaration.type_index,
            symbol: declaration.symbol.clone(),
            is_interface: interface.is_some(),
            descriptor: Some(descriptor),
            type_params: ty.type_params.clone(),
            interface_methods,
            actor,
        };
        if let Some(existing) = links.types.get(&source_path) {
            if existing.file != link.file || existing.type_index != link.type_index {
                return Err(projection_error(
                    package_id,
                    format!("implementation type link {source_path} conflicts with a public link"),
                ));
            }
        } else {
            links.types.insert(source_path, link);
        }
    }
    Ok(())
}

fn project_implementation_constants(
    package_id: &str,
    unit: &FileIrUnit,
    units: &[FileIrUnit],
    symbols: &mut BTreeMap<String, PackageLocalAbiSymbol>,
    links: &mut PackageImplementationLinks,
) -> Result<(), ProjectionError> {
    for (name, declaration) in &unit.declarations.constants {
        let source_path = format!("{}.{}", unit.module_path, name);
        let ty = normalization::normalize_implementation_type(
            package_id,
            &unit.module_path,
            &declaration.ty,
            units,
        )
        .map_err(|message| {
            projection_error(
                package_id,
                format!("implementation constant {source_path}: {message}"),
            )
        })?;
        insert_implementation_symbol(
            symbols,
            source_path.clone(),
            PackageLocalAbiSymbol::Constant {
                const_id: format!("pkg-const:{package_id}:top-level:{source_path}"),
                ty: PackageTypeRef::Local {
                    local_type: ty.clone(),
                },
            },
            package_id,
        )?;
        let link = ConstExport {
            file: implementation_file_ref(unit),
            const_index: declaration.const_index,
            symbol: declaration.symbol.clone(),
            ty,
        };
        if let Some(existing) = links.constants.get(&source_path) {
            if existing.file != link.file || existing.const_index != link.const_index {
                return Err(projection_error(
                    package_id,
                    format!(
                        "implementation constant link {source_path} conflicts with a public link"
                    ),
                ));
            }
        } else {
            links.constants.insert(source_path, link);
        }
    }
    Ok(())
}

fn insert_implementation_symbol(
    symbols: &mut BTreeMap<String, PackageLocalAbiSymbol>,
    source_path: String,
    symbol: PackageLocalAbiSymbol,
    package_id: &str,
) -> Result<(), ProjectionError> {
    if symbols.insert(source_path.clone(), symbol).is_some() {
        return Err(projection_error(
            package_id,
            format!("duplicate implementation source path {source_path}"),
        ));
    }
    Ok(())
}

fn implementation_file_ref(unit: &FileIrUnit) -> FileIrRef {
    FileIrRef {
        file_ir_identity: unit.file_ir_identity.clone(),
        module_path: unit.module_path.clone(),
        artifact_path: None,
        source_ast_hash: Some(unit.source_ast_hash.clone()),
    }
}

fn insert_callable_entry<T>(
    map: &mut BTreeMap<PackageCallableId, T>,
    callable_id: PackageCallableId,
    value: T,
    package_id: &str,
    label: &str,
) -> Result<(), ProjectionError> {
    if map.insert(callable_id.clone(), value).is_some() {
        return Err(projection_error(
            package_id,
            format!("duplicate {label} id {callable_id}"),
        ));
    }
    Ok(())
}

fn project_implementation_callable_identity(
    package_id: &str,
    module_path: &str,
    executable_symbol: &str,
    kind: ImplementationCallableKind,
) -> Result<(String, PackageCallableId), ProjectionError> {
    let source_path =
        canonical_implementation_callable_source_path(module_path, executable_symbol, kind)
            .map_err(|error| projection_error(package_id, error.to_string()))?;
    let callable_id =
        implementation_package_callable_id(package_id, module_path, executable_symbol, kind)
            .map_err(|error| projection_error(package_id, error.to_string()))?;
    Ok((source_path, callable_id))
}

pub(super) fn projection_error(package_id: &str, message: impl Into<String>) -> ProjectionError {
    ProjectionError::InvalidPackageArtifact {
        message: format!(
            "package {package_id} artifact projection: {}",
            message.into()
        ),
    }
}
