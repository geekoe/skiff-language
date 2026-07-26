use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{canonical_interface_method_abi_id, interface_instantiation_ref};
use skiff_artifact_model::{
    ContractTypeDescriptor, ContractTypeRef, FileIrUnit, FunctionTypeParamIr,
    InterfaceInstantiationRef, LiteralIr, NamedUnionBranchIr, NominalTypeRefBaseIr,
    PackageArtifact, PackageBuildId, PackageLocalAbiIdentity, PackageLocalAbiSymbol, PackageRefIr,
    PackageSchemaTypeRecord, PackageSymbolRef, PackageTypeRef, ServiceSymbolRef, TypeDescriptorIr,
    TypeRefIr,
};
use skiff_compiler_core::type_ref::substitute_type_params_in_type_ref_ref;

use crate::{
    package_export_resolver::{PackageExportResolver, ResolvedPackageSymbol},
    parsed_sources::ParsedCompilerSource,
    semantic::{
        interface::{
            object_safety_diagnostics_display, InterfaceInstantiation, InterfaceMethodSlotFact,
            InterfaceObjectSafetyDiagnostic, InterfaceOwnerKind, TypeInstantiationPattern,
        },
        InterfaceSemantics, SemanticPublication, SemanticSource,
    },
    shared::{
        ast::{AliasDecl, FunctionDecl, InterfaceOperation, SourceFile, TypeDecl, TypeRef},
        id::SKIFF_STD_PUBLICATION_ID,
        package_interface_methods::{
            instantiate_interface_method_signatures, normalize_package_interface_type_ref,
            package_interface_method_signatures, InterfaceMethodSignature, PackageTypeSymbolIndex,
        },
        prelude_registry::prelude_registry,
        type_expr::TypeExpr,
        type_syntax::generic_parts,
    },
};
use compiler_input_model::{PackageDependency, PackageDependencyAccess};

use super::{
    api::PublicTypeKind, type_indices, type_text_with_args, LocalDbObjectIndex,
    PackageInterfaceMethodIndex, PublicationTypeSymbolIndex, SourceDependencyAnalysisInput,
    SourceSymbolKey,
};

mod catch_leaves;
mod shape_assignability;

pub use catch_leaves::{CatchLeafIdentity, CatchLeaves};

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTypeRef {
    pub ir: TypeRefIr,
    pub source_text: String,
}

#[derive(Clone, Debug)]
pub struct TypeResolutionModel {
    modules: BTreeMap<String, ModuleTypeResolution>,
    source_types: BTreeMap<SourceSymbolKey, SourceTypeResolution>,
    source_interfaces: BTreeSet<SourceSymbolKey>,
    package_types: BTreeMap<PackageSymbolKey, SourceTypeResolution>,
    package_callables: BTreeMap<PackageSymbolKey, PackageCallableResolution>,
    package_constants: BTreeMap<PackageSymbolKey, PackageConstantResolution>,
    package_interfaces: BTreeMap<PackageSymbolKey, PackageInterfaceFact>,
    package_type_slots: BTreeMap<(String, String, u32), String>,
    /// Exact selected public path for a verified Local ABI implementation type.
    /// This keeps source `ServiceSymbol` recovery owner-aware without a short-name lookup.
    package_type_source_paths: BTreeMap<(String, String, String), String>,
    package_dependencies: BTreeMap<String, String>,
    package_dependency_access: BTreeMap<String, PackageDependencyAccess>,
    package_artifact_identities: BTreeMap<String, (PackageLocalAbiIdentity, PackageBuildId)>,
    package_aliases: BTreeMap<String, Vec<String>>,
    external_type_symbols: PublicationTypeSymbolIndex,
    interface_semantics: InterfaceSemantics,
    interface_conformances: Vec<InterfaceConformanceResolution>,
    local_impl_methods: BTreeMap<SourceSymbolKey, BTreeMap<String, LocalImplMethodSignature>>,
    /// Maps a package type's public api symbol path (e.g. `tools.ToolCall`) to its
    /// internal source symbol path (e.g. `agent.tools.ToolCall`). Used to canonicalize
    /// type identity toward internal names during assignability comparison, since a
    /// package can expose an internal module under a different public api name and the
    /// public and internal references otherwise produce non-matching `TypeRefIr`s.
    package_public_to_internal: BTreeMap<String, String>,
    service_api_schemas: BTreeMap<String, BTreeMap<String, PackageSchemaTypeRecord>>,
}

#[derive(Clone, Debug)]
struct ModuleTypeResolution {
    type_indices: BTreeMap<String, u32>,
    alias_targets: BTreeMap<String, String>,
    local_db_objects: LocalDbObjectIndex,
}

#[derive(Clone, Debug)]
struct SourceTypeResolution {
    name: String,
    type_params: Vec<String>,
    local_type_names: BTreeSet<String>,
    kind: SourceTypeKind,
    module_path: String,
    public_path: Option<String>,
}

#[derive(Clone, Debug)]
enum SourceTypeKind {
    Record {
        fields: BTreeMap<String, String>,
        canonical_fields: Option<BTreeMap<String, TypeRefIr>>,
    },
    Actor {
        id_type: String,
        fields: BTreeMap<String, String>,
    },
    Representation {
        target: String,
        named_union_branches: Option<Vec<NamedUnionBranchIr>>,
        discriminator: Option<String>,
    },
    Alias {
        target: String,
        canonical_target: Option<TypeRefIr>,
    },
    External,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PackageSymbolKey {
    dependency_ref: String,
    symbol_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum InterfaceTypeVisitKey {
    Source(SourceSymbolKey),
    Package(PackageSymbolKey),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum AliasTypeVisitKey {
    Source(SourceSymbolKey),
    Package(PackageSymbolKey),
}

struct ResolvedNamedType<'a> {
    resolution: &'a SourceTypeResolution,
    source_module_path: String,
    package_root: Option<String>,
    visit_key: InterfaceTypeVisitKey,
}

#[derive(Clone, Debug)]
struct InterfaceConformanceResolution {
    receiver: SourceSymbolKey,
    receiver_type_params: Vec<String>,
    interface: InterfaceInstantiationResolution,
}

#[derive(Clone, Debug)]
struct InterfaceInstantiationResolution {
    identity: TypeRefIr,
    args: Vec<TypeRefIr>,
}

#[derive(Clone, Debug)]
struct PackageInterfaceFact {
    type_params: Vec<String>,
    methods: Vec<InterfaceMethodSignature>,
    source_module: String,
}

#[derive(Clone, Debug)]
struct LocalImplMethodSignature {
    type_params: Vec<String>,
    params: Vec<FunctionTypeParamIr>,
    return_type: TypeRefIr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CanonicalInterfaceSelectorResolution {
    pub source_text: String,
    pub identity: TypeRefIr,
    pub args: Vec<TypeRefIr>,
    pub instantiation_ref: InterfaceInstantiationRef,
}

/// Canonical conformance owner selected from validated semantic and package facts.
///
/// Source exact conformance consumes only `SourceDeclaredExact`; the other
/// variants remain with their existing owners or fail closed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CanonicalInterfaceOwnerResolution {
    SourceDeclaredExact {
        interface: SourceSymbolKey,
        arguments: Vec<TypeRefIr>,
    },
    TypedPackage {
        identity: TypeRefIr,
        arguments: Vec<TypeRefIr>,
    },
    CompilerKnown {
        interface: SourceSymbolKey,
        arguments: Vec<TypeRefIr>,
    },
    InvalidOrUnresolved {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnyInterfaceMethodResolution {
    pub interface: InterfaceInstantiationRef,
    pub slot: u32,
    pub method_abi_id: String,
    pub params: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalAnyInterfaceConformanceResolution {
    pub receiver: SourceSymbolKey,
    pub interface: InterfaceInstantiationRef,
    pub slots: Vec<InterfaceMethodSlotFact>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InterfaceConformanceMatch {
    pub receiver: SourceSymbolKey,
    pub implemented_interface_identity: TypeRefIr,
    pub implemented_interface_args: Vec<TypeRefIr>,
    pub expected_interface_identity: TypeRefIr,
    pub expected_interface_args: Vec<TypeRefIr>,
}

#[derive(Clone, Debug)]
pub struct SourceInterfaceConformanceFact<'a> {
    pub interface_args: &'a [TypeRefIr],
}

pub struct TypeResolutionContext<'a> {
    pub module_path: &'a str,
    pub type_params: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub struct ConstructorTargetResolution {
    pub ty: ResolvedTypeRef,
    pub fields: BTreeMap<String, ResolvedTypeRef>,
    pub type_params: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ActorTypeResolution {
    pub ty: ResolvedTypeRef,
    pub id_type: ResolvedTypeRef,
    pub fields: BTreeMap<String, ResolvedTypeRef>,
}

#[derive(Clone, Debug)]
pub struct RepresentationConstructorResolution {
    pub wrapper: ResolvedTypeRef,
    pub payload: ResolvedTypeRef,
}

#[derive(Clone, Debug)]
pub struct PackageCallableResolution {
    pub module_path: String,
    pub source_symbol: String,
    pub type_params: Vec<String>,
    pub local_type_names: BTreeSet<String>,
    pub params: Vec<String>,
    pub return_type: String,
    pub exact_signature: Option<skiff_artifact_model::PackageCallableSignature>,
}

#[derive(Clone, Debug)]
pub struct PackageConstantResolution {
    pub symbol: PackageSymbolRef,
    pub ty: PackageTypeRef,
}

#[derive(Clone, Debug)]
pub struct PackageInterfaceResolution {
    pub identity: TypeRefIr,
    pub type_params: Vec<String>,
    pub methods: Vec<InterfaceMethodSignature>,
    pub source_module: String,
}

impl PackageInterfaceResolution {
    fn instantiate_methods(self, canonical_type_args: &[TypeRefIr]) -> Result<Self, String> {
        let methods = instantiate_interface_method_signatures(
            self.methods,
            &self.type_params,
            canonical_type_args,
        )
        .map_err(|error| {
            format!(
                "package interface expects {} type arguments, found {}",
                error.expected_type_args, error.actual_type_args
            )
        })?;
        Ok(Self {
            identity: self.identity,
            type_params: self.type_params,
            methods,
            source_module: self.source_module,
        })
    }
}

pub struct TypeResolutionPackageFacts<'a> {
    pub package_id: &'a str,
    pub dependencies: Vec<TypeResolutionPackageDependencyFact<'a>>,
    pub schema_types: Vec<TypeResolutionPackageSchemaTypeFact<'a>>,
    pub callables: Vec<TypeResolutionPackageCallableFact<'a>>,
}

pub struct TypeResolutionPackageDependencyFact<'a> {
    pub alias: &'a str,
    pub package_id: &'a str,
}

pub struct TypeResolutionPackageSchemaTypeFact<'a> {
    pub public_path: &'a str,
    pub source_module: &'a str,
    pub source_symbol: &'a str,
    pub kind: PublicTypeKind,
    pub source_ast: &'a SourceFile,
    pub file_ir_unit: Option<&'a FileIrUnit>,
}

pub struct TypeResolutionPackageCallableFact<'a> {
    pub public_path: &'a str,
    pub source_module: &'a str,
    pub source_symbol: &'a str,
    pub source_ast: &'a SourceFile,
    pub exact_signature: Option<&'a skiff_artifact_model::PackageCallableSignature>,
}

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

    fn build_inner(
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

        let mut package_dependency_access = package_dependencies
            .iter()
            .map(|dependency| (dependency.effective_alias().to_string(), dependency.access))
            .collect::<BTreeMap<_, _>>();
        let package_dependency_declarations = package_dependencies;
        let mut package_dependencies = package_dependency_declarations
            .iter()
            .map(|dependency| {
                (
                    dependency.effective_alias().to_string(),
                    dependency.id.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
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
                    dependency.access,
                    ArtifactPackageTypePathMode::DeclaredPublic,
                    &mut package_types,
                    &mut package_interfaces,
                    &mut package_type_slots,
                )?;
                index_artifact_package_type_source_paths(
                    artifact,
                    dependency_ref,
                    dependency.access,
                    &mut package_type_source_paths,
                )?;
                index_artifact_package_constants(
                    artifact,
                    dependency_ref,
                    dependency.access,
                    &mut package_constants,
                )?;
                package_artifact_identities.insert(
                    dependency_ref.to_string(),
                    (
                        artifact.package_local_abi.local_abi_identity.clone(),
                        artifact.package_build_id.clone(),
                    ),
                );
            }
        }
        if let Some(dependencies) = compiler_owned_dependencies {
            index_compiler_owned_package_artifacts(
                package_artifacts,
                dependencies,
                &mut package_types,
                &mut package_interfaces,
                &mut package_type_slots,
                &mut package_type_source_paths,
                &mut package_constants,
                &mut package_dependencies,
                &mut package_dependency_access,
                &mut package_artifact_identities,
            )?;
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
            package_dependency_access,
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
        model.interface_conformances = model.index_source_interface_conformances(parsed_sources)?;
        Ok(model)
    }

    /// Returns the artifact ABI identity selected for each declared or
    /// compiler-owned package dependency. Lowering uses this to keep type
    /// annotations aligned with the exact artifact source resolution inspected.
    pub fn package_dependency_abi_expectations(&self) -> BTreeMap<String, String> {
        self.package_artifact_identities
            .iter()
            .map(|(dependency_ref, (abi, _))| (dependency_ref.clone(), abi.as_str().to_string()))
            .collect()
    }

    /// Adds the published service APIs to the same external nominal-type model
    /// used by ordinary package dependencies. Service operation lowering keeps
    /// its own call target; only public type shapes are shared here.
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

    pub fn source_interface_conformance(
        &self,
        receiver: &SourceSymbolKey,
        interface_symbol: &ServiceSymbolRef,
    ) -> Option<SourceInterfaceConformanceFact<'_>> {
        self.source_interface_conformance_matching(receiver, |interface_identity| {
            interface_identity_matches_source_symbol(interface_identity, interface_symbol)
        })
    }

    pub fn source_interface_conformance_matching(
        &self,
        receiver: &SourceSymbolKey,
        matches_interface: impl Fn(&TypeRefIr) -> bool,
    ) -> Option<SourceInterfaceConformanceFact<'_>> {
        self.interface_conformances
            .iter()
            .find(|conformance| {
                &conformance.receiver == receiver
                    && matches_interface(&conformance.interface.identity)
            })
            .map(|conformance| SourceInterfaceConformanceFact {
                interface_args: &conformance.interface.args,
            })
    }

    pub fn resolve_type_ref(
        &self,
        ty: &TypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        self.resolve_type_text(&ty.name, context)
    }

    pub fn resolve_named_type_ref(
        &self,
        name: &str,
        arguments: &[TypeRef],
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        let argument_exprs = arguments
            .iter()
            .map(|argument| TypeExpr::parse(&argument.name))
            .collect::<Vec<_>>();
        let ir = self.resolve_named_type(name, &argument_exprs, context)?;
        let ir = self.expand_alias_type_ref(&ir, context)?;
        let source_text = if arguments.is_empty() {
            name.to_string()
        } else {
            format!(
                "{name}<{}>",
                arguments
                    .iter()
                    .map(|argument| argument.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        Ok(ResolvedTypeRef { source_text, ir })
    }

    pub fn resolve_type_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        self.reject_any_interface_selector_aliases(&TypeExpr::parse(raw), context)?;
        let expanded = self.expand_alias_text(raw, context)?;
        let expr = TypeExpr::parse(&expanded);
        let ir = self.resolve_type_expr(&expr, context)?;
        let ir = self.expand_alias_type_ref(&ir, context)?;
        Ok(ResolvedTypeRef {
            ir,
            source_text: expanded,
        })
    }

    /// Produces the exact semantic type represented by `ty`, recursively
    /// replacing every source or package alias with its RHS. Nominal
    /// declarations (records, representations, actors, interfaces, and named
    /// unions) remain named.
    pub fn expand_alias_type_ref_for_module(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
    ) -> Result<TypeRefIr, String> {
        self.expand_alias_type_ref(ty, &TypeResolutionContext::source(module_path))
    }

    pub fn expand_alias_type_ref(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<TypeRefIr, String> {
        self.expand_alias_type_ref_inner(ty, context, &mut BTreeSet::new())
    }

    fn expand_alias_type_ref_inner(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
        visiting: &mut BTreeSet<AliasTypeVisitKey>,
    ) -> Result<TypeRefIr, String> {
        match ty {
            TypeRefIr::Builtin { name, args } => Ok(TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.expand_alias_type_ref_inner(arg, context, visiting))
                    .collect::<Result<Vec<_>, _>>()?,
            }),
            TypeRefIr::LocalType { type_index } => {
                let Some(resolution) = self.local_type_resolution(context.module_path, *type_index)
                else {
                    return Err(format!(
                        "alias expansion cannot resolve local type index {type_index} in {}",
                        context.module_path
                    ));
                };
                if !matches!(resolution.kind, SourceTypeKind::Alias { .. }) {
                    return Ok(ty.clone());
                }
                self.expand_source_alias_resolution(resolution, context, visiting)
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let Some(resolution) = self.local_type_resolution(module_path, *type_index) else {
                    return Err(format!(
                        "alias expansion cannot resolve publication type index {type_index} in {module_path}"
                    ));
                };
                if !matches!(resolution.kind, SourceTypeKind::Alias { .. }) {
                    return Ok(ty.clone());
                }
                self.expand_source_alias_resolution(resolution, context, visiting)
            }
            TypeRefIr::ServiceSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                let key = SourceSymbolKey::new(module_path, &symbol.symbol);
                let Some(resolution) = self.source_types.get(&key) else {
                    return Ok(ty.clone());
                };
                if !matches!(resolution.kind, SourceTypeKind::Alias { .. }) {
                    return Ok(ty.clone());
                }
                self.expand_source_alias_resolution(resolution, context, visiting)
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => package_id.as_str(),
                };
                let canonical_package_id = self
                    .package_dependencies
                    .get(dependency_ref)
                    .map(String::as_str)
                    .unwrap_or(dependency_ref);
                let canonical_symbol = ty.clone();
                let Some(resolution) =
                    self.package_type_resolution(dependency_ref, &symbol.symbol_path)
                else {
                    return Ok(canonical_symbol);
                };
                let SourceTypeKind::Alias {
                    canonical_target, ..
                } = &resolution.kind
                else {
                    return Ok(canonical_symbol);
                };
                let public_path = resolution
                    .public_path
                    .as_deref()
                    .unwrap_or(&symbol.symbol_path);
                let visit_key = AliasTypeVisitKey::Package(PackageSymbolKey {
                    dependency_ref: canonical_package_id.to_string(),
                    symbol_path: public_path.to_string(),
                });
                if !visiting.insert(visit_key.clone()) {
                    return Err(format!(
                        "alias cycle detected while expanding package type {canonical_package_id}/{public_path}"
                    ));
                }
                let result = match canonical_target {
                    Some(target) => self.expand_alias_type_ref_inner(target, context, visiting),
                    None => Err(format!(
                        "package alias {canonical_package_id}/{public_path} has no exact RHS type"
                    )),
                };
                visiting.remove(&visit_key);
                result
            }
            TypeRefIr::AppliedNominal { base, arguments } => {
                let arguments = arguments
                    .iter()
                    .map(|argument| self.expand_alias_type_ref_inner(argument, context, visiting))
                    .collect::<Result<Vec<_>, _>>()?;
                let base_type = nominal_base_type_ref(base);
                let Some(named) = self.resolved_named_type(&base_type, context) else {
                    return Ok(TypeRefIr::AppliedNominal {
                        base: base.clone(),
                        arguments,
                    });
                };
                let SourceTypeKind::Alias {
                    target,
                    canonical_target,
                } = &named.resolution.kind
                else {
                    return Ok(TypeRefIr::AppliedNominal {
                        base: base.clone(),
                        arguments,
                    });
                };
                if named.resolution.type_params.len() != arguments.len() {
                    return Err(format!(
                        "alias {}.{} expects {} type arguments, found {}",
                        named.resolution.module_path,
                        named.resolution.name,
                        named.resolution.type_params.len(),
                        arguments.len()
                    ));
                }
                let visit_key = match &named.visit_key {
                    InterfaceTypeVisitKey::Source(key) => AliasTypeVisitKey::Source(key.clone()),
                    InterfaceTypeVisitKey::Package(key) => AliasTypeVisitKey::Package(key.clone()),
                };
                if !visiting.insert(visit_key.clone()) {
                    return Err(format!(
                        "alias cycle detected while expanding {}.{}",
                        named.resolution.module_path, named.resolution.name
                    ));
                }
                let substitutions = named
                    .resolution
                    .type_params
                    .iter()
                    .cloned()
                    .zip(arguments)
                    .collect::<BTreeMap<_, _>>();
                let target = if let Some(target) = canonical_target {
                    target.clone()
                } else {
                    let alias_context = TypeResolutionContext::with_type_params(
                        &named.resolution.module_path,
                        named.resolution.type_params.iter().cloned().collect(),
                    );
                    let target =
                        self.resolve_type_expr(&TypeExpr::parse(target), &alias_context)?;
                    if named.resolution.module_path == context.module_path {
                        target
                    } else {
                        self.externalize_local_type_ir(&target, &named.resolution.module_path)
                    }
                };
                let target = substitute_type_params_in_type_ref_ref(&target, &substitutions);
                let result = self.expand_alias_type_ref_inner(&target, context, visiting);
                visiting.remove(&visit_key);
                result
            }
            TypeRefIr::PackageSchema { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. } => Ok(ty.clone()),
            TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, field)| {
                        Ok((
                            name.clone(),
                            self.expand_alias_type_ref_inner(field, context, visiting)?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?,
            }),
            TypeRefIr::Union { items } => Ok(union_type_ir(
                items
                    .iter()
                    .map(|item| self.expand_alias_type_ref_inner(item, context, visiting))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
                inner: Box::new(self.expand_alias_type_ref_inner(inner, context, visiting)?),
            }),
            TypeRefIr::AnyInterface { interface } => {
                let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id)
                    .map_err(|error| {
                        format!(
                            "alias expansion found invalid interface ABI identity {}: {error}",
                            interface.interface_abi_id
                        )
                    })?;
                let identity = self.expand_alias_type_ref_inner(&identity, context, visiting)?;
                let args = interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| self.expand_alias_type_ref_inner(arg, context, visiting))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(TypeRefIr::AnyInterface {
                    interface: interface_instantiation_ref(identity, args),
                })
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => Ok(TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Ok(FunctionTypeParamIr {
                            name: param.name.clone(),
                            ty: self.expand_alias_type_ref_inner(&param.ty, context, visiting)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                return_type: Box::new(self.expand_alias_type_ref_inner(
                    return_type,
                    context,
                    visiting,
                )?),
            }),
        }
    }

    fn expand_source_alias_resolution(
        &self,
        resolution: &SourceTypeResolution,
        caller_context: &TypeResolutionContext<'_>,
        visiting: &mut BTreeSet<AliasTypeVisitKey>,
    ) -> Result<TypeRefIr, String> {
        let SourceTypeKind::Alias {
            target,
            canonical_target,
        } = &resolution.kind
        else {
            return Err(format!(
                "internal alias expansion requested for non-alias {}.{}",
                resolution.module_path, resolution.name
            ));
        };
        let visit_key = AliasTypeVisitKey::Source(SourceSymbolKey::new(
            &resolution.module_path,
            &resolution.name,
        ));
        if !visiting.insert(visit_key.clone()) {
            return Err(format!(
                "alias cycle detected while expanding {}.{}",
                resolution.module_path, resolution.name
            ));
        }
        let result = if let Some(target) = canonical_target {
            self.expand_alias_type_ref_inner(target, caller_context, visiting)
        } else {
            let alias_context = TypeResolutionContext::with_type_params(
                &resolution.module_path,
                caller_context.type_params.clone(),
            );
            let target_ir = self.resolve_type_expr(&TypeExpr::parse(target), &alias_context)?;
            let expanded =
                self.expand_alias_type_ref_inner(&target_ir, &alias_context, visiting)?;
            Ok(if resolution.module_path == caller_context.module_path {
                expanded
            } else {
                self.externalize_local_type_ir(&expanded, &resolution.module_path)
            })
        };
        visiting.remove(&visit_key);
        result
    }

    pub fn resolve_any_interface_type_ref(
        &self,
        interface: &TypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        let selector = self.resolve_canonical_interface_selector_type_ref(interface, context)?;
        Ok(ResolvedTypeRef {
            source_text: format!("any {}", selector.source_text),
            ir: TypeRefIr::AnyInterface {
                interface: selector.instantiation_ref,
            },
        })
    }

    pub fn resolve_canonical_interface_selector_type_ref(
        &self,
        interface: &TypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let expr = TypeExpr::parse(&interface.name);
        self.resolve_canonical_interface_selector_expr(&expr, context)
    }

    pub fn resolve_canonical_interface_selector_resolved_type_ref(
        &self,
        resolved: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let Some(interface) = self.interface_instantiation_from_resolved(resolved, context)? else {
            return Err(format!(
                "resolved type `{}` is not an interface instantiation",
                resolved.source_text
            ));
        };
        self.canonical_interface_selector_from_instantiation_resolution(
            resolved.source_text.clone(),
            interface,
        )
    }

    pub fn concrete_nominal_record_symbol(
        &self,
        actual: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Option<SourceSymbolKey> {
        self.actual_receiver_symbol(actual, context)
    }

    pub fn any_interface_method_signature(
        &self,
        receiver: &TypeRefIr,
        method_name: &str,
    ) -> Option<AnyInterfaceMethodResolution> {
        let TypeRefIr::AnyInterface { interface } = receiver else {
            return None;
        };
        let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id).ok()?;
        match identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                if let Some(service_interface) =
                    self.service_api_interface(&symbol.module_path, &symbol.symbol)
                {
                    let service_interface = service_interface
                        .instantiate_methods(&interface.canonical_type_args)
                        .ok()?;
                    let (slot, operation) = service_interface
                        .methods
                        .into_iter()
                        .enumerate()
                        .find(|(_, operation)| operation.name == method_name)?;
                    return Some(AnyInterfaceMethodResolution {
                        interface: interface.clone(),
                        slot: slot as u32,
                        method_abi_id: canonical_interface_method_abi_id(
                            interface,
                            &operation.name,
                        ),
                        params: interface_method_signature_params(&operation),
                        return_type: operation.return_type,
                    });
                }
                let key = SourceSymbolKey::new(
                    symbol
                        .module_path
                        .strip_prefix("root.")
                        .unwrap_or(&symbol.module_path),
                    &symbol.symbol,
                );
                if !self.source_type_is_interface(&key) {
                    return None;
                }
                let interface = InterfaceInstantiation {
                    symbol: key,
                    args: interface.canonical_type_args.clone(),
                };
                let canonical = self
                    .interface_semantics
                    .canonical_interface_instantiation_ref(&interface);
                self.interface_semantics
                    .method_slots_for_interface(&interface)
                    .ok()?
                    .into_iter()
                    .find(|slot| slot.name == method_name)
                    .map(|slot| method_slot_resolution(canonical, slot))
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let package_interface = self
                    .package_interface_for_type_ref(&TypeRefIr::PackageSymbol { symbol })?
                    .instantiate_methods(&interface.canonical_type_args)
                    .ok()?;
                let (slot, operation) = package_interface
                    .methods
                    .into_iter()
                    .enumerate()
                    .find(|(_, operation)| operation.name == method_name)?;
                Some(AnyInterfaceMethodResolution {
                    interface: interface.clone(),
                    slot: slot as u32,
                    method_abi_id: canonical_interface_method_abi_id(interface, &operation.name),
                    params: interface_method_signature_params(&operation),
                    return_type: operation.return_type,
                })
            }
            _ => None,
        }
    }

    pub fn interface_method_slots_for_instantiation(
        &self,
        interface: &InterfaceInstantiationRef,
    ) -> Result<Vec<InterfaceMethodSlotFact>, String> {
        let identity: TypeRefIr = serde_json::from_str(&interface.interface_abi_id)
            .map_err(|error| format!("interface ABI id is not a TypeRefIr: {error}"))?;
        match identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                let key = SourceSymbolKey::new(
                    symbol
                        .module_path
                        .strip_prefix("root.")
                        .unwrap_or(&symbol.module_path),
                    &symbol.symbol,
                );
                if !self.source_type_is_interface(&key) {
                    return Err(format!("{key} is not a source interface"));
                }
                let instantiation = InterfaceInstantiation {
                    symbol: key,
                    args: interface.canonical_type_args.clone(),
                };
                self.interface_semantics
                    .method_slots_for_interface(&instantiation)
                    .map_err(|error| error.to_string())
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let package_interface = self
                    .package_interface_for_type_ref(&TypeRefIr::PackageSymbol { symbol })
                    .ok_or_else(|| {
                        "interface ABI id does not resolve to a package interface".to_string()
                    })?
                    .instantiate_methods(&interface.canonical_type_args)?;
                Ok(package_interface
                    .methods
                    .into_iter()
                    .enumerate()
                    .map(|(slot, method)| InterfaceMethodSlotFact {
                        slot: slot as u32,
                        name: method.name.clone(),
                        method_abi_id: canonical_interface_method_abi_id(interface, &method.name),
                        params: interface_method_signature_params(&method),
                        return_type: method.return_type,
                    })
                    .collect())
            }
            other => Err(format!(
                "interface ABI id resolves to non-interface type {}",
                type_ref_debug_text(&other)
            )),
        }
    }

    pub fn resolve_constructor_target(
        &self,
        type_name: &str,
        type_args: &[TypeRef],
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorTargetResolution, String> {
        let target_text = type_text_with_args(type_name, type_args);
        let target = self.resolve_type_text(&target_text, context)?;
        self.resolve_constructor_target_resolved(&target, context)
    }

    pub fn actor_type_resolution(
        &self,
        ty: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Option<ActorTypeResolution> {
        let key = self.actual_receiver_symbol(ty, context)?;
        let resolution = self.source_types.get(&key)?;
        let SourceTypeKind::Actor { id_type, fields } = &resolution.kind else {
            return None;
        };
        let declaration_context = TypeResolutionContext::source(&resolution.module_path);
        let id_type = self
            .resolve_type_text(id_type, &declaration_context)
            .ok()
            .map(|resolved| {
                if resolution.module_path == context.module_path {
                    resolved
                } else {
                    self.externalize_local_type_refs(&resolved, &resolution.module_path)
                }
            })?;
        let fields = fields
            .iter()
            .map(|(name, ty)| {
                let resolved = self.resolve_type_text(ty, &declaration_context).ok()?;
                let resolved = if resolution.module_path == context.module_path {
                    resolved
                } else {
                    self.externalize_local_type_refs(&resolved, &resolution.module_path)
                };
                Some((name.clone(), resolved))
            })
            .collect::<Option<BTreeMap<_, _>>>()?;
        Some(ActorTypeResolution {
            ty: ty.clone(),
            id_type,
            fields,
        })
    }

    pub fn actor_method_signature(
        &self,
        ty: &ResolvedTypeRef,
        method_name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Option<(Vec<FunctionTypeParamIr>, TypeRefIr)> {
        let key = self.actual_receiver_symbol(ty, context)?;
        if !matches!(
            self.source_types.get(&key)?.kind,
            SourceTypeKind::Actor { .. }
        ) {
            return None;
        }
        let method = self.local_impl_methods.get(&key)?.get(method_name)?;
        Some((method.params.clone(), method.return_type.clone()))
    }

    pub fn actor_state_field_type(
        &self,
        ty: &ResolvedTypeRef,
        field: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Option<ResolvedTypeRef> {
        self.actor_type_resolution(ty, context)?
            .fields
            .remove(field)
    }

    pub fn resolve_constructor_target_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorTargetResolution, String> {
        let expr = TypeExpr::parse(raw);
        let TypeExpr::Named { name, args } = expr else {
            return Err(format!("constructor target `{raw}` is not a named type"));
        };
        let type_args = args
            .iter()
            .map(|arg| TypeRef {
                name: arg.to_type_string(),
            })
            .collect::<Vec<_>>();
        self.resolve_constructor_target(&name, &type_args, context)
    }

    pub fn resolve_constructor_target_resolved(
        &self,
        target: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorTargetResolution, String> {
        let (base, arguments) = match &target.ir {
            TypeRefIr::AppliedNominal { base, arguments } => {
                (nominal_base_type_ref(base), arguments.as_slice())
            }
            other => (other.clone(), &[][..]),
        };

        if let TypeRefIr::PackageSymbol { symbol } = &base {
            if let PackageRefIr::PackageId { package_id } = &symbol.package {
                if let Some((alias, schema_type)) =
                    self.service_api_schemas
                        .iter()
                        .find_map(|(alias, records)| {
                            records
                                .get(&symbol.symbol_path)
                                .filter(|record| record.package_id == *package_id)
                                .map(|record| (alias.as_str(), record))
                        })
                {
                    let ContractTypeDescriptor::Record { fields } =
                        &schema_type.canonical_descriptor.descriptor
                    else {
                        return Err(format!(
                            "constructor target `{}` is not a nominal record",
                            target.source_text
                        ));
                    };
                    let type_params = &schema_type.canonical_descriptor.type_params;
                    if type_params.len() != arguments.len() {
                        return Err(format!(
                            "constructor `{}` expects {} type arguments, found {}",
                            target.source_text,
                            type_params.len(),
                            arguments.len()
                        ));
                    }
                    let substitutions = type_params
                        .iter()
                        .cloned()
                        .zip(arguments.iter().cloned())
                        .collect::<BTreeMap<_, _>>();
                    let fields = fields
                        .iter()
                        .map(|(name, field_ty)| {
                            let field_ty = contract_type_ref_ir(alias, field_ty)?;
                            let field_ty =
                                substitute_type_params_in_type_ref_ref(&field_ty, &substitutions);
                            Ok((
                                name.clone(),
                                ResolvedTypeRef {
                                    source_text: type_ref_debug_text(&field_ty),
                                    ir: field_ty,
                                },
                            ))
                        })
                        .collect::<Result<_, String>>()?;
                    return Ok(ConstructorTargetResolution {
                        ty: target.clone(),
                        fields,
                        type_params: type_params.clone(),
                    });
                }
            }
        }

        let prelude_symbol = match &base {
            TypeRefIr::Builtin { name, .. } => Some(name.as_str()),
            TypeRefIr::PackageSymbol { symbol }
                if matches!(
                    &symbol.package,
                    PackageRefIr::PackageId { package_id }
                        if package_id == SKIFF_STD_PUBLICATION_ID
                ) =>
            {
                Some(symbol.symbol_path.as_str())
            }
            _ => None,
        };
        if let Some(shape) = prelude_symbol.and_then(prelude_constructor_shape) {
            return self.instantiate_constructor_shape(target, shape, arguments, context);
        }

        let named = self.resolved_named_type(&base, context).ok_or_else(|| {
            format!(
                "constructor target `{}` is not a resolved nominal type",
                target.source_text
            )
        })?;
        if named.resolution.type_params.len() != arguments.len() {
            return Err(format!(
                "constructor `{}` expects {} type arguments, found {}",
                target.source_text,
                named.resolution.type_params.len(),
                arguments.len()
            ));
        }
        let (fields, canonical_fields) = match &named.resolution.kind {
            SourceTypeKind::Record {
                fields,
                canonical_fields,
            } => (fields, canonical_fields),
            SourceTypeKind::Actor { .. } => {
                return Err(format!(
                    "actor `{}` is a nominal handle and cannot be constructed directly; use std.actor.getOrCreate or std.actor.replace",
                    target.source_text
                ));
            }
            _ => {
                return Err(format!(
                    "constructor target `{}` is not a nominal record",
                    target.source_text
                ));
            }
        };
        let substitutions = named
            .resolution
            .type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let declaration_context = TypeResolutionContext::with_type_params(
            &named.source_module_path,
            named.resolution.type_params.iter().cloned().collect(),
        );
        let fields = fields
            .iter()
            .map(|(name, field_text)| {
                let field_ty = if let Some(field_ty) = canonical_fields
                    .as_ref()
                    .and_then(|canonical| canonical.get(name))
                {
                    field_ty.clone()
                } else {
                    let qualified = named
                        .package_root
                        .as_deref()
                        .map(|package_root| {
                            qualify_package_type_text(
                                field_text,
                                package_root,
                                &named.resolution.local_type_names,
                            )
                        })
                        .unwrap_or_else(|| field_text.clone());
                    self.resolve_type_expr(&TypeExpr::parse(&qualified), &declaration_context)?
                };
                let field_ty = substitute_type_params_in_type_ref_ref(&field_ty, &substitutions);
                let field_ty = self.expand_alias_type_ref(&field_ty, &declaration_context)?;
                let field = ResolvedTypeRef {
                    source_text: type_ref_debug_text(&field_ty),
                    ir: field_ty,
                };
                Ok((
                    name.clone(),
                    if named.source_module_path == context.module_path {
                        field
                    } else {
                        self.externalize_local_type_refs(&field, &named.source_module_path)
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(ConstructorTargetResolution {
            ty: target.clone(),
            fields,
            type_params: named.resolution.type_params.clone(),
        })
    }

    fn instantiate_constructor_shape(
        &self,
        target: &ResolvedTypeRef,
        shape: ConstructorShape,
        arguments: &[TypeRefIr],
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorTargetResolution, String> {
        if shape.type_params.len() != arguments.len() {
            return Err(format!(
                "constructor `{}` expects {} type arguments, found {}",
                target.source_text,
                shape.type_params.len(),
                arguments.len()
            ));
        }
        let substitutions = shape
            .type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let declaration_context = TypeResolutionContext::with_type_params(
            &shape.module_path,
            shape.type_params.iter().cloned().collect(),
        );
        let fields = shape
            .fields
            .iter()
            .map(|(name, field_text)| {
                let field_ty = if let Some(field_ty) = shape
                    .canonical_fields
                    .as_ref()
                    .and_then(|canonical| canonical.get(name))
                {
                    field_ty.clone()
                } else {
                    self.resolve_type_expr(&TypeExpr::parse(field_text), &declaration_context)?
                };
                let field_ty = substitute_type_params_in_type_ref_ref(&field_ty, &substitutions);
                let field_ty = self.expand_alias_type_ref(&field_ty, &declaration_context)?;
                let field = ResolvedTypeRef {
                    source_text: type_ref_debug_text(&field_ty),
                    ir: field_ty,
                };
                Ok((
                    name.clone(),
                    if shape.module_path == context.module_path {
                        field
                    } else {
                        self.externalize_local_type_refs(&field, &shape.module_path)
                    },
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(ConstructorTargetResolution {
            ty: target.clone(),
            fields,
            type_params: shape.type_params,
        })
    }

    pub fn resolve_representation_constructor(
        &self,
        type_name: &str,
        type_args: &[TypeRef],
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<RepresentationConstructorResolution>, String> {
        let Some(shape) = self.representation_shape(type_name, context)? else {
            return Ok(None);
        };
        let target_text = type_text_with_args(type_name, type_args);
        let wrapper = self.resolve_type_text(&target_text, context)?;
        let arguments = match &wrapper.ir {
            TypeRefIr::AppliedNominal { arguments, .. }
            | TypeRefIr::Builtin {
                args: arguments, ..
            } => arguments.as_slice(),
            _ => &[],
        };
        if shape.type_params.len() != arguments.len() {
            return Err(format!(
                "representation constructor `{type_name}` expects {} type arguments, found {}",
                shape.type_params.len(),
                arguments.len()
            ));
        }
        let substitutions = shape
            .type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let payload_context = TypeResolutionContext::with_type_params(
            &shape.module_path,
            shape.type_params.iter().cloned().collect(),
        );
        let payload = self.resolve_type_expr(&TypeExpr::parse(&shape.payload), &payload_context)?;
        let payload = substitute_type_params_in_type_ref_ref(&payload, &substitutions);
        let payload = self.expand_alias_type_ref(&payload, &payload_context)?;
        let payload = ResolvedTypeRef {
            source_text: type_ref_debug_text(&payload),
            ir: payload,
        };
        let payload = if shape.module_path == context.module_path {
            payload
        } else {
            self.externalize_local_type_refs(&payload, &shape.module_path)
        };
        Ok(Some(RepresentationConstructorResolution {
            wrapper,
            payload,
        }))
    }

    pub fn resolve_package_callable(&self, path: &str) -> Option<&PackageCallableResolution> {
        let package_symbol =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(path)?;
        self.package_callable_resolution(
            &package_symbol.dependency_ref,
            &package_symbol.symbol_path,
        )
    }

    pub fn resolve_package_constant(&self, path: &str) -> Option<&PackageConstantResolution> {
        let package_symbol =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(path)?;
        if !path.contains('/') {
            return None;
        }
        self.package_constants.get(&PackageSymbolKey {
            dependency_ref: package_symbol.dependency_ref,
            symbol_path: package_symbol.symbol_path,
        })
    }

    pub fn resolve_package_interface(&self, path: &str) -> Option<PackageInterfaceResolution> {
        let package_symbol = self.resolve_package_type_symbol_path(path)?;
        let fact = self
            .package_interface_fact(&package_symbol.dependency_ref, &package_symbol.symbol_path)?;
        let public_path = self
            .package_type_resolution(&package_symbol.dependency_ref, &package_symbol.symbol_path)?
            .public_path
            .as_ref()?
            .clone();
        let package_id = self
            .package_dependencies
            .get(&package_symbol.dependency_ref)
            .cloned()
            .unwrap_or_else(|| package_symbol.dependency_ref.clone());
        Some(PackageInterfaceResolution {
            identity: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId { package_id },
                    symbol_path: public_path,
                    abi_expectation: self
                        .package_artifact_identities
                        .get(&package_symbol.dependency_ref)
                        .map(|(abi, _)| abi.as_str().to_string()),
                },
            },
            type_params: fact.type_params.clone(),
            methods: fact.methods.clone(),
            source_module: fact.source_module.clone(),
        })
    }

    fn resolve_package_type_symbol_path(&self, path: &str) -> Option<ResolvedPackageSymbol> {
        let resolved =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(path)?;
        let access = self
            .package_dependency_access
            .get(&resolved.dependency_ref)
            .copied()
            .unwrap_or(PackageDependencyAccess::Public);
        match access {
            PackageDependencyAccess::Public if path.contains('/') => None,
            PackageDependencyAccess::TopLevel if !path.contains('/') => None,
            PackageDependencyAccess::Public | PackageDependencyAccess::TopLevel => Some(resolved),
        }
    }

    pub fn package_interface_for_type_ref(
        &self,
        ty: &TypeRefIr,
    ) -> Option<PackageInterfaceResolution> {
        let TypeRefIr::PackageSymbol { symbol } = ty else {
            return None;
        };
        let dependency_ref = match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
            PackageRefIr::PackageId { package_id } => package_id.as_str(),
        };
        let fact = self.package_interface_fact(dependency_ref, &symbol.symbol_path)?;
        let public_path = self
            .package_type_resolution(dependency_ref, &symbol.symbol_path)?
            .public_path
            .as_ref()?
            .clone();
        let package_id = match &symbol.package {
            PackageRefIr::Dependency { dependency_ref } => self
                .package_dependencies
                .get(dependency_ref)
                .cloned()
                .unwrap_or_else(|| dependency_ref.clone()),
            PackageRefIr::PackageId { package_id } => package_id.clone(),
        };
        Some(PackageInterfaceResolution {
            identity: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId { package_id },
                    symbol_path: public_path,
                    abi_expectation: symbol.abi_expectation.clone(),
                },
            },
            type_params: fact.type_params.clone(),
            methods: fact.methods.clone(),
            source_module: fact.source_module.clone(),
        })
    }

    pub fn package_interface_method_index(&self) -> PackageInterfaceMethodIndex {
        let mut index = PackageInterfaceMethodIndex::default();
        for (key, interface) in &self.package_interfaces {
            index.insert_method_names(
                key.dependency_ref.clone(),
                key.symbol_path.clone(),
                interface.methods.iter().map(|method| method.name.clone()),
            );
        }
        for (alias, package_id) in &self.package_dependencies {
            for (key, interface) in &self.package_interfaces {
                if &key.dependency_ref != package_id {
                    continue;
                }
                index.insert_method_names(
                    alias.clone(),
                    key.symbol_path.clone(),
                    interface.methods.iter().map(|method| method.name.clone()),
                );
            }
        }
        index
    }

    pub fn is_nullable(&self, ty: &ResolvedTypeRef) -> bool {
        matches!(ty.ir, TypeRefIr::Nullable { .. })
            || matches!(&ty.ir, TypeRefIr::Union { items } if items.iter().any(is_null_type_ir))
    }

    pub fn contains_interface_type(
        &self,
        ty: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> bool {
        self.contains_interface_resolved_type(ty, context, &mut BTreeSet::new())
    }

    fn contains_interface_resolved_type(
        &self,
        ty: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        self.contains_interface_type_ref_inner(&ty.ir, context, visited)
    }

    fn contains_interface_type_ref_inner(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        if self.interface_identity_for_type_ref(ty, context).is_some() {
            return true;
        }
        if self.resolved_named_type(ty, context).is_some_and(|named| {
            let arguments = match ty {
                TypeRefIr::AppliedNominal { arguments, .. } => arguments.as_slice(),
                _ => &[],
            };
            self.contains_interface_named_type(named, arguments, context, visited)
        }) {
            return true;
        }
        match ty {
            TypeRefIr::Builtin { args, .. } => args
                .iter()
                .any(|arg| self.contains_interface_type_ref_inner(arg, context, visited)),
            TypeRefIr::AppliedNominal { arguments, .. } => arguments
                .iter()
                .any(|arg| self.contains_interface_type_ref_inner(arg, context, visited)),
            TypeRefIr::Record { fields } => fields
                .values()
                .any(|field| self.contains_interface_type_ref_inner(field, context, visited)),
            TypeRefIr::Union { items } => items
                .iter()
                .any(|item| self.contains_interface_type_ref_inner(item, context, visited)),
            TypeRefIr::Nullable { inner } => {
                self.contains_interface_type_ref_inner(inner, context, visited)
            }
            TypeRefIr::AnyInterface { interface } => interface
                .canonical_type_args
                .iter()
                .any(|arg| self.contains_interface_type_ref_inner(arg, context, visited)),
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                params.iter().any(|param| {
                    self.contains_interface_type_ref_inner(&param.ty, context, visited)
                }) || self.contains_interface_type_ref_inner(return_type, context, visited)
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::PackageSchema { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. } => false,
        }
    }

    fn contains_interface_named_type(
        &self,
        named: ResolvedNamedType<'_>,
        arguments: &[TypeRefIr],
        caller_context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        if !visited.insert(named.visit_key.clone()) {
            return false;
        }

        let substitutions = named
            .resolution
            .type_params
            .iter()
            .cloned()
            .zip(arguments.iter().cloned())
            .collect::<BTreeMap<_, _>>();
        let mut type_params = caller_context.type_params.clone();
        type_params.extend(named.resolution.type_params.iter().cloned());
        let source_context =
            TypeResolutionContext::with_type_params(&named.source_module_path, type_params);

        let contains = match &named.resolution.kind {
            SourceTypeKind::Record {
                fields,
                canonical_fields,
            } => {
                if let Some(fields) = canonical_fields {
                    fields.values().any(|field_ty| {
                        let field_ty =
                            substitute_type_params_in_type_ref_ref(field_ty, &substitutions);
                        self.contains_interface_type_ref_inner(&field_ty, &source_context, visited)
                    })
                } else {
                    fields.values().any(|field_ty| {
                        self.contains_interface_type_text_in_named_type(
                            field_ty,
                            named.package_root.as_deref(),
                            &named.resolution.local_type_names,
                            &substitutions,
                            &source_context,
                            visited,
                        )
                    })
                }
            }
            SourceTypeKind::Alias {
                target,
                canonical_target,
            } => {
                if let Some(target) = canonical_target {
                    let target = substitute_type_params_in_type_ref_ref(target, &substitutions);
                    self.contains_interface_type_ref_inner(&target, &source_context, visited)
                } else {
                    self.contains_interface_type_text_in_named_type(
                        target,
                        named.package_root.as_deref(),
                        &named.resolution.local_type_names,
                        &substitutions,
                        &source_context,
                        visited,
                    )
                }
            }
            SourceTypeKind::Representation { target, .. } => self
                .contains_interface_type_text_in_named_type(
                    target,
                    named.package_root.as_deref(),
                    &named.resolution.local_type_names,
                    &substitutions,
                    &source_context,
                    visited,
                ),
            SourceTypeKind::Actor { .. } | SourceTypeKind::External => false,
        };
        visited.remove(&named.visit_key);
        contains
    }

    fn contains_interface_type_text_in_named_type(
        &self,
        raw: &str,
        package_root: Option<&str>,
        local_type_names: &BTreeSet<String>,
        substitutions: &BTreeMap<String, TypeRefIr>,
        context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        let qualified = package_root
            .map(|package_root| qualify_package_type_text(raw, package_root, local_type_names))
            .unwrap_or_else(|| raw.to_string());
        let resolved = self
            .resolve_type_expr(&TypeExpr::parse(&qualified), context)
            .ok();
        resolved.is_some_and(|resolved| {
            let substituted = substitute_type_params_in_type_ref_ref(&resolved, substitutions);
            self.contains_interface_type_ref_inner(&substituted, context, visited)
        })
    }

    pub fn assignable(&self, actual: &ResolvedTypeRef, expected: &ResolvedTypeRef) -> bool {
        type_assignable(
            &self.canonicalize_type_ref(&actual.ir),
            &self.canonicalize_type_ref(&expected.ir),
        )
    }

    fn canonicalize_type_ref(&self, ty: &TypeRefIr) -> TypeRefIr {
        match ty {
            TypeRefIr::PackageSymbol { symbol } => {
                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => self
                        .package_dependencies
                        .iter()
                        .find_map(|(alias, id)| (id == package_id).then_some(alias.as_str()))
                        .unwrap_or(package_id.as_str()),
                };
                let package_id = self
                    .package_dependencies
                    .get(dependency_ref)
                    .cloned()
                    .unwrap_or_else(|| dependency_ref.to_string());
                let symbol_path = self
                    .package_type_resolution(dependency_ref, &symbol.symbol_path)
                    .map(|resolution| source_path(&resolution.module_path, &resolution.name))
                    .unwrap_or_else(|| symbol.symbol_path.clone());
                TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId { package_id },
                        symbol_path,
                        abi_expectation: symbol.abi_expectation.clone().or_else(|| {
                            self.package_artifact_identities
                                .get(dependency_ref)
                                .map(|(abi, _)| abi.as_str().to_string())
                        }),
                    },
                }
            }
            TypeRefIr::ServiceSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                canonical_named_symbol(
                    &self.canonical_symbol_path(&format!("{module_path}.{}", symbol.symbol)),
                )
            }
            TypeRefIr::AppliedNominal { base, arguments } => {
                let canonical_base = self.canonicalize_type_ref(&nominal_base_type_ref(base));
                TypeRefIr::AppliedNominal {
                    base: nominal_base_from_type_ref(canonical_base)
                        .expect("canonical nominal base remains nominal"),
                    arguments: arguments
                        .iter()
                        .map(|argument| self.canonicalize_type_ref(argument))
                        .collect(),
                }
            }
            TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.canonicalize_type_ref(arg))
                    .collect(),
            },
            TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
                inner: Box::new(self.canonicalize_type_ref(inner)),
            },
            TypeRefIr::Union { items } => TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.canonicalize_type_ref(item))
                    .collect(),
            },
            TypeRefIr::Record { fields } => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, field_ty)| (name.clone(), self.canonicalize_type_ref(field_ty)))
                    .collect(),
            },
            TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: interface.interface_abi_id.clone(),
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| self.canonicalize_type_ref(arg))
                        .collect(),
                },
            },
            other => other.clone(),
        }
    }

    pub fn canonicalize_type_ref_for_module(&self, module_path: &str, ty: &TypeRefIr) -> TypeRefIr {
        match ty {
            TypeRefIr::LocalType { type_index } => self
                .local_type_name_for_index(module_path, *type_index)
                .map(|name| canonical_named_symbol(&source_path(module_path, name)))
                .unwrap_or_else(|| ty.clone()),
            TypeRefIr::PublicationType {
                module_path: owner_module,
                type_index,
            } => self
                .local_type_name_for_index(owner_module, *type_index)
                .map(|name| canonical_named_symbol(&source_path(owner_module, name)))
                .unwrap_or_else(|| ty.clone()),
            TypeRefIr::AppliedNominal { base, arguments } => {
                let canonical_base = self
                    .canonicalize_type_ref_for_module(module_path, &nominal_base_type_ref(base));
                TypeRefIr::AppliedNominal {
                    base: nominal_base_from_type_ref(canonical_base)
                        .expect("canonical nominal base remains nominal"),
                    arguments: arguments
                        .iter()
                        .map(|argument| {
                            self.canonicalize_type_ref_for_module(module_path, argument)
                        })
                        .collect(),
                }
            }
            TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.canonicalize_type_ref_for_module(module_path, arg))
                    .collect(),
            },
            TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
                inner: Box::new(self.canonicalize_type_ref_for_module(module_path, inner)),
            },
            TypeRefIr::Union { items } => TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.canonicalize_type_ref_for_module(module_path, item))
                    .collect(),
            },
            TypeRefIr::Record { fields } => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, field_ty)| {
                        (
                            name.clone(),
                            self.canonicalize_type_ref_for_module(module_path, field_ty),
                        )
                    })
                    .collect(),
            },
            TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: interface.interface_abi_id.clone(),
                    canonical_type_args: interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| self.canonicalize_type_ref_for_module(module_path, arg))
                        .collect(),
                },
            },
            other => self.canonicalize_type_ref(other),
        }
    }

    fn local_type_name_for_index(&self, module_path: &str, type_index: u32) -> Option<&str> {
        self.modules
            .get(module_path)?
            .type_indices
            .iter()
            .find_map(|(name, index)| (*index == type_index).then_some(name.as_str()))
    }

    /// Normalize a `<module>.<symbol>` path toward its internal name. A public api
    /// symbol path (e.g. `tools.ToolCall`) is rewritten to its internal source path
    /// (e.g. `agent.tools.ToolCall`); internal paths already map to themselves.
    /// Canonicalizing toward internal names is well-defined because every public
    /// name resolves to exactly one internal name, while internal-only names have
    /// no public name.
    fn canonical_symbol_path(&self, symbol_path: &str) -> String {
        let stripped = symbol_path.strip_prefix("root.").unwrap_or(symbol_path);
        self.package_public_to_internal
            .get(stripped)
            .cloned()
            .unwrap_or_else(|| stripped.to_string())
    }

    fn representation_shape(
        &self,
        type_name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<RepresentationShape>, String> {
        let name = strip_generic(type_name.trim());
        if let Some(key) = self.resolve_source_type_key(name, context) {
            let resolved = self
                .source_types
                .get(&key)
                .ok_or_else(|| format!("unresolved representation target `{type_name}`"))?;
            return self.representation_shape_from_resolution(resolved, context);
        } else if let Some(key) = self.external_type_symbols.resolve_source_text(name) {
            let resolved = self
                .source_types
                .get(key)
                .ok_or_else(|| format!("unresolved representation target `{type_name}`"))?;
            return self.representation_shape_from_resolution(resolved, context);
        } else if let Some(package_symbol) = self.resolve_package_type_symbol_path(name) {
            if let Some(resolved) = self.package_type_resolution(
                &package_symbol.dependency_ref,
                &package_symbol.symbol_path,
            ) {
                return self.representation_shape_from_resolution(resolved, context);
            }
            return Ok(prelude_representation_shape(name));
        }
        Ok(prelude_representation_shape(name))
    }

    fn representation_shape_from_resolution(
        &self,
        resolved: &SourceTypeResolution,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<RepresentationShape>, String> {
        match &resolved.kind {
            SourceTypeKind::Representation {
                target,
                named_union_branches,
                ..
            } if named_union_branches.is_none()
                && !matches!(TypeExpr::parse(target), TypeExpr::Union(_)) =>
            {
                Ok(Some(RepresentationShape {
                    module_path: resolved.module_path.clone(),
                    type_params: resolved.type_params.clone(),
                    payload: target.clone(),
                }))
            }
            SourceTypeKind::Representation { .. } => Ok(None),
            SourceTypeKind::Alias { target, .. } => {
                let alias_context = TypeResolutionContext::with_type_params(
                    &resolved.module_path,
                    context.type_params.clone(),
                );
                self.representation_shape(target, &alias_context)
            }
            SourceTypeKind::Record { .. }
            | SourceTypeKind::Actor { .. }
            | SourceTypeKind::External => Ok(None),
        }
    }

    fn resolve_any_interface_type_expr(
        &self,
        interface: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        let selector = self.resolve_canonical_interface_selector_expr(interface, context)?;
        Ok(ResolvedTypeRef {
            source_text: format!("any {}", selector.source_text),
            ir: TypeRefIr::AnyInterface {
                interface: selector.instantiation_ref,
            },
        })
    }

    fn reject_any_interface_selector_aliases(
        &self,
        expr: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<(), String> {
        match expr {
            TypeExpr::AnyInterface { interface } => {
                if let TypeExpr::Named { name, .. } = interface.as_ref() {
                    if let Some(key) = self.resolve_source_type_key(name, context) {
                        if self.source_types.get(&key).is_some_and(|resolution| {
                            matches!(resolution.kind, SourceTypeKind::Alias { .. })
                        }) {
                            return Err(format!(
                                "interface selector `{}` targets alias `{name}`, not an interface",
                                interface.to_type_string()
                            ));
                        }
                    }
                    if let Some(package_symbol) = self.resolve_package_type_symbol_path(name) {
                        if self
                            .package_type_resolution(
                                &package_symbol.dependency_ref,
                                &package_symbol.symbol_path,
                            )
                            .is_some_and(|resolution| {
                                matches!(resolution.kind, SourceTypeKind::Alias { .. })
                            })
                        {
                            return Err(format!(
                                "interface selector `{}` targets alias `{name}`, not an interface",
                                interface.to_type_string()
                            ));
                        }
                    }
                }
                self.reject_any_interface_selector_aliases(interface, context)
            }
            TypeExpr::Named { args, .. } | TypeExpr::Union(args) => {
                for arg in args {
                    self.reject_any_interface_selector_aliases(arg, context)?;
                }
                Ok(())
            }
            TypeExpr::Nullable(inner) => self.reject_any_interface_selector_aliases(inner, context),
            TypeExpr::Record(fields) => {
                for field in fields {
                    self.reject_any_interface_selector_aliases(&field.ty, context)?;
                }
                Ok(())
            }
            TypeExpr::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.reject_any_interface_selector_aliases(&param.ty, context)?;
                }
                self.reject_any_interface_selector_aliases(return_type, context)
            }
            TypeExpr::EmptyRecord | TypeExpr::StringLiteral(_) => Ok(()),
        }
    }

    fn resolve_canonical_interface_selector_expr(
        &self,
        expr: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let selector_text = expr.to_type_string();
        match expr {
            TypeExpr::AnyInterface { .. } => Err(format!(
                "interface selector `{selector_text}` cannot be nested `any`; use the bare interface selector"
            )),
            TypeExpr::Record(_) | TypeExpr::EmptyRecord => Err(format!(
                "interface selector `{selector_text}` targets an anonymous record, not an interface"
            )),
            TypeExpr::Named { name, args } => {
                self.resolve_canonical_interface_selector_named(
                    name,
                    args,
                    &selector_text,
                    context,
                )
            }
            TypeExpr::StringLiteral(_) => Err(format!(
                "interface selector `{selector_text}` targets a literal type, not an interface"
            )),
            TypeExpr::Nullable(_) | TypeExpr::Union(_) | TypeExpr::Function { .. } => Err(
                format!("interface selector `{selector_text}` must be a named interface type"),
            ),
        }
    }

    fn resolve_canonical_interface_selector_named(
        &self,
        name: &str,
        args: &[TypeExpr],
        selector_text: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let name = name.trim();
        let service_name = name.strip_prefix("root.").unwrap_or(name);
        if args.is_empty() && context.type_params.contains(service_name) {
            return Err(format!(
                "interface selector `{selector_text}` targets type parameter `{service_name}`, not an interface"
            ));
        }
        if let Some(canonical_name) = builtin_type_name(name) {
            return Err(format!(
                "interface selector `{selector_text}` targets primitive/builtin type `{canonical_name}`, not an interface"
            ));
        }
        if prelude_known_type_ref(name, Vec::new()).is_some() {
            return Err(format!(
                "interface selector `{selector_text}` targets primitive/builtin type `{name}`, not an interface"
            ));
        }
        if let Some(key) = self.resolve_source_type_key(name, context) {
            return self.resolve_source_interface_selector_from_key(
                key,
                args,
                selector_text,
                context,
            );
        }
        if let Some(key) = self.external_type_symbols.resolve_source_text(name) {
            return self.resolve_source_interface_selector_from_key(
                key.clone(),
                args,
                selector_text,
                context,
            );
        }
        if let Some((alias, schema_type)) = self.service_api_type(name)? {
            let Some(interface) = self.service_api_interface(alias, &schema_type.stable_schema_key)
            else {
                return Err(format!(
                    "interface selector `{selector_text}` targets a non-interface service API type"
                ));
            };
            let args = self.resolve_interface_selector_args(args, context)?;
            self.require_package_interface_type_args(selector_text, &interface.type_params, &args)?;
            self.require_package_interface_object_safe(selector_text, &interface.methods)?;
            return Ok(CanonicalInterfaceSelectorResolution {
                source_text: selector_text.to_string(),
                identity: interface.identity.clone(),
                instantiation_ref: interface_instantiation_ref(interface.identity, args.clone()),
                args,
            });
        }
        if let Some(package_symbol) = self.resolve_package_type_symbol_path(name) {
            if let Some(interface) = self.resolve_package_interface(name) {
                let args = self.resolve_interface_selector_args(args, context)?;
                self.require_package_interface_type_args(
                    selector_text,
                    &interface.type_params,
                    &args,
                )?;
                self.require_package_interface_object_safe(selector_text, &interface.methods)?;
                return Ok(CanonicalInterfaceSelectorResolution {
                    source_text: selector_text.to_string(),
                    identity: interface.identity.clone(),
                    instantiation_ref: interface_instantiation_ref(
                        interface.identity,
                        args.clone(),
                    ),
                    args,
                });
            }
            let resolution = self.package_type_resolution(
                &package_symbol.dependency_ref,
                &package_symbol.symbol_path,
            );
            if self
                .package_artifact_identities
                .contains_key(&package_symbol.dependency_ref)
                && resolution.is_none()
            {
                let access = self
                    .package_dependency_access
                    .get(&package_symbol.dependency_ref)
                    .copied()
                    .unwrap_or(PackageDependencyAccess::Public);
                return Err(format!(
                    "package dependency `{}` has no {} type path `{}`",
                    package_symbol.dependency_ref,
                    match access {
                        PackageDependencyAccess::Public => "public",
                        PackageDependencyAccess::TopLevel => "top-level source",
                    },
                    package_symbol.symbol_path
                ));
            }
            if let Some(resolution) = resolution {
                return Err(format!(
                    "interface selector `{selector_text}` targets {}, not an interface",
                    source_type_kind_label(&resolution.kind)
                ));
            }
            return Err(format!(
                "interface selector `{selector_text}` does not resolve to an interface"
            ));
        }
        if let Some(symbol) = self.resolve_db_object_symbol(service_name, context)? {
            return Err(format!(
                "interface selector `{selector_text}` targets db object {}.{}, not an interface",
                symbol.module_path, symbol.symbol
            ));
        }
        if name.contains('.') {
            return Err(format!(
                "interface selector `{selector_text}` does not resolve to a known interface"
            ));
        }
        Err(format!(
            "interface selector `{selector_text}` does not resolve to an interface"
        ))
    }

    fn resolve_source_interface_selector_from_key(
        &self,
        key: SourceSymbolKey,
        args: &[TypeExpr],
        selector_text: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        let Some(resolution) = self.source_types.get(&key) else {
            return Err(format!(
                "interface selector `{selector_text}` does not resolve to an interface"
            ));
        };
        if !self.source_type_is_interface(&key) {
            return Err(format!(
                "interface selector `{selector_text}` targets {}, not an interface",
                source_type_kind_label(&resolution.kind)
            ));
        }
        if resolution.type_params.len() != args.len() {
            return Err(format!(
                "interface selector `{selector_text}` targets interface {}, which expects {} type arguments, found {}",
                key,
                resolution.type_params.len(),
                args.len()
            ));
        }
        let args = self.resolve_interface_selector_args(args, context)?;
        let interface = InterfaceInstantiation {
            symbol: key,
            args: args.clone(),
        };
        let diagnostics = self
            .interface_semantics
            .object_safety_diagnostics(&interface)
            .map_err(|error| error.to_string())?;
        if !diagnostics.is_empty() {
            return Err(format!(
                "interface selector `{selector_text}` is not object-safe: {}",
                object_safety_diagnostics_display(&diagnostics)
            ));
        }
        let identity = interface_symbol_type_ref(&interface.symbol);
        Ok(CanonicalInterfaceSelectorResolution {
            source_text: selector_text.to_string(),
            instantiation_ref: self
                .interface_semantics
                .canonical_interface_instantiation_ref(&interface),
            identity,
            args,
        })
    }

    fn resolve_interface_selector_args(
        &self,
        args: &[TypeExpr],
        context: &TypeResolutionContext<'_>,
    ) -> Result<Vec<TypeRefIr>, String> {
        args.iter()
            .map(|arg| {
                self.resolve_type_text(&arg.to_type_string(), context)
                    .map(|ty| ty.ir)
            })
            .collect()
    }

    fn canonical_interface_selector_from_instantiation_resolution(
        &self,
        source_text: String,
        interface: InterfaceInstantiationResolution,
    ) -> Result<CanonicalInterfaceSelectorResolution, String> {
        match &interface.identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                let source_interface = InterfaceInstantiation {
                    symbol: SourceSymbolKey::new(
                        symbol
                            .module_path
                            .strip_prefix("root.")
                            .unwrap_or(&symbol.module_path),
                        &symbol.symbol,
                    ),
                    args: interface.args.clone(),
                };
                let diagnostics = self
                    .interface_semantics
                    .object_safety_diagnostics(&source_interface)
                    .map_err(|error| error.to_string())?;
                if !diagnostics.is_empty() {
                    return Err(format!(
                        "interface selector `{source_text}` is not object-safe: {}",
                        object_safety_diagnostics_display(&diagnostics)
                    ));
                }
            }
            TypeRefIr::PackageSymbol { .. } => {
                let package_interface = self
                    .package_interface_for_type_ref(&interface.identity)
                    .ok_or_else(|| {
                        format!(
                            "interface selector `{source_text}` does not resolve to a package interface"
                        )
                    })?;
                self.require_package_interface_type_args(
                    &source_text,
                    &package_interface.type_params,
                    &interface.args,
                )?;
                self.require_package_interface_object_safe(
                    &source_text,
                    &package_interface.methods,
                )?;
            }
            _ => {
                return Err(format!(
                    "resolved type `{source_text}` is not an interface instantiation"
                ));
            }
        }
        Ok(CanonicalInterfaceSelectorResolution {
            source_text,
            instantiation_ref: interface_instantiation_ref(
                interface.identity.clone(),
                interface.args.clone(),
            ),
            identity: interface.identity,
            args: interface.args,
        })
    }

    fn require_package_interface_object_safe(
        &self,
        selector_text: &str,
        methods: &[InterfaceMethodSignature],
    ) -> Result<(), String> {
        let mut diagnostics = Vec::new();
        if methods.is_empty() {
            diagnostics.push(InterfaceObjectSafetyDiagnostic::MarkerInterface {
                interface: SourceSymbolKey::new("<package>", selector_text),
            });
        }
        for method in methods {
            if method.is_static {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: method.name.clone(),
                        message: "method requirement cannot be static".to_string(),
                    },
                );
            }
            if method.is_native {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: method.name.clone(),
                        message: "method requirement cannot be native".to_string(),
                    },
                );
            }
            if method.is_provider {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: method.name.clone(),
                        message: "method requirement cannot be provider-only".to_string(),
                    },
                );
            }
            if !method.type_params.is_empty() {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: method.name.clone(),
                        message: "method requirement cannot declare method-level type parameters"
                            .to_string(),
                    },
                );
            }
            let params = interface_method_signature_params(method);
            match params.first() {
                Some(param) if param.name == "self" && is_self_type_ref(&param.ty) => {
                    for param in params.iter().skip(1) {
                        if type_ref_contains_self(&param.ty) {
                            diagnostics.push(InterfaceObjectSafetyDiagnostic::InvalidSelfUsage {
                                method_name: method.name.clone(),
                                message: "Self can only appear in the first receiver parameter"
                                    .to_string(),
                            });
                        }
                    }
                    if type_ref_contains_self(&method.return_type) {
                        diagnostics.push(InterfaceObjectSafetyDiagnostic::InvalidSelfUsage {
                            method_name: method.name.clone(),
                            message: "Self cannot be used as a return type".to_string(),
                        });
                    }
                }
                _ if params.iter().any(|param| type_ref_contains_self(&param.ty))
                    || type_ref_contains_self(&method.return_type) =>
                {
                    diagnostics.push(InterfaceObjectSafetyDiagnostic::InvalidSelfUsage {
                        method_name: method.name.clone(),
                        message: "Self can only appear in the first receiver parameter".to_string(),
                    });
                }
                _ => diagnostics.push(InterfaceObjectSafetyDiagnostic::MissingSelfReceiver {
                    method_name: method.name.clone(),
                }),
            }
        }
        if diagnostics.is_empty() {
            return Ok(());
        }
        Err(format!(
            "interface selector `{selector_text}` is not object-safe: {}",
            object_safety_diagnostics_display(&diagnostics)
        ))
    }

    fn require_package_interface_type_args(
        &self,
        selector_text: &str,
        type_params: &[String],
        args: &[TypeRefIr],
    ) -> Result<(), String> {
        if type_params.len() == args.len() {
            return Ok(());
        }
        Err(format!(
            "interface selector `{selector_text}` expects {} type arguments, found {}",
            type_params.len(),
            args.len()
        ))
    }

    fn resolve_type_expr(
        &self,
        expr: &TypeExpr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<TypeRefIr, String> {
        Ok(match expr {
            TypeExpr::EmptyRecord => TypeRefIr::Record {
                fields: BTreeMap::new(),
            },
            TypeExpr::StringLiteral(value) => TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: value.clone(),
                },
            },
            TypeExpr::Named { name, args } => self.resolve_named_type(name, args, context)?,
            TypeExpr::Nullable(inner) => TypeRefIr::Nullable {
                inner: Box::new(self.resolve_type_expr(inner, context)?),
            },
            TypeExpr::Union(items) => TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.resolve_type_expr(item, context))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            TypeExpr::AnyInterface { interface } => {
                self.resolve_any_interface_type_expr(interface, context)?.ir
            }
            TypeExpr::Record(fields) => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            self.resolve_type_expr(&field.ty, context)?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?,
            },
            TypeExpr::Function {
                params,
                return_type,
            } => TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| {
                        Ok(FunctionTypeParamIr {
                            name: param.name.clone(),
                            ty: self.resolve_type_expr(&param.ty, context)?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
                return_type: Box::new(self.resolve_type_expr(return_type, context)?),
            },
        })
    }

    fn resolve_named_type(
        &self,
        name: &str,
        args: &[TypeExpr],
        context: &TypeResolutionContext<'_>,
    ) -> Result<TypeRefIr, String> {
        let resolved_args = args
            .iter()
            .map(|arg| self.resolve_type_expr(arg, context))
            .collect::<Result<Vec<_>, _>>()?;
        let name = name.trim();
        let service_name = name.strip_prefix("root.").unwrap_or(name);
        if args.is_empty() && context.type_params.contains(service_name) {
            return Ok(TypeRefIr::TypeParam {
                name: service_name.to_string(),
            });
        }
        let source_type_key = self.resolve_source_type_key(name, context);
        if source_type_key.is_none() {
            if let Some(canonical_name) = builtin_type_name(name) {
                if canonical_name == "Map"
                    && resolved_args.len() == 2
                    && type_ref_contains_any_interface(&resolved_args[0])
                {
                    return Err(format!(
                        "Map key type `{}` cannot contain an `any` interface value",
                        args[0].to_type_string()
                    ));
                }
                return Ok(TypeRefIr::Builtin {
                    name: canonical_name,
                    args: resolved_args,
                });
            }
        }
        if let Some(key) = source_type_key {
            let resolution = self
                .source_types
                .get(&key)
                .ok_or_else(|| format!("missing source type resolution for `{name}`"))?;
            if resolution.type_params.len() != resolved_args.len() {
                return Err(format!(
                    "source type `{name}` expects {} type arguments, found {}",
                    resolution.type_params.len(),
                    resolved_args.len()
                ));
            }
            if !resolved_args.is_empty()
                && (self.source_interfaces.contains(&key)
                    || matches!(
                        resolution.kind,
                        SourceTypeKind::Actor { .. } | SourceTypeKind::External
                    ))
            {
                return Err(format!(
                    "source type `{name}` cannot be used as an applied nominal base"
                ));
            }
            let module = self
                .modules
                .get(context.module_path)
                .ok_or_else(|| format!("missing type resolution module {}", context.module_path))?;
            if key.module_path() == context.module_path {
                if let Some(index) = module.type_indices.get(key.symbol()) {
                    return apply_nominal_arguments(
                        TypeRefIr::LocalType { type_index: *index },
                        resolved_args,
                    );
                }
            }
            return apply_nominal_arguments(
                TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: key.module_path().to_string(),
                        symbol: key.symbol().to_string(),
                    },
                },
                resolved_args,
            );
        }
        if let Some(type_ref) = contextual_prelude_type_ref(name, resolved_args.clone(), context) {
            validate_prelude_type_arity(name, resolved_args.len())?;
            return Ok(type_ref);
        }
        if let Some(type_ref) = prelude_known_type_ref(name, resolved_args.clone()) {
            validate_prelude_type_arity(name, resolved_args.len())?;
            return Ok(type_ref);
        }
        if let Some((_alias, schema_type)) = self.service_api_type(name)? {
            if schema_type.canonical_descriptor.type_params.len() != resolved_args.len() {
                return Err(format!(
                    "service API type `{name}` expects {} type arguments, found {}",
                    schema_type.canonical_descriptor.type_params.len(),
                    resolved_args.len()
                ));
            }
            if !resolved_args.is_empty()
                && matches!(
                    schema_type.canonical_descriptor.descriptor,
                    ContractTypeDescriptor::CallbackInterface { .. }
                )
            {
                return Err(format!(
                    "service API type `{name}` cannot be used as an applied nominal base"
                ));
            }
            return apply_nominal_arguments(
                TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: schema_type.package_id.clone(),
                        },
                        symbol_path: schema_type.stable_schema_key.clone(),
                        abi_expectation: None,
                    },
                },
                resolved_args,
            );
        }
        if let Some(package_symbol) = self.resolve_package_type_symbol_path(name) {
            let resolution = self.package_type_resolution(
                &package_symbol.dependency_ref,
                &package_symbol.symbol_path,
            );
            if let Some(resolution) = resolution {
                if resolution.type_params.len() != resolved_args.len() {
                    return Err(format!(
                        "package type `{name}` expects {} type arguments, found {}",
                        resolution.type_params.len(),
                        resolved_args.len()
                    ));
                }
                if !resolved_args.is_empty()
                    && (matches!(
                        resolution.kind,
                        SourceTypeKind::Actor { .. } | SourceTypeKind::External
                    ) || self
                        .package_interface_fact(
                            &package_symbol.dependency_ref,
                            &package_symbol.symbol_path,
                        )
                        .is_some())
                {
                    return Err(format!(
                        "package type `{name}` cannot be used as an applied nominal base"
                    ));
                }
            } else if !resolved_args.is_empty() {
                return Err(format!(
                    "package type `{name}` has no exact declaration for generic arity validation"
                ));
            }
            let abi_expectation = self
                .package_artifact_identities
                .get(&package_symbol.dependency_ref)
                .map(|(abi, _)| abi.as_str().to_string());
            return apply_nominal_arguments(
                TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::Dependency {
                            dependency_ref: package_symbol.dependency_ref,
                        },
                        symbol_path: package_symbol.symbol_path,
                        abi_expectation,
                    },
                },
                resolved_args,
            );
        }
        let dependency_root = name
            .split_once('/')
            .map(|(root, _)| root)
            .or_else(|| name.split_once('.').map(|(root, _)| root));
        if let Some((dependency_ref, access)) =
            dependency_root.and_then(|root| self.package_dependency_access.get_key_value(root))
        {
            return Err(match access {
                PackageDependencyAccess::Public => format!(
                    "package dependency `{dependency_ref}` uses public type syntax `{dependency_ref}.<public-path>`; source-path slash syntax is unavailable"
                ),
                PackageDependencyAccess::TopLevel => format!(
                    "package dependency `{dependency_ref}` uses top-level type syntax `{dependency_ref}/<source-module>.<name>`; dotted public syntax is unavailable"
                ),
            });
        }
        if let Some(symbol) = self.external_type_symbols.resolve_source_text(name) {
            if !resolved_args.is_empty() {
                return Err(format!(
                    "external type `{name}` has no exact declaration for generic arity validation"
                ));
            }
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref_from_source_key(symbol),
            });
        }
        if let Some(symbol) = self.resolve_db_object_symbol(service_name, context)? {
            if !resolved_args.is_empty() {
                return Err(format!(
                    "db object type `{name}` cannot be used as an applied nominal base"
                ));
            }
            return Ok(TypeRefIr::DbObjectSymbol { symbol });
        }
        if name.contains('.') {
            if !resolved_args.is_empty() {
                return Err(format!(
                    "unresolved nominal type `{name}` cannot accept type arguments"
                ));
            }
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref(name),
            });
        }
        Err(format!("unresolved type `{name}`"))
    }

    fn service_api_type(
        &self,
        name: &str,
    ) -> Result<Option<(&str, &PackageSchemaTypeRecord)>, String> {
        let name = name.strip_prefix("root.").unwrap_or(name);
        let Some((alias, stable_key)) = name.split_once('.') else {
            return Ok(None);
        };
        let Some((canonical_alias, records)) = self.service_api_schemas.get_key_value(alias) else {
            return Ok(None);
        };
        let schema_type = records.get(stable_key).ok_or_else(|| {
            format!("service dependency `{alias}` has no public API type `{stable_key}`")
        })?;
        Ok(Some((canonical_alias.as_str(), schema_type)))
    }

    fn service_api_interface(
        &self,
        alias: &str,
        stable_key: &str,
    ) -> Option<PackageInterfaceResolution> {
        let schema_type = self.service_api_schemas.get(alias)?.get(stable_key)?;
        let ContractTypeDescriptor::CallbackInterface { operations } =
            &schema_type.canonical_descriptor.descriptor
        else {
            return None;
        };
        let methods = operations
            .iter()
            .map(|(name, operation)| {
                Some(InterfaceMethodSignature {
                    name: name.clone(),
                    type_params: Vec::new(),
                    params: operation
                        .parameters
                        .iter()
                        .enumerate()
                        .map(|(index, ty)| {
                            Some(FunctionTypeParamIr {
                                name: format!("arg{index}"),
                                ty: contract_type_ref_ir(alias, ty).ok()?,
                            })
                        })
                        .collect::<Option<Vec<_>>>()?,
                    return_type: contract_type_ref_ir(alias, &operation.return_type).ok()?,
                    may_suspend: operation.may_suspend,
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(PackageInterfaceResolution {
            identity: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId {
                        package_id: schema_type.package_id.clone(),
                    },
                    symbol_path: stable_key.to_string(),
                    abi_expectation: None,
                },
            },
            type_params: schema_type.canonical_descriptor.type_params.clone(),
            methods,
            source_module: alias.to_string(),
        })
    }

    fn package_type_resolution(
        &self,
        dependency_ref: &str,
        symbol_path: &str,
    ) -> Option<&SourceTypeResolution> {
        let direct_key = PackageSymbolKey {
            dependency_ref: dependency_ref.to_string(),
            symbol_path: symbol_path.to_string(),
        };
        self.package_types
            .get(&direct_key)
            .or_else(|| {
                let package_id = self.package_dependencies.get(dependency_ref)?;
                let package_key = PackageSymbolKey {
                    dependency_ref: package_id.clone(),
                    symbol_path: symbol_path.to_string(),
                };
                self.package_types.get(&package_key)
            })
            .or_else(|| {
                self.package_dependencies
                    .iter()
                    .filter(|(_, package_id)| package_id.as_str() == dependency_ref)
                    .find_map(|(alias, _)| {
                        self.package_types.get(&PackageSymbolKey {
                            dependency_ref: alias.clone(),
                            symbol_path: symbol_path.to_string(),
                        })
                    })
            })
    }

    /// Resolve a package type by its symbol path alone, searching every indexed
    /// package. Used to recover the shape of a package type referenced through a
    /// package-internal `root.` path that did not carry its originating package id.
    fn package_type_by_symbol_path(&self, symbol_path: &str) -> Option<&SourceTypeResolution> {
        self.package_types
            .iter()
            .find(|(key, _)| key.symbol_path == symbol_path)
            .map(|(_, resolution)| resolution)
    }

    fn package_callable_resolution(
        &self,
        dependency_ref: &str,
        symbol_path: &str,
    ) -> Option<&PackageCallableResolution> {
        let direct_key = PackageSymbolKey {
            dependency_ref: dependency_ref.to_string(),
            symbol_path: symbol_path.to_string(),
        };
        self.package_callables.get(&direct_key).or_else(|| {
            let package_id = self.package_dependencies.get(dependency_ref)?;
            let package_key = PackageSymbolKey {
                dependency_ref: package_id.clone(),
                symbol_path: symbol_path.to_string(),
            };
            self.package_callables.get(&package_key)
        })
    }

    fn package_interface_fact(
        &self,
        dependency_ref: &str,
        symbol_path: &str,
    ) -> Option<&PackageInterfaceFact> {
        let direct_key = PackageSymbolKey {
            dependency_ref: dependency_ref.to_string(),
            symbol_path: symbol_path.to_string(),
        };
        self.package_interfaces
            .get(&direct_key)
            .or_else(|| {
                let package_id = self.package_dependencies.get(dependency_ref)?;
                let package_key = PackageSymbolKey {
                    dependency_ref: package_id.clone(),
                    symbol_path: symbol_path.to_string(),
                };
                self.package_interfaces.get(&package_key)
            })
            .or_else(|| {
                self.package_dependencies
                    .iter()
                    .filter(|(_, package_id)| package_id.as_str() == dependency_ref)
                    .find_map(|(alias, _)| {
                        self.package_interfaces.get(&PackageSymbolKey {
                            dependency_ref: alias.clone(),
                            symbol_path: symbol_path.to_string(),
                        })
                    })
            })
    }

    pub(crate) fn resolve_source_type_key(
        &self,
        name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Option<SourceSymbolKey> {
        let name = name.trim();
        let service_name = name.strip_prefix("root.").unwrap_or(name);
        if let Some((module_path, symbol)) = service_name.rsplit_once('.') {
            let key = SourceSymbolKey::new(module_path, symbol);
            return self.source_types.contains_key(&key).then_some(key);
        }
        let key = SourceSymbolKey::new(context.module_path, service_name);
        self.source_types.contains_key(&key).then_some(key)
    }

    fn resolved_named_type(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Option<ResolvedNamedType<'_>> {
        match ty {
            TypeRefIr::LocalType { type_index } => {
                let resolution = self.local_type_resolution(context.module_path, *type_index)?;
                Some(ResolvedNamedType {
                    resolution,
                    source_module_path: resolution.module_path.clone(),
                    package_root: None,
                    visit_key: InterfaceTypeVisitKey::Source(SourceSymbolKey::new(
                        &resolution.module_path,
                        &resolution.name,
                    )),
                })
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let resolution = self.local_type_resolution(module_path, *type_index)?;
                Some(ResolvedNamedType {
                    resolution,
                    source_module_path: resolution.module_path.clone(),
                    package_root: None,
                    visit_key: InterfaceTypeVisitKey::Source(SourceSymbolKey::new(
                        &resolution.module_path,
                        &resolution.name,
                    )),
                })
            }
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                let key = SourceSymbolKey::new(module_path, &symbol.symbol);
                self.source_types
                    .get(&key)
                    .map(|resolution| ResolvedNamedType {
                        resolution,
                        source_module_path: module_path.to_string(),
                        package_root: None,
                        visit_key: InterfaceTypeVisitKey::Source(key),
                    })
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => package_id.as_str(),
                };
                let resolution =
                    self.package_type_resolution(dependency_ref, &symbol.symbol_path)?;
                let package_id = self
                    .package_dependencies
                    .get(dependency_ref)
                    .map(String::as_str)
                    .unwrap_or(dependency_ref);
                Some(ResolvedNamedType {
                    resolution,
                    source_module_path: resolution.module_path.clone(),
                    package_root: package_root_for_symbol(
                        symbol,
                        &self.package_dependencies,
                        &self.package_dependency_access,
                    ),
                    visit_key: InterfaceTypeVisitKey::Package(PackageSymbolKey {
                        dependency_ref: package_id.to_string(),
                        symbol_path: source_path(&resolution.module_path, &resolution.name),
                    }),
                })
            }
            TypeRefIr::AppliedNominal { base, .. } => {
                self.resolved_named_type(&nominal_base_type_ref(base), context)
            }
            _ => None,
        }
    }

    fn resolve_db_object_symbol(
        &self,
        name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<ServiceSymbolRef>, String> {
        let Some(module) = self.modules.get(context.module_path) else {
            return Ok(None);
        };
        Ok(module.local_db_objects.resolve(name))
    }

    fn expand_alias_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<String, String> {
        let Some(module) = self.modules.get(context.module_path) else {
            return Ok(raw.to_string());
        };
        expand_alias_text(raw, &module.alias_targets)
    }

    fn index_source_interface_conformances(
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

    fn index_local_impl_methods(
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
                    let signature =
                        self.local_impl_method_signature(&receiver, method, &context)?;
                    receiver_methods.insert(method.name.clone(), signature);
                }
            }
        }
        Ok(methods_by_receiver)
    }

    fn local_impl_method_signature(
        &self,
        receiver: &SourceSymbolKey,
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
            type_params: method.type_params.clone(),
            params,
            return_type,
        })
    }

    fn resolve_impl_method_type_ref(
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

    /// Classifies one validated `implements` selector without inferring owner
    /// from display strings or retrying another owner after a failed handoff.
    pub(crate) fn classify_canonical_interface_owner(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> CanonicalInterfaceOwnerResolution {
        let semantic_interface = match self
            .interface_semantics
            .canonical_source_interface_instantiation_from_type_ref(
                context.module_path,
                &TypeRef {
                    name: raw.to_string(),
                },
                &context.type_params,
            ) {
            Ok(interface) => interface,
            Err(error) => {
                return CanonicalInterfaceOwnerResolution::InvalidOrUnresolved {
                    message: error.to_string(),
                };
            }
        };
        match self
            .interface_semantics
            .interface_owner_kind(&semantic_interface.symbol)
        {
            Some(InterfaceOwnerKind::Source) => {
                let interface = match self.resolve_interface_instantiation_text(raw, context) {
                    Ok(Some(interface)) => interface,
                    Ok(None) => {
                        return CanonicalInterfaceOwnerResolution::InvalidOrUnresolved {
                            message: format!("implements entry `{raw}` is not an interface"),
                        };
                    }
                    Err(message) => {
                        return CanonicalInterfaceOwnerResolution::InvalidOrUnresolved { message };
                    }
                };
                CanonicalInterfaceOwnerResolution::SourceDeclaredExact {
                    interface: semantic_interface.symbol,
                    arguments: interface.args,
                }
            }
            Some(InterfaceOwnerKind::CompilerKnown) => {
                CanonicalInterfaceOwnerResolution::CompilerKnown {
                    interface: semantic_interface.symbol,
                    arguments: semantic_interface.args,
                }
            }
            Some(InterfaceOwnerKind::External) => {
                let interface = match self.resolve_interface_instantiation_text(raw, context) {
                    Ok(interface) => interface,
                    Err(message) => {
                        return CanonicalInterfaceOwnerResolution::InvalidOrUnresolved { message };
                    }
                };
                match interface {
                    Some(interface)
                        if matches!(&interface.identity, TypeRefIr::PackageSymbol { .. }) =>
                    {
                        CanonicalInterfaceOwnerResolution::TypedPackage {
                            identity: interface.identity,
                            arguments: interface.args,
                        }
                    }
                    _ => CanonicalInterfaceOwnerResolution::InvalidOrUnresolved {
                        message: format!("implements entry `{raw}` is not an interface"),
                    },
                }
            }
            None => CanonicalInterfaceOwnerResolution::InvalidOrUnresolved {
                message: format!("implements entry `{raw}` is not an interface"),
            },
        }
    }

    fn resolve_interface_instantiation_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<InterfaceInstantiationResolution>, String> {
        let TypeExpr::Named { name, args } = TypeExpr::parse(raw) else {
            return Ok(None);
        };
        let arguments = self.resolve_interface_selector_args(&args, context)?;
        if let Some(key) = self.resolve_source_type_key(&name, context) {
            if !self.source_type_is_interface(&key) {
                return Ok(None);
            }
            let resolution = self
                .source_types
                .get(&key)
                .ok_or_else(|| format!("missing source interface declaration `{key}`"))?;
            if resolution.type_params.len() != arguments.len() {
                return Err(format!(
                    "interface `{name}` expects {} type arguments, found {}",
                    resolution.type_params.len(),
                    arguments.len()
                ));
            }
            return Ok(Some(InterfaceInstantiationResolution {
                identity: interface_symbol_type_ref(&key),
                args: arguments,
            }));
        }
        if let Some((alias, schema_type)) = self.service_api_type(&name)? {
            let Some(interface) = self.service_api_interface(alias, &schema_type.stable_schema_key)
            else {
                return Ok(None);
            };
            self.require_package_interface_type_args(raw, &interface.type_params, &arguments)?;
            return Ok(Some(InterfaceInstantiationResolution {
                identity: interface.identity,
                args: arguments,
            }));
        }
        let Some(interface) = self.resolve_package_interface(&name) else {
            return Ok(None);
        };
        self.require_package_interface_type_args(raw, &interface.type_params, &arguments)?;
        Ok(Some(InterfaceInstantiationResolution {
            identity: interface.identity,
            args: arguments,
        }))
    }

    fn interface_instantiation_from_resolved(
        &self,
        resolved: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<InterfaceInstantiationResolution>, String> {
        if let TypeRefIr::AnyInterface { interface } = &resolved.ir {
            let identity = serde_json::from_str(&interface.interface_abi_id).map_err(|error| {
                format!("resolved any-interface identity is not a canonical TypeRefIr: {error}")
            })?;
            return Ok(Some(InterfaceInstantiationResolution {
                identity,
                args: interface.canonical_type_args.clone(),
            }));
        }
        let Some(identity) = self.interface_identity_for_type_ref(&resolved.ir, context) else {
            return Ok(None);
        };
        let expected_arity = match &identity {
            TypeRefIr::ServiceSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                self.source_types
                    .get(&SourceSymbolKey::new(module_path, &symbol.symbol))
                    .map_or(0, |resolution| resolution.type_params.len())
            }
            TypeRefIr::PackageSymbol { symbol } => {
                let dependency_ref = match &symbol.package {
                    PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
                    PackageRefIr::PackageId { package_id } => package_id.as_str(),
                };
                self.package_interface_fact(dependency_ref, &symbol.symbol_path)
                    .map_or(0, |interface| interface.type_params.len())
            }
            _ => 0,
        };
        if expected_arity != 0 {
            return Err(format!(
                "resolved generic interface requires {expected_arity} structured type arguments"
            ));
        }
        Ok(Some(InterfaceInstantiationResolution {
            identity,
            args: Vec::new(),
        }))
    }

    fn interface_identity_for_type_ref(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Option<TypeRefIr> {
        match ty {
            TypeRefIr::LocalType { type_index } => {
                let resolution = self.local_type_resolution(context.module_path, *type_index)?;
                self.source_type_is_interface(&SourceSymbolKey::new(
                    &resolution.module_path,
                    &resolution.name,
                ))
                .then(|| TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: resolution.module_path.clone(),
                        symbol: resolution.name.clone(),
                    },
                })
            }
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let module_path = symbol
                    .module_path
                    .strip_prefix("root.")
                    .unwrap_or(&symbol.module_path);
                let key = SourceSymbolKey::new(module_path, &symbol.symbol);
                self.source_type_is_interface(&key)
                    .then(|| TypeRefIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: module_path.to_string(),
                            symbol: symbol.symbol.clone(),
                        },
                    })
            }
            TypeRefIr::PackageSymbol { .. } => self
                .package_interface_for_type_ref(ty)
                .map(|interface| interface.identity),
            _ => None,
        }
    }

    fn source_type_is_interface(&self, key: &SourceSymbolKey) -> bool {
        self.source_interfaces.contains(key)
    }
}

fn index_compiler_owned_package_artifacts(
    package_artifacts: Option<&[PackageArtifact]>,
    dependencies: &SourceDependencyAnalysisInput,
    package_types: &mut BTreeMap<PackageSymbolKey, SourceTypeResolution>,
    package_interfaces: &mut BTreeMap<PackageSymbolKey, PackageInterfaceFact>,
    package_type_slots: &mut BTreeMap<(String, String, u32), String>,
    package_type_source_paths: &mut BTreeMap<(String, String, String), String>,
    package_constants: &mut BTreeMap<PackageSymbolKey, PackageConstantResolution>,
    package_dependencies: &mut BTreeMap<String, String>,
    package_dependency_access: &mut BTreeMap<String, PackageDependencyAccess>,
    package_artifact_identities: &mut BTreeMap<String, (PackageLocalAbiIdentity, PackageBuildId)>,
) -> Result<(), String> {
    for (alias, expected_build_id, expected_local_abi) in
        dependencies.compiler_owned_package_owners()
    {
        let matches = package_artifacts
            .unwrap_or_default()
            .iter()
            .filter(|artifact| {
                &artifact.package_build_id == expected_build_id
                    && &artifact.package_local_abi.local_abi_identity == expected_local_abi
            })
            .collect::<Vec<_>>();
        let [artifact] = matches.as_slice() else {
            return Err(format!(
                "compiler-owned dependency alias `{alias}` requires exactly one verified package artifact owner, found {}",
                matches.len()
            ));
        };
        if package_dependencies.contains_key(alias)
            || package_artifact_identities.contains_key(alias)
        {
            return Err(format!(
                "compiler-owned dependency alias `{alias}` conflicts with a declared package owner"
            ));
        }
        let access = PackageDependencyAccess::Public;
        index_artifact_package_types(
            artifact,
            alias,
            access,
            ArtifactPackageTypePathMode::CompilerOwnedExact,
            package_types,
            package_interfaces,
            package_type_slots,
        )?;
        index_artifact_package_type_source_paths(
            artifact,
            alias,
            access,
            package_type_source_paths,
        )?;
        index_artifact_package_constants(artifact, alias, access, package_constants)?;
        package_dependencies.insert(alias.to_string(), artifact.package_id.clone());
        package_dependency_access.insert(alias.to_string(), access);
        package_artifact_identities.insert(
            alias.to_string(),
            (expected_local_abi.clone(), expected_build_id.clone()),
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ArtifactPackageTypePathMode {
    DeclaredPublic,
    CompilerOwnedExact,
}

fn index_artifact_package_types(
    artifact: &PackageArtifact,
    dependency_ref: &str,
    access: PackageDependencyAccess,
    path_mode: ArtifactPackageTypePathMode,
    package_types: &mut BTreeMap<PackageSymbolKey, SourceTypeResolution>,
    package_interfaces: &mut BTreeMap<PackageSymbolKey, PackageInterfaceFact>,
    package_type_slots: &mut BTreeMap<(String, String, u32), String>,
) -> Result<(), String> {
    let symbols = match access {
        PackageDependencyAccess::Public => &artifact.package_local_abi.public_symbols,
        PackageDependencyAccess::TopLevel => &artifact.package_local_abi.implementation_symbols,
    };
    let symbolic_types = artifact_symbolic_type_index(artifact, symbols)?;
    let type_symbols = artifact_package_type_symbol_index(artifact);
    for (selected_path, symbol) in symbols {
        if !matches!(symbol, PackageLocalAbiSymbol::Type { .. }) {
            continue;
        }
        let export = artifact
            .implementation_links
            .types
            .get(selected_path)
            .ok_or_else(|| {
                format!(
                    "package {} selected type {} has no exact implementation link",
                    artifact.package_id, selected_path
                )
            })?;
        let key = (
            dependency_ref.to_string(),
            export.file.module_path.clone(),
            export.type_index,
        );
        if let Some(existing) = package_type_slots.insert(key, selected_path.clone()) {
            return Err(format!(
                "package {} local type slot for {} is ambiguously exported as {} and {}",
                artifact.package_id, export.file.module_path, existing, selected_path
            ));
        }
    }
    let local_type_names = symbols
        .iter()
        .filter_map(|(path, symbol)| {
            matches!(symbol, PackageLocalAbiSymbol::Type { .. }).then(|| path.clone())
        })
        .collect::<BTreeSet<_>>();
    for (selected_path, symbol) in symbols {
        let PackageLocalAbiSymbol::Type {
            local_type_id,
            descriptor,
            is_alias,
            is_interface,
            type_params,
            interface_methods,
        } = symbol
        else {
            continue;
        };
        let expected_type_id = match access {
            PackageDependencyAccess::Public => format!("type:{selected_path}"),
            PackageDependencyAccess::TopLevel => {
                format!("type:{}:top-level:{selected_path}", artifact.package_id)
            }
        };
        if local_type_id != &expected_type_id {
            return Err(format!(
                "package {} exported type {} has mismatched local type identity {}",
                artifact.package_id, selected_path, local_type_id
            ));
        }
        let name = selected_path.rsplit('.').next().unwrap_or(selected_path);
        let module_path = selected_path
            .rsplit_once('.')
            .map_or("", |(module, _)| module);
        let export = artifact
            .implementation_links
            .types
            .get(selected_path)
            .expect("symbolic type index validated the implementation link");
        let kind = artifact_type_kind(
            descriptor,
            &symbolic_types,
            &artifact.package_id,
            &type_symbols,
            &export.file.module_path,
            selected_path,
            *is_alias,
        )
        .map_err(|message| {
            format!(
                "package {} exported type {} has unusable descriptor: {message}",
                artifact.package_id, selected_path
            )
        })?;
        let resolution = SourceTypeResolution {
            name: name.to_string(),
            type_params: type_params.clone(),
            local_type_names: local_type_names.clone(),
            kind,
            module_path: module_path.to_string(),
            public_path: Some(selected_path.clone()),
        };
        let indexed_paths = match (access, path_mode) {
            (PackageDependencyAccess::Public, ArtifactPackageTypePathMode::DeclaredPublic) => {
                vec![selected_path.as_str(), name]
            }
            (PackageDependencyAccess::Public, ArtifactPackageTypePathMode::CompilerOwnedExact)
            | (PackageDependencyAccess::TopLevel, _) => vec![selected_path.as_str()],
        };
        for path in indexed_paths.into_iter().collect::<BTreeSet<_>>() {
            let key = PackageSymbolKey {
                dependency_ref: dependency_ref.to_string(),
                symbol_path: path.to_string(),
            };
            if package_types
                .insert(key.clone(), resolution.clone())
                .is_some()
            {
                return Err(format!(
                    "package {} has duplicate or ambiguous public type path {}",
                    artifact.package_id, path
                ));
            }
            if *is_interface {
                let methods = reconstruct_artifact_interface_methods(
                    &artifact.package_id,
                    selected_path,
                    interface_methods,
                )?;
                let fact = PackageInterfaceFact {
                    type_params: type_params.clone(),
                    methods,
                    source_module: export.file.module_path.clone(),
                };
                if package_interfaces.insert(key, fact).is_some() {
                    return Err(format!(
                        "package {} has duplicate or ambiguous public interface path {}",
                        artifact.package_id, path
                    ));
                }
            } else if !interface_methods.is_empty() {
                return Err(format!(
                    "package {} public type {} carries interface methods without interface classification",
                    artifact.package_id, selected_path
                ));
            }
        }
    }
    Ok(())
}

fn index_artifact_package_type_source_paths(
    artifact: &PackageArtifact,
    dependency_ref: &str,
    access: PackageDependencyAccess,
    source_paths: &mut BTreeMap<(String, String, String), String>,
) -> Result<(), String> {
    let symbols = match access {
        PackageDependencyAccess::Public => &artifact.package_local_abi.public_symbols,
        PackageDependencyAccess::TopLevel => &artifact.package_local_abi.implementation_symbols,
    };
    for (public_path, symbol) in symbols {
        if !matches!(symbol, PackageLocalAbiSymbol::Type { .. }) {
            continue;
        }
        let export = artifact
            .implementation_links
            .types
            .get(public_path)
            .ok_or_else(|| {
                format!(
                    "package {} selected type {} has no exact implementation link",
                    artifact.package_id, public_path
                )
            })?;
        let key = (
            dependency_ref.to_string(),
            export.file.module_path.clone(),
            export.symbol.clone(),
        );
        if let Some(existing) = source_paths.insert(key, public_path.clone()) {
            return Err(format!(
                "package {} source type {}.{} is ambiguously exported as {} and {}",
                artifact.package_id, export.file.module_path, export.symbol, existing, public_path
            ));
        }
    }
    Ok(())
}

fn index_artifact_package_constants(
    artifact: &PackageArtifact,
    dependency_ref: &str,
    access: PackageDependencyAccess,
    package_constants: &mut BTreeMap<PackageSymbolKey, PackageConstantResolution>,
) -> Result<(), String> {
    let symbols = match access {
        PackageDependencyAccess::Public => &artifact.package_local_abi.public_symbols,
        PackageDependencyAccess::TopLevel => &artifact.package_local_abi.implementation_symbols,
    };
    for (selected_path, symbol) in symbols {
        let PackageLocalAbiSymbol::Constant { ty, .. } = symbol else {
            continue;
        };
        if !artifact
            .implementation_links
            .constants
            .contains_key(selected_path)
        {
            return Err(format!(
                "package {} selected constant {} has no exact implementation link",
                artifact.package_id, selected_path
            ));
        }
        let key = PackageSymbolKey {
            dependency_ref: dependency_ref.to_string(),
            symbol_path: selected_path.clone(),
        };
        let resolution = PackageConstantResolution {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: dependency_ref.to_string(),
                },
                symbol_path: selected_path.clone(),
                abi_expectation: Some(
                    artifact
                        .package_local_abi
                        .local_abi_identity
                        .as_str()
                        .to_string(),
                ),
            },
            ty: ty.clone(),
        };
        if package_constants.insert(key, resolution).is_some() {
            return Err(format!(
                "package {} has duplicate or ambiguous selected constant path {}",
                artifact.package_id, selected_path
            ));
        }
    }
    Ok(())
}

fn reconstruct_artifact_interface_methods(
    package_id: &str,
    public_path: &str,
    methods: &[InterfaceMethodSignature],
) -> Result<Vec<InterfaceMethodSignature>, String> {
    let mut method_names = BTreeSet::new();
    methods
        .iter()
        .map(|method| {
            if !method_names.insert(method.name.as_str()) {
                return Err(format!(
                    "package {package_id} exported interface {public_path} has duplicate method {}",
                    method.name
                ));
            }
            let mut method = method.clone();
            if method.is_static {
                if method.implicit_self.is_some()
                    || method.params.iter().any(|param| param.name == "self")
                {
                    return Err(format!(
                        "package {package_id} exported interface {public_path} static method {} carries a receiver",
                        method.name
                    ));
                }
                return Ok(method);
            }
            let explicit_receivers = method
                .params
                .iter()
                .enumerate()
                .filter(|(_, param)| param.name == "self")
                .collect::<Vec<_>>();
            if method.implicit_self.is_some() && !explicit_receivers.is_empty()
                || explicit_receivers.len() > 1
            {
                return Err(format!(
                    "package {package_id} exported interface {public_path} method {} has duplicate receivers",
                    method.name
                ));
            }
            if let Some(receiver) = method.implicit_self.as_mut() {
                canonicalize_artifact_self_type(receiver).map_err(|actual| {
                    format!(
                        "package {package_id} exported interface {public_path} method {} has non-Self receiver {actual}",
                        method.name
                    )
                })?;
                return Ok(method);
            }
            let Some((index, _receiver)) = explicit_receivers.into_iter().next() else {
                return Err(format!(
                    "package {package_id} exported interface {public_path} method {} is missing self: Self",
                    method.name
                ));
            };
            if index != 0 {
                return Err(format!(
                    "package {package_id} exported interface {public_path} method {} has a non-leading receiver",
                    method.name
                ));
            }
            canonicalize_artifact_self_type(&mut method.params[0].ty).map_err(|actual| {
                format!(
                    "package {package_id} exported interface {public_path} method {} has non-Self receiver {actual}",
                    method.name
                )
            })?;
            Ok(method)
        })
        .collect()
}

fn canonicalize_artifact_self_type(ty: &mut TypeRefIr) -> Result<(), String> {
    match ty {
        TypeRefIr::TypeParam { name } if name == "Self" => {
            *ty = TypeRefIr::builtin("Self");
            Ok(())
        }
        TypeRefIr::Builtin { name, args } if name == "Self" && args.is_empty() => Ok(()),
        actual => Err(format!("{actual:?}")),
    }
}

#[derive(Default)]
struct ArtifactSymbolicTypeIndex {
    by_symbol: BTreeMap<(String, String), String>,
    by_slot: BTreeMap<(String, u32), String>,
}

fn artifact_package_type_symbol_index(artifact: &PackageArtifact) -> PackageTypeSymbolIndex {
    let mut index = PackageTypeSymbolIndex::default();
    for requirement in &artifact.package_requirements {
        index.insert_dependency(&requirement.alias, &requirement.package_id);
        index.insert_dependency(&requirement.package_id, &requirement.package_id);
    }
    for (public_path, export) in &artifact.implementation_links.types {
        index.insert_type(
            &export.file.module_path,
            export.type_index,
            &export.symbol,
            public_path,
        );
    }
    index
}

fn artifact_symbolic_type_index(
    artifact: &PackageArtifact,
    symbols: &BTreeMap<String, PackageLocalAbiSymbol>,
) -> Result<ArtifactSymbolicTypeIndex, String> {
    let mut index = ArtifactSymbolicTypeIndex::default();
    for (selected_path, symbol) in symbols {
        let PackageLocalAbiSymbol::Type {
            descriptor,
            is_interface,
            type_params,
            interface_methods,
            ..
        } = symbol
        else {
            continue;
        };
        let export = artifact
            .implementation_links
            .types
            .get(selected_path)
            .ok_or_else(|| {
                format!(
                    "package {} selected type {} has no exact implementation link",
                    artifact.package_id, selected_path
                )
            })?;
        if export.descriptor.as_ref() != Some(descriptor) {
            return Err(format!(
                "package {} selected type {} descriptor disagrees with its implementation link",
                artifact.package_id, selected_path
            ));
        }
        if export.is_interface != *is_interface
            || export.type_params != *type_params
            || export.interface_methods != *interface_methods
        {
            return Err(format!(
                "package {} selected type {} interface facts disagree with its implementation link",
                artifact.package_id, selected_path
            ));
        }
        if export.symbol.is_empty() || export.file.module_path.is_empty() {
            return Err(format!(
                "package {} selected type {} has an incomplete implementation link",
                artifact.package_id, selected_path
            ));
        }
        let symbol_name = export
            .symbol
            .strip_prefix(&format!("{}.", export.file.module_path))
            .unwrap_or(&export.symbol)
            .to_string();
        let symbol_key = (export.file.module_path.clone(), symbol_name);
        if let Some(existing) = index
            .by_symbol
            .insert(symbol_key.clone(), selected_path.clone())
        {
            return Err(format!(
                "package {} selected types {} and {} ambiguously identify {}.{}",
                artifact.package_id, existing, selected_path, symbol_key.0, symbol_key.1
            ));
        }
        let slot_key = (export.file.module_path.clone(), export.type_index);
        if let Some(existing) = index
            .by_slot
            .insert(slot_key.clone(), selected_path.clone())
        {
            return Err(format!(
                "package {} selected types {} and {} ambiguously identify {}#{}",
                artifact.package_id, existing, selected_path, slot_key.0, slot_key.1
            ));
        }
    }
    Ok(index)
}

fn artifact_type_kind(
    descriptor: &TypeDescriptorIr,
    symbolic_types: &ArtifactSymbolicTypeIndex,
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    public_path: &str,
    is_alias: bool,
) -> Result<SourceTypeKind, String> {
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            if is_alias {
                return Err("transparent alias carries a record declaration descriptor".to_string());
            }
            Ok(SourceTypeKind::Record {
                fields: fields
                    .iter()
                    .map(|(name, ty)| {
                        Ok((
                            name.clone(),
                            artifact_type_text(package_id, ty, symbolic_types)?,
                        ))
                    })
                    .collect::<Result<_, String>>()?,
                canonical_fields: Some(
                    fields
                        .iter()
                        .map(|(name, ty)| {
                            Ok((
                                name.clone(),
                                normalize_package_interface_type_ref(
                                    package_id,
                                    type_symbols,
                                    module_path,
                                    ty,
                                    &format!("record {public_path}.{name}"),
                                )?,
                            ))
                        })
                        .collect::<Result<_, String>>()?,
                ),
            })
        }
        TypeDescriptorIr::Alias { target } if is_alias => Ok(SourceTypeKind::Alias {
            target: artifact_type_text(package_id, target, symbolic_types)?,
            canonical_target: Some(normalize_package_interface_type_ref(
                package_id,
                type_symbols,
                module_path,
                target,
                &format!("alias {public_path}"),
            )?),
        }),
        TypeDescriptorIr::Alias { .. } => Err(format!(
            "package {package_id} nominal type {public_path} carries an alias descriptor"
        )),
        TypeDescriptorIr::Representation { representation } => {
            if is_alias {
                return Err(format!(
                    "package {package_id} transparent alias {public_path} carries a representation descriptor"
                ));
            }
            Ok(SourceTypeKind::Representation {
                target: artifact_type_text(package_id, representation, symbolic_types)?,
                named_union_branches: None,
                discriminator: None,
            })
        }
        TypeDescriptorIr::Union { branches } => {
            if is_alias {
                return Err(format!(
                    "package {package_id} transparent alias {public_path} carries a named union descriptor"
                ));
            }
            Ok(SourceTypeKind::Representation {
                target: branches
                    .iter()
                    .map(|branch| {
                        artifact_named_union_branch_text(package_id, branch, symbolic_types)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join(" | "),
                named_union_branches: Some(branches.clone()),
                discriminator: None,
            })
        }
        TypeDescriptorIr::Interface => {
            if is_alias {
                return Err(format!(
                    "package {package_id} transparent alias {public_path} carries an interface descriptor"
                ));
            }
            Ok(SourceTypeKind::External)
        }
    }
}

fn artifact_named_union_branch_text(
    package_id: &str,
    branch: &NamedUnionBranchIr,
    symbolic_types: &ArtifactSymbolicTypeIndex,
) -> Result<String, String> {
    match branch {
        NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
            artifact_type_text(package_id, nominal_type, symbolic_types)
        }
        NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
            artifact_type_text(package_id, payload_type, symbolic_types)
        }
        NamedUnionBranchIr::Literal { value } => artifact_type_text(
            package_id,
            &TypeRefIr::Literal {
                value: value.clone(),
            },
            symbolic_types,
        ),
    }
}

fn artifact_type_text(
    package_id: &str,
    ty: &TypeRefIr,
    symbolic_types: &ArtifactSymbolicTypeIndex,
) -> Result<String, String> {
    match ty {
        TypeRefIr::Builtin { name, args } if args.is_empty() => Ok(name.clone()),
        TypeRefIr::Builtin { name, args } => Ok(format!(
            "{name}<{}>",
            args.iter()
                .map(|arg| artifact_type_text(package_id, arg, symbolic_types))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )),
        TypeRefIr::AppliedNominal { base, arguments } => Ok(format!(
            "{}<{}>",
            artifact_type_text(package_id, &nominal_base_type_ref(base), symbolic_types)?,
            arguments
                .iter()
                .map(|argument| artifact_type_text(package_id, argument, symbolic_types))
                .collect::<Result<Vec<_>, _>>()?
                .join(", ")
        )),
        TypeRefIr::ServiceSymbol { symbol } => symbolic_types
            .by_symbol
            .get(&(symbol.module_path.clone(), symbol.symbol.clone()))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "service symbol {}.{} is not an identity-validated selected artifact type",
                    symbol.module_path, symbol.symbol
                )
            }),
        TypeRefIr::DbObjectSymbol { symbol } => Err(format!(
            "db object symbol {}.{} has no package type semantics",
            symbol.module_path, symbol.symbol
        )),
        TypeRefIr::PackageSymbol { symbol } => match &symbol.package {
            PackageRefIr::PackageId { package_id: owner } if owner == package_id => {
                Ok(symbol.symbol_path.clone())
            }
            PackageRefIr::PackageId { package_id }
                if package_id == SKIFF_STD_PUBLICATION_ID =>
            {
                Ok(symbol.symbol_path.clone())
            }
            PackageRefIr::Dependency { dependency_ref } => {
                Ok(format!("{dependency_ref}.{}", symbol.symbol_path))
            }
            PackageRefIr::PackageId { package_id } => {
                Ok(format!("{package_id}.{}", symbol.symbol_path))
            }
        },
        TypeRefIr::Record { fields } => Ok(format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(name, ty)| {
                    Ok(format!(
                        "{name}: {}",
                        artifact_type_text(package_id, ty, symbolic_types)?
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?
                .join(", ")
        )),
        TypeRefIr::Union { items } => Ok(items
            .iter()
            .map(|item| artifact_type_text(package_id, item, symbolic_types))
            .collect::<Result<Vec<_>, _>>()?
            .join(" | ")),
        TypeRefIr::Nullable { inner } => Ok(format!(
            "{}?",
            artifact_type_text(package_id, inner, symbolic_types)?
        )),
        TypeRefIr::Literal {
            value: LiteralIr::String { value },
        } => serde_json::to_string(value).map_err(|error| error.to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Bool { value },
        } => Ok(value.to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Number { value },
        } => Ok(value.to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Null,
        } => Ok("null".to_string()),
        TypeRefIr::TypeParam { name } => Ok(name.clone()),
        TypeRefIr::LocalType { type_index } => Err(format!(
            "local type index {type_index} is not self-describing without an owner module"
        )),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => symbolic_types
            .by_slot
            .get(&(module_path.clone(), *type_index))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "publication type {module_path}#{type_index} is not an identity-validated selected artifact type"
                )
            }),
        TypeRefIr::AnyInterface { interface } => {
            let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(
                |error| {
                    format!(
                        "any-interface identity is not a canonical artifact type reference: {error}"
                    )
                },
            )?;
            let name = artifact_type_text(package_id, &identity, symbolic_types)?;
            if interface.canonical_type_args.is_empty() {
                Ok(format!("any {name}"))
            } else {
                Ok(format!(
                    "any {name}<{}>",
                    interface
                        .canonical_type_args
                        .iter()
                        .map(|arg| artifact_type_text(package_id, arg, symbolic_types))
                        .collect::<Result<Vec<_>, _>>()?
                        .join(", ")
                ))
            }
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => Ok(format!(
            "fn({}) -> {}",
            params
                .iter()
                .map(|param| {
                    Ok(format!(
                        "{}: {}",
                        param.name,
                        artifact_type_text(package_id, &param.ty, symbolic_types)?
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?
                .join(", "),
            artifact_type_text(package_id, return_type, symbolic_types)?
        )),
        other => Err(format!("unsupported artifact type reference {other:?}")),
    }
}

fn interface_identity_matches_source_symbol(
    identity: &TypeRefIr,
    interface_symbol: &ServiceSymbolRef,
) -> bool {
    matches!(
        identity,
        TypeRefIr::ServiceSymbol { symbol }
            if symbol.module_path == interface_symbol.module_path
                && symbol.symbol == interface_symbol.symbol
    )
}

fn method_slot_resolution(
    interface: InterfaceInstantiationRef,
    slot: InterfaceMethodSlotFact,
) -> AnyInterfaceMethodResolution {
    AnyInterfaceMethodResolution {
        interface,
        slot: slot.slot,
        method_abi_id: slot.method_abi_id,
        params: slot.params,
        return_type: slot.return_type,
    }
}

fn interface_symbol_type_ref(symbol: &SourceSymbolKey) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: symbol.module_path().to_string(),
            symbol: symbol.symbol().to_string(),
        },
    }
}

fn source_type_kind_label(kind: &SourceTypeKind) -> &'static str {
    match kind {
        SourceTypeKind::Record { .. } => "concrete type",
        SourceTypeKind::Actor { .. } => "actor nominal handle",
        SourceTypeKind::Representation { .. } => "concrete representation type",
        SourceTypeKind::Alias { .. } => "alias",
        SourceTypeKind::External => "non-interface type",
    }
}

struct ConstructorShape {
    module_path: String,
    type_params: Vec<String>,
    fields: BTreeMap<String, String>,
    canonical_fields: Option<BTreeMap<String, TypeRefIr>>,
}

struct RepresentationShape {
    module_path: String,
    type_params: Vec<String>,
    payload: String,
}

fn prelude_constructor_shape(type_name: &str) -> Option<ConstructorShape> {
    let registry = prelude_registry();
    let canonical;
    let lookup_name = if registry.is_bare_raw_http_envelope_type(type_name) {
        canonical = registry.known_type_symbol(type_name)?;
        canonical.as_str()
    } else {
        type_name
    };
    let ty = registry.type_decl(lookup_name)?;
    if ty.alias.is_some() {
        return None;
    }
    let module_path = registry.type_decl_module(lookup_name)?.to_string();
    Some(ConstructorShape {
        module_path,
        type_params: ty.type_params.clone(),
        fields: ty
            .fields
            .iter()
            .map(|field| (field.name.clone(), field.ty.name.clone()))
            .collect(),
        canonical_fields: None,
    })
}

fn prelude_representation_shape(type_name: &str) -> Option<RepresentationShape> {
    let registry = prelude_registry();
    let ty = registry.type_decl(type_name)?;
    let alias = ty.alias.as_ref()?;
    let module_path = registry.type_decl_module(type_name)?.to_string();
    Some(RepresentationShape {
        module_path,
        type_params: ty.type_params.clone(),
        payload: alias.name.clone(),
    })
}

impl TypeResolutionContext<'_> {
    pub fn source(module_path: &str) -> TypeResolutionContext<'_> {
        TypeResolutionContext {
            module_path,
            type_params: BTreeSet::new(),
        }
    }

    pub fn with_type_params(
        module_path: &str,
        type_params: BTreeSet<String>,
    ) -> TypeResolutionContext<'_> {
        TypeResolutionContext {
            module_path,
            type_params,
        }
    }
}

fn index_source_types(
    module_path: &str,
    ast: &SourceFile,
    source_types: &mut BTreeMap<SourceSymbolKey, SourceTypeResolution>,
) {
    for ty in &ast.types {
        source_types.insert(
            SourceSymbolKey::new(module_path, &ty.name),
            source_type_resolution(module_path, &ty.name, &ty.type_params, ty),
        );
    }
    for actor in &ast.actors {
        source_types.insert(
            SourceSymbolKey::new(module_path, &actor.name),
            SourceTypeResolution {
                name: actor.name.clone(),
                type_params: Vec::new(),
                local_type_names: BTreeSet::new(),
                kind: SourceTypeKind::Actor {
                    id_type: actor.id_type.name.clone(),
                    fields: actor
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), field.ty.name.clone()))
                        .collect(),
                },
                module_path: module_path.to_string(),
                public_path: None,
            },
        );
    }
    for alias in &ast.aliases {
        source_types.insert(
            SourceSymbolKey::new(module_path, &alias.name),
            alias_type_resolution(module_path, alias),
        );
    }
    for interface in &ast.interfaces {
        source_types.insert(
            SourceSymbolKey::new(module_path, &interface.name),
            SourceTypeResolution {
                name: interface.name.clone(),
                type_params: interface.type_params.clone(),
                local_type_names: BTreeSet::new(),
                kind: SourceTypeKind::External,
                module_path: module_path.to_string(),
                public_path: None,
            },
        );
    }
}

fn index_source_interfaces(
    module_path: &str,
    ast: &SourceFile,
    source_interfaces: &mut BTreeSet<SourceSymbolKey>,
) {
    for interface in &ast.interfaces {
        source_interfaces.insert(SourceSymbolKey::new(module_path, &interface.name));
    }
}

fn index_package_types(
    package: &TypeResolutionPackageFacts<'_>,
    package_types: &mut BTreeMap<PackageSymbolKey, SourceTypeResolution>,
) -> Result<(), String> {
    let type_symbols = package_type_symbol_index(package)?;
    for binding in &package.schema_types {
        let Some(mut resolution) = package_source_type_resolution(
            binding.source_ast,
            binding.source_module,
            binding.source_symbol,
            Some(binding.public_path.to_string()),
        ) else {
            continue;
        };
        if binding.kind == PublicTypeKind::Alias {
            let SourceTypeKind::Alias {
                target,
                canonical_target,
            } = &mut resolution.kind
            else {
                return Err(format!(
                    "package {} public alias {} does not resolve to a source alias declaration",
                    package.package_id, binding.public_path
                ));
            };
            *canonical_target = Some(package_fact_alias_target(
                package,
                binding,
                &type_symbols,
                target,
            )?);
        }
        for path in [
            binding.public_path.to_string(),
            source_path(binding.source_module, binding.source_symbol),
            binding.source_symbol.to_string(),
        ] {
            package_types.insert(
                PackageSymbolKey {
                    dependency_ref: package.package_id.to_string(),
                    symbol_path: path,
                },
                resolution.clone(),
            );
        }
    }
    Ok(())
}

fn package_fact_alias_target(
    package: &TypeResolutionPackageFacts<'_>,
    binding: &TypeResolutionPackageSchemaTypeFact<'_>,
    type_symbols: &PackageTypeSymbolIndex,
    source_target: &str,
) -> Result<TypeRefIr, String> {
    if let Some(unit) = binding.file_ir_unit {
        let declaration = unit
            .declarations
            .types
            .get(binding.source_symbol)
            .and_then(|declaration| unit.type_table.get(declaration.type_index as usize))
            .ok_or_else(|| {
                format!(
                    "package {} public alias {} has no File IR declaration",
                    package.package_id, binding.public_path
                )
            })?;
        let target = match &declaration.descriptor {
            TypeDescriptorIr::Alias { target } => target.clone(),
            TypeDescriptorIr::Record { .. }
            | TypeDescriptorIr::Representation { .. }
            | TypeDescriptorIr::Union { .. }
            | TypeDescriptorIr::Interface => {
                return Err(format!(
                    "package {} public alias {} has a non-alias descriptor",
                    package.package_id, binding.public_path
                ));
            }
        };
        return normalize_package_interface_type_ref(
            package.package_id,
            type_symbols,
            binding.source_module,
            &target,
            &format!("alias {}", binding.public_path),
        );
    }
    resolve_package_fact_alias_expr(
        package,
        binding.source_module,
        &TypeExpr::parse(source_target),
    )
}

fn resolve_package_fact_alias_expr(
    package: &TypeResolutionPackageFacts<'_>,
    source_module: &str,
    expr: &TypeExpr,
) -> Result<TypeRefIr, String> {
    match expr {
        TypeExpr::EmptyRecord => Ok(TypeRefIr::Record {
            fields: BTreeMap::new(),
        }),
        TypeExpr::StringLiteral(value) => Ok(TypeRefIr::Literal {
            value: LiteralIr::String {
                value: value.clone(),
            },
        }),
        TypeExpr::Named { name, args } => {
            let args = args
                .iter()
                .map(|arg| resolve_package_fact_alias_expr(package, source_module, arg))
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(name) = builtin_type_name(name) {
                return Ok(TypeRefIr::Builtin { name, args });
            }
            if let Some((dependency_alias, symbol_path)) = name.split_once('.') {
                if let Some(dependency) = package
                    .dependencies
                    .iter()
                    .find(|dependency| dependency.alias == dependency_alias)
                {
                    if symbol_path.is_empty() {
                        return Err(format!(
                            "package {} alias target {name} has no dependency symbol path",
                            package.package_id
                        ));
                    }
                    if !args.is_empty() {
                        return Err(format!(
                            "package {} alias target {} cannot validate the exact generic declaration owned by dependency {}",
                            package.package_id,
                            expr.to_type_string(),
                            dependency.package_id
                        ));
                    }
                    return Ok(TypeRefIr::PackageSymbol {
                        symbol: PackageSymbolRef {
                            package: PackageRefIr::PackageId {
                                package_id: dependency.package_id.to_string(),
                            },
                            symbol_path: symbol_path.to_string(),
                            abi_expectation: None,
                        },
                    });
                }
            }
            let source_name = name.strip_prefix("root.").unwrap_or(name);
            let mut matches = package.schema_types.iter().filter(|candidate| {
                candidate.public_path == source_name
                    || source_path(candidate.source_module, candidate.source_symbol) == source_name
                    || (candidate.source_module == source_module
                        && candidate.source_symbol == source_name)
            });
            let Some(candidate) = matches.next() else {
                return Err(format!(
                    "package {} alias target `{name}` is missing or unresolved",
                    package.package_id
                ));
            };
            if matches.next().is_some() {
                return Err(format!(
                    "package {} alias target `{name}` is ambiguous",
                    package.package_id
                ));
            }
            let resolution = package_source_type_resolution(
                candidate.source_ast,
                candidate.source_module,
                candidate.source_symbol,
                Some(candidate.public_path.to_string()),
            )
            .ok_or_else(|| {
                format!(
                    "package {} alias target `{name}` has no exact source declaration",
                    package.package_id
                )
            })?;
            if resolution.type_params.len() != args.len() {
                return Err(format!(
                    "package {} alias target `{name}` expects {} type arguments, found {}",
                    package.package_id,
                    resolution.type_params.len(),
                    args.len()
                ));
            }
            if !args.is_empty()
                && (candidate.kind != PublicTypeKind::Type
                    || matches!(
                        resolution.kind,
                        SourceTypeKind::Actor { .. } | SourceTypeKind::External
                    ))
            {
                return Err(format!(
                    "package {} alias target `{name}` cannot be used as an applied nominal base",
                    package.package_id
                ));
            }
            apply_nominal_arguments(
                TypeRefIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::PackageId {
                            package_id: package.package_id.to_string(),
                        },
                        symbol_path: candidate.public_path.to_string(),
                        abi_expectation: None,
                    },
                },
                args,
            )
        }
        TypeExpr::Nullable(inner) => Ok(TypeRefIr::Nullable {
            inner: Box::new(resolve_package_fact_alias_expr(
                package,
                source_module,
                inner,
            )?),
        }),
        TypeExpr::Union(items) => Ok(TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| resolve_package_fact_alias_expr(package, source_module, item))
                .collect::<Result<Vec<_>, _>>()?,
        }),
        TypeExpr::AnyInterface { interface } => {
            let (identity, args) = match interface.as_ref() {
                TypeExpr::Named { name, args } => {
                    let identity = resolve_package_fact_alias_expr(
                        package,
                        source_module,
                        &TypeExpr::Named {
                            name: name.clone(),
                            args: Vec::new(),
                        },
                    )?;
                    let args = args
                        .iter()
                        .map(|arg| resolve_package_fact_alias_expr(package, source_module, arg))
                        .collect::<Result<Vec<_>, _>>()?;
                    (identity, args)
                }
                other => (
                    resolve_package_fact_alias_expr(package, source_module, other)?,
                    Vec::new(),
                ),
            };
            Ok(TypeRefIr::AnyInterface {
                interface: interface_instantiation_ref(identity, args),
            })
        }
        TypeExpr::Record(fields) => Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|field| {
                    Ok((
                        field.name.clone(),
                        resolve_package_fact_alias_expr(package, source_module, &field.ty)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, String>>()?,
        }),
        TypeExpr::Function {
            params,
            return_type,
        } => Ok(TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| {
                    Ok(FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: resolve_package_fact_alias_expr(package, source_module, &param.ty)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
            return_type: Box::new(resolve_package_fact_alias_expr(
                package,
                source_module,
                return_type,
            )?),
        }),
    }
}

fn index_package_callables(
    package: &TypeResolutionPackageFacts<'_>,
    package_callables: &mut BTreeMap<PackageSymbolKey, PackageCallableResolution>,
) {
    for binding in &package.callables {
        let Some(mut resolution) = package_callable_resolution(
            binding.source_ast,
            binding.source_module,
            binding.source_symbol,
        ) else {
            continue;
        };
        resolution.exact_signature = binding.exact_signature.cloned();
        for path in [
            binding.public_path.to_string(),
            source_path(binding.source_module, binding.source_symbol),
            binding.source_symbol.to_string(),
        ] {
            package_callables.insert(
                PackageSymbolKey {
                    dependency_ref: package.package_id.to_string(),
                    symbol_path: path,
                },
                resolution.clone(),
            );
        }
    }
}

fn index_package_public_to_internal(
    package: &TypeResolutionPackageFacts<'_>,
    package_public_to_internal: &mut BTreeMap<String, String>,
) {
    for binding in &package.schema_types {
        let internal = source_path(binding.source_module, binding.source_symbol);
        if binding.public_path != internal {
            package_public_to_internal.insert(binding.public_path.to_string(), internal);
        }
    }
}

fn index_package_interfaces(
    package: &TypeResolutionPackageFacts<'_>,
    package_interfaces: &mut BTreeMap<PackageSymbolKey, PackageInterfaceFact>,
) -> Result<(), String> {
    let type_symbols = package_type_symbol_index(package)?;
    for binding in &package.schema_types {
        if binding.kind != PublicTypeKind::Interface {
            continue;
        }
        let Some(unit) = binding.file_ir_unit else {
            continue;
        };
        let Some(interface) = unit.declarations.interfaces.get(binding.source_symbol) else {
            continue;
        };
        let mut methods = package_interface_method_signatures(
            package.package_id,
            &type_symbols,
            binding.source_module,
            interface,
        )?;
        restore_package_interface_source_receivers(
            &mut methods,
            binding.source_ast,
            binding.source_symbol,
        );
        let fact = PackageInterfaceFact {
            type_params: interface.type_params.clone(),
            methods,
            source_module: binding.source_module.to_string(),
        };
        for path in [
            binding.public_path.to_string(),
            source_path(binding.source_module, binding.source_symbol),
            binding.source_symbol.to_string(),
        ] {
            package_interfaces.insert(
                PackageSymbolKey {
                    dependency_ref: package.package_id.to_string(),
                    symbol_path: path,
                },
                fact.clone(),
            );
        }
    }
    Ok(())
}

fn index_package_type_slots(
    package: &TypeResolutionPackageFacts<'_>,
    package_type_slots: &mut BTreeMap<(String, String, u32), String>,
) -> Result<(), String> {
    for binding in &package.schema_types {
        let Some(type_index) = type_indices(binding.source_ast)
            .get(binding.source_symbol)
            .copied()
        else {
            continue;
        };
        let key = (
            package.package_id.to_string(),
            binding.source_module.to_string(),
            type_index,
        );
        if let Some(existing) = package_type_slots.insert(key, binding.public_path.to_string()) {
            return Err(format!(
                "package {} local type slot for {} is ambiguously exported as {} and {}",
                package.package_id, binding.source_module, existing, binding.public_path
            ));
        }
    }
    Ok(())
}

fn restore_package_interface_source_receivers(
    methods: &mut [InterfaceMethodSignature],
    source_ast: &SourceFile,
    interface_name: &str,
) {
    let Some(source_interface) = source_ast
        .interfaces
        .iter()
        .find(|interface| interface.name == interface_name)
    else {
        return;
    };
    for method in methods {
        let Some(source_method) = source_interface
            .operations
            .iter()
            .find(|operation| operation.name == method.name)
        else {
            continue;
        };
        if source_interface_operation_has_self_receiver(source_method) {
            normalize_package_interface_self_receiver(method);
        }
    }
}

fn normalize_package_interface_self_receiver(method: &mut InterfaceMethodSignature) {
    if let Some(param) = method
        .params
        .first_mut()
        .filter(|param| param.name == "self")
    {
        param.ty = TypeRefIr::builtin("Self");
        method.implicit_self = None;
    } else {
        method.implicit_self = Some(TypeRefIr::builtin("Self"));
    }
}

fn source_interface_operation_has_self_receiver(operation: &InterfaceOperation) -> bool {
    operation
        .params
        .first()
        .is_some_and(|param| param.name == "self" && param.ty.name == "Self")
        || operation
            .implicit_self
            .as_ref()
            .is_some_and(|ty| ty.name == "Self")
}

fn package_type_symbol_index(
    package: &TypeResolutionPackageFacts<'_>,
) -> Result<PackageTypeSymbolIndex, String> {
    let mut index = PackageTypeSymbolIndex::default();
    for dependency in &package.dependencies {
        index.insert_dependency(dependency.alias, dependency.package_id);
        index.insert_dependency(dependency.package_id, dependency.package_id);
    }
    for binding in &package.schema_types {
        let Some(unit) = binding.file_ir_unit else {
            continue;
        };
        let Some(target) = unit.link_targets.types.get(binding.source_symbol) else {
            continue;
        };
        let Some(type_decl) = unit.type_table.get(target.type_index as usize) else {
            return Err(format!(
                "package {} exported type {} points to missing type index {} in {}",
                package.package_id, binding.public_path, target.type_index, binding.source_module
            ));
        };
        index.insert_type(
            binding.source_module.to_string(),
            target.type_index,
            type_decl.name.clone(),
            binding.public_path.to_string(),
        );
    }
    Ok(index)
}

fn package_source_type_resolution(
    ast: &SourceFile,
    module_path: &str,
    source_symbol: &str,
    public_path: Option<String>,
) -> Option<SourceTypeResolution> {
    let local_type_names = local_type_names(ast);
    ast.types
        .iter()
        .find(|ty| ty.name == source_symbol)
        .map(|ty| source_type_resolution(module_path, &ty.name, &ty.type_params, ty))
        .or_else(|| {
            ast.aliases
                .iter()
                .find(|alias| alias.name == source_symbol)
                .map(|alias| alias_type_resolution(module_path, alias))
        })
        .or_else(|| {
            ast.interfaces
                .iter()
                .find(|interface| interface.name == source_symbol)
                .map(|interface| SourceTypeResolution {
                    name: interface.name.clone(),
                    type_params: interface.type_params.clone(),
                    local_type_names: BTreeSet::new(),
                    kind: SourceTypeKind::External,
                    module_path: module_path.to_string(),
                    public_path: None,
                })
        })
        .map(|mut resolution| {
            resolution.local_type_names = local_type_names;
            resolution.public_path = public_path;
            resolution
        })
}

fn source_type_resolution(
    module_path: &str,
    name: &str,
    type_params: &[String],
    ty: &TypeDecl,
) -> SourceTypeResolution {
    let kind = if let Some(alias) = &ty.alias {
        SourceTypeKind::Representation {
            target: alias.name.clone(),
            named_union_branches: None,
            discriminator: ty.discriminator.clone(),
        }
    } else {
        SourceTypeKind::Record {
            fields: ty
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.name.clone()))
                .collect(),
            canonical_fields: None,
        }
    };
    SourceTypeResolution {
        name: name.to_string(),
        type_params: type_params.to_vec(),
        local_type_names: BTreeSet::new(),
        kind,
        module_path: module_path.to_string(),
        public_path: None,
    }
}

fn alias_type_resolution(module_path: &str, alias: &AliasDecl) -> SourceTypeResolution {
    SourceTypeResolution {
        name: alias.name.clone(),
        type_params: Vec::new(),
        local_type_names: BTreeSet::new(),
        kind: SourceTypeKind::Alias {
            target: alias.target_type.name.clone(),
            canonical_target: None,
        },
        module_path: module_path.to_string(),
        public_path: None,
    }
}

fn local_type_names(ast: &SourceFile) -> BTreeSet<String> {
    ast.types
        .iter()
        .map(|ty| ty.name.clone())
        .chain(ast.aliases.iter().map(|alias| alias.name.clone()))
        .chain(ast.actors.iter().map(|actor| actor.name.clone()))
        .chain(
            ast.interfaces
                .iter()
                .map(|interface| interface.name.clone()),
        )
        .collect()
}

fn package_callable_resolution(
    ast: &SourceFile,
    module_path: &str,
    symbol: &str,
) -> Option<PackageCallableResolution> {
    let local_type_names = local_type_names(ast);
    ast.function_signatures
        .iter()
        .find(|function| function.name == symbol)
        .map(|operation| {
            operation_callable_resolution(module_path, symbol, operation, &[], &local_type_names)
        })
        .or_else(|| {
            ast.functions
                .iter()
                .find(|function| function.name == symbol)
                .map(|function| {
                    function_callable_resolution(
                        module_path,
                        symbol,
                        function,
                        &[],
                        &local_type_names,
                    )
                })
        })
        .or_else(|| {
            let (target, method_name) = symbol.rsplit_once('.')?;
            ast.impls
                .iter()
                .find(|implementation| {
                    impl_target_matches(&implementation.target, module_path, target)
                })
                .and_then(|implementation| {
                    let inherited = generic_type_params_from_text(&implementation.target);
                    implementation
                        .methods
                        .iter()
                        .find(|method| method.name == method_name)
                        .map(|method| {
                            operation_callable_resolution(
                                module_path,
                                symbol,
                                method,
                                &inherited,
                                &local_type_names,
                            )
                        })
                        .or_else(|| {
                            implementation
                                .method_bodies
                                .iter()
                                .find(|method| method.name == method_name)
                                .map(|method| {
                                    function_callable_resolution(
                                        module_path,
                                        symbol,
                                        method,
                                        &inherited,
                                        &local_type_names,
                                    )
                                })
                        })
                })
        })
}

fn operation_callable_resolution(
    module_path: &str,
    source_symbol: &str,
    operation: &InterfaceOperation,
    inherited_type_params: &[String],
    local_type_names: &BTreeSet<String>,
) -> PackageCallableResolution {
    PackageCallableResolution {
        module_path: module_path.to_string(),
        source_symbol: source_symbol.to_string(),
        type_params: inherited_type_params
            .iter()
            .chain(&operation.type_params)
            .cloned()
            .collect(),
        local_type_names: local_type_names.clone(),
        params: operation
            .implicit_self
            .iter()
            .chain(operation.params.iter().map(|param| &param.ty))
            .map(|ty| ty.name.clone())
            .collect(),
        return_type: operation.return_type.name.clone(),
        exact_signature: None,
    }
}

fn function_callable_resolution(
    module_path: &str,
    source_symbol: &str,
    function: &FunctionDecl,
    inherited_type_params: &[String],
    local_type_names: &BTreeSet<String>,
) -> PackageCallableResolution {
    PackageCallableResolution {
        module_path: module_path.to_string(),
        source_symbol: source_symbol.to_string(),
        type_params: inherited_type_params
            .iter()
            .chain(&function.type_params)
            .cloned()
            .collect(),
        local_type_names: local_type_names.clone(),
        params: function
            .implicit_self
            .iter()
            .chain(function.params.iter().map(|param| &param.ty))
            .map(|ty| ty.name.clone())
            .collect(),
        return_type: function.return_type.name.clone(),
        exact_signature: None,
    }
}

fn impl_target_matches(target: &str, module_path: &str, local_target: &str) -> bool {
    let target = target.strip_prefix("root.").unwrap_or(target);
    target == local_target || target == format!("{module_path}.{local_target}")
}

fn generic_type_params_from_text(name: &str) -> Vec<String> {
    generic_parts(name)
        .map(|parts| {
            parts
                .args
                .iter()
                .map(|arg| arg.trim())
                .filter(|arg| {
                    !arg.is_empty()
                        && arg
                            .chars()
                            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn expand_alias_text(raw: &str, aliases: &BTreeMap<String, String>) -> Result<String, String> {
    fn expand_seen(
        raw: &str,
        aliases: &BTreeMap<String, String>,
        seen: &mut Vec<String>,
    ) -> String {
        TypeExpr::parse(raw)
            .map_named_types(|name| {
                let Some(target) = aliases.get(name) else {
                    return name.to_string();
                };
                if seen.iter().any(|entry| entry == name) {
                    return target.clone();
                }
                seen.push(name.to_string());
                let expanded = expand_seen(target, aliases, seen);
                seen.pop();
                expanded
            })
            .to_type_string()
    }
    reject_generic_alias_uses(&TypeExpr::parse(raw), aliases)?;
    Ok(expand_seen(raw, aliases, &mut Vec::new()))
}

fn reject_generic_alias_uses(
    ty: &TypeExpr,
    aliases: &BTreeMap<String, String>,
) -> Result<(), String> {
    match ty {
        TypeExpr::Named { name, args } => {
            if !args.is_empty() && aliases.contains_key(name) {
                return Err(format!(
                    "alias {name} does not accept type arguments in type reference {}",
                    ty.to_type_string()
                ));
            }
            for arg in args {
                reject_generic_alias_uses(arg, aliases)?;
            }
        }
        TypeExpr::Nullable(inner) => reject_generic_alias_uses(inner, aliases)?,
        TypeExpr::AnyInterface { interface } => reject_generic_alias_uses(interface, aliases)?,
        TypeExpr::Union(parts) => {
            for part in parts {
                reject_generic_alias_uses(part, aliases)?;
            }
        }
        TypeExpr::Record(fields) => {
            for field in fields {
                reject_generic_alias_uses(&field.ty, aliases)?;
            }
        }
        TypeExpr::Function {
            params,
            return_type,
        } => {
            for param in params {
                reject_generic_alias_uses(&param.ty, aliases)?;
            }
            reject_generic_alias_uses(return_type, aliases)?;
        }
        TypeExpr::EmptyRecord | TypeExpr::StringLiteral(_) => {}
    }
    Ok(())
}

fn strip_generic(name: &str) -> &str {
    name.split('<').next().unwrap_or(name).trim()
}

fn validate_prelude_type_arity(name: &str, found: usize) -> Result<(), String> {
    let registry = prelude_registry();
    let Some(decl_name) = registry.prelude_type_decl_name(name) else {
        return Ok(());
    };
    let Some(decl) = registry.type_decl(decl_name) else {
        return Ok(());
    };
    if decl.type_params.len() == found {
        return Ok(());
    }
    Err(format!(
        "package type `{name}` expects {} type arguments, found {found}",
        decl.type_params.len()
    ))
}

fn package_root_for_module(module_path: &str) -> Option<&str> {
    module_path
        .split('.')
        .next()
        .filter(|root| !root.is_empty())
}

fn package_root_for_symbol(
    symbol: &PackageSymbolRef,
    package_dependencies: &BTreeMap<String, String>,
    package_dependency_access: &BTreeMap<String, PackageDependencyAccess>,
) -> Option<String> {
    let dependency_ref = match &symbol.package {
        PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
        PackageRefIr::PackageId { package_id } => package_dependencies
            .iter()
            .find_map(|(alias, id)| (id == package_id).then_some(alias.as_str()))
            .unwrap_or(package_id),
    };
    let access = package_dependency_access
        .get(dependency_ref)
        .copied()
        .unwrap_or(PackageDependencyAccess::Public);
    Some(match access {
        PackageDependencyAccess::Public => dependency_ref.to_string(),
        PackageDependencyAccess::TopLevel => format!("{dependency_ref}/"),
    })
}

fn qualify_package_type_text(
    raw: &str,
    package_root: &str,
    local_type_names: &BTreeSet<String>,
) -> String {
    TypeExpr::parse(raw)
        .map_named_types(|name| {
            if local_type_names.contains(name) {
                if package_root.ends_with('/') {
                    format!("{package_root}{name}")
                } else {
                    format!("{package_root}.{name}")
                }
            } else {
                name.to_string()
            }
        })
        .to_type_string()
}

fn type_assignable(actual: &TypeRefIr, expected: &TypeRefIr) -> bool {
    if actual == expected {
        return true;
    }
    if matches!(actual, TypeRefIr::Literal { .. }) && literal_assignable_to(actual, expected) {
        return true;
    }
    if let TypeRefIr::Union { items } = actual {
        return items.iter().all(|item| type_assignable(item, expected));
    }
    match expected {
        TypeRefIr::Builtin { name, .. } if name == "unknown" => true,
        TypeRefIr::Builtin { name, .. } if name == "void" => is_null_type_ir(actual),
        TypeRefIr::Builtin { name, .. } if name == "Stream" => is_null_type_ir(actual),
        TypeRefIr::Builtin { name, .. } if name == "Json" => json_assignable(actual),
        TypeRefIr::Builtin { name, .. } if name == "JsonObject" => json_object_assignable(actual),
        TypeRefIr::Builtin { name, .. } if name == "number" => {
            matches!(actual, TypeRefIr::Builtin { name, .. } if name == "integer")
        }
        TypeRefIr::Nullable { inner } => is_null_type_ir(actual) || type_assignable(actual, inner),
        TypeRefIr::Union { items } => items
            .iter()
            .any(|expected_item| type_assignable(actual, expected_item)),
        TypeRefIr::Record {
            fields: expected_fields,
        } => {
            let TypeRefIr::Record {
                fields: actual_fields,
            } = actual
            else {
                return false;
            };
            expected_fields.iter().all(|(name, expected_ty)| {
                actual_fields
                    .get(name)
                    .is_some_and(|actual_ty| type_assignable(actual_ty, expected_ty))
            })
        }
        _ => false,
    }
}

fn record_field_type_from_ir(ty: &TypeRefIr, field: &str) -> Option<TypeRefIr> {
    match ty {
        TypeRefIr::Record { fields } => fields.get(field).cloned(),
        TypeRefIr::Union { items } => {
            let mut field_types = Vec::new();
            for item in items {
                field_types.push(record_field_type_from_ir(item, field)?);
            }
            Some(union_type_ir(field_types))
        }
        TypeRefIr::Builtin { name, args } if name == "Exception" && args.len() == 1 => {
            match field {
                "error" => Some(args[0].clone()),
                _ => None,
            }
        }
        _ => None,
    }
}

fn union_type_ir(mut items: Vec<TypeRefIr>) -> TypeRefIr {
    items.sort_by_key(type_ref_debug_text);
    items.dedup();
    match items.as_slice() {
        [only] => only.clone(),
        _ => TypeRefIr::Union { items },
    }
}

fn type_ref_debug_text(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::Builtin { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Builtin { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(type_ref_debug_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::Nullable { inner } => format!("{}?", type_ref_debug_text(inner)),
        TypeRefIr::Union { items } => items
            .iter()
            .map(type_ref_debug_text)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeRefIr::Literal {
            value: LiteralIr::String { value },
        } => serde_json::to_string(value).unwrap_or_else(|_| "\"<string>\"".to_string()),
        TypeRefIr::Literal {
            value: LiteralIr::Null,
        } => "null".to_string(),
        TypeRefIr::Literal { .. } => "<literal>".to_string(),
        TypeRefIr::LocalType { type_index } => format!("#{type_index}"),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => format!("{module_path}#{type_index}"),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            symbol.symbol_path()
        }
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => format!("{package_id}::{stable_schema_key}"),
        TypeRefIr::AppliedNominal { base, arguments } => format!(
            "{}<{}>",
            type_ref_debug_text(&nominal_base_type_ref(base)),
            arguments
                .iter()
                .map(type_ref_debug_text)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::AnyInterface { interface } => {
            let interface_name = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
                .map_or_else(
                    |_| interface.interface_abi_id.clone(),
                    |identity| type_ref_debug_text(&identity),
                );
            if interface.canonical_type_args.is_empty() {
                format!("any {interface_name}")
            } else {
                format!(
                    "any {}<{}>",
                    interface_name,
                    interface
                        .canonical_type_args
                        .iter()
                        .map(type_ref_debug_text)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        TypeRefIr::Record { .. } => "{}".to_string(),
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::Function { .. } => "fn".to_string(),
    }
}

fn contract_type_shape_ir(
    alias: &str,
    descriptor: &ContractTypeDescriptor,
) -> Result<TypeRefIr, String> {
    match descriptor {
        ContractTypeDescriptor::Record { fields } => Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| Ok((name.clone(), contract_type_ref_ir(alias, ty)?)))
                .collect::<Result<_, String>>()?,
        }),
        ContractTypeDescriptor::Alias { target }
        | ContractTypeDescriptor::Representation { target } => contract_type_ref_ir(alias, target),
        ContractTypeDescriptor::StructuralUnion { variants } => Ok(TypeRefIr::Union {
            items: variants
                .iter()
                .map(|ty| contract_type_ref_ir(alias, ty))
                .collect::<Result<_, _>>()?,
        }),
        ContractTypeDescriptor::DiscriminatedUnion { branches, .. } => Ok(TypeRefIr::Union {
            items: branches
                .iter()
                .map(|branch| contract_type_ref_ir(alias, &branch.branch_type))
                .collect::<Result<_, _>>()?,
        }),
        ContractTypeDescriptor::Enumeration { variants } => Ok(TypeRefIr::Union {
            items: variants
                .iter()
                .map(|value| TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: value.clone(),
                    },
                })
                .collect(),
        }),
        ContractTypeDescriptor::CallbackInterface { .. } => {
            Err("callback interface is not a record shape".to_string())
        }
    }
}

fn contract_type_ref_ir(alias: &str, ty: &ContractTypeRef) -> Result<TypeRefIr, String> {
    match ty {
        ContractTypeRef::Builtin { name, arguments } => Ok(TypeRefIr::Builtin {
            name: name.clone(),
            args: arguments
                .iter()
                .map(|argument| contract_type_ref_ir(alias, argument))
                .collect::<Result<_, _>>()?,
        }),
        ContractTypeRef::PackageSchema {
            package_id,
            stable_schema_key,
            ..
        } => Ok(TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: package_id.clone(),
                },
                symbol_path: stable_schema_key.clone(),
                abi_expectation: None,
            },
        }),
        ContractTypeRef::TypeParam { name } => Ok(TypeRefIr::TypeParam { name: name.clone() }),
        ContractTypeRef::Record { fields } => Ok(TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| Ok((name.clone(), contract_type_ref_ir(alias, ty)?)))
                .collect::<Result<_, String>>()?,
        }),
        ContractTypeRef::StructuralUnion { variants } => Ok(TypeRefIr::Union {
            items: variants
                .iter()
                .map(|ty| contract_type_ref_ir(alias, ty))
                .collect::<Result<_, _>>()?,
        }),
        ContractTypeRef::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(contract_type_ref_ir(alias, inner)?),
        }),
        ContractTypeRef::AnyInterface { interface, .. } => {
            let identity = contract_type_ref_ir(alias, interface)?;
            Ok(TypeRefIr::AnyInterface {
                interface: skiff_artifact_model::InterfaceInstantiationRef {
                    interface_abi_id: serde_json::to_string(&identity)
                        .map_err(|error| error.to_string())?,
                    canonical_type_args: Vec::new(),
                },
            })
        }
        ContractTypeRef::Literal {
            value: skiff_artifact_model::ContractLiteral::String { value },
        } => Ok(TypeRefIr::Literal {
            value: LiteralIr::String {
                value: value.clone(),
            },
        }),
    }
}

fn literal_assignable_to(actual: &TypeRefIr, expected: &TypeRefIr) -> bool {
    match (actual, expected) {
        (
            TypeRefIr::Literal {
                value: LiteralIr::String { .. },
            },
            TypeRefIr::Builtin { name, .. },
        ) if name == "string" => true,
        (
            TypeRefIr::Literal {
                value: LiteralIr::Null,
            },
            TypeRefIr::Builtin { name, .. },
        ) if name == "null" => true,
        _ => false,
    }
}

fn json_assignable(actual: &TypeRefIr) -> bool {
    match actual {
        TypeRefIr::Builtin { name, .. } => {
            matches!(
                name.as_str(),
                "string" | "integer" | "number" | "bool" | "null" | "Json" | "JsonObject"
            ) || matches!(actual, TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 && json_assignable(&args[0]))
                || matches!(actual, TypeRefIr::Builtin { name, args } if name == "Map" && args.len() == 2 && json_assignable(&args[1]))
        }
        TypeRefIr::Literal { value } => matches!(
            value,
            LiteralIr::String { .. }
                | LiteralIr::Number { .. }
                | LiteralIr::Bool { .. }
                | LiteralIr::Null
        ),
        TypeRefIr::Record { fields } => fields.values().all(json_assignable),
        TypeRefIr::Nullable { inner } => json_assignable(inner),
        TypeRefIr::Union { items } => items.iter().all(json_assignable),
        _ => false,
    }
}

fn json_object_assignable(actual: &TypeRefIr) -> bool {
    match actual {
        TypeRefIr::Builtin { name, .. } if name == "JsonObject" => true,
        TypeRefIr::Record { fields } => fields.values().all(json_assignable),
        _ => false,
    }
}

fn is_null_type_ir(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, .. } if name == "null")
        || matches!(
            ty,
            TypeRefIr::Literal {
                value: LiteralIr::Null
            }
        )
}

fn is_self_type_ref(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if name == "Self" && args.is_empty())
}

fn interface_method_signature_params(
    method: &InterfaceMethodSignature,
) -> Vec<FunctionTypeParamIr> {
    let has_explicit_self = method
        .params
        .first()
        .is_some_and(|param| param.name == "self" && is_self_type_ref(&param.ty));
    let mut params = Vec::new();
    if !has_explicit_self && method.implicit_self.is_some() {
        params.push(FunctionTypeParamIr {
            name: "self".to_string(),
            ty: TypeRefIr::builtin("Self"),
        });
    }
    params.extend(method.params.iter().cloned());
    params
}

fn type_ref_contains_self(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::Builtin { args, .. } => {
            is_self_type_ref(ty) || args.iter().any(type_ref_contains_self)
        }
        TypeRefIr::AppliedNominal { arguments, .. } => arguments.iter().any(type_ref_contains_self),
        TypeRefIr::Record { fields } => fields.values().any(type_ref_contains_self),
        TypeRefIr::Union { items } => items.iter().any(type_ref_contains_self),
        TypeRefIr::Nullable { inner } => type_ref_contains_self(inner),
        TypeRefIr::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(type_ref_contains_self),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params.iter().any(|param| type_ref_contains_self(&param.ty))
                || type_ref_contains_self(return_type)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

fn type_ref_contains_any_interface(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::AnyInterface { .. } => true,
        TypeRefIr::Builtin { args, .. } => args.iter().any(type_ref_contains_any_interface),
        TypeRefIr::AppliedNominal { arguments, .. } => {
            arguments.iter().any(type_ref_contains_any_interface)
        }
        TypeRefIr::Record { fields } => fields.values().any(type_ref_contains_any_interface),
        TypeRefIr::Union { items } => items.iter().any(type_ref_contains_any_interface),
        TypeRefIr::Nullable { inner } => type_ref_contains_any_interface(inner),
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| type_ref_contains_any_interface(&param.ty))
                || type_ref_contains_any_interface(return_type)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

fn builtin_type_name(name: &str) -> Option<String> {
    let name = name.trim();
    match name {
        "boolean" => return Some("bool".to_string()),
        "String" => return Some("string".to_string()),
        "string" | "integer" | "number" | "bool" | "null" | "unknown" | "void" | "never"
        | "Json" | "JsonObject" | "Date" | "Config" | "bytes" | "Array" | "Map" | "Stream"
        | "Exception" | "CatchResult" | "DbInsertManyResult" | "DbUpdateManyResult"
        | "DbDeleteManyResult" | "DbUpsertResult" => return Some(name.to_string()),
        _ => {}
    }
    if name.contains('.') {
        let symbol = prelude_registry().known_type_symbol(name)?;
        let canonical = canonical_native_prelude_type_symbol(&symbol)?;
        return Some(canonical);
    }
    None
}

fn prelude_known_type_ref(name: &str, args: Vec<TypeRefIr>) -> Option<TypeRefIr> {
    if !name.contains('.')
        && !prelude_registry().is_prelude_type_name(name)
        && !prelude_registry().is_bare_raw_http_envelope_type(name)
    {
        return None;
    }
    let symbol = prelude_registry().known_type_symbol(name)?;
    Some(prelude_symbol_type_ref(symbol, args))
}

fn contextual_prelude_type_ref(
    name: &str,
    args: Vec<TypeRefIr>,
    context: &TypeResolutionContext<'_>,
) -> Option<TypeRefIr> {
    let symbol = prelude_registry().known_type_symbol(name)?;
    let (module_path, _) = symbol.rsplit_once('.')?;
    (module_path == context.module_path).then(|| prelude_symbol_type_ref(symbol, args))
}

fn prelude_symbol_type_ref(symbol: String, args: Vec<TypeRefIr>) -> TypeRefIr {
    if let Some(name) = canonical_native_prelude_type_symbol(&symbol) {
        return TypeRefIr::Builtin { name, args };
    }
    if is_std_abi_generic_type_symbol(&symbol) {
        return TypeRefIr::Builtin { name: symbol, args };
    }
    let base = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
            },
            symbol_path: symbol,
            abi_expectation: None,
        },
    };
    apply_nominal_arguments(base, args)
        .expect("prelude registry nominal symbols always lower to a nominal base")
}

fn apply_nominal_arguments(
    base: TypeRefIr,
    arguments: Vec<TypeRefIr>,
) -> Result<TypeRefIr, String> {
    if arguments.is_empty() {
        return Ok(base);
    }
    let base = nominal_base_from_type_ref(base)?;
    Ok(TypeRefIr::AppliedNominal { base, arguments })
}

fn nominal_base_from_type_ref(ty: TypeRefIr) -> Result<NominalTypeRefBaseIr, String> {
    match ty {
        TypeRefIr::LocalType { type_index } => Ok(NominalTypeRefBaseIr::LocalType { type_index }),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => Ok(NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        }),
        TypeRefIr::ServiceSymbol { symbol } => Ok(NominalTypeRefBaseIr::ServiceSymbol { symbol }),
        TypeRefIr::PackageSymbol { symbol } => Ok(NominalTypeRefBaseIr::PackageSymbol { symbol }),
        TypeRefIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => Ok(NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        }),
        TypeRefIr::DbObjectSymbol { .. } => {
            Err("db object types cannot be applied nominal bases".to_string())
        }
        other => Err(format!(
            "`{}` is not a legal applied nominal base",
            type_ref_debug_text(&other)
        )),
    }
}

fn nominal_base_type_ref(base: &NominalTypeRefBaseIr) -> TypeRefIr {
    match base {
        NominalTypeRefBaseIr::LocalType { type_index } => TypeRefIr::LocalType {
            type_index: *type_index,
        },
        NominalTypeRefBaseIr::PublicationType {
            module_path,
            type_index,
        } => TypeRefIr::PublicationType {
            module_path: module_path.clone(),
            type_index: *type_index,
        },
        NominalTypeRefBaseIr::ServiceSymbol { symbol } => TypeRefIr::ServiceSymbol {
            symbol: symbol.clone(),
        },
        NominalTypeRefBaseIr::PackageSymbol { symbol } => TypeRefIr::PackageSymbol {
            symbol: symbol.clone(),
        },
        NominalTypeRefBaseIr::PackageSchema {
            package_id,
            stable_schema_key,
            package_schema_type_id,
        } => TypeRefIr::PackageSchema {
            package_id: package_id.clone(),
            stable_schema_key: stable_schema_key.clone(),
            package_schema_type_id: package_schema_type_id.clone(),
        },
    }
}

fn canonical_native_prelude_type_symbol(symbol: &str) -> Option<String> {
    match symbol {
        "std.collection.Array" => Some("Array".to_string()),
        "std.collection.Map" => Some("Map".to_string()),
        "std.stream.Stream" => Some("Stream".to_string()),
        "std.bytes.bytes" => Some("bytes".to_string()),
        "std.date.Date" | "Date" => Some("Date".to_string()),
        "Json" => Some("Json".to_string()),
        "JsonObject" => Some("JsonObject".to_string()),
        "Config" => Some("Config".to_string()),
        "config.DecodeError" => Some("config.DecodeError".to_string()),
        other if prelude_registry().is_builtin_type_name(other) => Some(other.to_string()),
        _ => None,
    }
}

fn is_std_abi_generic_type_symbol(symbol: &str) -> bool {
    matches!(
        symbol,
        "std.websocket.WebSocketConnectResult"
            | "std.websocket.WebSocketIngressEvent"
            | "std.websocket.WebSocketConnection"
            | "std.websocket.WebSocketReceiveEvent"
    )
}

fn service_symbol_ref(path: &str) -> ServiceSymbolRef {
    if let Some((module_path, symbol)) = path.rsplit_once('.') {
        ServiceSymbolRef {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        }
    } else {
        ServiceSymbolRef {
            module_path: String::new(),
            symbol: path.to_string(),
        }
    }
}

fn service_symbol_ref_from_source_key(source_key: &SourceSymbolKey) -> ServiceSymbolRef {
    ServiceSymbolRef {
        module_path: source_key.module_path().to_string(),
        symbol: source_key.symbol().to_string(),
    }
}

/// Canonical comparison form for a named type referenced by `<module>.<symbol>`
/// path, independent of whether it originated from a package symbol or a service
/// symbol. Used only for assignability comparison, never for projection.
fn canonical_named_symbol(symbol_path: &str) -> TypeRefIr {
    let path = symbol_path.strip_prefix("root.").unwrap_or(symbol_path);
    TypeRefIr::ServiceSymbol {
        symbol: service_symbol_ref(path),
    }
}

fn type_resolution_semantic_publication<'a>(
    parsed_sources: &'a [ParsedCompilerSource],
) -> SemanticPublication<'a> {
    SemanticPublication::new(
        parsed_sources
            .iter()
            .map(|parsed| {
                SemanticSource::new(
                    parsed.relative_path().display().to_string(),
                    parsed.module_path(),
                    parsed.ast(),
                    parsed.alias_targets(),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use crate::{
        expression_type_model::ExpressionTypeModel,
        parsed_sources::{parse_publication_sources, ParsedCompilerSource},
        source_graph::CompilerSourceFile,
        ExpressionSourceMap, PublicationTypeSymbolIndex,
    };
    use skiff_artifact_model::{
        InterfaceDeclIr, InterfaceOperationIr, TypeDeclIr, TypeDeclarationIr,
    };

    use super::*;

    const MODULE: &str = "internal.assignability";

    fn parsed_sources(source_text: &str) -> Vec<ParsedCompilerSource> {
        let source = CompilerSourceFile::parse(
            PathBuf::from("internal/assignability.skiff"),
            MODULE.to_string(),
            false,
            false,
            source_text.to_string(),
            "internal/assignability.skiff",
        )
        .expect("test source should parse");
        parse_publication_sources(&PathBuf::from("/test"), &[source])
            .expect("test source facts should build")
    }

    fn type_resolution(source_text: &str) -> (Vec<ParsedCompilerSource>, TypeResolutionModel) {
        let parsed_sources = parsed_sources(source_text);
        let type_resolution = TypeResolutionModel::build(
            &parsed_sources,
            &BTreeMap::new(),
            &[],
            None,
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");
        (parsed_sources, type_resolution)
    }

    fn package_type_resolution(
        source_text: &str,
    ) -> (Vec<ParsedCompilerSource>, TypeResolutionModel) {
        let parsed_sources = parsed_sources(source_text);
        let package_source = CompilerSourceFile::parse(
            PathBuf::from("pkg/reader.skiff"),
            "pkg.reader".to_string(),
            false,
            false,
            r#"
              interface Reader<T> {
                function read(self: Self, fallback: T) -> T
              }
            "#
            .to_string(),
            "pkg/reader.skiff",
        )
        .expect("package source should parse");
        let package_parsed =
            parse_publication_sources(&PathBuf::from("/package"), &[package_source])
                .expect("package source facts should build");
        let mut package_unit = FileIrUnit::empty("pkg.reader", "reader-package");
        package_unit.declarations.interfaces.insert(
            "Reader".to_string(),
            InterfaceDeclIr {
                name: "Reader".to_string(),
                type_params: vec!["T".to_string()],
                operations: vec![InterfaceOperationIr {
                    name: "read".to_string(),
                    type_params: Vec::new(),
                    params: vec![
                        FunctionTypeParamIr {
                            name: "self".to_string(),
                            ty: TypeRefIr::builtin("Self"),
                        },
                        FunctionTypeParamIr {
                            name: "fallback".to_string(),
                            ty: TypeRefIr::TypeParam {
                                name: "T".to_string(),
                            },
                        },
                    ],
                    return_type: TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    },
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                }],
                source_span: None,
            },
        );
        let package_facts = vec![TypeResolutionPackageFacts {
            package_id: "dep.pkg",
            dependencies: Vec::new(),
            schema_types: vec![TypeResolutionPackageSchemaTypeFact {
                public_path: "Reader",
                source_module: "pkg.reader",
                source_symbol: "Reader",
                kind: PublicTypeKind::Interface,
                source_ast: package_parsed[0].ast(),
                file_ir_unit: Some(&package_unit),
            }],
            callables: Vec::new(),
        }];
        let mut dependency = PackageDependency::id("dep.pkg");
        dependency.alias = Some("pkg".to_string());
        let package_aliases = BTreeMap::from([("pkg".to_string(), vec![String::new()])]);
        let type_resolution = TypeResolutionModel::build(
            &parsed_sources,
            &package_aliases,
            &[dependency],
            Some(&package_facts),
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution with package facts should build");
        (parsed_sources, type_resolution)
    }

    fn context() -> TypeResolutionContext<'static> {
        TypeResolutionContext::source(MODULE)
    }

    fn initialize_test_prelude() {
        let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root resolves");
        let platform_sources = skiff_compiler_input::CompilerPlatformSources::new(&platform_root)
            .expect("platform sources load");
        crate::prelude_registry::initialize_prelude_registry(&platform_sources)
            .expect("prelude registry initializes");
    }

    #[test]
    fn applied_nominal_resolution_preserves_ordered_nested_arguments_and_alias_targets() {
        let (_parsed, model) = type_resolution(
            r#"
              type Id = string
              type Box<T> { value: T }
              type Outer<A, B> { first: A, second: B }
              type Token<T> = string
              alias StringBox = Box<string>
            "#,
        );
        let module = model.modules.get(MODULE).expect("test module is indexed");
        let box_index = module.type_indices["Box"];
        let outer_index = module.type_indices["Outer"];
        let token_index = module.type_indices["Token"];

        let string_box = model
            .resolve_type_text("Box<string>", &context())
            .expect("generic local record resolves");
        let number_box = model
            .resolve_type_text("Box<number>", &context())
            .expect("same declaration with another argument resolves");
        assert_eq!(
            string_box.ir,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType {
                    type_index: box_index,
                },
                arguments: vec![TypeRefIr::builtin("string")],
            }
        );
        assert_eq!(
            number_box.ir,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType {
                    type_index: box_index,
                },
                arguments: vec![TypeRefIr::builtin("number")],
            }
        );
        assert_ne!(string_box.ir, number_box.ir);

        let nested = model
            .resolve_type_text("Outer<Box<string>, Array<Id>>", &context())
            .expect("nested nominal arguments resolve structurally");
        assert_eq!(
            nested.ir,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType {
                    type_index: outer_index,
                },
                arguments: vec![
                    string_box.ir.clone(),
                    TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::LocalType {
                            type_index: module.type_indices["Id"],
                        }],
                    },
                ],
            }
        );

        let token = model
            .resolve_type_text("Token<string>", &context())
            .expect("generic representation resolves");
        assert_eq!(
            token.ir,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType {
                    type_index: token_index,
                },
                arguments: vec![TypeRefIr::builtin("string")],
            },
            "representation use must retain its nominal owner"
        );
        let alias = model
            .resolve_type_text("StringBox", &context())
            .expect("transparent alias to an applied nominal resolves");
        assert_eq!(alias.ir, string_box.ir);

        let string_fields = model
            .resolve_constructor_target_resolved(&string_box, &context())
            .expect("structured applied record is constructible")
            .fields;
        let number_fields = model
            .resolve_constructor_target_resolved(&number_box, &context())
            .expect("structured applied record is constructible")
            .fields;
        assert_eq!(string_fields["value"].ir, TypeRefIr::builtin("string"));
        assert_eq!(number_fields["value"].ir, TypeRefIr::builtin("number"));
    }

    #[test]
    fn applied_nominal_resolution_keeps_local_and_package_owners_distinct() {
        initialize_test_prelude();
        let parsed_sources = parsed_sources("type Box<T> { value: T }");
        let package_source = CompilerSourceFile::parse(
            PathBuf::from("pkg/box.skiff"),
            "pkg.box".to_string(),
            false,
            false,
            "type Box<T> { value: T }".to_string(),
            "pkg/box.skiff",
        )
        .expect("package source parses");
        let package_parsed =
            parse_publication_sources(&PathBuf::from("/package"), &[package_source])
                .expect("package source facts build");
        let mut package_unit = FileIrUnit::empty("pkg.box", "generic-package");
        package_unit.type_table.push(TypeDeclIr {
            name: "Box".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "value".to_string(),
                    TypeRefIr::TypeParam {
                        name: "T".to_string(),
                    },
                )]),
            },
            type_params: vec!["T".to_string()],
            implements: Vec::new(),
            source_span: None,
        });
        package_unit.declarations.types.insert(
            "Box".to_string(),
            TypeDeclarationIr {
                type_index: 0,
                symbol: "Box".to_string(),
                source_span: None,
            },
        );
        let package_facts = vec![TypeResolutionPackageFacts {
            package_id: "dep.generic",
            dependencies: Vec::new(),
            schema_types: vec![TypeResolutionPackageSchemaTypeFact {
                public_path: "Box",
                source_module: "pkg.box",
                source_symbol: "Box",
                kind: PublicTypeKind::Type,
                source_ast: package_parsed[0].ast(),
                file_ir_unit: Some(&package_unit),
            }],
            callables: Vec::new(),
        }];
        let mut dependency = PackageDependency::id("dep.generic");
        dependency.alias = Some("pkg".to_string());
        let model = TypeResolutionModel::build(
            &parsed_sources,
            &BTreeMap::from([("pkg".to_string(), vec![String::new()])]),
            &[dependency],
            Some(&package_facts),
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("consumer and package generic facts build");

        let local = model
            .resolve_type_text("Box<string>", &context())
            .expect("local Box resolves");
        let package = model
            .resolve_type_text("pkg.Box<string>", &context())
            .expect("package Box resolves");
        assert!(matches!(
            local.ir,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::LocalType { .. },
                ..
            }
        ));
        assert_eq!(
            package.ir,
            TypeRefIr::AppliedNominal {
                base: NominalTypeRefBaseIr::PackageSymbol {
                    symbol: PackageSymbolRef {
                        package: PackageRefIr::Dependency {
                            dependency_ref: "pkg".to_string(),
                        },
                        symbol_path: "Box".to_string(),
                        abi_expectation: None,
                    },
                },
                arguments: vec![TypeRefIr::builtin("string")],
            }
        );
        assert_ne!(local.ir, package.ir);
    }

    #[test]
    fn invalid_applied_nominal_bases_arity_and_type_param_scope_fail_closed() {
        initialize_test_prelude();
        let (_parsed, model) = type_resolution(
            r#"
              type Box<T> { value: T }
              type Plain { value: string }
              type WorkerBox { value: string }
              alias Alias = Box<string>
              interface Provider<T> {
                function get(self: Self) -> T
              }
              actor Worker id string {}
            "#,
        );
        let cases = [
            ("Box", "expects 1 type arguments, found 0"),
            ("Box<string, number>", "expects 1 type arguments, found 2"),
            ("Box<Missing>", "unresolved type `Missing`"),
            ("Plain<string>", "expects 0 type arguments, found 1"),
            ("Alias<string>", "does not accept type arguments"),
            (
                "Provider<string>",
                "cannot be used as an applied nominal base",
            ),
            ("Worker<string>", "expects 0 type arguments, found 1"),
            ("T", "unresolved type `T`"),
        ];
        for (source, expected) in cases {
            let error = match model.resolve_type_text(source, &context()) {
                Ok(resolved) => panic!("`{source}` must fail closed, found {:?}", resolved.ir),
                Err(error) => error,
            };
            assert!(error.contains(expected), "`{source}`: {error}");
        }

        let generic_context =
            TypeResolutionContext::with_type_params(MODULE, BTreeSet::from(["T".to_string()]));
        assert_eq!(
            model
                .resolve_type_text("T", &generic_context)
                .expect("in-scope declaration parameter resolves")
                .ir,
            TypeRefIr::TypeParam {
                name: "T".to_string(),
            }
        );
        assert!(
            model
                .resolve_type_text("T<string>", &generic_context)
                .is_err(),
            "a type parameter cannot become an applied nominal base"
        );
    }

    #[test]
    fn generic_catch_leaves_keep_applied_union_owner_and_substituted_branch_identity() {
        let (_parsed, model) = type_resolution(
            r#"
              type Branch<T> { value: T }
              type Choice<T> discriminator "kind" =
                Branch<T> |
                { kind: "inline", value: T } |
                "literal"
            "#,
        );
        let string_choice = model
            .resolve_type_text("Choice<string>", &context())
            .expect("string choice resolves");
        let number_choice = model
            .resolve_type_text("Choice<number>", &context())
            .expect("number choice resolves");
        let string_leaves = model
            .catch_leaves(&string_choice, &context())
            .expect("fully instantiated generic named union has catch leaves");
        let number_leaves = model
            .catch_leaves(&number_choice, &context())
            .expect("same generic union with another argument has catch leaves");

        assert_eq!(string_leaves.len(), 3);
        assert_eq!(number_leaves.len(), 3);
        assert_ne!(string_leaves, number_leaves);
        assert!(string_leaves.identities().iter().all(|leaf| {
            matches!(
                leaf,
                CatchLeafIdentity::NamedUnionBranch { union_type, .. }
                    if union_type == &string_choice.ir
            )
        }));
        assert!(matches!(
            &string_leaves.identities()[0],
            CatchLeafIdentity::NamedUnionBranch {
                branch:
                    NamedUnionBranchIr::ConcreteNominal {
                        nominal_type:
                            TypeRefIr::AppliedNominal {
                                arguments,
                                ..
                            },
                    },
                ..
            } if arguments == &vec![TypeRefIr::builtin("string")]
        ));
        assert!(matches!(
            &string_leaves.identities()[1],
            CatchLeafIdentity::NamedUnionBranch {
                branch:
                    NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type: TypeRefIr::Record { fields },
                        ..
                    },
                ..
            } if fields["value"] == TypeRefIr::builtin("string")
        ));
        assert!(matches!(
            &string_leaves.identities()[2],
            CatchLeafIdentity::NamedUnionBranch {
                branch: NamedUnionBranchIr::Literal { .. },
                ..
            }
        ));
    }

    fn signature_rehydration_artifact() -> PackageArtifact {
        use skiff_artifact_model::{
            PackageImplementationLinks, PackageLocalAbi, PackageRuntimeRequirements,
            PackageSchemaIndexRef, TypeExport,
        };

        let file = skiff_artifact_model::FileIrRef {
            file_ir_identity: "provider-file".to_string(),
            artifact_path: Some("types.json".to_string()),
            module_path: "types".to_string(),
            source_ast_hash: Some("provider-source".to_string()),
        };
        let descriptor = TypeDescriptorIr::Record {
            fields: BTreeMap::new(),
        };
        let type_symbol = |public_path: &str| PackageLocalAbiSymbol::Type {
            local_type_id: format!("type:{public_path}"),
            descriptor: descriptor.clone(),
            is_alias: false,
            is_interface: false,
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        };
        let type_export = |type_index, symbol: &str| TypeExport {
            file: file.clone(),
            type_index,
            symbol: symbol.to_string(),
            is_interface: false,
            descriptor: Some(descriptor.clone()),
            type_params: Vec::new(),
            interface_methods: Vec::new(),
        };
        PackageArtifact {
            schema_version: "skiff-package-artifact-v7".to_string(),
            package_id: "example.com/provider".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("provider-build"),
            files: vec![file.clone()],
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("provider-abi"),
                public_symbols: BTreeMap::from([
                    ("Bindings".to_string(), type_symbol("Bindings")),
                    ("Result".to_string(), type_symbol("Result")),
                ]),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: PackageSchemaIndexRef {
                package_id: "example.com/provider".to_string(),
                package_schema_index_identity: "provider-schema-index".into(),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks {
                types: BTreeMap::from([
                    ("Bindings".to_string(), type_export(0, "Bindings")),
                    ("Result".to_string(), type_export(1, "Result")),
                ]),
                ..PackageImplementationLinks::default()
            },
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
            service_call_roots: Vec::new(),
            service_call_refs: Vec::new(),
        }
    }

    #[test]
    fn package_signature_exact_symbols_rehydrate_and_ownerless_slots_fail_closed() {
        let parsed_sources = parsed_sources("function noop() -> void {}");
        let mut dependency = PackageDependency::id("example.com/provider");
        dependency.alias = Some("provider".to_string());
        let artifact = signature_rehydration_artifact();
        let model = TypeResolutionModel::build(
            &parsed_sources,
            &BTreeMap::from([("provider".to_string(), vec![String::new()])]),
            &[dependency],
            None,
            Some(std::slice::from_ref(&artifact)),
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("artifact-only dependency type facts should build");

        let dependency_symbol = |symbol_path: &str| TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "provider".to_string(),
                },
                symbol_path: symbol_path.to_string(),
                abi_expectation: Some("provider-abi".to_string()),
            },
        };
        let interface_identity = serde_json::to_string(&TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: "types".to_string(),
                symbol: "Bindings".to_string(),
            },
        })
        .unwrap();
        let signature_type = PackageTypeRef::Local {
            local_type: TypeRefIr::Function {
                params: vec![
                    FunctionTypeParamIr {
                        name: "service".to_string(),
                        ty: TypeRefIr::ServiceSymbol {
                            symbol: ServiceSymbolRef {
                                module_path: "types".to_string(),
                                symbol: "Bindings".to_string(),
                            },
                        },
                    },
                    FunctionTypeParamIr {
                        name: "publication".to_string(),
                        ty: TypeRefIr::PublicationType {
                            module_path: "types".to_string(),
                            type_index: 1,
                        },
                    },
                    FunctionTypeParamIr {
                        name: "nested".to_string(),
                        ty: TypeRefIr::Builtin {
                            name: "Array".to_string(),
                            args: vec![TypeRefIr::Nullable {
                                inner: Box::new(TypeRefIr::Union {
                                    items: vec![
                                        TypeRefIr::ServiceSymbol {
                                            symbol: ServiceSymbolRef {
                                                module_path: "types".to_string(),
                                                symbol: "Bindings".to_string(),
                                            },
                                        },
                                        TypeRefIr::PublicationType {
                                            module_path: "types".to_string(),
                                            type_index: 1,
                                        },
                                    ],
                                }),
                            }],
                        },
                    },
                ],
                return_type: Box::new(TypeRefIr::Record {
                    fields: BTreeMap::from([
                        (
                            "service".to_string(),
                            TypeRefIr::ServiceSymbol {
                                symbol: ServiceSymbolRef {
                                    module_path: "types".to_string(),
                                    symbol: "Bindings".to_string(),
                                },
                            },
                        ),
                        (
                            "package".to_string(),
                            TypeRefIr::PackageSymbol {
                                symbol: PackageSymbolRef {
                                    package: PackageRefIr::PackageId {
                                        package_id: "example.com/provider".to_string(),
                                    },
                                    symbol_path: "types.Result".to_string(),
                                    abi_expectation: None,
                                },
                            },
                        ),
                        (
                            "interface".to_string(),
                            TypeRefIr::AnyInterface {
                                interface: InterfaceInstantiationRef {
                                    interface_abi_id: interface_identity,
                                    canonical_type_args: vec![TypeRefIr::PublicationType {
                                        module_path: "types".to_string(),
                                        type_index: 1,
                                    }],
                                },
                            },
                        ),
                    ]),
                }),
            },
        };
        let normalized = model
            .rehydrate_package_signature_type_for_dependency("provider", &signature_type)
            .expect("all public owner-local references should normalize");
        let PackageTypeRef::Local {
            local_type:
                TypeRefIr::Function {
                    params,
                    return_type,
                },
        } = normalized
        else {
            panic!("normalized signature should retain its function shape")
        };
        assert_eq!(params[0].ty, dependency_symbol("Bindings"));
        assert_eq!(params[1].ty, dependency_symbol("Result"));
        assert_eq!(
            params[2].ty,
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::Nullable {
                    inner: Box::new(TypeRefIr::Union {
                        items: vec![dependency_symbol("Bindings"), dependency_symbol("Result"),],
                    }),
                }],
            }
        );
        let TypeRefIr::Record { fields } = return_type.as_ref() else {
            panic!("normalized return should retain its record shape")
        };
        assert_eq!(fields["service"], dependency_symbol("Bindings"));
        assert_eq!(fields["package"], dependency_symbol("Result"));
        let TypeRefIr::AnyInterface { interface } = &fields["interface"] else {
            panic!("normalized nested interface should retain its existential shape")
        };
        assert_eq!(
            serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).unwrap(),
            dependency_symbol("Bindings")
        );
        assert_eq!(
            interface.canonical_type_args,
            vec![dependency_symbol("Result")]
        );
        let wrapped = PackageTypeRef::Container {
            name: "Array".to_string(),
            arguments: vec![PackageTypeRef::Nullable {
                inner: Box::new(PackageTypeRef::AnyInterface {
                    interface: Box::new(PackageTypeRef::Local {
                        local_type: TypeRefIr::ServiceSymbol {
                            symbol: ServiceSymbolRef {
                                module_path: "types".to_string(),
                                symbol: "Bindings".to_string(),
                            },
                        },
                    }),
                    arguments: vec![PackageTypeRef::Local {
                        local_type: TypeRefIr::PublicationType {
                            module_path: "types".to_string(),
                            type_index: 1,
                        },
                    }],
                }),
            }],
        };
        assert_eq!(
            model
                .rehydrate_package_signature_type_for_dependency("provider", &wrapped)
                .unwrap(),
            PackageTypeRef::Container {
                name: "Array".to_string(),
                arguments: vec![PackageTypeRef::Nullable {
                    inner: Box::new(PackageTypeRef::AnyInterface {
                        interface: Box::new(PackageTypeRef::Local {
                            local_type: dependency_symbol("Bindings"),
                        }),
                        arguments: vec![PackageTypeRef::Local {
                            local_type: dependency_symbol("Result"),
                        }],
                    }),
                }],
            }
        );

        let exact_schema = PackageTypeRef::PackageSchema {
            package_id: "example.com/provider".to_string(),
            stable_schema_key: "Result".to_string(),
            package_schema_type_id: "schema-result".into(),
        };
        assert_eq!(
            model
                .rehydrate_package_signature_type_for_dependency("provider", &exact_schema)
                .unwrap(),
            exact_schema,
            "exact PackageSchema owner/key/type id must remain unchanged"
        );

        let error = model
            .rehydrate_package_signature_type_for_dependency(
                "provider",
                &PackageTypeRef::Local {
                    local_type: TypeRefIr::LocalType { type_index: 0 },
                },
            )
            .unwrap_err();
        assert!(
            error.contains("artifact producer wrote ownerless package signature LocalType slot #0"),
            "{error}"
        );

        let mut ambiguous = model.clone();
        ambiguous.package_type_slots.insert(
            ("provider".to_string(), "other".to_string(), 0),
            "other.Bindings".to_string(),
        );
        let error = ambiguous
            .rehydrate_package_signature_type_for_dependency(
                "provider",
                &PackageTypeRef::Local {
                    local_type: TypeRefIr::LocalType { type_index: 0 },
                },
            )
            .unwrap_err();
        assert!(
            error.contains("artifact producer wrote ownerless package signature LocalType slot #0"),
            "{error}"
        );

        let error = model
            .rehydrate_package_signature_type_for_dependency(
                "provider",
                &PackageTypeRef::Local {
                    local_type: TypeRefIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: "private".to_string(),
                            symbol: "Hidden".to_string(),
                        },
                    },
                },
            )
            .unwrap_err();
        assert!(error.contains("no unique public Local ABI type"), "{error}");
    }

    #[test]
    fn compiler_owned_package_owner_rejects_ownerless_package_signature_slots() {
        let parsed_sources = parsed_sources("function noop() -> void {}");
        let artifact = signature_rehydration_artifact();
        let dependencies = compiler_owned_dependencies(&artifact);
        let model = TypeResolutionModel::build_with_compiler_owned_packages(
            &parsed_sources,
            &BTreeMap::new(),
            &[],
            None,
            Some(std::slice::from_ref(&artifact)),
            &dependencies,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("compiler-owned artifact owner should build");

        assert_eq!(
            model.package_dependencies.get("std").map(String::as_str),
            Some("example.com/provider")
        );
        let error = model
            .rehydrate_package_signature_type_for_dependency(
                "std",
                &PackageTypeRef::Local {
                    local_type: TypeRefIr::LocalType { type_index: 0 },
                },
            )
            .unwrap_err();
        assert!(
            error.contains("artifact producer wrote ownerless package signature LocalType slot #0"),
            "{error}"
        );
    }

    #[test]
    fn compiler_owned_package_owner_requires_one_exact_artifact() {
        let parsed_sources = parsed_sources("function noop() -> void {}");
        let artifact = signature_rehydration_artifact();
        let dependencies = compiler_owned_dependencies(&artifact);
        for (artifacts, expected_count) in [
            (Vec::new(), 0),
            (vec![artifact.clone(), artifact.clone()], 2),
        ] {
            let error = TypeResolutionModel::build_with_compiler_owned_packages(
                &parsed_sources,
                &BTreeMap::new(),
                &[],
                None,
                Some(&artifacts),
                &dependencies,
                &PublicationTypeSymbolIndex::default(),
            )
            .unwrap_err();
            assert!(
                error.contains(&format!(
                    "requires exactly one verified package artifact owner, found {expected_count}"
                )),
                "{error}"
            );
        }
    }

    #[test]
    fn compiler_owned_available_artifacts_require_explicit_owner_facts() {
        let parsed_sources = parsed_sources("function noop() -> void {}");
        let artifact = signature_rehydration_artifact();
        let model = TypeResolutionModel::build_with_compiler_owned_packages(
            &parsed_sources,
            &BTreeMap::new(),
            &[],
            None,
            Some(std::slice::from_ref(&artifact)),
            &SourceDependencyAnalysisInput::default(),
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("unselected available artifacts must stay outside type resolution");
        assert!(model.package_dependencies.is_empty());
        assert!(model.package_artifact_identities.is_empty());
        assert!(model.package_types.is_empty());
    }

    fn compiler_owned_dependencies(artifact: &PackageArtifact) -> SourceDependencyAnalysisInput {
        SourceDependencyAnalysisInput::new(
            [(
                "std".to_string(),
                crate::PackageDependencyAnalysisFacts::new(
                    artifact.package_build_id.clone(),
                    artifact.package_local_abi.local_abi_identity.clone(),
                    BTreeMap::new(),
                )
                .compiler_owned(),
            )],
            [],
        )
        .unwrap()
    }

    fn conformance_source() -> &'static str {
        r#"
          interface I<T> {}

          type Box<T> implements I<T> {
            value: T,
          }

          type Payload {
            value: string,
          }

          type Wrapped = Box<string>
        "#
    }

    fn resolved_test_interface(argument: TypeRefIr) -> ResolvedTypeRef {
        let identity = TypeRefIr::ServiceSymbol {
            symbol: ServiceSymbolRef {
                module_path: MODULE.to_string(),
                symbol: "I".to_string(),
            },
        };
        ResolvedTypeRef {
            source_text: format!("I<{}>", type_ref_debug_text(&argument)),
            ir: TypeRefIr::AnyInterface {
                interface: interface_instantiation_ref(identity, vec![argument]),
            },
        }
    }

    fn object_safe_interface_source() -> &'static str {
        r#"
          interface Provider {
            function name(self: Self) -> string
          }

          interface Box<T> {
            function get(self: Self) -> T
          }

          type Concrete {
            value: string,
          }

          alias ProviderAlias = Provider
        "#
    }

    fn package_reader_conformance_source() -> &'static str {
        r#"
          type Host implements pkg.Reader<string> {
            value: string,
          }

          impl Host {
            function read(fallback: string) -> string {
              return fallback
            }
          }
        "#
    }

    #[test]
    fn any_interface_selector_resolution_rejects_non_interface_targets() {
        let (_parsed_sources, type_resolution) = type_resolution(object_safe_interface_source());
        let context = context();

        let any_provider = type_resolution
            .resolve_type_text("any Provider", &context)
            .expect("object-safe interface selector should resolve");
        assert!(
            matches!(any_provider.ir, TypeRefIr::AnyInterface { .. }),
            "any Provider should resolve to TypeRefIr::AnyInterface"
        );
        let provider = type_resolution
            .resolve_type_text("Provider", &context)
            .expect("bare Provider should resolve as a named type");
        type_resolution
            .resolve_canonical_interface_selector_resolved_type_ref(&provider, &context)
            .expect("resolved Provider should validate as a canonical interface selector");

        for (raw, expected) in [
            ("any string", "primitive/builtin"),
            ("any Concrete", "concrete type"),
            ("any ProviderAlias", "alias"),
            ("any { value: string }", "anonymous record"),
            ("any any Provider", "nested `any`"),
            ("any Box", "expects 1 type arguments"),
        ] {
            let error = type_resolution
                .resolve_type_text(raw, &context)
                .expect_err("invalid interface selector should fail");
            assert!(
                error.contains(expected),
                "expected `{raw}` error to contain `{expected}`, got: {error}"
            );
        }
    }

    #[test]
    fn externalized_any_interface_source_text_remains_parseable() {
        let (_parsed_sources, type_resolution) = type_resolution(object_safe_interface_source());
        let context = context();
        let resolved = type_resolution
            .resolve_type_text("any Provider", &context)
            .expect("any Provider should resolve");

        let externalized = type_resolution.externalize_local_type_refs(&resolved, MODULE);

        assert_eq!(
            externalized.source_text,
            "any internal.assignability.Provider"
        );
        let reparsed = type_resolution
            .resolve_type_text(&externalized.source_text, &context)
            .expect("externalized interface text should remain valid source syntax");
        assert_eq!(reparsed.ir, externalized.ir);
    }

    #[test]
    fn map_key_rejects_any_interface_without_rejecting_map_value() {
        let (_parsed_sources, type_resolution) = type_resolution(object_safe_interface_source());
        let context = context();

        type_resolution
            .resolve_type_text("Map<string, any Provider>", &context)
            .expect("any interface should be allowed in Map value position");
        let error = type_resolution
            .resolve_type_text("Map<any Provider, string>", &context)
            .expect_err("any interface map key should fail at source type resolution");
        assert!(
            error.contains("Map key type"),
            "unexpected Map key diagnostic: {error}"
        );
    }

    #[test]
    fn any_package_interface_method_signature_substitutes_interface_type_args() {
        let (_parsed_sources, type_resolution) =
            package_type_resolution(package_reader_conformance_source());
        let context = context();
        let any_reader = type_resolution
            .resolve_type_text("any pkg.Reader<string>", &context)
            .expect("package any interface should resolve");

        let read = type_resolution
            .any_interface_method_signature(&any_reader.ir, "read")
            .expect("Reader.read should resolve on any package interface");

        assert_eq!(read.params.len(), 2);
        assert_eq!(read.params[0].name, "self");
        assert_eq!(read.params[0].ty, TypeRefIr::builtin("Self"));
        assert_eq!(read.params[1].name, "fallback");
        assert_eq!(read.params[1].ty, TypeRefIr::builtin("string"));
        assert_eq!(read.return_type, TypeRefIr::builtin("string"));
        assert!(!read.method_abi_id.is_empty());
    }

    #[test]
    fn local_conformance_lookup_accepts_package_interface_selector() {
        let (_parsed_sources, type_resolution) =
            package_type_resolution(package_reader_conformance_source());
        let context = context();
        let actual = type_resolution
            .resolve_type_text("Host", &context)
            .expect("Host should resolve");
        let expected = type_resolution
            .resolve_type_text("any pkg.Reader<string>", &context)
            .expect("package interface should resolve");

        let conformance = type_resolution
            .local_any_interface_conformance_for_boxing(&actual, &expected, &context)
            .expect(
                "package selector conformance lookup should not report source-only selector errors",
            )
            .expect("Host should conform to pkg.Reader<string>");

        assert_eq!(conformance.receiver, SourceSymbolKey::new(MODULE, "Host"));
        assert!(matches!(
            serde_json::from_str::<TypeRefIr>(&conformance.interface.interface_abi_id)
                .expect("interface abi id should decode"),
            TypeRefIr::PackageSymbol { .. }
        ));
        assert_eq!(
            conformance.interface.canonical_type_args,
            vec![TypeRefIr::builtin("string")]
        );
        assert_eq!(conformance.slots.len(), 1);
        let slot = &conformance.slots[0];
        assert_eq!(slot.slot, 0);
        assert_eq!(slot.name, "read");
        assert_eq!(
            slot.params,
            vec![
                FunctionTypeParamIr {
                    name: "self".to_string(),
                    ty: TypeRefIr::ServiceSymbol {
                        symbol: service_symbol_ref_from_source_key(&SourceSymbolKey::new(
                            MODULE, "Host"
                        )),
                    },
                },
                FunctionTypeParamIr {
                    name: "fallback".to_string(),
                    ty: TypeRefIr::builtin("string"),
                },
            ]
        );
        assert_eq!(slot.return_type, TypeRefIr::builtin("string"));
    }

    #[test]
    fn package_interface_conformance_matches_public_alias_signature_types() {
        let parsed_sources = parsed_sources(
            r#"
              import agent
              import api

              type Host implements agent.llm.Client {}

              impl Host {
                function stream(input: agent.llm.Request) -> Stream<agent.llm.Event> {
                  return null
                }
              }
            "#,
        );
        let api_source = CompilerSourceFile::parse(
            PathBuf::from("api/types.skiff"),
            "api.types".to_string(),
            false,
            false,
            r#"
              type Request {
                text: string,
              }

              type Event {
                text: string,
              }
            "#
            .to_string(),
            "api/types.skiff",
        )
        .expect("api package source should parse");
        let api_parsed = parse_publication_sources(&PathBuf::from("/api"), &[api_source])
            .expect("api package source facts should build");
        let agent_source = CompilerSourceFile::parse(
            PathBuf::from("agent/llm.skiff"),
            "agent.llm".to_string(),
            false,
            false,
            r#"
              import api

              alias Request = api.Request
              alias Event = api.Event

              interface Client {
                function stream(self: Self, input: Request) -> Stream<Event>
              }
            "#
            .to_string(),
            "agent/llm.skiff",
        )
        .expect("agent package source should parse");
        let agent_parsed = parse_publication_sources(&PathBuf::from("/agent"), &[agent_source])
            .expect("agent package source facts should build");
        let api_request = TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "api".to_string(),
                },
                symbol_path: "Request".to_string(),
                abi_expectation: None,
            },
        };
        let api_event = TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "api".to_string(),
                },
                symbol_path: "Event".to_string(),
                abi_expectation: None,
            },
        };
        let mut agent_unit = FileIrUnit::empty("agent.llm", "agent-package");
        agent_unit.declarations.interfaces.insert(
            "Client".to_string(),
            InterfaceDeclIr {
                name: "Client".to_string(),
                type_params: Vec::new(),
                operations: vec![InterfaceOperationIr {
                    name: "stream".to_string(),
                    type_params: Vec::new(),
                    params: vec![
                        FunctionTypeParamIr {
                            name: "self".to_string(),
                            ty: TypeRefIr::builtin("Self"),
                        },
                        FunctionTypeParamIr {
                            name: "input".to_string(),
                            ty: api_request,
                        },
                    ],
                    return_type: TypeRefIr::Builtin {
                        name: "Stream".to_string(),
                        args: vec![api_event],
                    },
                    is_native: false,
                    is_provider: false,
                    is_static: false,
                    implicit_self: None,
                }],
                source_span: None,
            },
        );
        let package_facts = vec![
            TypeResolutionPackageFacts {
                package_id: "api.pkg",
                dependencies: Vec::new(),
                schema_types: vec![
                    TypeResolutionPackageSchemaTypeFact {
                        public_path: "Request",
                        source_module: "api.types",
                        source_symbol: "Request",
                        kind: PublicTypeKind::Type,
                        source_ast: api_parsed[0].ast(),
                        file_ir_unit: None,
                    },
                    TypeResolutionPackageSchemaTypeFact {
                        public_path: "Event",
                        source_module: "api.types",
                        source_symbol: "Event",
                        kind: PublicTypeKind::Type,
                        source_ast: api_parsed[0].ast(),
                        file_ir_unit: None,
                    },
                ],
                callables: Vec::new(),
            },
            TypeResolutionPackageFacts {
                package_id: "agent.pkg",
                dependencies: vec![TypeResolutionPackageDependencyFact {
                    alias: "api",
                    package_id: "api.pkg",
                }],
                schema_types: vec![
                    TypeResolutionPackageSchemaTypeFact {
                        public_path: "llm.Request",
                        source_module: "agent.llm",
                        source_symbol: "Request",
                        kind: PublicTypeKind::Alias,
                        source_ast: agent_parsed[0].ast(),
                        file_ir_unit: None,
                    },
                    TypeResolutionPackageSchemaTypeFact {
                        public_path: "llm.Event",
                        source_module: "agent.llm",
                        source_symbol: "Event",
                        kind: PublicTypeKind::Alias,
                        source_ast: agent_parsed[0].ast(),
                        file_ir_unit: None,
                    },
                    TypeResolutionPackageSchemaTypeFact {
                        public_path: "llm.Client",
                        source_module: "agent.llm",
                        source_symbol: "Client",
                        kind: PublicTypeKind::Interface,
                        source_ast: agent_parsed[0].ast(),
                        file_ir_unit: Some(&agent_unit),
                    },
                ],
                callables: Vec::new(),
            },
        ];
        let mut agent_dependency = PackageDependency::id("agent.pkg");
        agent_dependency.alias = Some("agent".to_string());
        let mut api_dependency = PackageDependency::id("api.pkg");
        api_dependency.alias = Some("api".to_string());
        let package_aliases = BTreeMap::from([
            ("agent".to_string(), vec![String::new()]),
            ("api".to_string(), vec![String::new()]),
        ]);
        let type_resolution = TypeResolutionModel::build(
            &parsed_sources,
            &package_aliases,
            &[agent_dependency, api_dependency],
            Some(&package_facts),
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution with package alias facts should build");
        let context = context();
        let actual = type_resolution
            .resolve_type_text("Host", &context)
            .expect("Host should resolve");
        let expected = type_resolution
            .resolve_type_text("agent.llm.Client", &context)
            .expect("package interface should resolve");

        assert!(
            type_resolution
                .concrete_type_conforms_to_interface(&actual, &expected, &context)
                .expect("conformance lookup should not fail")
                .is_some(),
            "package public aliases in interface method signatures should match service implementation signatures"
        );
    }

    #[test]
    fn package_interface_conformance_rejects_local_impl_signature_mismatch() {
        let (_parsed_sources, type_resolution) = package_type_resolution(
            r#"
              type Host implements pkg.Reader<string> {
                value: string,
              }

              impl Host {
                function read(fallback: number) -> string {
                  return "bad"
                }
              }
            "#,
        );
        let context = context();
        let actual = type_resolution
            .resolve_type_text("Host", &context)
            .expect("Host should resolve");
        let expected = type_resolution
            .resolve_type_text("any pkg.Reader<string>", &context)
            .expect("package interface should resolve");

        assert!(
            type_resolution
                .concrete_type_conforms_to_interface(&actual, &expected, &context)
                .expect("package conformance lookup should not fail")
                .is_none(),
            "package conformance must fail closed when local impl method signature mismatches"
        );
        assert!(
            type_resolution
                .local_any_interface_conformance_for_boxing(&actual, &expected, &context)
                .expect("package selector conformance lookup should not fail")
                .is_none(),
            "local method table slots must not be generated for mismatched package conformance"
        );
    }

    #[test]
    fn ordinary_assignability_does_not_use_interface_conformance() {
        let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
        let context = context();
        let actual = type_resolution
            .resolve_type_text("Box<string>", &context)
            .expect("actual type should resolve");
        let expected = resolved_test_interface(TypeRefIr::builtin("string"));

        assert!(
            !type_resolution.assignable_in_context(&actual, &expected, &context),
            "ordinary value assignability must not treat implements I as implicit interface boxing"
        );
    }

    #[test]
    fn concrete_type_conformance_matches_declared_interface_instantiation() {
        let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
        let context = context();
        let actual = type_resolution
            .resolve_type_text("Box<string>", &context)
            .expect("actual type should resolve");
        let expected = resolved_test_interface(TypeRefIr::builtin("string"));

        let matched = type_resolution
            .concrete_type_conforms_to_interface(&actual, &expected, &context)
            .expect("conformance lookup should not fail")
            .expect("Box<string> should conform to I<string>");

        assert_eq!(
            matched.receiver,
            SourceSymbolKey::new(MODULE, "Box"),
            "match should report the concrete receiver symbol"
        );
        assert_eq!(
            matched.implemented_interface_args,
            vec![TypeRefIr::builtin("string")]
        );
        assert_eq!(
            matched.expected_interface_args,
            vec![TypeRefIr::builtin("string")]
        );
    }

    #[test]
    fn concrete_type_conformance_rejects_mismatched_interface_args() {
        let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
        let context = context();
        let actual = type_resolution
            .resolve_type_text("Box<string>", &context)
            .expect("actual type should resolve");
        let expected = resolved_test_interface(TypeRefIr::builtin("number"));

        assert!(
            type_resolution
                .concrete_type_conforms_to_interface(&actual, &expected, &context)
                .expect("conformance lookup should not fail")
                .is_none(),
            "Box<string> must not conform to I<number>"
        );
    }

    #[test]
    fn concrete_type_conformance_requires_exact_nominal_receiver_and_interface() {
        let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
        let context = context();
        let expected = resolved_test_interface(TypeRefIr::builtin("string"));

        let nullable = type_resolution
            .resolve_type_text("Box<string>?", &context)
            .expect("nullable actual should resolve");
        let union = type_resolution
            .resolve_type_text("Box<string> | null", &context)
            .expect("union actual should resolve");
        let record = ResolvedTypeRef {
            ir: TypeRefIr::Record {
                fields: BTreeMap::from([("value".to_string(), TypeRefIr::builtin("string"))]),
            },
            source_text: "{ value: string }".to_string(),
        };
        let representation = type_resolution
            .resolve_type_text("Wrapped", &context)
            .expect("representation actual should resolve");
        let non_interface = type_resolution
            .resolve_type_text("Payload", &context)
            .expect("non-interface expected should resolve");

        for actual in [&nullable, &union, &record, &representation] {
            assert!(
                type_resolution
                    .concrete_type_conforms_to_interface(actual, &expected, &context)
                    .expect("conformance lookup should not fail")
                    .is_none(),
                "{:?} must not conform through nullable, union, record shape, or representation payload",
                actual.ir
            );
        }
        assert!(
            type_resolution
                .concrete_type_conforms_to_interface(&representation, &non_interface, &context)
                .expect("non-interface expected should not fail")
                .is_none(),
            "non-interface expected type should return None"
        );
    }

    #[test]
    fn json_contextual_assignability_remains_ordinary_value_behavior() {
        let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
        let context = context();
        let payload = type_resolution
            .resolve_type_text("Payload", &context)
            .expect("payload should resolve");
        let json = type_resolution
            .resolve_type_text("Json", &context)
            .expect("Json should resolve");
        let json_object = type_resolution
            .resolve_type_text("JsonObject", &context)
            .expect("JsonObject should resolve");

        assert!(type_resolution.assignable_in_context(&payload, &json, &context));
        assert!(type_resolution.assignable_in_context(&payload, &json_object, &context));
    }

    #[test]
    fn function_argument_check_does_not_implicitly_box_concrete_to_interface() {
        let (parsed_sources, type_resolution) = type_resolution(
            r#"
              interface I {}

              type Concrete implements I {
                value: string,
              }

              function accepts(input: I) -> void {}

              function run() -> void {
                accepts(Concrete { value: "x" })
              }
            "#,
        );
        let expression_sources = ExpressionSourceMap::build(&parsed_sources)
            .expect("expression source map should build");

        let error = ExpressionTypeModel::build(
            &parsed_sources,
            &expression_sources,
            &type_resolution,
            &crate::PublicationDbMetadataIndex::default(),
            None,
        )
        .expect_err("Concrete argument should not be assignable to bare interface parameter");

        let message = error.message();
        assert!(
            message.contains("argument"),
            "expected an argument assignability diagnostic, got: {message}"
        );
    }

    fn test_artifact_type_kind(
        descriptor: &TypeDescriptorIr,
        symbolic_types: &BTreeMap<(String, String), String>,
        is_alias: bool,
    ) -> Result<SourceTypeKind, String> {
        let symbolic_types = ArtifactSymbolicTypeIndex {
            by_symbol: symbolic_types.clone(),
            ..ArtifactSymbolicTypeIndex::default()
        };
        artifact_type_kind(
            descriptor,
            &symbolic_types,
            "example.pkg",
            &PackageTypeSymbolIndex::default(),
            "types",
            "PublicType",
            is_alias,
        )
    }

    #[test]
    fn artifact_descriptors_preserve_nested_records_arrays_aliases_and_literal_unions() {
        let descriptor = TypeDescriptorIr::Record {
            fields: BTreeMap::from([
                (
                    "items".to_string(),
                    TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::Record {
                            fields: BTreeMap::from([(
                                "label".to_string(),
                                TypeRefIr::builtin("string"),
                            )]),
                        }],
                    },
                ),
                (
                    "state".to_string(),
                    TypeRefIr::Union {
                        items: vec![
                            TypeRefIr::Literal {
                                value: LiteralIr::String {
                                    value: "ready".to_string(),
                                },
                            },
                            TypeRefIr::Literal {
                                value: LiteralIr::String {
                                    value: "done".to_string(),
                                },
                            },
                        ],
                    },
                ),
                (
                    "format".to_string(),
                    TypeRefIr::Nullable {
                        inner: Box::new(TypeRefIr::Union {
                            items: vec![
                                TypeRefIr::Literal {
                                    value: LiteralIr::String {
                                        value: "chat".to_string(),
                                    },
                                },
                                TypeRefIr::Literal {
                                    value: LiteralIr::String {
                                        value: "responses".to_string(),
                                    },
                                },
                            ],
                        }),
                    },
                ),
                (
                    "header".to_string(),
                    TypeRefIr::PackageSymbol {
                        symbol: PackageSymbolRef {
                            package: PackageRefIr::PackageId {
                                package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
                            },
                            symbol_path: "std.http.HttpHeader".to_string(),
                            abi_expectation: None,
                        },
                    },
                ),
            ]),
        };
        let SourceTypeKind::Record {
            fields,
            canonical_fields: Some(canonical_fields),
        } = test_artifact_type_kind(&descriptor, &BTreeMap::new(), false)
            .expect("descriptor should be self-contained")
        else {
            panic!("record descriptor should remain a record")
        };
        assert_eq!(fields["items"], "Array<{ label: string }>");
        assert_eq!(fields["state"], "\"ready\" | \"done\"");
        assert!(matches!(
            &canonical_fields["format"],
            TypeRefIr::Nullable { inner }
                if matches!(inner.as_ref(), TypeRefIr::Union { items } if items.len() == 2)
        ));
        assert_eq!(fields["header"], "std.http.HttpHeader");

        let alias = test_artifact_type_kind(
            &TypeDescriptorIr::Alias {
                target: TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::builtin("string")],
                },
            },
            &BTreeMap::new(),
            true,
        )
        .expect("alias descriptor should be self-contained");
        assert!(matches!(
            alias,
            SourceTypeKind::Alias { target, .. } if target == "Array<string>"
        ));

        let representation = test_artifact_type_kind(
            &TypeDescriptorIr::Representation {
                representation: TypeRefIr::builtin("string"),
            },
            &BTreeMap::new(),
            false,
        )
        .expect("a nominal representation keeps its declaration kind");
        assert!(matches!(
            representation,
            SourceTypeKind::Representation { target, .. } if target == "string"
        ));

        let callback = test_artifact_type_kind(
            &TypeDescriptorIr::Alias {
                target: TypeRefIr::Function {
                    params: vec![FunctionTypeParamIr {
                        name: "status".to_string(),
                        ty: TypeRefIr::Union {
                            items: vec![
                                TypeRefIr::Literal {
                                    value: LiteralIr::String {
                                        value: "running".to_string(),
                                    },
                                },
                                TypeRefIr::Literal {
                                    value: LiteralIr::String {
                                        value: "completed".to_string(),
                                    },
                                },
                            ],
                        },
                    }],
                    return_type: Box::new(TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::builtin("string")],
                    }),
                },
            },
            &BTreeMap::new(),
            true,
        )
        .expect("callback alias descriptor should stay exact");
        assert!(matches!(
            callback,
            SourceTypeKind::Alias {
                canonical_target: Some(TypeRefIr::Function { params, return_type }),
                ..
            } if matches!(
                params.as_slice(),
                [FunctionTypeParamIr {
                    ty: TypeRefIr::Union { items },
                    ..
                }] if items.len() == 2
            ) && matches!(
                return_type.as_ref(),
                TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1
            )
        ));
    }

    #[test]
    fn aliases_expand_exactly_through_callbacks_and_nested_structural_types() {
        let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root should resolve");
        crate::prelude_registry::initialize_prelude_registry(
            &skiff_compiler_input::CompilerPlatformSources::new(&platform_root)
                .expect("platform sources should load"),
        )
        .expect("prelude registry should initialize");

        let (_parsed, model) = type_resolution(
            r#"
              type Payload {
                value: string
              }

              alias Status = "running" | "completed"
              alias Payloads = Array<Payload?>
              alias Handler = fn(status: Status) -> Payloads
            "#,
        );

        let handler = model
            .resolve_type_text("Handler", &context())
            .expect("callback alias should resolve to its exact RHS");
        let TypeRefIr::Function {
            params,
            return_type,
        } = handler.ir
        else {
            panic!("Handler must expand to a callback type");
        };
        assert!(matches!(
            params.as_slice(),
            [FunctionTypeParamIr {
                name,
                ty: TypeRefIr::Union { items },
            }] if name == "status"
                && items.len() == 2
                && items.iter().all(|item| matches!(
                    item,
                    TypeRefIr::Literal {
                        value: LiteralIr::String { .. }
                    }
                ))
        ));
        assert!(matches!(
            return_type.as_ref(),
            TypeRefIr::Builtin { name, args }
                if name == "Array"
                    && matches!(
                        args.as_slice(),
                        [TypeRefIr::Nullable { inner }]
                            if matches!(inner.as_ref(), TypeRefIr::LocalType { .. })
                    )
        ));

        let missing = type_resolution("alias MissingAlias = Missing");
        let error = missing
            .1
            .resolve_type_text("MissingAlias", &context())
            .expect_err("an alias with an unresolved RHS must fail closed");
        assert!(error.contains("unresolved type `Missing`"));
    }

    #[test]
    fn artifact_descriptors_reject_non_self_describing_local_indices() {
        let error = test_artifact_type_kind(
            &TypeDescriptorIr::Alias {
                target: TypeRefIr::LocalType { type_index: 7 },
            },
            &BTreeMap::new(),
            true,
        )
        .expect_err("ambient FileIR lookup must not be used");
        assert!(error.contains("not self-describing"));
    }

    #[test]
    fn artifact_descriptors_resolve_only_exported_symbolic_type_closure() {
        let symbol = ServiceSymbolRef {
            module_path: "types".to_string(),
            symbol: "LlmContentPart".to_string(),
        };
        let descriptor = TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "content".to_string(),
                TypeRefIr::Builtin {
                    name: "Array".to_string(),
                    args: vec![TypeRefIr::ServiceSymbol {
                        symbol: symbol.clone(),
                    }],
                },
            )]),
        };
        let symbolic_types = BTreeMap::from([(
            (symbol.module_path.clone(), symbol.symbol.clone()),
            "LlmContentPart".to_string(),
        )]);
        let SourceTypeKind::Record { fields, .. } =
            test_artifact_type_kind(&descriptor, &symbolic_types, false)
                .expect("public symbolic type should reconstruct")
        else {
            panic!("record descriptor should remain a record")
        };
        assert_eq!(fields["content"], "Array<LlmContentPart>");

        let error = test_artifact_type_kind(&descriptor, &BTreeMap::new(), false)
            .expect_err("a private or missing symbolic type must fail closed");
        assert!(error.contains("identity-validated selected artifact type"));

        let db_error = test_artifact_type_kind(
            &TypeDescriptorIr::Alias {
                target: TypeRefIr::DbObjectSymbol { symbol },
            },
            &symbolic_types,
            true,
        )
        .expect_err("db object symbols are not package-public type facts");
        assert!(db_error.contains("no package type semantics"));
    }

    #[test]
    fn package_record_field_qualification_uses_the_exact_dependency_root() {
        assert_eq!(
            qualify_package_type_text(
                "chatgptPlan.OauthError?",
                "llmProviders",
                &BTreeSet::from(["chatgptPlan.OauthError".to_string()]),
            ),
            "llmProviders.chatgptPlan.OauthError?"
        );
        assert_eq!(
            qualify_package_type_text(
                "chatgptPlan.OauthError?",
                "llmProviders/",
                &BTreeSet::from(["chatgptPlan.OauthError".to_string()]),
            ),
            "llmProviders/chatgptPlan.OauthError?"
        );
    }

    #[test]
    fn artifact_exported_interface_facts_preserve_classification_and_methods() {
        use skiff_artifact_model::{
            PackageBuildId, PackageImplementationLinks, PackageLocalAbi, PackageLocalAbiIdentity,
            PackageRuntimeRequirements, TypeExport,
        };

        let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("workspace root should resolve");
        crate::prelude_registry::initialize_prelude_registry(
            &skiff_compiler_input::CompilerPlatformSources::new(&platform_root)
                .expect("platform sources should load"),
        )
        .expect("prelude registry should initialize");

        let file = skiff_artifact_model::FileIrRef {
            file_ir_identity: "file-ir".to_string(),
            artifact_path: Some("llm.json".to_string()),
            module_path: "llm".to_string(),
            source_ast_hash: Some("source".to_string()),
        };
        let method = InterfaceMethodSignature {
            name: "complete".to_string(),
            type_params: Vec::new(),
            params: vec![
                FunctionTypeParamIr {
                    name: "self".to_string(),
                    ty: TypeRefIr::TypeParam {
                        name: "Self".to_string(),
                    },
                },
                FunctionTypeParamIr {
                    name: "input".to_string(),
                    ty: TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::Nullable {
                            inner: Box::new(TypeRefIr::LocalType { type_index: 7 }),
                        }],
                    },
                },
            ],
            return_type: TypeRefIr::Union {
                items: vec![
                    TypeRefIr::LocalType { type_index: 7 },
                    TypeRefIr::builtin("null"),
                ],
            },
            may_suspend: false,
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self: None,
        };
        let descriptor = TypeDescriptorIr::Interface;
        let tool_descriptor = TypeDescriptorIr::Record {
            fields: BTreeMap::from([("name".to_string(), TypeRefIr::builtin("string"))]),
        };
        let role_descriptor = TypeDescriptorIr::Alias {
            target: TypeRefIr::Union {
                items: vec![
                    TypeRefIr::Literal {
                        value: LiteralIr::String {
                            value: "user".to_string(),
                        },
                    },
                    TypeRefIr::Literal {
                        value: LiteralIr::String {
                            value: "assistant".to_string(),
                        },
                    },
                ],
            },
        };
        let message_descriptor = TypeDescriptorIr::Record {
            fields: BTreeMap::from([(
                "role".to_string(),
                TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: "llm".to_string(),
                        symbol: "LlmRole".to_string(),
                    },
                },
            )]),
        };
        let artifact = PackageArtifact {
            schema_version: "skiff-package-artifact-v7".to_string(),
            package_id: "llm-api".to_string(),
            package_version: "1.0.0".to_string(),
            package_build_id: PackageBuildId::new("build"),
            files: vec![file.clone()],
            static_resources: Vec::new(),
            package_local_abi: PackageLocalAbi {
                local_abi_identity: PackageLocalAbiIdentity::new("abi"),
                public_symbols: BTreeMap::from([
                    (
                        "types.LlmClient".to_string(),
                        PackageLocalAbiSymbol::Type {
                            local_type_id: "type:types.LlmClient".to_string(),
                            descriptor: descriptor.clone(),
                            is_alias: false,
                            is_interface: true,
                            type_params: Vec::new(),
                            interface_methods: vec![method.clone()],
                        },
                    ),
                    (
                        "tools.ToolDeclaration".to_string(),
                        PackageLocalAbiSymbol::Type {
                            local_type_id: "type:tools.ToolDeclaration".to_string(),
                            descriptor: tool_descriptor.clone(),
                            is_alias: false,
                            is_interface: false,
                            type_params: Vec::new(),
                            interface_methods: Vec::new(),
                        },
                    ),
                    (
                        "LlmRole".to_string(),
                        PackageLocalAbiSymbol::Type {
                            local_type_id: "type:LlmRole".to_string(),
                            descriptor: role_descriptor.clone(),
                            is_alias: true,
                            is_interface: false,
                            type_params: Vec::new(),
                            interface_methods: Vec::new(),
                        },
                    ),
                    (
                        "LlmMessage".to_string(),
                        PackageLocalAbiSymbol::Type {
                            local_type_id: "type:LlmMessage".to_string(),
                            descriptor: message_descriptor.clone(),
                            is_alias: false,
                            is_interface: false,
                            type_params: Vec::new(),
                            interface_methods: Vec::new(),
                        },
                    ),
                ]),
                implementation_symbols: BTreeMap::new(),
            },
            package_schema_index: skiff_artifact_model::PackageSchemaIndexRef {
                package_id: "llm-api".to_string(),
                package_schema_index_identity: "index".into(),
            },
            package_schema_type_records: BTreeMap::new(),
            implementation_links: PackageImplementationLinks {
                types: BTreeMap::from([
                    (
                        "types.LlmClient".to_string(),
                        TypeExport {
                            file: file.clone(),
                            type_index: 0,
                            symbol: "LlmClient".to_string(),
                            is_interface: true,
                            descriptor: Some(descriptor),
                            type_params: Vec::new(),
                            interface_methods: vec![method],
                        },
                    ),
                    (
                        "tools.ToolDeclaration".to_string(),
                        TypeExport {
                            file: file.clone(),
                            type_index: 7,
                            symbol: "ToolDeclaration".to_string(),
                            is_interface: false,
                            descriptor: Some(tool_descriptor),
                            type_params: Vec::new(),
                            interface_methods: Vec::new(),
                        },
                    ),
                    (
                        "LlmRole".to_string(),
                        TypeExport {
                            file: file.clone(),
                            type_index: 8,
                            symbol: "LlmRole".to_string(),
                            is_interface: false,
                            descriptor: Some(role_descriptor),
                            type_params: Vec::new(),
                            interface_methods: Vec::new(),
                        },
                    ),
                    (
                        "LlmMessage".to_string(),
                        TypeExport {
                            file,
                            type_index: 9,
                            symbol: "LlmMessage".to_string(),
                            is_interface: false,
                            descriptor: Some(message_descriptor),
                            type_params: Vec::new(),
                            interface_methods: Vec::new(),
                        },
                    ),
                ]),
                ..PackageImplementationLinks::default()
            },
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
            service_call_roots: Vec::new(),
            service_call_refs: Vec::new(),
        };
        let mut package_types = BTreeMap::new();
        let mut package_interfaces = BTreeMap::new();
        index_artifact_package_types(
            &artifact,
            "llm-api",
            PackageDependencyAccess::Public,
            ArtifactPackageTypePathMode::DeclaredPublic,
            &mut package_types,
            &mut package_interfaces,
            &mut BTreeMap::new(),
        )
        .expect("identity-verified artifact ABI facts should index");

        let interface = package_interfaces
            .get(&PackageSymbolKey {
                dependency_ref: "llm-api".to_string(),
                symbol_path: "LlmClient".to_string(),
            })
            .expect("exported interface classification should survive publication");
        assert_eq!(interface.methods.len(), 1);
        assert_eq!(interface.methods[0].name, "complete");
        assert!(matches!(
            interface.methods[0].return_type,
            TypeRefIr::Union { .. }
        ));
        assert_eq!(interface.methods[0].params[0].name, "self");
        assert_eq!(
            interface.methods[0].params[0].ty,
            TypeRefIr::builtin("Self")
        );

        let consumer_sources = parsed_sources(
            r#"
              import llmApi

              type LocalClient implements llmApi.LlmClient {}

              impl LocalClient {
                function complete(input: Array<llmApi.ToolDeclaration?>) -> llmApi.ToolDeclaration | null {
                  return null
                }
              }
            "#,
        );
        let mut dependency = PackageDependency::id("llm-api");
        dependency.alias = Some("llmApi".to_string());
        let model = TypeResolutionModel::build(
            &consumer_sources,
            &BTreeMap::from([("llmApi".to_string(), vec![String::new()])]),
            &[dependency],
            None,
            Some(std::slice::from_ref(&artifact)),
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("artifact-only package interface facts should build");
        let message = model
            .resolve_type_text("llmApi.LlmMessage", &context())
            .expect("package record should resolve");
        let role = model
            .record_field_type(&message, "role", &context())
            .expect("package record projection should recover its nominal field");
        let expected_role = model
            .resolve_type_text("llmApi.LlmRole", &context())
            .expect("package alias should resolve");
        assert_eq!(role.ir, expected_role.ir);
        assert!(model.assignable(&role, &expected_role));
        assert!(
            !model.assignable(
                &ResolvedTypeRef {
                    ir: TypeRefIr::PackageSymbol {
                        symbol: PackageSymbolRef {
                            package: PackageRefIr::PackageId {
                                package_id: "other.example/llm-api".to_string(),
                            },
                            symbol_path: "LlmRole".to_string(),
                            abi_expectation: None,
                        },
                    },
                    source_text: "otherRole.LlmRole".to_string(),
                },
                &expected_role,
            ),
            "same-shaped type from another package owner must remain nominally distinct"
        );
        let actual = model
            .resolve_type_text("LocalClient", &context())
            .expect("local implementation type should resolve");
        let expected = model
            .resolve_type_text("llmApi.LlmClient", &context())
            .expect("imported public interface should resolve");
        let conformance = model
            .local_any_interface_conformance_for_boxing(&actual, &expected, &context())
            .expect("artifact-backed interface conformance lookup should not fail")
            .expect("declared imported interface implementation should match for boxing");
        let TypeRefIr::PackageSymbol { symbol } =
            serde_json::from_str::<TypeRefIr>(&conformance.interface.interface_abi_id)
                .expect("interface ABI identity should decode")
        else {
            panic!("imported interface must retain package identity")
        };
        assert_eq!(
            symbol.package,
            PackageRefIr::PackageId {
                package_id: "llm-api".to_string()
            }
        );
        assert_eq!(symbol.symbol_path, "types.LlmClient");

        let mut tampered = artifact;
        tampered
            .implementation_links
            .types
            .get_mut("types.LlmClient")
            .unwrap()
            .interface_methods
            .clear();
        let error = index_artifact_package_types(
            &tampered,
            "llm-api",
            PackageDependencyAccess::Public,
            ArtifactPackageTypePathMode::DeclaredPublic,
            &mut BTreeMap::new(),
            &mut BTreeMap::new(),
            &mut BTreeMap::new(),
        )
        .expect_err("mismatched artifact interface facts must fail closed");
        assert!(error.contains("interface facts disagree"));
    }

    #[test]
    fn artifact_interface_receiver_reconstruction_fails_closed() {
        let method = |params, implicit_self| InterfaceMethodSignature {
            name: "streamChat".to_string(),
            type_params: vec!["Chunk".to_string()],
            params,
            return_type: TypeRefIr::builtin("string"),
            may_suspend: false,
            is_native: false,
            is_provider: false,
            is_static: false,
            implicit_self,
        };
        let self_param = FunctionTypeParamIr {
            name: "self".to_string(),
            ty: TypeRefIr::TypeParam {
                name: "Self".to_string(),
            },
        };

        let missing = reconstruct_artifact_interface_methods(
            "llm-api",
            "LlmClient",
            &[method(Vec::new(), None)],
        )
        .expect_err("missing receiver must fail closed");
        assert!(missing.contains("missing self: Self"));

        let wrong = reconstruct_artifact_interface_methods(
            "llm-api",
            "LlmClient",
            &[method(
                vec![FunctionTypeParamIr {
                    name: "self".to_string(),
                    ty: TypeRefIr::builtin("string"),
                }],
                None,
            )],
        )
        .expect_err("non-Self receiver must fail closed");
        assert!(wrong.contains("non-Self receiver"));

        let duplicate = reconstruct_artifact_interface_methods(
            "llm-api",
            "LlmClient",
            &[method(
                vec![self_param],
                Some(TypeRefIr::TypeParam {
                    name: "Self".to_string(),
                }),
            )],
        )
        .expect_err("duplicate receiver must fail closed");
        assert!(duplicate.contains("duplicate receivers"));
    }

    #[test]
    fn publication_type_slots_use_their_exact_owner_module() {
        let cleanup_source = CompilerSourceFile::parse(
            PathBuf::from("child_cleanup.skiff"),
            "child_cleanup".to_string(),
            false,
            false,
            r#"
              alias ChildCleanupEligibilityScope = "force" | "global" | "parent"

              type ChildCleanupConsumeResult {
                consumed: Bool
              }
            "#
            .to_string(),
            "child_cleanup.skiff",
        )
        .expect("cleanup source should parse");
        let consumer_source = CompilerSourceFile::parse(
            PathBuf::from("consumer.skiff"),
            "consumer".to_string(),
            false,
            false,
            "type Unrelated { value: String }".to_string(),
            "consumer.skiff",
        )
        .expect("consumer source should parse");
        let parsed =
            parse_publication_sources(&PathBuf::from("/test"), &[cleanup_source, consumer_source])
                .expect("multi-file publication should parse");
        let model = TypeResolutionModel::build(
            &parsed,
            &BTreeMap::new(),
            &[],
            None,
            None,
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution should build");

        let consume_result = model.canonicalize_type_ref_for_module(
            "consumer",
            &TypeRefIr::PublicationType {
                module_path: "child_cleanup".to_string(),
                type_index: 0,
            },
        );
        assert_eq!(
            consume_result,
            canonical_named_symbol("child_cleanup.ChildCleanupConsumeResult")
        );
        let eligibility_alias = model
            .canonicalize_package_interface_signature_type(
                "child_cleanup",
                &skiff_artifact_model::PackageTypeRef::Local {
                    local_type: TypeRefIr::ServiceSymbol {
                        symbol: ServiceSymbolRef {
                            module_path: "child_cleanup".to_string(),
                            symbol: "ChildCleanupEligibilityScope".to_string(),
                        },
                    },
                },
            )
            .expect("same-publication alias should canonicalize");
        assert!(matches!(
            eligibility_alias,
            skiff_artifact_model::PackageTypeRef::Local {
                local_type: TypeRefIr::Union { ref items }
            } if items.len() == 3
        ));

        let unknown = TypeRefIr::PublicationType {
            module_path: "missing".to_string(),
            type_index: 0,
        };
        assert_eq!(
            model.canonicalize_type_ref_for_module("consumer", &unknown),
            unknown,
            "an unknown owner module must not fall back to the caller module"
        );
    }
}

fn source_path(module_path: &str, symbol: &str) -> String {
    format!("{module_path}.{symbol}")
}
