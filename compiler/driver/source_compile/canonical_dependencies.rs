use std::collections::BTreeMap;

use skiff_artifact_identity::validate_package_artifact_identities;
use skiff_artifact_model::{
    InterfaceInstantiationRef, NominalTypeRefBaseIr, PackageArtifact, PackageCallableSignature,
    PackageLocalAbiSymbol, PackageRefIr, PackageTypeRef, TypeRefIr,
};
use skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID;
use skiff_compiler_input::{PackageDependencyAccess, ResolvedContractDependency};
use skiff_compiler_projection_input::ResolvedPackageSchema;
use skiff_compiler_source::{
    PackageDependencyAnalysisFacts, PackageDependencyCallableAnalysis,
    PackageDependencyConstantAnalysis, SourceDependencyAnalysisInput,
};

use crate::{
    input::{compile_input::PackageCompileInput, PackageDependency},
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
) -> Result<CanonicalSourceDependencies, PackageCompileError> {
    let compiler_owned_std =
        compiler_owned_std_artifact(input.package_id, input.available_packages)?;
    let analysis = SourceDependencyAnalysisInput::new(
        package_analysis(input, resolved_package_schemas, compiler_owned_std)?,
        validated_contract_dependencies(input, resolved_package_schemas)?,
    )
    .map_err(dependency_analysis_error)?;
    let mut type_resolution_artifacts = input.dependency_packages.to_vec();
    if let Some(artifact) = compiler_owned_std {
        type_resolution_artifacts.push(artifact.clone());
    }
    Ok(CanonicalSourceDependencies {
        analysis,
        type_resolution_artifacts,
    })
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
            .find(|dependency| dependency.access == PackageDependencyAccess::TopLevel)
        {
            return Err(validation_error(format!(
                "package dependency {} uses access: topLevel outside a test service",
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
        let callables = package_callable_analysis(dependency, artifact)?;
        let mut analysis = PackageDependencyAnalysisFacts::new(
            artifact.package_build_id.clone(),
            artifact.package_local_abi.local_abi_identity.clone(),
            callables,
        )
        .with_constants(package_constant_analysis(dependency, artifact));
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
        facts.push((alias, analysis));
    }
    Ok(facts)
}

fn package_constant_analysis(
    dependency: &PackageDependency,
    artifact: &PackageArtifact,
) -> BTreeMap<String, PackageDependencyConstantAnalysis> {
    selected_package_symbols(dependency, artifact)
        .iter()
        .filter_map(|(selected_path, symbol)| {
            let PackageLocalAbiSymbol::Constant { const_id, ty } = symbol else {
                return None;
            };
            Some((
                dependency_member_path(dependency, selected_path),
                PackageDependencyConstantAnalysis::new(
                    const_id.clone(),
                    bind_package_type_identity(ty, artifact),
                ),
            ))
        })
        .collect()
}

fn package_callable_analysis(
    dependency: &PackageDependency,
    artifact: &PackageArtifact,
) -> Result<BTreeMap<String, PackageDependencyCallableAnalysis>, PackageCompileError> {
    package_callable_analysis_from_symbols(
        dependency.effective_alias(),
        selected_package_symbols(dependency, artifact),
        artifact,
        |path| dependency_member_path(dependency, path),
    )
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
                .and_then(|identity| serde_json::to_string(&identity))
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

fn selected_package_symbols<'a>(
    dependency: &PackageDependency,
    artifact: &'a PackageArtifact,
) -> &'a BTreeMap<String, PackageLocalAbiSymbol> {
    match dependency.access {
        PackageDependencyAccess::Public => &artifact.package_local_abi.public_symbols,
        PackageDependencyAccess::TopLevel => &artifact.package_local_abi.implementation_symbols,
    }
}

fn dependency_member_path(dependency: &PackageDependency, public_path: &str) -> String {
    if dependency.id == skiff_compiler_core::id::SKIFF_STD_PUBLICATION_ID {
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
mod tests {
    use skiff_artifact_identity::{
        assign_package_artifact_identities, package_schema_index_identity,
    };
    use skiff_artifact_model::{
        PackageBuildId, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
        PackageRuntimeRequirements, PackageSchemaIndexIdentity, PackageSchemaIndexRef,
        PackageSymbolRef, PACKAGE_ARTIFACT_SCHEMA_VERSION,
    };

    use super::*;

    #[test]
    fn compiler_owned_std_selection_is_exact_and_fail_closed() {
        let std = canonical_artifact(SKIFF_STD_PUBLICATION_ID);
        let other = canonical_artifact("example.other");
        let available = [other.clone(), std.clone()];
        let selected = compiler_owned_std_artifact("example.consumer", &available)
            .unwrap()
            .expect("one exact std artifact should be selected");
        assert_eq!(selected.package_build_id, std.package_build_id);
        assert!(
            compiler_owned_std_artifact("example.consumer", std::slice::from_ref(&other))
                .unwrap()
                .is_none(),
            "an undeclared non-std available artifact must not become source-visible"
        );
        assert!(
            compiler_owned_std_artifact("example.consumer", &[])
                .unwrap()
                .is_none(),
            "absence must not fabricate a std owner"
        );

        let error = compiler_owned_std_artifact("example.consumer", &[std.clone(), std.clone()])
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("duplicate exact canonical artifacts"),
            "{error}"
        );

        let mut wrong_identity = std;
        wrong_identity.package_build_id = PackageBuildId::new("forged");
        let error =
            compiler_owned_std_artifact("example.consumer", std::slice::from_ref(&wrong_identity))
                .unwrap_err()
                .to_string();
        assert!(error.contains("identity validation failed"), "{error}");
    }

    #[test]
    fn applied_base_and_nested_arguments_bind_only_the_exact_package_abi() {
        let dependency = package_artifact("example.dep", "abi:dep");
        let package_symbol =
            |package_id: &str, symbol_path: &str, abi_expectation| PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.to_string(),
                },
                symbol_path: symbol_path.to_string(),
                abi_expectation,
            };
        let input = TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol {
                symbol: package_symbol("example.dep", "Box", Some("abi:stale-base".to_string())),
            },
            arguments: vec![
                TypeRefIr::PackageSymbol {
                    symbol: package_symbol("example.dep", "Value", None),
                },
                TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::PackageSymbol {
                        symbol: package_symbol("example.dep", "Nested", None),
                    },
                    arguments: vec![TypeRefIr::PackageSymbol {
                        symbol: package_symbol(
                            "example.other",
                            "Value",
                            Some("abi:other".to_string()),
                        ),
                    }],
                },
            ],
        };

        assert_eq!(
            bind_type_identity(&input, &dependency),
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol {
                    symbol: package_symbol("example.dep", "Box", Some("abi:dep".to_string())),
                },
                arguments: vec![
                    TypeRefIr::PackageSymbol {
                        symbol: package_symbol("example.dep", "Value", Some("abi:dep".to_string()),),
                    },
                    TypeRefIr::AppliedNominal {
                        base: NominalTypeRefBaseIr::PackageSymbol {
                            symbol: package_symbol(
                                "example.dep",
                                "Nested",
                                Some("abi:dep".to_string()),
                            ),
                        },
                        arguments: vec![TypeRefIr::PackageSymbol {
                            symbol: package_symbol(
                                "example.other",
                                "Value",
                                Some("abi:other".to_string()),
                            ),
                        }],
                    },
                ],
            }
        );
    }

    #[test]
    fn dependency_signature_identity_binding_preserves_ordered_callable_type_parameters() {
        let dependency = package_artifact("example.dep", "abi:dep");
        let signature = PackageCallableSignature {
            type_params: vec!["T".to_string(), "Id".to_string()],
            parameters: vec![skiff_artifact_model::PackageCallableParameter {
                name: "id".to_string(),
                ty: PackageTypeRef::Local {
                    local_type: TypeRefIr::TypeParam {
                        name: "Id".to_string(),
                    },
                },
            }],
            return_type: PackageTypeRef::Nullable {
                inner: Box::new(PackageTypeRef::Local {
                    local_type: TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    },
                }),
            },
            may_suspend: false,
        };

        let bound = bind_callable_signature_identity(&signature, &dependency);
        assert_eq!(bound.type_params, ["T", "Id"]);
        assert_eq!(bound.parameters, signature.parameters);
        assert_eq!(bound.return_type, signature.return_type);
    }

    fn package_artifact(package_id: &str, local_abi: &str) -> PackageArtifact {
        PackageArtifact {
            schema_version: PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string(),
            package_id: package_id.to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("build"),
            files: Vec::new(),
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new(local_abi),
                public_symbols: BTreeMap::new(),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: package_id.to_string(),
                package_schema_index_identity: PackageSchemaIndexIdentity::new("schema"),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks::default(),
            callable_links: BTreeMap::new(),
            package_requirements: Vec::new(),
            contract_requirements: Vec::new(),
            service_requirements: Vec::new(),
            runtime_requirements: PackageRuntimeRequirements {
                config: Vec::new(),
                state: Vec::new(),
                resources: Vec::new(),
                runtime_capabilities: Vec::new(),
            },
            callable_semantic_facts: BTreeMap::new(),
            boundary_projections: BTreeMap::new(),
            service_call_refs: Vec::new(),
        }
    }

    fn canonical_artifact(package_id: &str) -> PackageArtifact {
        let mut artifact = package_artifact(package_id, "unassigned");
        artifact.schema_version = PACKAGE_ARTIFACT_SCHEMA_VERSION.to_string();
        artifact.package_build_id = PackageBuildId::new("unassigned");
        artifact.package_local_abi.local_abi_identity = PackageLocalAbiIdentity::new("unassigned");
        artifact.package_schema_index.package_schema_index_identity =
            package_schema_index_identity(package_id, &BTreeMap::new()).unwrap();
        assign_package_artifact_identities(&mut artifact).unwrap();
        artifact
    }
}
