mod normalization;
mod signatures;
mod surface;

use std::collections::BTreeMap;

use skiff_artifact_model::{
    BoundaryCallableProjection, CallableSemanticFacts, ConstExport, ExecutableExport,
    ExecutableKind, ExecutableSignatureIr, FileIrRef, FileIrUnit, FunctionTypeParamIr,
    InterfaceMethodSignature, OperationCallableKind, OperationTargetRef, PackageCallableId,
    PackageCallableLinkFact, PackageCallableParameter, PackageCallableSignature,
    PackageImplementationLinks, PackageLocalAbiSymbol, PackageRuntimeRequirements, PackageTypeRef,
    TypeExport,
};
use skiff_compiler_projection_input::{
    ProjectionExecutableKey, ProjectionPackageCallableSignatureFacts, ResolvedPackageSchema,
};

use crate::{
    error::ProjectionError,
    package_artifact::{api_exports::PackageExports, export_links::ProjectedPackageExportLinks},
};

use super::boundary::project_boundary_callable_with_package_schemas;

pub(super) struct ProjectedPackageCallableSurface {
    pub public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    pub implementation_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    pub implementation_links: PackageImplementationLinks,
    pub callable_links: BTreeMap<PackageCallableId, PackageCallableLinkFact>,
    pub semantic_facts: BTreeMap<PackageCallableId, CallableSemanticFacts>,
    pub boundary_projections: BTreeMap<PackageCallableId, BoundaryCallableProjection>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn project_package_callable_surface(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &ProjectedPackageExportLinks,
    file_ir_units: &[FileIrUnit],
    semantic_facts_by_executable: &BTreeMap<ProjectionExecutableKey, CallableSemanticFacts>,
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
    let implementation_symbols = project_implementation_symbols(
        package_id,
        file_ir_units,
        semantic_facts_by_executable,
        &mut local_surface.implementation_links,
        &mut callable_links,
        &mut semantic_facts,
    )?;
    Ok(ProjectedPackageCallableSurface {
        public_symbols: local_surface.public_symbols,
        implementation_symbols,
        implementation_links: local_surface.implementation_links,
        callable_links,
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
) -> Result<BTreeMap<String, PackageLocalAbiSymbol>, ProjectionError> {
    let mut symbols = BTreeMap::new();
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
            let (callable_kind, self_type) = match executable.kind {
                ExecutableKind::Function => (OperationCallableKind::InternalFunction, None),
                ExecutableKind::ImplMethod => {
                    let self_type = executable.self_type.as_ref().ok_or_else(|| {
                        projection_error(
                            package_id,
                            format!(
                                "implementation method {} has no exact receiver type",
                                declaration.symbol
                            ),
                        )
                    })?;
                    (OperationCallableKind::ImplMethod, Some(self_type))
                }
            };
            let top_level_name = declaration
                .symbol
                .strip_prefix(&format!("{}.", unit.module_path))
                .unwrap_or(&declaration.symbol);
            let source_path = format!("{}.{}", unit.module_path, top_level_name);
            let callable_id = PackageCallableId::new(format!(
                "pkg-callable:{package_id}:top-level:{source_path}"
            ));
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
                });
            }
            parameters.extend(
                executable
                    .params
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
                            format!(
                                "implementation callable {source_path} return type: {message}"
                            ),
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
                    None if implementation_links
                        .impl_methods
                        .values()
                        .any(|existing| {
                            existing.file == implementation_export.file
                                && existing.executable_index
                                    == implementation_export.executable_index
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
    Ok(symbols)
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

pub(super) fn projection_error(package_id: &str, message: impl Into<String>) -> ProjectionError {
    ProjectionError::InvalidPackageArtifact {
        message: format!(
            "package {package_id} artifact projection: {}",
            message.into()
        ),
    }
}
