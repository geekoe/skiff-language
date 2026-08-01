use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_identity::{
    canonical_interface_method_abi_id, interface_instantiation_ref, type_ref_abi_key,
};
use skiff_artifact_model::{
    ActorAbiInput, ActorCreateSignatureIr, ActorFieldIr, ActorPublicMethodIr,
    ContractTypeDescriptor, ContractTypeRef, FileIrUnit, FunctionTypeParamIr,
    InterfaceInstantiationRef, LiteralIr, NamedUnionBranchIr, NominalTypeRefBaseIr,
    PackageActorAbi, PackageArtifact, PackageBuildId, PackageLocalAbiIdentity,
    PackageLocalAbiSymbol, PackageRefIr, PackageSchemaTypeRecord, PackageSymbolRef, PackageTypeRef,
    ServiceSymbolRef, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::{
    prelude_registry::canonical_file_ir_builtin_name,
    type_ref::{
        contains_type_param, debug_text, is_null_type, normalize_union, record_field_type,
        substitute_type_params_in_type_ref_ref, BuiltinShape,
    },
};

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
        ast::{AliasDecl, FunctionDecl, InterfaceOperation, Param, SourceFile, TypeDecl, TypeRef},
        id::SKIFF_STD_PUBLICATION_ID,
        package_interface_methods::{
            instantiate_interface_method_signatures, normalize_package_interface_method_signatures,
            normalize_package_interface_type_ref, package_interface_method_signatures,
            InterfaceMethodSignature, PackageTypeSymbolIndex,
        },
        prelude_registry::prelude_registry,
        type_expr::TypeExpr,
        type_syntax::generic_type_parameter_names,
    },
};
use compiler_input_model::PackageDependency;

use super::{
    api::PublicTypeKind, type_indices, type_text_with_args, LocalDbObjectIndex,
    PackageInterfaceMethodIndex, PublicationTypeSymbolIndex, SourceDependencyAnalysisInput,
    SourceSymbolKey,
};

mod catch_leaves;
mod index;
mod query;
mod shape_assignability;

pub use catch_leaves::{CatchLeafIdentity, CatchLeaves};

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedTypeRef {
    pub ir: TypeRefIr,
    pub source_text: Option<String>,
}

impl ResolvedTypeRef {
    pub fn new(ir: TypeRefIr) -> Self {
        Self {
            ir,
            source_text: None,
        }
    }

    pub fn with_text(ir: TypeRefIr, text: String) -> Self {
        Self {
            ir,
            source_text: Some(text),
        }
    }
}

impl std::fmt::Display for ResolvedTypeRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.source_text {
            Some(text) => f.write_str(text),
            None => write!(f, "{}", debug_text(&self.ir)),
        }
    }
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
    package_dependency_views: BTreeMap<String, PackageDependencyView>,
    package_dependency_canonical_refs: BTreeMap<String, String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PackageDependencyView {
    Public,
    TopLevel,
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
        key_field: String,
        fields: BTreeMap<String, String>,
        create: Option<Vec<(String, String)>>,
        /// Exact normalized artifact type references for a package actor.
        /// Source actors keep these `None` and resolve field text lazily.
        canonical_id_type: Option<TypeRefIr>,
        canonical_fields: Option<BTreeMap<String, TypeRefIr>>,
        canonical_create: Option<Vec<(String, TypeRefIr)>>,
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
    source_callable: SourceSymbolKey,
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
    pub name: String,
    pub module_path: String,
    pub id_type: ResolvedTypeRef,
    pub key_field: String,
    pub fields: BTreeMap<String, ResolvedTypeRef>,
    pub create: Option<Vec<(String, ResolvedTypeRef)>>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct PackageReceiverMethodResolution {
    pub dependency_ref: String,
    pub canonical_dependency_ref: String,
    pub expected_local_abi: PackageLocalAbiIdentity,
    pub expected_package_build: PackageBuildId,
    pub source_method_path: String,
    pub receiver_type_params: Vec<String>,
    pub receiver_type_arguments: Vec<TypeRefIr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalReceiverMethodResolution {
    pub source_callable: SourceSymbolKey,
    pub receiver_type_arguments: Vec<TypeRefIr>,
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

fn index_compiler_owned_package_artifacts(
    package_artifacts: Option<&[PackageArtifact]>,
    dependencies: &SourceDependencyAnalysisInput,
    package_types: &mut BTreeMap<PackageSymbolKey, SourceTypeResolution>,
    package_interfaces: &mut BTreeMap<PackageSymbolKey, PackageInterfaceFact>,
    package_type_slots: &mut BTreeMap<(String, String, u32), String>,
    package_type_source_paths: &mut BTreeMap<(String, String, String), String>,
    package_constants: &mut BTreeMap<PackageSymbolKey, PackageConstantResolution>,
    package_dependencies: &mut BTreeMap<String, String>,
    package_dependency_views: &mut BTreeMap<String, PackageDependencyView>,
    package_dependency_canonical_refs: &mut BTreeMap<String, String>,
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
        let view = PackageDependencyView::Public;
        index_artifact_package_types(
            artifact,
            alias,
            view,
            ArtifactPackageTypePathMode::CompilerOwnedExact,
            package_types,
            package_interfaces,
            package_type_slots,
        )?;
        index_artifact_package_type_source_paths(artifact, alias, view, package_type_source_paths)?;
        index_artifact_package_constants(artifact, alias, alias, view, package_constants)?;
        package_dependencies.insert(alias.to_string(), artifact.package_id.clone());
        package_dependency_views.insert(alias.to_string(), view);
        package_dependency_canonical_refs.insert(alias.to_string(), alias.to_string());
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
    view: PackageDependencyView,
    path_mode: ArtifactPackageTypePathMode,
    package_types: &mut BTreeMap<PackageSymbolKey, SourceTypeResolution>,
    package_interfaces: &mut BTreeMap<PackageSymbolKey, PackageInterfaceFact>,
    package_type_slots: &mut BTreeMap<(String, String, u32), String>,
) -> Result<(), String> {
    let symbols = match view {
        PackageDependencyView::Public => &artifact.package_local_abi.public_symbols,
        PackageDependencyView::TopLevel => &artifact.package_local_abi.implementation_symbols,
    };
    let type_symbols = artifact_package_type_symbol_index(artifact, symbols)?;
    let symbolic_types = artifact_symbolic_type_index(artifact, symbols, &type_symbols)?;
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
            actor,
        } = symbol
        else {
            continue;
        };
        let expected_type_id = match view {
            PackageDependencyView::Public => format!("type:{selected_path}"),
            PackageDependencyView::TopLevel => {
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
        let kind = match actor {
            Some(actor) => {
                if *is_alias || *is_interface {
                    return Err(format!(
                        "package {} actor type {} cannot be an alias or interface declaration",
                        artifact.package_id, selected_path
                    ));
                }
                if !matches!(descriptor, TypeDescriptorIr::Record { .. }) {
                    return Err(format!(
                        "package {} actor type {} must attach to a record declaration",
                        artifact.package_id, selected_path
                    ));
                }
                let normalized = normalize_artifact_actor_abi(
                    &artifact.package_id,
                    &type_symbols,
                    &export.file.module_path,
                    selected_path,
                    actor,
                )?;
                let key_field = normalized.abi.key_field.clone();
                let fields = normalized
                    .abi
                    .fields
                    .iter()
                    .map(|field| {
                        Ok((
                            field.name.clone(),
                            artifact_type_text(&artifact.package_id, &field.ty, &symbolic_types)?,
                        ))
                    })
                    .collect::<Result<BTreeMap<_, _>, String>>()?;
                let create = normalized
                    .abi
                    .create
                    .as_ref()
                    .map(|create| {
                        create
                            .parameters
                            .iter()
                            .map(|parameter| {
                                Ok((
                                    parameter.name.clone(),
                                    artifact_type_text(
                                        &artifact.package_id,
                                        &parameter.ty,
                                        &symbolic_types,
                                    )?,
                                ))
                            })
                            .collect::<Result<Vec<_>, String>>()
                    })
                    .transpose()?;
                let canonical_fields = Some(
                    normalized
                        .abi
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), field.ty.clone()))
                        .collect::<BTreeMap<_, _>>(),
                );
                let canonical_create = normalized.abi.create.as_ref().map(|create| {
                    create
                        .parameters
                        .iter()
                        .map(|parameter| (parameter.name.clone(), parameter.ty.clone()))
                        .collect::<Vec<_>>()
                });
                SourceTypeKind::Actor {
                    key_field,
                    fields,
                    create,
                    canonical_id_type: Some(normalized.abi.actor_id_type),
                    canonical_fields,
                    canonical_create,
                }
            }
            None => kind,
        };
        let resolution = SourceTypeResolution {
            name: name.to_string(),
            type_params: type_params.clone(),
            local_type_names: local_type_names.clone(),
            kind,
            module_path: module_path.to_string(),
            public_path: Some(selected_path.clone()),
        };
        let indexed_paths = match (view, path_mode) {
            (PackageDependencyView::Public, ArtifactPackageTypePathMode::DeclaredPublic) => {
                vec![selected_path.as_str(), name]
            }
            (PackageDependencyView::Public, ArtifactPackageTypePathMode::CompilerOwnedExact)
            | (PackageDependencyView::TopLevel, _) => vec![selected_path.as_str()],
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
    view: PackageDependencyView,
    source_paths: &mut BTreeMap<(String, String, String), String>,
) -> Result<(), String> {
    let symbols = match view {
        PackageDependencyView::Public => &artifact.package_local_abi.public_symbols,
        PackageDependencyView::TopLevel => &artifact.package_local_abi.implementation_symbols,
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
    canonical_dependency_ref: &str,
    view: PackageDependencyView,
    package_constants: &mut BTreeMap<PackageSymbolKey, PackageConstantResolution>,
) -> Result<(), String> {
    let symbols = match view {
        PackageDependencyView::Public => &artifact.package_local_abi.public_symbols,
        PackageDependencyView::TopLevel => &artifact.package_local_abi.implementation_symbols,
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
                    dependency_ref: canonical_dependency_ref.to_string(),
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

fn artifact_package_type_symbol_index(
    artifact: &PackageArtifact,
    selected_symbols: &BTreeMap<String, PackageLocalAbiSymbol>,
) -> Result<PackageTypeSymbolIndex, String> {
    let mut index = PackageTypeSymbolIndex::default();
    for requirement in &artifact.package_requirements {
        index.insert_dependency(&requirement.alias, &requirement.package_id);
        index.insert_dependency(&requirement.package_id, &requirement.package_id);
    }
    // The selected ABI surface owns the canonical path. This is observable for
    // top-level access when a source type is also exported under an API alias:
    // source-only selection must not be rebound to that unrelated public path.
    for (selected_path, symbol) in selected_symbols {
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
        index.insert_type(
            &export.file.module_path,
            export.type_index,
            &export.symbol,
            selected_path,
        );
    }
    for (public_path, export) in &artifact.implementation_links.types {
        index.insert_type(
            &export.file.module_path,
            export.type_index,
            &export.symbol,
            public_path,
        );
    }
    Ok(index)
}

fn artifact_symbolic_type_index(
    artifact: &PackageArtifact,
    symbols: &BTreeMap<String, PackageLocalAbiSymbol>,
    type_symbols: &PackageTypeSymbolIndex,
) -> Result<ArtifactSymbolicTypeIndex, String> {
    let mut index = ArtifactSymbolicTypeIndex::default();
    for (selected_path, symbol) in symbols {
        let PackageLocalAbiSymbol::Type {
            descriptor,
            is_interface,
            type_params,
            interface_methods,
            actor,
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
        let implementation_descriptor = normalize_artifact_type_descriptor(
            &artifact.package_id,
            type_symbols,
            &export.file.module_path,
            selected_path,
            descriptor,
        )?;
        let linked_descriptor = export
            .descriptor
            .as_ref()
            .map(|descriptor| {
                normalize_artifact_type_descriptor(
                    &artifact.package_id,
                    type_symbols,
                    &export.file.module_path,
                    selected_path,
                    descriptor,
                )
            })
            .transpose()?;
        if linked_descriptor.as_ref() != Some(&implementation_descriptor) {
            return Err(format!(
                "package {} selected type {} descriptor disagrees with its implementation link",
                artifact.package_id, selected_path
            ));
        }
        let implementation_methods = normalize_artifact_interface_methods(
            &artifact.package_id,
            type_symbols,
            &export.file.module_path,
            selected_path,
            interface_methods,
        )?;
        let linked_methods = normalize_artifact_interface_methods(
            &artifact.package_id,
            type_symbols,
            &export.file.module_path,
            selected_path,
            &export.interface_methods,
        )?;
        if export.is_interface != *is_interface
            || export.type_params != *type_params
            || linked_methods != implementation_methods
        {
            return Err(format!(
                "package {} selected type {} interface facts disagree with its implementation link",
                artifact.package_id, selected_path
            ));
        }
        let implementation_actor = actor
            .as_ref()
            .map(|actor| {
                normalize_artifact_actor_abi(
                    &artifact.package_id,
                    type_symbols,
                    &export.file.module_path,
                    selected_path,
                    actor,
                )
            })
            .transpose()?;
        let linked_actor = export
            .actor
            .as_ref()
            .map(|actor| {
                normalize_artifact_actor_abi(
                    &artifact.package_id,
                    type_symbols,
                    &export.file.module_path,
                    selected_path,
                    actor,
                )
            })
            .transpose()?;
        if linked_actor != implementation_actor {
            return Err(format!(
                "package {} selected type {} actor facts disagree with its implementation link",
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

fn normalize_artifact_type_descriptor(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    selected_path: &str,
    descriptor: &TypeDescriptorIr,
) -> Result<TypeDescriptorIr, String> {
    let normalize = |ty: &TypeRefIr, context: &str| {
        let context = format!("type {selected_path} {context}");
        let normalized = normalize_package_interface_type_ref(
            package_id,
            type_symbols,
            module_path,
            ty,
            &context,
        )?;
        normalize_artifact_interface_identities(
            package_id,
            type_symbols,
            module_path,
            normalized,
            &context,
        )
    };
    match descriptor {
        TypeDescriptorIr::Record { fields } => Ok(TypeDescriptorIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| Ok((name.clone(), normalize(ty, &format!("field {name}"))?)))
                .collect::<Result<_, String>>()?,
        }),
        TypeDescriptorIr::Representation { representation } => {
            Ok(TypeDescriptorIr::Representation {
                representation: normalize(representation, "representation")?,
            })
        }
        TypeDescriptorIr::Union { branches } => Ok(TypeDescriptorIr::Union {
            branches: branches
                .iter()
                .map(|branch| match branch {
                    NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                        Ok(NamedUnionBranchIr::ConcreteNominal {
                            nominal_type: normalize(nominal_type, "union branch")?,
                        })
                    }
                    NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type,
                        discriminator_field,
                        discriminator_value,
                    } => Ok(NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type: normalize(payload_type, "union payload")?,
                        discriminator_field: discriminator_field.clone(),
                        discriminator_value: discriminator_value.clone(),
                    }),
                    NamedUnionBranchIr::Literal { value } => Ok(NamedUnionBranchIr::Literal {
                        value: value.clone(),
                    }),
                })
                .collect::<Result<_, String>>()?,
        }),
        TypeDescriptorIr::Alias { target } => Ok(TypeDescriptorIr::Alias {
            target: normalize(target, "alias target")?,
        }),
        TypeDescriptorIr::Interface => Ok(TypeDescriptorIr::Interface),
    }
}

fn normalize_artifact_actor_abi(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    selected_path: &str,
    actor: &PackageActorAbi,
) -> Result<PackageActorAbi, String> {
    let normalize = |ty: &TypeRefIr, context: &str| {
        let context = format!("actor {selected_path} {context}");
        let normalized = normalize_package_interface_type_ref(
            package_id,
            type_symbols,
            module_path,
            ty,
            &context,
        )?;
        normalize_artifact_interface_identities(
            package_id,
            type_symbols,
            module_path,
            normalized,
            &context,
        )
    };
    let normalize_parameters =
        |parameters: &[FunctionTypeParamIr]| -> Result<Vec<FunctionTypeParamIr>, String> {
            parameters
                .iter()
                .map(|parameter| {
                    Ok(FunctionTypeParamIr {
                        name: parameter.name.clone(),
                        ty: normalize(&parameter.ty, &format!("parameter {}", parameter.name))?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        };
    Ok(PackageActorAbi {
        actor_abi_identity: actor.actor_abi_identity.clone(),
        abi: ActorAbiInput {
            actor_name: actor.abi.actor_name.clone(),
            actor_id_type: normalize(&actor.abi.actor_id_type, "id type")?,
            key_field: actor.abi.key_field.clone(),
            fields: actor
                .abi
                .fields
                .iter()
                .map(|field| {
                    Ok(ActorFieldIr {
                        name: field.name.clone(),
                        ty: normalize(&field.ty, &format!("field {}", field.name))?,
                        encoding: field.encoding,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
            create: actor
                .abi
                .create
                .as_ref()
                .map(|create| -> Result<ActorCreateSignatureIr, String> {
                    Ok(ActorCreateSignatureIr {
                        parameters: normalize_parameters(&create.parameters)?,
                    })
                })
                .transpose()?,
            public_methods: actor
                .abi
                .public_methods
                .iter()
                .map(|method| {
                    Ok(ActorPublicMethodIr {
                        method_identity: method.method_identity.clone(),
                        name: method.name.clone(),
                        parameters: normalize_parameters(&method.parameters)?,
                        return_type: normalize(
                            &method.return_type,
                            &format!("method {}", method.name),
                        )?,
                        may_suspend: method.may_suspend,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
            actor_runtime_abi_version: actor.abi.actor_runtime_abi_version.clone(),
        },
    })
}

fn normalize_artifact_interface_methods(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    interface_name: &str,
    methods: &[InterfaceMethodSignature],
) -> Result<Vec<InterfaceMethodSignature>, String> {
    let mut methods = normalize_package_interface_method_signatures(
        package_id,
        type_symbols,
        module_path,
        interface_name,
        methods,
    )?;
    for method in &mut methods {
        let context = format!("{module_path}.{interface_name}.{}", method.name);
        for param in &mut method.params {
            param.ty = normalize_artifact_interface_identities(
                package_id,
                type_symbols,
                module_path,
                param.ty.clone(),
                &context,
            )?;
        }
        method.return_type = normalize_artifact_interface_identities(
            package_id,
            type_symbols,
            module_path,
            method.return_type.clone(),
            &context,
        )?;
        method.implicit_self = method
            .implicit_self
            .clone()
            .map(|ty| {
                normalize_artifact_interface_identities(
                    package_id,
                    type_symbols,
                    module_path,
                    ty,
                    &context,
                )
            })
            .transpose()?;
    }
    Ok(methods)
}

fn normalize_artifact_interface_identities(
    package_id: &str,
    type_symbols: &PackageTypeSymbolIndex,
    module_path: &str,
    ty: TypeRefIr,
    context: &str,
) -> Result<TypeRefIr, String> {
    let recurse = |ty| {
        normalize_artifact_interface_identities(package_id, type_symbols, module_path, ty, context)
    };
    match ty {
        TypeRefIr::Builtin { name, args } => Ok(TypeRefIr::Builtin {
            name,
            args: args.into_iter().map(recurse).collect::<Result<_, _>>()?,
        }),
        TypeRefIr::AppliedNominal { base, arguments } => Ok(TypeRefIr::AppliedNominal {
            base,
            arguments: arguments
                .into_iter()
                .map(recurse)
                .collect::<Result<_, _>>()?,
        }),
        TypeRefIr::Record { fields } => Ok(TypeRefIr::Record {
            fields: fields
                .into_iter()
                .map(|(name, ty)| Ok((name, recurse(ty)?)))
                .collect::<Result<_, String>>()?,
        }),
        TypeRefIr::Union { items } => Ok(TypeRefIr::Union {
            items: items.into_iter().map(recurse).collect::<Result<_, _>>()?,
        }),
        TypeRefIr::Nullable { inner } => Ok(TypeRefIr::Nullable {
            inner: Box::new(recurse(*inner)?),
        }),
        TypeRefIr::AnyInterface { interface } => {
            let identity = serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id).map_err(
                |error| {
                    format!(
                        "package {package_id} {context} has invalid interface ABI identity: {error}"
                    )
                },
            )?;
            let identity = normalize_package_interface_type_ref(
                package_id,
                type_symbols,
                module_path,
                &identity,
                context,
            )?;
            let identity = recurse(identity)?;
            Ok(TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: type_ref_abi_key(&identity),
                    canonical_type_args: interface
                        .canonical_type_args
                        .into_iter()
                        .map(recurse)
                        .collect::<Result<_, _>>()?,
                },
            })
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => Ok(TypeRefIr::Function {
            params: params
                .into_iter()
                .map(|param| {
                    Ok(FunctionTypeParamIr {
                        name: param.name,
                        ty: recurse(param.ty)?,
                    })
                })
                .collect::<Result<_, String>>()?,
            return_type: Box::new(recurse(*return_type)?),
        }),
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => Ok(ty),
    }
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
        let attached_fields = ast
            .types
            .iter()
            .find(|ty| ty.name == actor.name)
            .map(|ty| {
                ty.fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.name.clone()))
                    .collect::<BTreeMap<_, _>>()
            })
            .unwrap_or_default();
        source_types.insert(
            SourceSymbolKey::new(module_path, &actor.name),
            SourceTypeResolution {
                name: actor.name.clone(),
                type_params: Vec::new(),
                local_type_names: BTreeSet::new(),
                kind: SourceTypeKind::Actor {
                    key_field: actor.key_field.clone(),
                    fields: attached_fields,
                    create: actor.create.as_ref().map(|create| {
                        create
                            .params
                            .iter()
                            .map(|param| (param.name.clone(), param.ty.name.clone()))
                            .collect::<Vec<_>>()
                    }),
                    canonical_id_type: None,
                    canonical_fields: None,
                    canonical_create: None,
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
            if let Some(name) = canonical_file_ir_builtin_name(name) {
                return Ok(TypeRefIr::Builtin {
                    name: name.to_string(),
                    args,
                });
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
                    let inherited = generic_type_parameter_names(&implementation.target);
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

fn callable_resolution_from_parts(
    module_path: &str,
    source_symbol: &str,
    inherited_type_params: &[String],
    decl_type_params: &[String],
    implicit_self: Option<&TypeRef>,
    params: &[Param],
    return_type: &TypeRef,
    local_type_names: &BTreeSet<String>,
) -> PackageCallableResolution {
    PackageCallableResolution {
        module_path: module_path.to_string(),
        source_symbol: source_symbol.to_string(),
        type_params: inherited_type_params
            .iter()
            .chain(decl_type_params)
            .cloned()
            .collect(),
        local_type_names: local_type_names.clone(),
        params: implicit_self
            .into_iter()
            .chain(params.iter().map(|param| &param.ty))
            .map(|ty| ty.name.clone())
            .collect(),
        return_type: return_type.name.clone(),
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
    callable_resolution_from_parts(
        module_path,
        source_symbol,
        inherited_type_params,
        &function.type_params,
        function.implicit_self.as_ref(),
        &function.params,
        &function.return_type,
        local_type_names,
    )
}

fn operation_callable_resolution(
    module_path: &str,
    source_symbol: &str,
    operation: &InterfaceOperation,
    inherited_type_params: &[String],
    local_type_names: &BTreeSet<String>,
) -> PackageCallableResolution {
    callable_resolution_from_parts(
        module_path,
        source_symbol,
        inherited_type_params,
        &operation.type_params,
        operation.implicit_self.as_ref(),
        &operation.params,
        &operation.return_type,
        local_type_names,
    )
}

fn impl_target_matches(target: &str, module_path: &str, local_target: &str) -> bool {
    let target = target.strip_prefix("root.").unwrap_or(target);
    target == local_target || target == format!("{module_path}.{local_target}")
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
    package_dependency_views: &BTreeMap<String, PackageDependencyView>,
    package_dependency_canonical_refs: &BTreeMap<String, String>,
) -> Option<String> {
    let dependency_ref = match &symbol.package {
        PackageRefIr::Dependency { dependency_ref } => dependency_ref.as_str(),
        PackageRefIr::PackageId { package_id } => package_dependencies
            .iter()
            .find_map(|(alias, id)| {
                (id == package_id
                    && package_dependency_canonical_refs
                        .get(alias)
                        .is_none_or(|canonical| canonical == alias))
                .then_some(alias.as_str())
            })
            .unwrap_or(package_id),
    };
    let view = package_dependency_views
        .get(dependency_ref)
        .copied()
        .unwrap_or(PackageDependencyView::Public);
    Some(match view {
        PackageDependencyView::Public => dependency_ref.to_string(),
        PackageDependencyView::TopLevel => format!("{dependency_ref}/"),
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
        TypeRefIr::Builtin { name, .. } if name == BuiltinShape::Unknown.name() => true,
        TypeRefIr::Builtin { name, .. } if name == BuiltinShape::Void.name() => {
            is_null_type(actual)
        }
        TypeRefIr::Builtin { name, .. } if name == BuiltinShape::Stream.name() => {
            is_null_type(actual)
        }
        TypeRefIr::Builtin { name, .. } if name == BuiltinShape::Json.name() => {
            json_assignable(actual)
        }
        TypeRefIr::Builtin { name, .. } if name == BuiltinShape::JsonObject.name() => {
            json_object_assignable(actual)
        }
        TypeRefIr::Builtin { name, .. } if name == BuiltinShape::Number.name() => {
            matches!(
                actual,
                TypeRefIr::Builtin { name, .. } if name == BuiltinShape::Integer.name()
            )
        }
        TypeRefIr::Nullable { inner } => is_null_type(actual) || type_assignable(actual, inner),
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
                    interface_abi_id: type_ref_abi_key(&identity),
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
        ) if name == BuiltinShape::String.name() => true,
        (
            TypeRefIr::Literal {
                value: LiteralIr::Null,
            },
            TypeRefIr::Builtin { name, .. },
        ) if name == BuiltinShape::Null.name() => true,
        _ => false,
    }
}

fn json_assignable(actual: &TypeRefIr) -> bool {
    match actual {
        TypeRefIr::Builtin { name, .. } => {
            matches!(
                BuiltinShape::of_name(name),
                Some(
                    BuiltinShape::String
                        | BuiltinShape::Integer
                        | BuiltinShape::Number
                        | BuiltinShape::Bool
                        | BuiltinShape::Null
                        | BuiltinShape::Json
                        | BuiltinShape::JsonObject
                )
            ) || matches!(actual, TypeRefIr::Builtin { name, args } if name == BuiltinShape::Array.name() && args.len() == 1 && json_assignable(&args[0]))
                || matches!(actual, TypeRefIr::Builtin { name, args } if name == BuiltinShape::Map.name() && args.len() == 2 && json_assignable(&args[1]))
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
        TypeRefIr::Builtin { name, .. } if name == BuiltinShape::JsonObject.name() => true,
        TypeRefIr::Record { fields } => fields.values().all(json_assignable),
        _ => false,
    }
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
    if let Some(name) = canonical_file_ir_builtin_name(&symbol) {
        return TypeRefIr::Builtin {
            name: name.to_string(),
            args,
        };
    }
    if symbol == "config.DecodeError" {
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
            debug_text(&other)
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
mod tests;

fn source_path(module_path: &str, symbol: &str) -> String {
    format!("{module_path}.{symbol}")
}
