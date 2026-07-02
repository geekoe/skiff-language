use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    canonical_interface_method_abi_id, interface_instantiation_ref, FileIrUnit,
    FunctionTypeParamIr, InterfaceInstantiationRef, LiteralIr, PackageRefIr, PackageSymbolRef,
    ServiceSymbolRef, TypeRefIr,
};

use crate::{
    package_export_resolver::PackageExportResolver,
    parsed_sources::ParsedCompilerSource,
    semantic::{
        interface::{
            object_safety_diagnostics_display, InterfaceInstantiation, InterfaceMethodSlotFact,
            InterfaceObjectSafetyDiagnostic, TypeInstantiationPattern,
        },
        InterfaceSemantics, SemanticPublication, SemanticSource, SourceOrigin,
    },
    shared::{
        ast::{AliasDecl, FunctionDecl, InterfaceOperation, SourceFile, TypeDecl, TypeRef},
        id::SKIFF_STD_PUBLICATION_ID,
        package_interface_methods::{
            instantiate_interface_method_signatures, package_interface_method_signatures,
            InterfaceMethodSignature, PackageTypeSymbolIndex,
        },
        prelude_registry::prelude_registry,
        type_expr::TypeExpr,
        type_syntax::generic_parts,
    },
};
use compiler_input_model::PackageDependency;

use super::{
    api::PublicTypeKind, type_indices, type_text_with_args, LocalDbObjectIndex,
    PackageInterfaceMethodIndex, PublicationTypeSymbolIndex, SourceSymbolKey,
};

mod shape_assignability;

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
    package_interfaces: BTreeMap<PackageSymbolKey, PackageInterfaceFact>,
    package_dependencies: BTreeMap<String, String>,
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
    Record { fields: BTreeMap<String, String> },
    Representation { target: String },
    Alias { target: String },
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
}

#[derive(Clone, Debug)]
pub struct PackageInterfaceResolution {
    pub identity: TypeRefIr,
    pub type_params: Vec<String>,
    pub methods: Vec<InterfaceMethodSignature>,
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
}

impl TypeResolutionModel {
    pub fn build(
        parsed_sources: &[ParsedCompilerSource],
        package_aliases: &BTreeMap<String, Vec<String>>,
        package_dependencies: &[PackageDependency],
        package_facts: Option<&[TypeResolutionPackageFacts<'_>]>,
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

        let package_dependencies = package_dependencies
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
        let mut package_interfaces = BTreeMap::new();
        let mut package_public_to_internal = BTreeMap::new();
        if let Some(package_facts) = package_facts {
            for package in package_facts {
                index_package_types(package, &mut package_types);
                index_package_callables(package, &mut package_callables);
                index_package_interfaces(package, &mut package_interfaces)?;
                index_package_public_to_internal(package, &mut package_public_to_internal);
            }
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
            package_interfaces,
            package_dependencies,
            package_aliases: package_aliases.clone(),
            external_type_symbols: external_type_symbols.clone(),
            interface_semantics,
            interface_conformances: Vec::new(),
            local_impl_methods: BTreeMap::new(),
            package_public_to_internal,
        };
        model.local_impl_methods = model.index_local_impl_methods(parsed_sources)?;
        model.interface_conformances = model.index_source_interface_conformances(parsed_sources)?;
        Ok(model)
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

    pub fn resolve_type_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ResolvedTypeRef, String> {
        self.reject_any_interface_selector_aliases(&TypeExpr::parse(raw), context)?;
        let expanded = self.expand_alias_text(raw, context)?;
        let expr = TypeExpr::parse(&expanded);
        let ir = self.resolve_type_expr(&expr, context)?;
        Ok(ResolvedTypeRef {
            ir,
            source_text: expanded,
        })
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
        let shape = self.constructor_shape(type_name, context)?;
        let resolved_args = type_args
            .iter()
            .map(|arg| self.resolve_type_ref(arg, context))
            .collect::<Result<Vec<_>, _>>()?;
        if !shape.type_params.is_empty() && shape.type_params.len() != resolved_args.len() {
            return Err(format!(
                "constructor `{type_name}` expects {} type arguments, found {}",
                shape.type_params.len(),
                resolved_args.len()
            ));
        }
        let substitutions = shape
            .type_params
            .iter()
            .cloned()
            .zip(resolved_args.iter().map(|arg| arg.source_text.clone()))
            .collect::<BTreeMap<_, _>>();
        let field_context = TypeResolutionContext {
            module_path: shape.module_path.as_str(),
            type_params: context.type_params.clone(),
        };
        let mut fields = BTreeMap::new();
        for (name, field_ty) in shape.fields {
            let substituted = substitute_type_params(&field_ty, &substitutions);
            let resolved = self.resolve_type_text(&substituted, &field_context)?;
            let resolved = if shape.module_path == context.module_path {
                resolved
            } else {
                self.externalize_local_type_refs(&resolved, &shape.module_path)
            };
            fields.insert(name, resolved);
        }
        Ok(ConstructorTargetResolution {
            ty: target,
            fields,
            type_params: shape.type_params,
        })
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
        let resolved_args = type_args
            .iter()
            .map(|arg| self.resolve_type_ref(arg, context))
            .collect::<Result<Vec<_>, _>>()?;
        if !shape.type_params.is_empty() && shape.type_params.len() != resolved_args.len() {
            return Err(format!(
                "representation constructor `{type_name}` expects {} type arguments, found {}",
                shape.type_params.len(),
                resolved_args.len()
            ));
        }
        let substitutions = shape
            .type_params
            .iter()
            .cloned()
            .zip(resolved_args.iter().map(|arg| arg.source_text.clone()))
            .collect::<BTreeMap<_, _>>();
        let payload_text = substitute_type_params(&shape.payload, &substitutions);
        let payload_context = TypeResolutionContext {
            module_path: shape.module_path.as_str(),
            type_params: context.type_params.clone(),
        };
        let payload = self.resolve_type_text(&payload_text, &payload_context)?;
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

    pub fn resolve_package_interface(&self, path: &str) -> Option<PackageInterfaceResolution> {
        let package_symbol =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(path)?;
        let fact = self
            .package_interface_fact(&package_symbol.dependency_ref, &package_symbol.symbol_path)?;
        let package_id = self
            .package_dependencies
            .get(&package_symbol.dependency_ref)
            .cloned()
            .unwrap_or_else(|| package_symbol.dependency_ref.clone());
        Some(PackageInterfaceResolution {
            identity: TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::PackageId { package_id },
                    symbol_path: package_symbol.symbol_path,
                    abi_expectation: None,
                },
            },
            type_params: fact.type_params.clone(),
            methods: fact.methods.clone(),
        })
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
                    symbol_path: symbol.symbol_path.clone(),
                    abi_expectation: symbol.abi_expectation.clone(),
                },
            },
            type_params: fact.type_params.clone(),
            methods: fact.methods.clone(),
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
        self.contains_interface_type_ref_inner(
            &ty.ir,
            Some(ty.source_text.as_str()),
            context,
            visited,
        )
    }

    fn contains_interface_type_ref_inner(
        &self,
        ty: &TypeRefIr,
        source_text: Option<&str>,
        context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        if self.interface_identity_for_type_ref(ty, context).is_some() {
            return true;
        }
        if let Some(source_text) = source_text {
            if self
                .resolved_type_arg_texts(source_text)
                .into_iter()
                .filter_map(|arg| self.resolve_type_text(&arg, context).ok())
                .any(|arg| self.contains_interface_resolved_type(&arg, context, visited))
            {
                return true;
            }
        }
        if self.resolved_named_type(ty, context).is_some_and(|named| {
            self.contains_interface_named_type(named, source_text, context, visited)
        }) {
            return true;
        }
        match ty {
            TypeRefIr::Native { args, .. } => args
                .iter()
                .any(|arg| self.contains_interface_type_ref_inner(arg, None, context, visited)),
            TypeRefIr::Record { fields } => fields
                .values()
                .any(|field| self.contains_interface_type_ref_inner(field, None, context, visited)),
            TypeRefIr::Union { items } => items
                .iter()
                .any(|item| self.contains_interface_type_ref_inner(item, None, context, visited)),
            TypeRefIr::Nullable { inner } => {
                self.contains_interface_type_ref_inner(inner, None, context, visited)
            }
            TypeRefIr::AnyInterface { interface } => interface
                .canonical_type_args
                .iter()
                .any(|arg| self.contains_interface_type_ref_inner(arg, None, context, visited)),
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                params.iter().any(|param| {
                    self.contains_interface_type_ref_inner(&param.ty, None, context, visited)
                }) || self.contains_interface_type_ref_inner(return_type, None, context, visited)
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. }
            | TypeRefIr::Literal { .. }
            | TypeRefIr::TypeParam { .. } => false,
        }
    }

    fn contains_interface_named_type(
        &self,
        named: ResolvedNamedType<'_>,
        source_text: Option<&str>,
        caller_context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        if !visited.insert(named.visit_key.clone()) {
            return false;
        }

        let type_arg_texts = source_text
            .map(|source_text| self.resolved_type_arg_texts(source_text))
            .unwrap_or_default();
        let substitutions = named
            .resolution
            .type_params
            .iter()
            .cloned()
            .zip(type_arg_texts)
            .collect::<BTreeMap<_, _>>();
        let mut type_params = caller_context.type_params.clone();
        type_params.extend(named.resolution.type_params.iter().cloned());
        let source_context =
            TypeResolutionContext::with_type_params(&named.source_module_path, type_params);

        let contains = match &named.resolution.kind {
            SourceTypeKind::Record { fields } => fields.values().any(|field_ty| {
                self.contains_interface_type_text_in_named_type(
                    field_ty,
                    named.package_root.as_deref(),
                    &named.resolution.local_type_names,
                    &substitutions,
                    &source_context,
                    visited,
                )
            }),
            SourceTypeKind::Representation { target } | SourceTypeKind::Alias { target } => self
                .contains_interface_type_text_in_named_type(
                    target,
                    named.package_root.as_deref(),
                    &named.resolution.local_type_names,
                    &substitutions,
                    &source_context,
                    visited,
                ),
            SourceTypeKind::External => false,
        };
        visited.remove(&named.visit_key);
        contains
    }

    fn contains_interface_type_text_in_named_type(
        &self,
        raw: &str,
        package_root: Option<&str>,
        local_type_names: &BTreeSet<String>,
        substitutions: &BTreeMap<String, String>,
        context: &TypeResolutionContext<'_>,
        visited: &mut BTreeSet<InterfaceTypeVisitKey>,
    ) -> bool {
        let qualified = package_root
            .map(|package_root| qualify_package_type_text(raw, package_root, local_type_names))
            .unwrap_or_else(|| raw.to_string());
        let substituted = substitute_type_params(&qualified, substitutions);
        self.resolve_type_text(&substituted, context)
            .ok()
            .is_some_and(|resolved| {
                self.contains_interface_resolved_type(&resolved, context, visited)
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
                    PackageRefIr::PackageId { package_id } => package_id.as_str(),
                };
                if let Some(resolution) =
                    self.package_type_resolution(dependency_ref, &symbol.symbol_path)
                {
                    return canonical_named_symbol(&source_path(
                        &resolution.module_path,
                        &resolution.name,
                    ));
                }
                canonical_named_symbol(&self.canonical_symbol_path(&symbol.symbol_path))
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
            TypeRefIr::Native { name, args } => TypeRefIr::Native {
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
            TypeRefIr::Native { name, args } => TypeRefIr::Native {
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

    fn constructor_shape(
        &self,
        type_name: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorShape, String> {
        let name = strip_generic(type_name.trim());
        if let Some(key) = self.resolve_source_type_key(name, context) {
            let resolved = self
                .source_types
                .get(&key)
                .ok_or_else(|| format!("unresolved constructor target `{type_name}`"))?;
            return self.constructor_shape_from_resolution(type_name, resolved, context);
        } else if let Some(key) = self.external_type_symbols.resolve_source_text(name) {
            let resolved = self
                .source_types
                .get(key)
                .ok_or_else(|| format!("unresolved constructor target `{type_name}`"))?;
            return self.constructor_shape_from_resolution(type_name, resolved, context);
        } else if let Some(package_symbol) =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(name)
        {
            if let Some(resolved) = self.package_type_resolution(
                &package_symbol.dependency_ref,
                &package_symbol.symbol_path,
            ) {
                return self.constructor_shape_from_resolution(type_name, resolved, context);
            }
            if let Some(shape) = prelude_constructor_shape(name) {
                return Ok(shape);
            }
            return Err(format!(
                "package constructor target `{name}` is unavailable in loaded package facts"
            ));
        } else if let Some(shape) = prelude_constructor_shape(name) {
            return Ok(shape);
        } else {
            return Err(format!("unresolved constructor target `{type_name}`"));
        }
    }

    fn constructor_shape_from_resolution(
        &self,
        type_name: &str,
        resolved: &SourceTypeResolution,
        context: &TypeResolutionContext<'_>,
    ) -> Result<ConstructorShape, String> {
        match &resolved.kind {
            SourceTypeKind::Record { fields } => {
                let fields = resolved
                    .public_path
                    .as_deref()
                    .and_then(|_| package_root_from_type_name(type_name))
                    .map(|package_root| {
                        qualify_package_record_fields(
                            fields,
                            package_root,
                            &resolved.local_type_names,
                        )
                    })
                    .unwrap_or_else(|| fields.clone());
                Ok(ConstructorShape {
                    module_path: resolved.module_path.clone(),
                    type_params: resolved.type_params.clone(),
                    fields,
                })
            }
            SourceTypeKind::Representation { .. } => Err(format!(
                "constructor target `{type_name}` is not a nominal record"
            )),
            SourceTypeKind::Alias { target } => {
                let target = resolved
                    .public_path
                    .as_deref()
                    .and_then(|_| package_root_from_type_name(type_name))
                    .map(|package_root| {
                        qualify_package_type_text(target, package_root, &resolved.local_type_names)
                    })
                    .unwrap_or_else(|| target.clone());
                let alias_context = TypeResolutionContext::with_type_params(
                    &resolved.module_path,
                    context.type_params.clone(),
                );
                self.constructor_shape(&target, &alias_context)
            }
            SourceTypeKind::External => Err(format!(
                "constructor target `{type_name}` is not a nominal record"
            )),
        }
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
        } else if let Some(package_symbol) =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(name)
        {
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
            SourceTypeKind::Representation { target } => Ok(Some(RepresentationShape {
                module_path: resolved.module_path.clone(),
                type_params: resolved.type_params.clone(),
                payload: target.clone(),
            })),
            SourceTypeKind::Alias { target } => {
                let alias_context = TypeResolutionContext::with_type_params(
                    &resolved.module_path,
                    context.type_params.clone(),
                );
                self.representation_shape(target, &alias_context)
            }
            SourceTypeKind::Record { .. } | SourceTypeKind::External => Ok(None),
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
                    if let Some(package_symbol) = PackageExportResolver::new(&self.package_aliases)
                        .resolve_package_symbol_path(name)
                    {
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
        if let Some(package_symbol) =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(name)
        {
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
            if let Some(resolution) = self.package_type_resolution(
                &package_symbol.dependency_ref,
                &package_symbol.symbol_path,
            ) {
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
            return Ok(TypeRefIr::Native {
                name: canonical_name,
                args: resolved_args,
            });
        }
        if let Some(key) = self.resolve_source_type_key(name, context) {
            let module = self
                .modules
                .get(context.module_path)
                .ok_or_else(|| format!("missing type resolution module {}", context.module_path))?;
            if key.module_path() == context.module_path {
                if let Some(index) = module.type_indices.get(key.symbol()) {
                    return Ok(TypeRefIr::LocalType { type_index: *index });
                }
            }
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: ServiceSymbolRef {
                    module_path: key.module_path().to_string(),
                    symbol: key.symbol().to_string(),
                },
            });
        }
        if let Some(type_ref) = contextual_prelude_type_ref(name, resolved_args.clone(), context) {
            return Ok(type_ref);
        }
        if let Some(type_ref) = prelude_known_type_ref(name, resolved_args.clone()) {
            return Ok(type_ref);
        }
        if let Some(package_symbol) =
            PackageExportResolver::new(&self.package_aliases).resolve_package_symbol_path(name)
        {
            return Ok(TypeRefIr::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::Dependency {
                        dependency_ref: package_symbol.dependency_ref,
                    },
                    symbol_path: package_symbol.symbol_path,
                    abi_expectation: None,
                },
            });
        }
        if let Some(symbol) = self.external_type_symbols.resolve_source_text(name) {
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref_from_source_key(symbol),
            });
        }
        if let Some(symbol) = self.resolve_db_object_symbol(service_name, context)? {
            return Ok(TypeRefIr::DbObjectSymbol { symbol });
        }
        if name.contains('.') {
            return Ok(TypeRefIr::ServiceSymbol {
                symbol: service_symbol_ref(name),
            });
        }
        Err(format!("unresolved type `{name}`"))
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
        self.package_types.get(&direct_key).or_else(|| {
            let package_id = self.package_dependencies.get(dependency_ref)?;
            let package_key = PackageSymbolKey {
                dependency_ref: package_id.clone(),
                symbol_path: symbol_path.to_string(),
            };
            self.package_types.get(&package_key)
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
        self.package_interfaces.get(&direct_key).or_else(|| {
            let package_id = self.package_dependencies.get(dependency_ref)?;
            let package_key = PackageSymbolKey {
                dependency_ref: package_id.clone(),
                symbol_path: symbol_path.to_string(),
            };
            self.package_interfaces.get(&package_key)
        })
    }

    fn resolve_source_type_key(
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
                    package_root: package_root_for_symbol(symbol).map(str::to_string),
                    visit_key: InterfaceTypeVisitKey::Package(PackageSymbolKey {
                        dependency_ref: package_id.to_string(),
                        symbol_path: source_path(&resolution.module_path, &resolution.name),
                    }),
                })
            }
            _ => None,
        }
    }

    fn resolved_type_arg_texts(&self, source_text: &str) -> Vec<String> {
        match TypeExpr::parse(source_text) {
            TypeExpr::Named { args, .. } => args.iter().map(TypeExpr::to_type_string).collect(),
            _ => Vec::new(),
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
                    let Some(interface) = self
                        .resolve_interface_instantiation_text(&implemented.name, &type_context)?
                    else {
                        continue;
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

    fn resolve_interface_instantiation_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<InterfaceInstantiationResolution>, String> {
        let resolved = self.resolve_type_text(raw, context)?;
        self.interface_instantiation_from_resolved(&resolved, context)
    }

    pub fn resolve_interface_instantiation_parts_text(
        &self,
        raw: &str,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<(TypeRefIr, Vec<TypeRefIr>)>, String> {
        Ok(self
            .resolve_interface_instantiation_text(raw, context)?
            .map(|interface| (interface.identity, interface.args)))
    }

    fn interface_instantiation_from_resolved(
        &self,
        resolved: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<InterfaceInstantiationResolution>, String> {
        let Some(identity) = self.interface_identity_for_type_ref(&resolved.ir, context) else {
            return Ok(None);
        };
        let TypeExpr::Named { args, .. } = TypeExpr::parse(&resolved.source_text) else {
            return Ok(None);
        };
        let args = args
            .iter()
            .map(|arg| self.resolve_type_expr(arg, context))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(InterfaceInstantiationResolution { identity, args }))
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
        SourceTypeKind::Representation { .. } => "concrete representation type",
        SourceTypeKind::Alias { .. } => "alias",
        SourceTypeKind::External => "non-interface type",
    }
}

struct ConstructorShape {
    module_path: String,
    type_params: Vec<String>,
    fields: BTreeMap<String, String>,
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
) {
    for binding in &package.schema_types {
        let Some(resolution) = package_source_type_resolution(
            binding.source_ast,
            binding.source_module,
            binding.source_symbol,
            Some(binding.public_path.to_string()),
        ) else {
            continue;
        };
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
}

fn index_package_callables(
    package: &TypeResolutionPackageFacts<'_>,
    package_callables: &mut BTreeMap<PackageSymbolKey, PackageCallableResolution>,
) {
    for binding in &package.callables {
        let Some(resolution) = package_callable_resolution(
            binding.source_ast,
            binding.source_module,
            binding.source_symbol,
        ) else {
            continue;
        };
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
        param.ty = TypeRefIr::native("Self");
        method.implicit_self = None;
    } else {
        method.implicit_self = Some(TypeRefIr::native("Self"));
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
        }
    } else {
        SourceTypeKind::Record {
            fields: ty
                .fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.name.clone()))
                .collect(),
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

fn substitute_type_params(raw: &str, substitutions: &BTreeMap<String, String>) -> String {
    TypeExpr::parse(raw)
        .map_named_types(|name| {
            substitutions
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.to_string())
        })
        .to_type_string()
}

fn package_root_from_type_name(type_name: &str) -> Option<&str> {
    strip_generic(type_name.trim())
        .rsplit_once('.')
        .map(|(root, _)| root)
}

fn package_root_for_module(module_path: &str) -> Option<&str> {
    module_path
        .split('.')
        .next()
        .filter(|root| !root.is_empty())
}

fn package_root_for_symbol(symbol: &PackageSymbolRef) -> Option<&str> {
    match &symbol.package {
        PackageRefIr::Dependency { dependency_ref } => Some(dependency_ref.as_str()),
        PackageRefIr::PackageId { package_id } => package_id
            .rsplit('/')
            .next()
            .or_else(|| package_root_from_type_name(&symbol.symbol_path)),
    }
}

fn qualify_package_record_fields(
    fields: &BTreeMap<String, String>,
    package_root: &str,
    local_type_names: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    fields
        .iter()
        .map(|(name, ty)| {
            (
                name.clone(),
                qualify_package_type_text(ty, package_root, local_type_names),
            )
        })
        .collect()
}

fn qualify_package_type_text(
    raw: &str,
    package_root: &str,
    local_type_names: &BTreeSet<String>,
) -> String {
    TypeExpr::parse(raw)
        .map_named_types(|name| {
            if local_type_names.contains(name) {
                format!("{package_root}.{name}")
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
        TypeRefIr::Native { name, .. } if name == "unknown" => true,
        TypeRefIr::Native { name, .. } if name == "void" => is_null_type_ir(actual),
        TypeRefIr::Native { name, .. } if name == "Stream" => is_null_type_ir(actual),
        TypeRefIr::Native { name, .. } if name == "Json" => json_assignable(actual),
        TypeRefIr::Native { name, .. } if name == "JsonObject" => json_object_assignable(actual),
        TypeRefIr::Native { name, .. } if name == "number" => {
            matches!(actual, TypeRefIr::Native { name, .. } if name == "integer")
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
        TypeRefIr::Native { name, args } if name == "Exception" && args.len() == 1 => match field {
            "error" => Some(args[0].clone()),
            _ => None,
        },
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
        TypeRefIr::Native { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Native { name, args } => format!(
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
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            symbol.symbol_path()
        }
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::AnyInterface { interface } => {
            if interface.canonical_type_args.is_empty() {
                format!("any {}", interface.interface_abi_id)
            } else {
                format!(
                    "any {}<{}>",
                    interface.interface_abi_id,
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

fn literal_assignable_to(actual: &TypeRefIr, expected: &TypeRefIr) -> bool {
    match (actual, expected) {
        (
            TypeRefIr::Literal {
                value: LiteralIr::String { .. },
            },
            TypeRefIr::Native { name, .. },
        ) if name == "string" => true,
        (
            TypeRefIr::Literal {
                value: LiteralIr::Null,
            },
            TypeRefIr::Native { name, .. },
        ) if name == "null" => true,
        _ => false,
    }
}

fn json_assignable(actual: &TypeRefIr) -> bool {
    match actual {
        TypeRefIr::Native { name, .. } => {
            matches!(
                name.as_str(),
                "string" | "integer" | "number" | "bool" | "null" | "Json" | "JsonObject"
            ) || matches!(actual, TypeRefIr::Native { name, args } if name == "Array" && args.len() == 1 && json_assignable(&args[0]))
                || matches!(actual, TypeRefIr::Native { name, args } if name == "Map" && args.len() == 2 && json_assignable(&args[1]))
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
        TypeRefIr::Native { name, .. } if name == "JsonObject" => true,
        TypeRefIr::Record { fields } => fields.values().all(json_assignable),
        _ => false,
    }
}

fn is_null_type_ir(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, .. } if name == "null")
        || matches!(
            ty,
            TypeRefIr::Literal {
                value: LiteralIr::Null
            }
        )
}

fn is_self_type_ref(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, args } if name == "Self" && args.is_empty())
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
            ty: TypeRefIr::native("Self"),
        });
    }
    params.extend(method.params.iter().cloned());
    params
}

fn type_ref_contains_self(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::Native { args, .. } => {
            is_self_type_ref(ty) || args.iter().any(type_ref_contains_self)
        }
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
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

fn type_ref_contains_any_interface(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::AnyInterface { .. } => true,
        TypeRefIr::Native { args, .. } => args.iter().any(type_ref_contains_any_interface),
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
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
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
        "string" | "integer" | "number" | "bool" | "null" | "void" | "never" | "Json"
        | "JsonObject" | "Date" | "Config" | "bytes" | "Array" | "Map" | "Stream" | "Exception"
        | "CatchResult" | "DbInsertManyResult" | "DbUpdateManyResult" | "DbDeleteManyResult"
        | "DbUpsertResult" => return Some(name.to_string()),
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
        return TypeRefIr::Native { name, args };
    }
    if is_std_abi_generic_type_symbol(&symbol) {
        return TypeRefIr::Native { name: symbol, args };
    }
    TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: SKIFF_STD_PUBLICATION_ID.to_string(),
            },
            symbol_path: symbol,
            abi_expectation: None,
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
        other if prelude_registry().is_native_type_name(other) => Some(other.to_string()),
        _ => None,
    }
}

fn is_std_abi_generic_type_symbol(symbol: &str) -> bool {
    matches!(
        symbol,
        "std.websocket.WebSocketConnectResult"
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
                    SourceOrigin::Service,
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
    use skiff_artifact_model::{InterfaceDeclIr, InterfaceOperationIr};

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
                            ty: TypeRefIr::native("Self"),
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
            &PublicationTypeSymbolIndex::default(),
        )
        .expect("type resolution with package facts should build");
        (parsed_sources, type_resolution)
    }

    fn context() -> TypeResolutionContext<'static> {
        TypeResolutionContext::source(MODULE)
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
        assert_eq!(read.params[0].ty, TypeRefIr::native("Self"));
        assert_eq!(read.params[1].name, "fallback");
        assert_eq!(read.params[1].ty, TypeRefIr::native("string"));
        assert_eq!(read.return_type, TypeRefIr::native("string"));
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
            .resolve_type_text("pkg.Reader<string>", &context)
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
            vec![TypeRefIr::native("string")]
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
                    ty: TypeRefIr::native("string"),
                },
            ]
        );
        assert_eq!(slot.return_type, TypeRefIr::native("string"));
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
                            ty: TypeRefIr::native("Self"),
                        },
                        FunctionTypeParamIr {
                            name: "input".to_string(),
                            ty: api_request,
                        },
                    ],
                    return_type: TypeRefIr::Native {
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
            .resolve_type_text("pkg.Reader<string>", &context)
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
        let expected = type_resolution
            .resolve_type_text("I<string>", &context)
            .expect("interface type should resolve");

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
        let expected = type_resolution
            .resolve_type_text("I<string>", &context)
            .expect("interface type should resolve");

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
            vec![TypeRefIr::native("string")]
        );
        assert_eq!(
            matched.expected_interface_args,
            vec![TypeRefIr::native("string")]
        );
    }

    #[test]
    fn concrete_type_conformance_rejects_mismatched_interface_args() {
        let (_parsed_sources, type_resolution) = type_resolution(conformance_source());
        let context = context();
        let actual = type_resolution
            .resolve_type_text("Box<string>", &context)
            .expect("actual type should resolve");
        let expected = type_resolution
            .resolve_type_text("I<number>", &context)
            .expect("interface type should resolve");

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
        let expected = type_resolution
            .resolve_type_text("I<string>", &context)
            .expect("interface type should resolve");

        let nullable = type_resolution
            .resolve_type_text("Box<string>?", &context)
            .expect("nullable actual should resolve");
        let union = type_resolution
            .resolve_type_text("Box<string> | null", &context)
            .expect("union actual should resolve");
        let record = ResolvedTypeRef {
            ir: TypeRefIr::Record {
                fields: BTreeMap::from([("value".to_string(), TypeRefIr::native("string"))]),
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
            None,
        )
        .expect_err("Concrete argument should not be assignable to bare interface parameter");

        let message = error.message();
        assert!(
            message.contains("argument"),
            "expected an argument assignability diagnostic, got: {message}"
        );
    }
}

fn source_path(module_path: &str, symbol: &str) -> String {
    format!("{module_path}.{symbol}")
}
