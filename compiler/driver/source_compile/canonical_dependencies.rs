use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    package_artifact_ref, type_ref_abi_key, validate_package_artifact_identities,
};
use skiff_artifact_model::{
    InterfaceInstantiationRef, NominalTypeRefBaseIr, PackageArtifact, PackageCallableSignature,
    PackageLocalAbiSymbol, PackageRefIr, PackageTypeRef, TypeRefIr,
};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_input::ResolvedContractDependency;
use skiff_compiler_projection_input::ResolvedPackageSchema;
use skiff_compiler_source::{
    foreign_package_db_metadata_index, ForeignPackageDbDependency, PackageDependencyAnalysisFacts,
    PackageDependencyCallableAnalysis, PackageDependencyConstantAnalysis,
    PublicationDbMetadataIndex, SourceDependencyAnalysisInput,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_syntax::ast::DbDeclKind;

use crate::{
    input::compile_input::PackageCompileInput, input::PackageDependency,
    shared::package_compile_error::PackageCompileError,
};

pub(super) struct CanonicalSourceDependencies {
    pub(super) analysis: SourceDependencyAnalysisInput,
    /// Declared artifacts plus the one exact compiler-owned std selected below.
    /// No other available package crosses the source type-resolution boundary.
    pub(super) type_resolution_artifacts: Vec<PackageArtifact>,
}

pub(super) fn source_dependencies(
    input: &PackageCompileInput<'_>,
    resolved_package_schemas: &[ResolvedPackageSchema],
    canonical_artifact_store: Option<&CanonicalArtifactStore>,
) -> Result<CanonicalSourceDependencies, PackageCompileError> {
    let compiler_owned_std =
        compiler_owned_std_artifact(input.package_id, input.available_packages)?;
    let analysis = SourceDependencyAnalysisInput::new(
        package_analysis(input, resolved_package_schemas, compiler_owned_std)?,
        validated_contract_dependencies(input, resolved_package_schemas)?,
    )
    .map_err(dependency_analysis_error)?
    .with_foreign_db_metadata(foreign_db_metadata(input, canonical_artifact_store)?);
    let mut type_resolution_artifacts = input.dependency_packages.to_vec();
    if let Some(artifact) = compiler_owned_std {
        type_resolution_artifacts.push(artifact.clone());
    }
    Ok(CanonicalSourceDependencies {
        analysis,
        type_resolution_artifacts,
    })
}

fn foreign_db_metadata(
    input: &PackageCompileInput<'_>,
    canonical_artifact_store: Option<&CanonicalArtifactStore>,
) -> Result<PublicationDbMetadataIndex, PackageCompileError> {
    let Some(store) = canonical_artifact_store else {
        // In-memory compiler unit fixtures may omit a store. A source DB
        // target still fails closed because no foreign attachment is indexed.
        return Ok(PublicationDbMetadataIndex::default());
    };
    let test_service = input.is_test_service();
    let implements_aliases =
        (!test_service).then(|| implements_referenced_dependency_aliases(input));
    let mut index = PublicationDbMetadataIndex::default();
    for dependency in input.package_dependencies.iter().filter(|dependency| {
        if test_service {
            dependency.top_level_alias.is_some()
        } else {
            implements_aliases
                .as_ref()
                .is_some_and(|aliases| aliases.contains(dependency.effective_alias()))
        }
    }) {
        let metadata = foreign_dependency_db_metadata(input, dependency, store)?;
        index.extend(metadata);
    }
    Ok(index)
}

/// Dependency aliases referenced by `db object ... implements` clauses in the
/// production source graph. Production compilation loads canonical File IR
/// only for packages whose contracts the host actually references; test
/// services keep their whole topLevelAlias view unchanged. The spelling rules
/// mirror the lowering `resolve_implements_contract` lookup so the whitelist
/// selects exactly the dependencies that can resolve.
fn implements_referenced_dependency_aliases(input: &PackageCompileInput<'_>) -> BTreeSet<String> {
    let mut aliases = BTreeSet::new();
    for source in input.package.source_graph.production() {
        for db in &source.ast.dbs {
            let Some(implements) = &db.implements else {
                continue;
            };
            if db.kind != DbDeclKind::Object {
                continue;
            }
            let name = implements.name.trim();
            let name = name.strip_prefix("root.").unwrap_or(name);
            let alias = if let Some((alias, _)) = name.split_once('/') {
                Some(alias)
            } else if let Some((alias, _)) = name.split_once('.') {
                input.package_aliases.contains_key(alias).then_some(alias)
            } else {
                None
            };
            if let Some(alias) = alias {
                aliases.insert(alias.to_string());
            }
        }
    }
    aliases
}

fn foreign_dependency_db_metadata(
    input: &PackageCompileInput<'_>,
    dependency: &PackageDependency,
    store: &CanonicalArtifactStore,
) -> Result<PublicationDbMetadataIndex, PackageCompileError> {
    let matches = input
        .dependency_packages
        .iter()
        .filter(|artifact| {
            artifact.package_id == dependency.id && artifact.package_version == dependency.version
        })
        .collect::<Vec<_>>();
    let [artifact] = matches.as_slice() else {
        return Err(validation_error(format!(
            "foreign DB dependency {}@{} requires one exact direct PackageArtifact, found {}",
            dependency.id,
            dependency.version,
            matches.len()
        )));
    };
    validate_package_artifact_identities(artifact).map_err(|error| {
        validation_error(format!(
            "foreign DB dependency {}@{} PackageArtifact identity validation failed: {error}",
            dependency.id, dependency.version
        ))
    })?;
    let reference = package_artifact_ref(artifact).map_err(|error| {
        validation_error(format!(
            "foreign DB dependency {}@{} PackageArtifact reference failed: {error}",
            dependency.id, dependency.version
        ))
    })?;
    let files = artifact
        .files
        .iter()
        .map(|file_ref| {
            store
                .read_file_ir(&reference, file_ref)
                .map(|file| file.as_ref().clone())
                .map_err(|error| {
                    validation_error(format!(
                        "foreign DB dependency {}@{} canonical File IR {} load failed: {error}",
                        dependency.id, dependency.version, file_ref.file_ir_identity
                    ))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let metadata = foreign_package_db_metadata_index(&[ForeignPackageDbDependency {
        primary_alias: dependency.effective_alias(),
        top_level_alias: dependency
            .top_level_alias
            .as_deref()
            .unwrap_or_else(|| dependency.effective_alias()),
        contracts_only: !input.is_test_service(),
        artifact,
        files: &files,
    }])
    .map_err(validation_error)?;
    Ok(metadata)
}

fn compiler_owned_std_artifact<'a>(
    package_id: &str,
    available_packages: &'a [PackageArtifact],
) -> Result<Option<&'a PackageArtifact>, PackageCompileError> {
    if package_id == SKIFF_STD_PUBLICATION_ID {
        return Ok(None);
    }
    let std_artifacts = available_packages
        .iter()
        .filter(|artifact| artifact.package_id == SKIFF_STD_PUBLICATION_ID)
        .collect::<Vec<_>>();
    if std_artifacts.len() > 1 {
        return Err(validation_error(format!(
            "compiler-owned package {SKIFF_STD_PUBLICATION_ID} has duplicate exact canonical artifacts"
        )));
    }
    let Some(artifact) = std_artifacts.first().copied() else {
        return Ok(None);
    };
    validate_package_artifact_identities(artifact).map_err(|error| {
        validation_error(format!(
            "compiler-owned package {}@{} identity validation failed: {error}",
            artifact.package_id, artifact.package_version
        ))
    })?;
    Ok(Some(artifact))
}

fn validated_contract_dependencies(
    input: &PackageCompileInput<'_>,
    resolved_package_schemas: &[ResolvedPackageSchema],
) -> Result<Vec<ResolvedContractDependency>, PackageCompileError> {
    input
        .contract_dependencies
        .iter()
        .map(|dependency| {
            ResolvedContractDependency::validated(
                dependency.requirement.clone(),
                dependency.contract.clone(),
                resolved_package_schemas,
            )
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(contract_error)
}

fn package_analysis(
    input: &PackageCompileInput<'_>,
    resolved_package_schemas: &[ResolvedPackageSchema],
    compiler_owned_std: Option<&PackageArtifact>,
) -> Result<Vec<(String, PackageDependencyAnalysisFacts)>, PackageCompileError> {
    if !input.is_test_service() {
        if let Some(dependency) = input
            .package_dependencies
            .iter()
            .find(|dependency| dependency.top_level_alias.is_some())
        {
            return Err(validation_error(format!(
                "package dependency {} declares topLevelAlias outside a test service",
                dependency.effective_alias()
            )));
        }
    }
    let artifacts = input
        .dependency_packages
        .iter()
        .map(|artifact| {
            (
                (
                    artifact.package_id.as_str(),
                    artifact.package_version.as_str(),
                ),
                artifact,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut facts = Vec::new();
    if let Some(artifact) = compiler_owned_std {
        let callables = package_callable_analysis_from_symbols(
            "compiler-owned std",
            &artifact.package_local_abi.public_symbols,
            artifact,
            std_dependency_member_path,
        )?;
        let mut analysis = PackageDependencyAnalysisFacts::new(
            artifact.package_build_id.clone(),
            artifact.package_local_abi.local_abi_identity.clone(),
            callables,
        )
        .compiler_owned();
        if let Some(schema) = resolved_package_schemas.iter().find(|schema| {
            schema.alias() == "std"
                && schema.package_id() == artifact.package_id
                && schema.exact_version() == artifact.package_version
        }) {
            analysis = analysis.with_schema_bindings(schema.index().types.iter().filter_map(
                |(stable_key, entry)| {
                    let record = schema.records().get(&entry.package_schema_type_id)?;
                    Some((
                        entry
                            .public_path
                            .clone()
                            .unwrap_or_else(|| stable_key.clone()),
                        record.clone(),
                    ))
                },
            ));
        }
        facts.push(("std".to_string(), analysis));
    }
    for dependency in input.package_dependencies {
        let Some(artifact) = artifacts.get(&(dependency.id.as_str(), dependency.version.as_str()))
        else {
            return Err(validation_error(format!(
                "package {} dependency {}@{} has no canonical PackageArtifact",
                input.package_id, dependency.id, dependency.version
            )));
        };
        validate_package_artifact_identities(artifact).map_err(|error| {
            validation_error(format!(
                "package dependency {}@{} identity validation failed: {error}",
                dependency.id, dependency.version
            ))
        })?;
        let alias = dependency.effective_alias().to_string();
        let callables = package_callable_analysis(
            &alias,
            &artifact.package_local_abi.public_symbols,
            artifact,
        )?;
        let mut analysis = PackageDependencyAnalysisFacts::new(
            artifact.package_build_id.clone(),
            artifact.package_local_abi.local_abi_identity.clone(),
            callables,
        )
        .with_canonical_alias(alias.clone())
        .with_constants(package_constant_analysis(
            &artifact.package_local_abi.public_symbols,
            artifact,
        ));
        if let Some(schema) = resolved_package_schemas.iter().find(|schema| {
            schema.alias() == alias
                && schema.package_id() == dependency.id
                && schema.exact_version() == dependency.version
        }) {
            analysis = analysis.with_schema_bindings(schema.index().types.iter().filter_map(
                |(stable_key, entry)| {
                    let record = schema.records().get(&entry.package_schema_type_id)?;
                    Some((
                        entry
                            .public_path
                            .clone()
                            .unwrap_or_else(|| stable_key.clone()),
                        record.clone(),
                    ))
                },
            ));
        }
        facts.push((alias.clone(), analysis));
        if let Some(top_level_alias) = &dependency.top_level_alias {
            let analysis = PackageDependencyAnalysisFacts::new(
                artifact.package_build_id.clone(),
                artifact.package_local_abi.local_abi_identity.clone(),
                package_callable_analysis(
                    top_level_alias,
                    &artifact.package_local_abi.implementation_symbols,
                    artifact,
                )?,
            )
            .with_canonical_alias(alias.clone())
            .with_constants(package_constant_analysis(
                &artifact.package_local_abi.implementation_symbols,
                artifact,
            ));
            facts.push((top_level_alias.clone(), analysis));
        }
    }
    Ok(facts)
}

fn package_constant_analysis(
    symbols: &BTreeMap<String, PackageLocalAbiSymbol>,
    artifact: &PackageArtifact,
) -> BTreeMap<String, PackageDependencyConstantAnalysis> {
    symbols
        .iter()
        .filter_map(|(selected_path, symbol)| {
            let PackageLocalAbiSymbol::Constant { const_id, ty } = symbol else {
                return None;
            };
            Some((
                dependency_member_path(&artifact.package_id, selected_path),
                PackageDependencyConstantAnalysis::new(
                    const_id.clone(),
                    bind_package_type_identity(ty, artifact),
                ),
            ))
        })
        .collect()
}

fn package_callable_analysis(
    dependency_label: &str,
    symbols: &BTreeMap<String, PackageLocalAbiSymbol>,
    artifact: &PackageArtifact,
) -> Result<BTreeMap<String, PackageDependencyCallableAnalysis>, PackageCompileError> {
    package_callable_analysis_from_symbols(dependency_label, symbols, artifact, |path| {
        dependency_member_path(&artifact.package_id, path)
    })
}

fn package_callable_analysis_from_symbols(
    dependency_label: &str,
    symbols: &BTreeMap<String, PackageLocalAbiSymbol>,
    artifact: &PackageArtifact,
    member_path: impl Fn(&str) -> String,
) -> Result<BTreeMap<String, PackageDependencyCallableAnalysis>, PackageCompileError> {
    symbols
        .iter()
        .filter_map(|(selected_path, symbol)| {
            let PackageLocalAbiSymbol::Callable {
                callable_id,
                signature,
            } = symbol
            else {
                return None;
            };
            let result = artifact
                .callable_semantic_facts
                .get(callable_id)
                .cloned()
                .ok_or_else(|| {
                    validation_error(format!(
                        "package dependency {} callable {} has no semantic facts",
                        dependency_label, callable_id
                    ))
                })
                .map(|semantic_facts| {
                    (
                        member_path(selected_path),
                        PackageDependencyCallableAnalysis::new(callable_id.clone(), semantic_facts)
                            .with_signature(bind_callable_signature_identity(signature, artifact)),
                    )
                });
            Some(result)
        })
        .collect()
}

fn bind_callable_signature_identity(
    signature: &PackageCallableSignature,
    artifact: &PackageArtifact,
) -> PackageCallableSignature {
    PackageCallableSignature {
        type_params: signature.type_params.clone(),
        parameters: signature
            .parameters
            .iter()
            .map(|parameter| skiff_artifact_model::PackageCallableParameter {
                name: parameter.name.clone(),
                ty: bind_package_type_identity(&parameter.ty, artifact),
                mode: parameter.mode,
            })
            .collect(),
        return_type: bind_package_type_identity(&signature.return_type, artifact),
        may_suspend: signature.may_suspend,
    }
}

fn bind_package_type_identity(ty: &PackageTypeRef, artifact: &PackageArtifact) -> PackageTypeRef {
    match ty {
        PackageTypeRef::Local { local_type } => PackageTypeRef::Local {
            local_type: bind_type_identity(local_type, artifact),
        },
        PackageTypeRef::PackageSchema { .. } => ty.clone(),
        PackageTypeRef::AnyInterface {
            interface,
            arguments,
        } => PackageTypeRef::AnyInterface {
            interface: Box::new(bind_package_type_identity(interface, artifact)),
            arguments: arguments
                .iter()
                .map(|argument| bind_package_type_identity(argument, artifact))
                .collect(),
        },
        PackageTypeRef::Container { name, arguments } => PackageTypeRef::Container {
            name: name.clone(),
            arguments: arguments
                .iter()
                .map(|argument| bind_package_type_identity(argument, artifact))
                .collect(),
        },
        PackageTypeRef::Nullable { inner } => PackageTypeRef::Nullable {
            inner: Box::new(bind_package_type_identity(inner, artifact)),
        },
    }
}

fn bind_type_identity(ty: &TypeRefIr, artifact: &PackageArtifact) -> TypeRefIr {
    let bind = |ty: &TypeRefIr| bind_type_identity(ty, artifact);
    match ty {
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: bind_nominal_base_identity(base, artifact),
            arguments: arguments.iter().map(bind).collect(),
        },
        TypeRefIr::PackageSymbol { symbol } => {
            let mut symbol = symbol.clone();
            if matches!(
                &symbol.package,
                PackageRefIr::PackageId { package_id } if package_id == &artifact.package_id
            ) {
                symbol.abi_expectation = Some(
                    artifact
                        .package_local_abi
                        .local_abi_identity
                        .as_str()
                        .to_string(),
                );
            }
            TypeRefIr::PackageSymbol { symbol }
        }
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args.iter().map(bind).collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| (name.clone(), bind(ty)))
                .collect(),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items.iter().map(bind).collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(bind(inner)),
        },
        TypeRefIr::AnyInterface { interface } => {
            let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map(|identity| bind(&identity))
                .map(|identity| type_ref_abi_key(&identity))
                .unwrap_or_else(|_| interface.interface_abi_id.clone());
            TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: identity,
                    canonical_type_args: interface.canonical_type_args.iter().map(bind).collect(),
                },
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|parameter| skiff_artifact_model::FunctionTypeParamIr {
                    name: parameter.name.clone(),
                    ty: bind(&parameter.ty),
                })
                .collect(),
            return_type: Box::new(bind(return_type)),
        },
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => ty.clone(),
    }
}

fn bind_nominal_base_identity(
    base: &NominalTypeRefBaseIr,
    artifact: &PackageArtifact,
) -> NominalTypeRefBaseIr {
    let NominalTypeRefBaseIr::PackageSymbol { symbol } = base else {
        return base.clone();
    };
    let TypeRefIr::PackageSymbol { symbol } = bind_type_identity(
        &TypeRefIr::PackageSymbol {
            symbol: symbol.clone(),
        },
        artifact,
    ) else {
        unreachable!("package-symbol binding must preserve the nominal base kind")
    };
    NominalTypeRefBaseIr::PackageSymbol { symbol }
}

fn dependency_member_path(package_id: &str, public_path: &str) -> String {
    if package_id == skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID {
        std_dependency_member_path(public_path)
    } else {
        public_path.to_string()
    }
}

fn std_dependency_member_path(public_path: &str) -> String {
    public_path
        .strip_prefix("std.")
        .unwrap_or(public_path)
        .to_string()
}

fn contract_error(error: impl std::fmt::Display) -> PackageCompileError {
    validation_error(format!("contract dependency validation failed: {error}"))
}

fn dependency_analysis_error(error: impl std::fmt::Display) -> PackageCompileError {
    validation_error(format!("dependency alias validation failed: {error}"))
}

fn validation_error(message: String) -> PackageCompileError {
    PackageCompileError::ContractValidation { message }
}

#[cfg(test)]
mod tests;
