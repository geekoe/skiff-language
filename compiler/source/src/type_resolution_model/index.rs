use super::*;

impl TypeResolutionModel {
    pub fn build(
        parsed_sources: &[ParsedCompilerSource],
        package_aliases: &BTreeMap<String, Vec<String>>,
        package_dependencies: &[PackageDependency],
        package_facts: Option<&[TypeResolutionPackageFacts<'_>]>,
        package_artifacts: Option<&[PackageArtifact]>,
        external_type_symbols: &PublicationTypeSymbolIndex,
    ) -> Result<Self, String> {
        Self::build_inner(
            parsed_sources,
            package_aliases,
            package_dependencies,
            package_facts,
            package_artifacts,
            None,
            external_type_symbols,
        )
    }

    pub(crate) fn build_with_compiler_owned_packages(
        parsed_sources: &[ParsedCompilerSource],
        package_aliases: &BTreeMap<String, Vec<String>>,
        package_dependencies: &[PackageDependency],
        package_facts: Option<&[TypeResolutionPackageFacts<'_>]>,
        package_artifacts: Option<&[PackageArtifact]>,
        dependency_analysis: &SourceDependencyAnalysisInput,
        external_type_symbols: &PublicationTypeSymbolIndex,
    ) -> Result<Self, String> {
        Self::build_inner(
            parsed_sources,
            package_aliases,
            package_dependencies,
            package_facts,
            package_artifacts,
            Some(dependency_analysis),
            external_type_symbols,
        )
    }

    pub(super) fn build_inner(
        parsed_sources: &[ParsedCompilerSource],
        package_aliases: &BTreeMap<String, Vec<String>>,
        package_dependencies: &[PackageDependency],
        package_facts: Option<&[TypeResolutionPackageFacts<'_>]>,
        package_artifacts: Option<&[PackageArtifact]>,
        compiler_owned_dependencies: Option<&SourceDependencyAnalysisInput>,
        external_type_symbols: &PublicationTypeSymbolIndex,
    ) -> Result<Self, String> {
        let mut modules = BTreeMap::new();
        let mut source_types = BTreeMap::new();
        let mut source_interfaces = BTreeSet::new();
        for parsed in parsed_sources {
            let module_path = parsed.source().module_path.clone();
            let ast = parsed.ast();
            modules.insert(
                module_path.clone(),
                ModuleTypeResolution {
                    type_indices: type_indices(ast),
                    alias_targets: parsed.alias_targets().clone(),
                    local_db_objects: LocalDbObjectIndex::from_declarations(&module_path, ast)
                        .map_err(|error| {
                            format!(
                                "failed to build type resolution db attachment index for {}: {error}",
                                parsed.source().relative_path.display()
                            )
                        })?,
                },
            );
            index_source_types(&module_path, ast, &mut source_types);
            index_source_interfaces(&module_path, ast, &mut source_interfaces);
        }

        let package_dependency_declarations = package_dependencies;
        let mut package_dependency_views = BTreeMap::new();
        let mut package_dependency_canonical_refs = BTreeMap::new();
        let mut package_dependencies = BTreeMap::new();
        for dependency in package_dependency_declarations {
            let primary_alias = dependency.effective_alias().to_string();
            package_dependency_views.insert(primary_alias.clone(), PackageDependencyView::Public);
            package_dependency_canonical_refs.insert(primary_alias.clone(), primary_alias.clone());
            package_dependencies.insert(primary_alias.clone(), dependency.id.clone());
            if let Some(top_level_alias) = &dependency.top_level_alias {
                package_dependency_views
                    .insert(top_level_alias.clone(), PackageDependencyView::TopLevel);
                package_dependency_canonical_refs
                    .insert(top_level_alias.clone(), primary_alias.clone());
                package_dependencies.insert(top_level_alias.clone(), dependency.id.clone());
            }
        }
        let mut package_types = BTreeMap::new();
        let mut package_callables = BTreeMap::new();
        let mut package_constants = BTreeMap::new();
        let mut package_interfaces = BTreeMap::new();
        let mut package_type_slots = BTreeMap::new();
        let mut package_type_source_paths = BTreeMap::new();
        let mut package_public_to_internal = BTreeMap::new();
        if let Some(package_facts) = package_facts {
            for package in package_facts {
                index_package_types(package, &mut package_types)?;
                index_package_callables(package, &mut package_callables);
                index_package_interfaces(package, &mut package_interfaces)?;
                index_package_type_slots(package, &mut package_type_slots)?;
                index_package_public_to_internal(package, &mut package_public_to_internal);
            }
        }
        let mut package_artifact_identities = BTreeMap::new();
        if let Some(package_artifacts) = package_artifacts {
            for dependency in package_dependency_declarations {
                let Some(artifact) = package_artifacts.iter().find(|artifact| {
                    artifact.package_id == dependency.id
                        && artifact.package_version == dependency.version
                }) else {
                    continue;
                };
                let dependency_ref = dependency.effective_alias();
                index_artifact_package_types(
                    artifact,
                    dependency_ref,
                    PackageDependencyView::Public,
                    ArtifactPackageTypePathMode::DeclaredPublic,
                    &mut package_types,
                    &mut package_interfaces,
                    &mut package_type_slots,
                )?;
                index_artifact_package_type_source_paths(
                    artifact,
                    dependency_ref,
                    PackageDependencyView::Public,
                    &mut package_type_source_paths,
                )?;
                index_artifact_package_constants(
                    artifact,
                    dependency_ref,
                    dependency_ref,
                    PackageDependencyView::Public,
                    &mut package_constants,
                )?;
                package_artifact_identities.insert(
                    dependency_ref.to_string(),
                    (
                        artifact.package_local_abi.local_abi_identity.clone(),
                        artifact.package_build_id.clone(),
                    ),
                );
                if let Some(top_level_alias) = &dependency.top_level_alias {
                    index_artifact_package_types(
                        artifact,
                        top_level_alias,
                        PackageDependencyView::TopLevel,
                        ArtifactPackageTypePathMode::DeclaredPublic,
                        &mut package_types,
                        &mut package_interfaces,
                        &mut package_type_slots,
                    )?;
                    index_artifact_package_type_source_paths(
                        artifact,
                        top_level_alias,
                        PackageDependencyView::TopLevel,
                        &mut package_type_source_paths,
                    )?;
                    index_artifact_package_constants(
                        artifact,
                        top_level_alias,
                        dependency_ref,
                        PackageDependencyView::TopLevel,
                        &mut package_constants,
                    )?;
                    package_artifact_identities.insert(
                        top_level_alias.clone(),
                        (
                            artifact.package_local_abi.local_abi_identity.clone(),
                            artifact.package_build_id.clone(),
                        ),
                    );
                }
            }
        }
        if let Some(dependencies) = compiler_owned_dependencies {
            let mut indexes = CompilerOwnedPackageIndexes {
                types: &mut package_types,
                interfaces: &mut package_interfaces,
                type_slots: &mut package_type_slots,
                type_source_paths: &mut package_type_source_paths,
                constants: &mut package_constants,
                dependencies: &mut package_dependencies,
                dependency_views: &mut package_dependency_views,
                dependency_canonical_refs: &mut package_dependency_canonical_refs,
                artifact_identities: &mut package_artifact_identities,
            };
            index_compiler_owned_package_artifacts(package_artifacts, dependencies, &mut indexes)?;
        }
        let semantic_publication = type_resolution_semantic_publication(parsed_sources);
        let interface_semantics = InterfaceSemantics::build(&semantic_publication)
            .map_err(|error| format!("interface semantics failed: {error}"))?;

        let mut model = Self {
            modules,
            source_types,
            source_interfaces,
            package_types,
            package_callables,
            package_constants,
            package_interfaces,
            package_type_slots,
            package_type_source_paths,
            package_dependencies,
            package_dependency_views,
            package_dependency_canonical_refs,
            package_artifact_identities,
            package_aliases: package_aliases.clone(),
            external_type_symbols: external_type_symbols.clone(),
            interface_semantics,
            interface_conformances: Vec::new(),
            local_impl_methods: BTreeMap::new(),
            package_public_to_internal,
            service_api_schemas: BTreeMap::new(),
        };
        model.local_impl_methods = model.index_local_impl_methods(parsed_sources)?;
        if let Some(dependencies) = compiler_owned_dependencies {
            model.index_service_api_contracts(dependencies)?;
        }
        model.interface_conformances = model.index_source_interface_conformances(parsed_sources)?;
        Ok(model)
    }

    /// Returns the artifact ABI identity selected for each declared or
    /// compiler-owned package dependency. Lowering uses this to keep type
    /// annotations aligned with the exact artifact source resolution inspected.
    pub(crate) fn index_service_api_contracts(
        &mut self,
        dependencies: &SourceDependencyAnalysisInput,
    ) -> Result<(), String> {
        let mut schemas = BTreeMap::new();
        for dependency in dependencies.contract_dependencies().dependencies() {
            let alias = dependency.requirement().alias.clone();
            if self.package_aliases.contains_key(&alias) {
                return Err(format!(
                    "dependency alias `{alias}` is declared by both a package and a service"
                ));
            }
            let records = dependency
                .schema_records()
                .values()
                .map(|record| (record.stable_schema_key.clone(), record.clone()))
                .collect();
            if schemas.insert(alias.clone(), records).is_some() {
                return Err(format!(
                    "service dependency alias `{alias}` is declared more than once"
                ));
            }
        }
        self.service_api_schemas = schemas;
        Ok(())
    }

    pub(super) fn index_source_interface_conformances(
        &self,
        parsed_sources: &[ParsedCompilerSource],
    ) -> Result<Vec<InterfaceConformanceResolution>, String> {
        let mut conformances = Vec::new();
        for parsed in parsed_sources {
            let module_path = parsed.source().module_path.as_str();
            for ty in &parsed.ast().types {
                if ty.alias.is_some() {
                    continue;
                }
                let receiver = SourceSymbolKey::new(module_path, &ty.name);
                let type_context = TypeResolutionContext::with_type_params(
                    module_path,
                    ty.type_params.iter().cloned().collect(),
                );
                for implemented in &ty.implements {
                    let interface = match self
                        .classify_canonical_interface_owner(&implemented.name, &type_context)
                    {
                        CanonicalInterfaceOwnerResolution::SourceDeclaredExact {
                            interface,
                            arguments,
                        }
                        | CanonicalInterfaceOwnerResolution::CompilerKnown {
                            interface,
                            arguments,
                        } => InterfaceInstantiationResolution {
                            identity: interface_symbol_type_ref(&interface),
                            args: arguments,
                        },
                        CanonicalInterfaceOwnerResolution::TypedPackage {
                            identity,
                            arguments,
                        } => InterfaceInstantiationResolution {
                            identity,
                            args: arguments,
                        },
                        CanonicalInterfaceOwnerResolution::InvalidOrUnresolved { message } => {
                            return Err(message);
                        }
                    };
                    conformances.push(InterfaceConformanceResolution {
                        receiver: receiver.clone(),
                        receiver_type_params: ty.type_params.clone(),
                        interface,
                    });
                }
            }
        }
        Ok(conformances)
    }

    pub(super) fn index_local_impl_methods(
        &self,
        parsed_sources: &[ParsedCompilerSource],
    ) -> Result<BTreeMap<SourceSymbolKey, BTreeMap<String, LocalImplMethodSignature>>, String> {
        let mut methods_by_receiver =
            BTreeMap::<SourceSymbolKey, BTreeMap<String, LocalImplMethodSignature>>::new();
        for parsed in parsed_sources {
            let module_path = parsed.source().module_path.as_str();
            for implementation in &parsed.ast().impls {
                let TypeExpr::Named { name, .. } = TypeExpr::parse(&implementation.target) else {
                    continue;
                };
                let Some(receiver) = self.resolve_source_type_key(
                    name.strip_prefix("root.").unwrap_or(&name),
                    &TypeResolutionContext::source(module_path),
                ) else {
                    continue;
                };
                let receiver_type_params = self
                    .source_types
                    .get(&receiver)
                    .map(|resolution| resolution.type_params.iter().cloned().collect())
                    .unwrap_or_default();
                let context =
                    TypeResolutionContext::with_type_params(module_path, receiver_type_params);
                let receiver_methods = methods_by_receiver.entry(receiver.clone()).or_default();
                for method in &implementation.methods {
                    if method.is_static {
                        continue;
                    }
                    let signature = self.local_impl_method_signature(
                        &receiver,
                        &implementation.target,
                        method,
                        &context,
                    )?;
                    receiver_methods.insert(method.name.clone(), signature);
                }
            }
        }
        Ok(methods_by_receiver)
    }

    pub(super) fn local_impl_method_signature(
        &self,
        receiver: &SourceSymbolKey,
        receiver_declaration: &str,
        method: &InterfaceOperation,
        context: &TypeResolutionContext<'_>,
    ) -> Result<LocalImplMethodSignature, String> {
        let mut params = Vec::new();
        if let Some(implicit_self) = &method.implicit_self {
            params.push(FunctionTypeParamIr {
                name: "self".to_string(),
                ty: self.resolve_impl_method_type_ref(receiver, implicit_self, context)?,
            });
        }
        params.extend(
            method
                .params
                .iter()
                .map(|param| {
                    Ok(FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: self.resolve_impl_method_type_ref(receiver, &param.ty, context)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        );
        let return_type =
            self.resolve_impl_method_type_ref(receiver, &method.return_type, context)?;
        Ok(LocalImplMethodSignature {
            source_callable: SourceSymbolKey::new(
                receiver.module_path(),
                crate::semantic::impl_method_declaration_name(receiver_declaration, &method.name),
            ),
            type_params: method.type_params.clone(),
            params,
            return_type,
        })
    }

    pub(super) fn resolve_impl_method_type_ref(
        &self,
        receiver: &SourceSymbolKey,
        ty: &TypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<TypeRefIr, String> {
        if ty.name == "Self" {
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref_from_source_key(receiver),
            });
        }
        self.resolve_type_ref(ty, context)
            .map(|resolved| resolved.ir)
    }
}
