#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{
    canonical_interface_method_abi_id, interface_instantiation_ref, FunctionTypeParamIr,
    InterfaceInstantiationRef, LiteralIr, ServiceSymbolRef, TypeRefIr,
};
use skiff_compiler_core::type_ref::substitute_type_params_in_type_ref_ref;

use crate::{
    shared::ast::{ImplDecl, InterfaceOperation, SourceFile, TypeRef},
    shared::error::{CompileError, Result},
    shared::type_expr::TypeExpr,
    SourceSymbolKey,
};

use super::SemanticPublication;

const ACTOR_INTERFACE_MODULE: &str = "std.actor";
const ACTOR_INTERFACE_SYMBOL: &str = "Actor";
const ERROR_PAYLOAD_INTERFACE_MODULE: &str = "std.error";
const ERROR_PAYLOAD_INTERFACE_SYMBOL: &str = "ErrorPayload";

#[derive(Debug, Clone)]
pub struct InterfaceDeclFact {
    pub symbol: SourceSymbolKey,
    pub type_params: Vec<String>,
    pub requirements: Vec<InterfaceRequirementFact>,
    source_kind: InterfaceSourceKind,
}

#[derive(Debug, Clone)]
pub struct InterfaceRequirementFact {
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
    pub is_native: bool,
    pub is_provider: bool,
    pub is_static: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceInstantiation {
    pub symbol: SourceSymbolKey,
    pub args: Vec<TypeRefIr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeInstantiationPattern {
    pub symbol: SourceSymbolKey,
    pub args: Vec<TypeRefIr>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceConformanceFact {
    pub receiver_type_params: Vec<String>,
    pub receiver: TypeInstantiationPattern,
    pub interface: InterfaceInstantiation,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceMethodSlotFact {
    pub slot: u32,
    pub name: String,
    pub method_abi_id: String,
    pub params: Vec<FunctionTypeParamIr>,
    pub return_type: TypeRefIr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterfaceObjectSafetyDiagnostic {
    MarkerInterface {
        interface: SourceSymbolKey,
    },
    MissingSelfReceiver {
        method_name: String,
    },
    UnsupportedMethodRequirement {
        method_name: String,
        message: String,
    },
    InvalidSelfUsage {
        method_name: String,
        message: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct InterfaceSemantics {
    interfaces: BTreeMap<SourceSymbolKey, InterfaceDeclFact>,
    source_types: BTreeMap<SourceSymbolKey, SourceTypeFact>,
    interfaces_by_bare: BTreeMap<String, Vec<SourceSymbolKey>>,
    types_by_bare: BTreeMap<String, Vec<SourceSymbolKey>>,
    conformances: Vec<InterfaceConformanceFact>,
    conformances_by_receiver: BTreeMap<SourceSymbolKey, Vec<usize>>,
    actor_conformances_by_receiver: BTreeMap<SourceSymbolKey, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InterfaceSourceKind {
    Source,
    CompilerKnown,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SourceTypeKind {
    Nominal,
    Alias,
}

#[derive(Debug, Clone)]
struct SourceTypeFact {
    type_params: Vec<String>,
    kind: SourceTypeKind,
}

#[derive(Debug, Clone)]
struct InterfaceIndex {
    interfaces: BTreeMap<SourceSymbolKey, InterfaceDeclFact>,
    source_types: BTreeMap<SourceSymbolKey, SourceTypeFact>,
    interfaces_by_bare: BTreeMap<String, Vec<SourceSymbolKey>>,
    types_by_bare: BTreeMap<String, Vec<SourceSymbolKey>>,
}

#[derive(Debug, Clone, Default)]
struct ImplMethodIndex {
    methods_by_receiver: BTreeMap<SourceSymbolKey, BTreeMap<String, ImplMethodSignatureFact>>,
}

#[derive(Debug, Clone)]
struct ImplMethodSignatureFact {
    name: String,
    type_params: Vec<String>,
    params: Vec<FunctionTypeParamIr>,
    return_type: TypeRefIr,
}

impl InterfaceSemantics {
    pub fn build(publication: &SemanticPublication<'_>) -> Result<Self> {
        let mut index = InterfaceIndex::build(publication)?;
        let impl_methods = ImplMethodIndex::build(publication, &index)?;
        let mut conformances: Vec<InterfaceConformanceFact> = Vec::new();
        let mut conformances_by_receiver = BTreeMap::<SourceSymbolKey, Vec<usize>>::new();
        let mut actor_conformances_by_receiver = BTreeMap::<SourceSymbolKey, usize>::new();
        let mut seen = BTreeSet::<(SourceSymbolKey, SourceSymbolKey)>::new();

        for source in &publication.sources {
            for ty in &source.ast.types {
                if ty.alias.is_some() {
                    continue;
                }
                let receiver_symbol = SourceSymbolKey::new(source.module_path, &ty.name);
                let receiver_type_params = ty.type_params.clone();
                let receiver = TypeInstantiationPattern {
                    symbol: receiver_symbol.clone(),
                    args: ty
                        .type_params
                        .iter()
                        .map(|name| TypeRefIr::TypeParam { name: name.clone() })
                        .collect(),
                };
                let type_param_scope = ty.type_params.iter().cloned().collect::<BTreeSet<_>>();

                for implemented in &ty.implements {
                    let interface = index.resolve_interface_instantiation(
                        source.module_path,
                        implemented,
                        &type_param_scope,
                    )?;
                    let duplicate_key = (receiver_symbol.clone(), interface.symbol.clone());
                    if !seen.insert(duplicate_key) {
                        return Err(CompileError::Semantic(format!(
                            "type {}.{} declares conformance to interface {} more than once",
                            source.module_path, ty.name, interface.symbol
                        )));
                    }
                    let fact = InterfaceConformanceFact {
                        receiver_type_params: receiver_type_params.clone(),
                        receiver: receiver.clone(),
                        interface,
                    };
                    validate_conformance_requirements(&index, &impl_methods, &fact)?;
                    let index_in_vec = conformances.len();
                    if fact.interface.symbol == actor_interface_symbol_key() {
                        if let Some(existing) = actor_conformances_by_receiver
                            .insert(receiver_symbol.clone(), index_in_vec)
                        {
                            let previous = &conformances[existing];
                            return Err(CompileError::Semantic(format!(
                                "actor type {}.{} implements both {} and {}; an actor type can only implement one std.actor.Actor<Id> instantiation",
                                source.module_path,
                                ty.name,
                                interface_instantiation_display(&previous.interface),
                                interface_instantiation_display(&fact.interface)
                            )));
                        }
                    }
                    conformances_by_receiver
                        .entry(receiver_symbol.clone())
                        .or_default()
                        .push(index_in_vec);
                    conformances.push(fact);
                }
            }
        }

        Ok(Self {
            interfaces: index.interfaces,
            source_types: index.source_types,
            interfaces_by_bare: index.interfaces_by_bare,
            types_by_bare: index.types_by_bare,
            conformances,
            conformances_by_receiver,
            actor_conformances_by_receiver,
        })
    }

    pub fn interface(&self, symbol: &SourceSymbolKey) -> Option<&InterfaceDeclFact> {
        self.interfaces.get(symbol)
    }

    pub fn conformances(&self) -> &[InterfaceConformanceFact] {
        &self.conformances
    }

    pub fn conformances_for_receiver(
        &self,
        receiver: &SourceSymbolKey,
    ) -> impl Iterator<Item = &InterfaceConformanceFact> {
        self.conformances_by_receiver
            .get(receiver)
            .into_iter()
            .flatten()
            .map(|index| &self.conformances[*index])
    }

    pub fn actor_conformance_for_receiver(
        &self,
        receiver: &SourceSymbolKey,
    ) -> Option<&InterfaceConformanceFact> {
        self.actor_conformances_by_receiver
            .get(receiver)
            .map(|index| &self.conformances[*index])
    }

    pub fn actor_conformances(&self) -> impl Iterator<Item = &InterfaceConformanceFact> {
        self.actor_conformances_by_receiver
            .values()
            .map(|index| &self.conformances[*index])
    }

    pub fn is_nominal_source_type(&self, symbol: &SourceSymbolKey) -> bool {
        self.source_types
            .get(symbol)
            .is_some_and(|fact| fact.kind == SourceTypeKind::Nominal)
    }

    pub fn canonical_interface_instantiation_ref(
        &self,
        interface: &InterfaceInstantiation,
    ) -> InterfaceInstantiationRef {
        interface_instantiation_ref(
            interface_symbol_type_ref(&interface.symbol),
            interface.args.clone(),
        )
    }

    pub fn canonical_source_interface_instantiation_from_type_ref(
        &self,
        module_path: &str,
        ty: &TypeRef,
        type_param_scope: &BTreeSet<String>,
    ) -> Result<InterfaceInstantiation> {
        let expr = TypeExpr::parse(&ty.name);
        self.canonical_source_interface_instantiation_from_type_expr(
            module_path,
            &expr,
            type_param_scope,
        )
    }

    pub fn canonical_source_interface_instantiation_from_type_expr(
        &self,
        module_path: &str,
        expr: &TypeExpr,
        type_param_scope: &BTreeSet<String>,
    ) -> Result<InterfaceInstantiation> {
        let TypeExpr::Named { name, args } = expr else {
            return Err(CompileError::Semantic(format!(
                "interface selector `{}` must be a named interface type",
                expr.to_type_string()
            )));
        };
        let symbol = self.resolve_source_interface_symbol(module_path, name)?;
        let fact = self.interface(&symbol).ok_or_else(|| {
            CompileError::Semantic(format!(
                "interface selector `{}` references unknown interface {symbol}",
                expr.to_type_string()
            ))
        })?;
        if fact.type_params.len() != args.len() {
            return Err(CompileError::Semantic(format!(
                "interface selector `{}` targets interface {}, which expects {} type arguments, found {}",
                expr.to_type_string(),
                symbol,
                fact.type_params.len(),
                args.len()
            )));
        }
        let args = args
            .iter()
            .map(|arg| {
                resolve_type_expr(
                    module_path,
                    arg,
                    type_param_scope,
                    &self.source_types,
                    &self.types_by_bare,
                    &self.interfaces,
                    &self.interfaces_by_bare,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(InterfaceInstantiation { symbol, args })
    }

    pub fn require_object_safe_interface(&self, interface: &InterfaceInstantiation) -> Result<()> {
        let diagnostics = self.object_safety_diagnostics(interface)?;
        if diagnostics.is_empty() {
            return Ok(());
        }
        Err(CompileError::Semantic(format!(
            "interface {} is not object-safe: {}",
            interface_instantiation_display(interface),
            object_safety_diagnostics_display(&diagnostics)
        )))
    }

    pub fn canonical_object_safe_interface_instantiation_ref(
        &self,
        interface: &InterfaceInstantiation,
    ) -> Result<InterfaceInstantiationRef> {
        self.require_object_safe_interface(interface)?;
        Ok(self.canonical_interface_instantiation_ref(interface))
    }

    pub fn object_safety_diagnostics(
        &self,
        interface: &InterfaceInstantiation,
    ) -> Result<Vec<InterfaceObjectSafetyDiagnostic>> {
        let fact = self.interface(&interface.symbol).ok_or_else(|| {
            CompileError::Semantic(format!(
                "interface {} is not known to interface semantics",
                interface.symbol
            ))
        })?;
        let mut diagnostics = Vec::new();
        if fact.requirements.is_empty() {
            diagnostics.push(InterfaceObjectSafetyDiagnostic::MarkerInterface {
                interface: interface.symbol.clone(),
            });
        }
        for requirement in &fact.requirements {
            if requirement.is_static {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: requirement.name.clone(),
                        message: "method requirement cannot be static".to_string(),
                    },
                );
            }
            if requirement.is_native {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: requirement.name.clone(),
                        message: "method requirement cannot be native".to_string(),
                    },
                );
            }
            if requirement.is_provider {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: requirement.name.clone(),
                        message: "method requirement cannot be provider-only".to_string(),
                    },
                );
            }
            if !requirement.type_params.is_empty() {
                diagnostics.push(
                    InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                        method_name: requirement.name.clone(),
                        message: "method requirement cannot declare method-level type parameters"
                            .to_string(),
                    },
                );
            }
            match validate_requirement_self_usage(&fact.symbol, requirement) {
                Ok(true) => {}
                Ok(false) => {
                    diagnostics.push(InterfaceObjectSafetyDiagnostic::MissingSelfReceiver {
                        method_name: requirement.name.clone(),
                    });
                }
                Err(error) => {
                    diagnostics.push(InterfaceObjectSafetyDiagnostic::InvalidSelfUsage {
                        method_name: requirement.name.clone(),
                        message: error.to_string(),
                    });
                }
            }
        }
        Ok(diagnostics)
    }

    pub fn is_object_safe_interface(&self, interface: &InterfaceInstantiation) -> Result<bool> {
        Ok(self.object_safety_diagnostics(interface)?.is_empty())
    }

    pub fn method_slots_for_interface(
        &self,
        interface: &InterfaceInstantiation,
    ) -> Result<Vec<InterfaceMethodSlotFact>> {
        let diagnostics = self.object_safety_diagnostics(interface)?;
        if !diagnostics.is_empty() {
            return Err(CompileError::Semantic(format!(
                "interface {} is not object-safe: {}",
                interface_instantiation_display(interface),
                object_safety_diagnostics_display(&diagnostics)
            )));
        }
        let fact = self.interface(&interface.symbol).ok_or_else(|| {
            CompileError::Semantic(format!(
                "interface {} is not known to interface semantics",
                interface.symbol
            ))
        })?;
        let substitutions = interface_type_arg_substitutions(fact, interface)?;
        let canonical = self.canonical_interface_instantiation_ref(interface);
        fact.requirements
            .iter()
            .enumerate()
            .map(|(slot, requirement)| {
                Ok(InterfaceMethodSlotFact {
                    slot: slot as u32,
                    name: requirement.name.clone(),
                    method_abi_id: canonical_interface_method_abi_id(&canonical, &requirement.name),
                    params: requirement
                        .params
                        .iter()
                        .map(|param| FunctionTypeParamIr {
                            name: param.name.clone(),
                            ty: substitute_type_params_in_type_ref_ref(&param.ty, &substitutions),
                        })
                        .collect(),
                    return_type: substitute_type_params_in_type_ref_ref(
                        &requirement.return_type,
                        &substitutions,
                    ),
                })
            })
            .collect()
    }

    pub fn local_conformance_for_receiver_instantiation(
        &self,
        receiver: &TypeInstantiationPattern,
        interface: &InterfaceInstantiation,
    ) -> Option<InterfaceConformanceFact> {
        self.conformances_for_receiver(&receiver.symbol)
            .filter_map(|fact| instantiate_conformance_for_receiver(fact, receiver))
            .find(|fact| &fact.interface == interface)
    }

    pub fn method_slots_for_local_conformance(
        &self,
        conformance: &InterfaceConformanceFact,
    ) -> Result<Vec<InterfaceMethodSlotFact>> {
        let fact = self
            .interface(&conformance.interface.symbol)
            .ok_or_else(|| {
                CompileError::Semantic(format!(
                    "interface {} is not known to interface semantics",
                    conformance.interface.symbol
                ))
            })?;
        let canonical = self.canonical_interface_instantiation_ref(&conformance.interface);
        fact.requirements
            .iter()
            .enumerate()
            .map(|(slot, requirement)| {
                validate_requirement_self_usage(&fact.symbol, requirement)?;
                Ok(InterfaceMethodSlotFact {
                    slot: slot as u32,
                    name: requirement.name.clone(),
                    method_abi_id: canonical_interface_method_abi_id(&canonical, &requirement.name),
                    params: requirement
                        .params
                        .iter()
                        .map(|param| {
                            Ok(FunctionTypeParamIr {
                                name: param.name.clone(),
                                ty: substitute_requirement_type(&param.ty, fact, conformance)?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?,
                    return_type: substitute_requirement_type(
                        &requirement.return_type,
                        fact,
                        conformance,
                    )?,
                })
            })
            .collect()
    }
}

impl InterfaceSemantics {
    fn resolve_source_interface_symbol(
        &self,
        module_path: &str,
        name: &str,
    ) -> Result<SourceSymbolKey> {
        source_interface_symbol(
            module_path,
            name,
            &self.source_types,
            &self.types_by_bare,
            &self.interfaces,
            &self.interfaces_by_bare,
        )
    }
}

impl InterfaceIndex {
    fn build(publication: &SemanticPublication<'_>) -> Result<Self> {
        let mut index = Self {
            interfaces: BTreeMap::new(),
            source_types: BTreeMap::new(),
            interfaces_by_bare: BTreeMap::new(),
            types_by_bare: BTreeMap::new(),
        };
        index.insert_compiler_known_interface(
            actor_interface_symbol_key(),
            vec!["Id".to_string()],
        )?;
        index.insert_compiler_known_interface(
            SourceSymbolKey::new(
                ERROR_PAYLOAD_INTERFACE_MODULE,
                ERROR_PAYLOAD_INTERFACE_SYMBOL,
            ),
            Vec::new(),
        )?;

        for source in &publication.sources {
            index.index_source_types(source.module_path, source.ast);
            for interface in &source.ast.interfaces {
                let symbol = SourceSymbolKey::new(source.module_path, &interface.name);
                let requirements = interface
                    .operations
                    .iter()
                    .map(|operation| {
                        interface_requirement_fact(
                            source.module_path,
                            &interface.type_params,
                            operation,
                            &index,
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;
                index.insert_interface(InterfaceDeclFact {
                    symbol,
                    type_params: interface.type_params.clone(),
                    requirements,
                    source_kind: InterfaceSourceKind::Source,
                })?;
            }
        }
        Ok(index)
    }

    fn index_source_types(&mut self, module_path: &str, ast: &SourceFile) {
        for ty in &ast.types {
            let symbol = SourceSymbolKey::new(module_path, &ty.name);
            self.types_by_bare
                .entry(ty.name.clone())
                .or_default()
                .push(symbol.clone());
            self.source_types.insert(
                symbol,
                SourceTypeFact {
                    type_params: ty.type_params.clone(),
                    kind: SourceTypeKind::Nominal,
                },
            );
        }
        for alias in &ast.aliases {
            let symbol = SourceSymbolKey::new(module_path, &alias.name);
            self.types_by_bare
                .entry(alias.name.clone())
                .or_default()
                .push(symbol.clone());
            self.source_types.insert(
                symbol,
                SourceTypeFact {
                    type_params: Vec::new(),
                    kind: SourceTypeKind::Alias,
                },
            );
        }
    }

    fn insert_compiler_known_interface(
        &mut self,
        symbol: SourceSymbolKey,
        type_params: Vec<String>,
    ) -> Result<()> {
        self.insert_interface(InterfaceDeclFact {
            symbol,
            type_params,
            requirements: Vec::new(),
            source_kind: InterfaceSourceKind::CompilerKnown,
        })
    }

    fn insert_interface(&mut self, fact: InterfaceDeclFact) -> Result<()> {
        let symbol = fact.symbol.clone();
        if let Some(existing) = self.interfaces.get(&symbol) {
            if existing.source_kind == InterfaceSourceKind::CompilerKnown
                && fact.source_kind == InterfaceSourceKind::Source
            {
                if existing.type_params.len() != fact.type_params.len() {
                    return Err(CompileError::Semantic(format!(
                        "source interface {symbol} has {} type parameters, but the compiler-known interface expects {}",
                        fact.type_params.len(),
                        existing.type_params.len()
                    )));
                }
                self.interfaces.insert(symbol, fact);
                return Ok(());
            }
            return Err(CompileError::Semantic(format!(
                "duplicate interface symbol {symbol}"
            )));
        }
        self.interfaces_by_bare
            .entry(symbol.symbol().to_string())
            .or_default()
            .push(symbol.clone());
        self.interfaces.insert(symbol, fact);
        Ok(())
    }

    fn resolve_interface_instantiation(
        &mut self,
        module_path: &str,
        ty: &TypeRef,
        type_param_scope: &BTreeSet<String>,
    ) -> Result<InterfaceInstantiation> {
        let expr = TypeExpr::parse(&ty.name);
        let TypeExpr::Named { name, args } = &expr else {
            return Err(CompileError::Semantic(format!(
                "implements entry `{}` must be a named interface type",
                ty.name
            )));
        };
        if ty.name.contains("<>") {
            return Err(CompileError::Semantic(format!(
                "interface `{name}` with zero type arguments must be written without <>"
            )));
        }
        let symbol = self.resolve_interface_symbol(module_path, name)?;
        if let Some(fact) = self.interfaces.get_mut(&symbol) {
            if fact.source_kind == InterfaceSourceKind::External && fact.type_params.is_empty() {
                fact.type_params = (0..args.len()).map(|index| format!("T{index}")).collect();
            }
        }
        let arity = self
            .interfaces
            .get(&symbol)
            .map(|fact| fact.type_params.len())
            .unwrap_or(0);
        if arity != args.len() {
            return Err(CompileError::Semantic(format!(
                "interface {} expects {} type arguments, found {} in implements entry `{}`",
                symbol,
                arity,
                args.len(),
                ty.name
            )));
        }
        let args = args
            .iter()
            .map(|arg| {
                resolve_type_expr(
                    module_path,
                    arg,
                    type_param_scope,
                    &self.source_types,
                    &self.types_by_bare,
                    &self.interfaces,
                    &self.interfaces_by_bare,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(InterfaceInstantiation { symbol, args })
    }

    fn resolve_interface_symbol(
        &mut self,
        module_path: &str,
        name: &str,
    ) -> Result<SourceSymbolKey> {
        let normalized = name.strip_prefix("root.").unwrap_or(name);
        if let Some(symbol) = source_symbol_from_qualified(normalized) {
            if self.interfaces.contains_key(&symbol) {
                return Ok(symbol);
            }
            if self.source_types.contains_key(&symbol) {
                return Err(CompileError::Semantic(format!(
                    "implements entry `{name}` resolves to type {symbol}, not an interface"
                )));
            }
            if is_probable_external_interface(name) {
                self.insert_external_interface(symbol.clone())?;
                return Ok(symbol);
            }
            return Err(CompileError::Semantic(format!(
                "implements entry `{name}` does not resolve to an interface"
            )));
        }

        let local_symbol = SourceSymbolKey::new(module_path, normalized);
        if self.interfaces.contains_key(&local_symbol) {
            return Ok(local_symbol);
        }
        if self.source_types.contains_key(&local_symbol) {
            return Err(CompileError::Semantic(format!(
                "implements entry `{name}` resolves to type {local_symbol}, not an interface"
            )));
        }

        match self.interfaces_by_bare.get(normalized).map(Vec::as_slice) {
            Some([symbol]) => return Ok(symbol.clone()),
            Some(symbols) if symbols.len() > 1 => {
                return Err(CompileError::Semantic(format!(
                    "bare interface `{name}` is ambiguous across {}; use a qualified interface name",
                    symbols
                        .iter()
                        .map(SourceSymbolKey::to_source_symbol)
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
            _ => {}
        }
        if self
            .types_by_bare
            .get(normalized)
            .is_some_and(|symbols| !symbols.is_empty())
        {
            return Err(CompileError::Semantic(format!(
                "implements entry `{name}` resolves to a type name, not an interface"
            )));
        }
        Err(CompileError::Semantic(format!(
            "implements entry `{name}` does not resolve to an interface"
        )))
    }

    fn insert_external_interface(&mut self, symbol: SourceSymbolKey) -> Result<()> {
        if self.interfaces.contains_key(&symbol) {
            return Ok(());
        }
        self.insert_interface(InterfaceDeclFact {
            symbol,
            type_params: Vec::new(),
            requirements: Vec::new(),
            source_kind: InterfaceSourceKind::External,
        })
    }
}

impl ImplMethodIndex {
    fn build(publication: &SemanticPublication<'_>, index: &InterfaceIndex) -> Result<Self> {
        let mut methods_by_receiver =
            BTreeMap::<SourceSymbolKey, BTreeMap<String, ImplMethodSignatureFact>>::new();
        for source in &publication.sources {
            for implementation in &source.ast.impls {
                let receiver_symbol =
                    receiver_symbol_from_impl_target(source.module_path, implementation, index)?;
                let receiver_type_params = index
                    .source_types
                    .get(&receiver_symbol)
                    .map(|fact| fact.type_params.clone())
                    .unwrap_or_default();
                let type_param_scope = receiver_type_params.into_iter().collect::<BTreeSet<_>>();
                let receiver_methods = methods_by_receiver
                    .entry(receiver_symbol.clone())
                    .or_default();
                for method in &implementation.methods {
                    if method.is_static {
                        continue;
                    }
                    let signature = impl_method_signature_fact(
                        source.module_path,
                        method,
                        &type_param_scope,
                        index,
                    )?;
                    if receiver_methods
                        .insert(method.name.clone(), signature)
                        .is_some()
                    {
                        return Err(CompileError::Semantic(format!(
                            "impl {} declares method {} more than once",
                            receiver_symbol, method.name
                        )));
                    }
                }
            }
        }
        Ok(Self {
            methods_by_receiver,
        })
    }

    fn method(
        &self,
        receiver: &SourceSymbolKey,
        method_name: &str,
    ) -> Option<&ImplMethodSignatureFact> {
        self.methods_by_receiver
            .get(receiver)
            .and_then(|methods| methods.get(method_name))
    }
}

fn receiver_symbol_from_impl_target(
    module_path: &str,
    implementation: &ImplDecl,
    index: &InterfaceIndex,
) -> Result<SourceSymbolKey> {
    let expr = TypeExpr::parse(&implementation.target);
    let TypeExpr::Named { name, .. } = expr else {
        return Err(CompileError::Semantic(format!(
            "impl target `{}` must be a nominal source type",
            implementation.target
        )));
    };
    source_type_symbol(
        module_path,
        name.strip_prefix("root.").unwrap_or(&name),
        &index.source_types,
        &index.types_by_bare,
    )?
    .ok_or_else(|| {
        CompileError::Semantic(format!(
            "impl target `{}` does not resolve to a source type",
            implementation.target
        ))
    })
}

fn impl_method_signature_fact(
    module_path: &str,
    method: &InterfaceOperation,
    receiver_type_param_scope: &BTreeSet<String>,
    index: &InterfaceIndex,
) -> Result<ImplMethodSignatureFact> {
    let mut params = Vec::new();
    if let Some(implicit_self) = &method.implicit_self {
        params.push(FunctionTypeParamIr {
            name: "self".to_string(),
            ty: resolve_type_ref(module_path, implicit_self, receiver_type_param_scope, index)?,
        });
    }
    params.extend(
        method
            .params
            .iter()
            .map(|param| {
                Ok(FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: resolve_type_ref(module_path, &param.ty, receiver_type_param_scope, index)?,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    );
    let return_type = resolve_type_ref(
        module_path,
        &method.return_type,
        receiver_type_param_scope,
        index,
    )?;
    Ok(ImplMethodSignatureFact {
        name: method.name.clone(),
        type_params: method.type_params.clone(),
        params,
        return_type,
    })
}

fn validate_conformance_requirements(
    index: &InterfaceIndex,
    impl_methods: &ImplMethodIndex,
    conformance: &InterfaceConformanceFact,
) -> Result<()> {
    let Some(interface) = index.interfaces.get(&conformance.interface.symbol) else {
        return Err(CompileError::Semantic(format!(
            "conformance references unknown interface {}",
            conformance.interface.symbol
        )));
    };
    for requirement in &interface.requirements {
        let explicit_self = validate_requirement_self_usage(&interface.symbol, requirement)?;
        let expected_params = requirement
            .params
            .iter()
            .map(|param| {
                Ok(FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: substitute_requirement_type(&param.ty, interface, conformance)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let expected_return =
            substitute_requirement_type(&requirement.return_type, interface, conformance)?;
        let Some(method) = impl_methods.method(&conformance.receiver.symbol, &requirement.name)
        else {
            return Err(CompileError::Semantic(format!(
                "type {} declares conformance to {}, but method {} is missing",
                type_instantiation_pattern_display(&conformance.receiver),
                interface_instantiation_display(&conformance.interface),
                requirement.name
            )));
        };
        if !method.type_params.is_empty() {
            return Err(CompileError::Semantic(format!(
                "type {} method {} cannot satisfy interface {} because method-level type parameters are not supported in interface conformance",
                type_instantiation_pattern_display(&conformance.receiver),
                method.name,
                interface_instantiation_display(&conformance.interface)
            )));
        }
        let actual_params =
            method_params_for_requirement_compare(method, conformance, explicit_self);
        if actual_params.len() != expected_params.len()
            || actual_params
                .iter()
                .zip(&expected_params)
                .any(|(actual, expected)| actual.ty != expected.ty)
            || method.return_type != expected_return
        {
            return Err(CompileError::Semantic(format!(
                "type {} method {} signature does not match interface {}; expected params {:?} return {:?}, got params {:?} return {:?}",
                type_instantiation_pattern_display(&conformance.receiver),
                method.name,
                interface_instantiation_display(&conformance.interface),
                expected_params
                    .iter()
                    .map(|param| type_ref_display(&param.ty))
                    .collect::<Vec<_>>(),
                type_ref_display(&expected_return),
                actual_params
                    .iter()
                    .map(|param| type_ref_display(&param.ty))
                    .collect::<Vec<_>>(),
                type_ref_display(&method.return_type)
            )));
        }
    }
    Ok(())
}

fn validate_requirement_self_usage(
    interface: &SourceSymbolKey,
    requirement: &InterfaceRequirementFact,
) -> Result<bool> {
    let explicit_self = requirement
        .params
        .first()
        .is_some_and(|param| param.name == "self" && is_self_type(&param.ty));
    if !explicit_self {
        if requirement
            .params
            .iter()
            .any(|param| contains_self_type(&param.ty))
            || contains_self_type(&requirement.return_type)
        {
            return Err(CompileError::Semantic(format!(
                "interface {} method {} can only use Self in the first receiver parameter",
                interface, requirement.name
            )));
        }
        return Ok(false);
    }
    for param in requirement.params.iter().skip(1) {
        if contains_self_type(&param.ty) {
            return Err(CompileError::Semantic(format!(
                "interface {} method {} can only use Self in the first receiver parameter",
                interface, requirement.name
            )));
        }
    }
    if contains_self_type(&requirement.return_type) {
        return Err(CompileError::Semantic(format!(
            "interface {} method {} cannot use Self as a return type",
            interface, requirement.name
        )));
    }
    Ok(true)
}

fn method_params_for_requirement_compare<'a>(
    method: &'a ImplMethodSignatureFact,
    conformance: &InterfaceConformanceFact,
    explicit_self: bool,
) -> &'a [FunctionTypeParamIr] {
    if explicit_self {
        return &method.params;
    }
    let receiver_ty = receiver_type_ref(conformance);
    if method
        .params
        .first()
        .is_some_and(|param| param.ty == receiver_ty)
    {
        &method.params[1..]
    } else {
        &method.params
    }
}

fn substitute_requirement_type(
    ty: &TypeRefIr,
    interface: &InterfaceDeclFact,
    conformance: &InterfaceConformanceFact,
) -> Result<TypeRefIr> {
    if is_self_type(ty) {
        return Ok(receiver_type_ref(conformance));
    }
    if let TypeRefIr::TypeParam { name } = ty {
        if let Some(index) = interface.type_params.iter().position(|param| param == name) {
            return conformance
                .interface
                .args
                .get(index)
                .cloned()
                .ok_or_else(|| {
                    CompileError::Semantic(format!(
                        "interface {} conformance is missing type argument {}",
                        interface.symbol, name
                    ))
                });
        }
    }
    Ok(match ty {
        TypeRefIr::Native { name, args } => TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_requirement_type(arg, interface, conformance))
                .collect::<Result<Vec<_>>>()?,
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|(name, ty)| {
                    Ok((
                        name.clone(),
                        substitute_requirement_type(ty, interface, conformance)?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| substitute_requirement_type(item, interface, conformance))
                .collect::<Result<Vec<_>>>()?,
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(substitute_requirement_type(inner, interface, conformance)?),
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| {
                    Ok(FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: substitute_requirement_type(&param.ty, interface, conformance)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            return_type: Box::new(substitute_requirement_type(
                return_type,
                interface,
                conformance,
            )?),
        },
        TypeRefIr::AnyInterface {
            interface: any_interface,
        } => TypeRefIr::AnyInterface {
            interface: InterfaceInstantiationRef {
                interface_abi_id: any_interface.interface_abi_id.clone(),
                canonical_type_args: any_interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| substitute_requirement_type(arg, interface, conformance))
                    .collect::<Result<Vec<_>>>()?,
            },
        },
        TypeRefIr::TypeParam { .. }
        | TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. } => ty.clone(),
    })
}

fn receiver_type_ref(conformance: &InterfaceConformanceFact) -> TypeRefIr {
    TypeRefIr::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: conformance.receiver.symbol.module_path().to_string(),
            symbol: conformance.receiver.symbol.symbol().to_string(),
        },
    }
}

fn is_self_type(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Native { name, args } if name == "Self" && args.is_empty())
}

fn contains_self_type(ty: &TypeRefIr) -> bool {
    match ty {
        TypeRefIr::Native { args, .. } => is_self_type(ty) || args.iter().any(contains_self_type),
        TypeRefIr::Record { fields } => fields.values().any(contains_self_type),
        TypeRefIr::Union { items } => items.iter().any(contains_self_type),
        TypeRefIr::Nullable { inner } => contains_self_type(inner),
        TypeRefIr::AnyInterface { interface } => {
            interface.canonical_type_args.iter().any(contains_self_type)
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params.iter().any(|param| contains_self_type(&param.ty))
                || contains_self_type(return_type)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

fn interface_requirement_fact(
    module_path: &str,
    interface_type_params: &[String],
    operation: &InterfaceOperation,
    index: &InterfaceIndex,
) -> Result<InterfaceRequirementFact> {
    let type_param_scope = interface_type_params
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let params = operation
        .params
        .iter()
        .map(|param| {
            Ok(FunctionTypeParamIr {
                name: param.name.clone(),
                ty: resolve_type_ref(module_path, &param.ty, &type_param_scope, index)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let return_type = resolve_type_ref(
        module_path,
        &operation.return_type,
        &type_param_scope,
        index,
    )?;
    Ok(InterfaceRequirementFact {
        name: operation.name.clone(),
        type_params: operation.type_params.clone(),
        params,
        return_type,
        is_native: operation.is_native,
        is_provider: operation.is_provider,
        is_static: operation.is_static,
    })
}

fn resolve_type_ref(
    module_path: &str,
    ty: &TypeRef,
    type_param_scope: &BTreeSet<String>,
    index: &InterfaceIndex,
) -> Result<TypeRefIr> {
    let expr = TypeExpr::parse(&ty.name);
    resolve_type_expr(
        module_path,
        &expr,
        type_param_scope,
        &index.source_types,
        &index.types_by_bare,
        &index.interfaces,
        &index.interfaces_by_bare,
    )
}

fn resolve_type_expr(
    module_path: &str,
    expr: &TypeExpr,
    type_param_scope: &BTreeSet<String>,
    source_types: &BTreeMap<SourceSymbolKey, SourceTypeFact>,
    types_by_bare: &BTreeMap<String, Vec<SourceSymbolKey>>,
    interfaces: &BTreeMap<SourceSymbolKey, InterfaceDeclFact>,
    interfaces_by_bare: &BTreeMap<String, Vec<SourceSymbolKey>>,
) -> Result<TypeRefIr> {
    Ok(match expr {
        TypeExpr::EmptyRecord => TypeRefIr::Record {
            fields: BTreeMap::new(),
        },
        TypeExpr::StringLiteral(value) => TypeRefIr::Literal {
            value: LiteralIr::String {
                value: value.clone(),
            },
        },
        TypeExpr::Named { name, args } => {
            let resolved_args = args
                .iter()
                .map(|arg| {
                    resolve_type_expr(
                        module_path,
                        arg,
                        type_param_scope,
                        source_types,
                        types_by_bare,
                        interfaces,
                        interfaces_by_bare,
                    )
                })
                .collect::<Result<Vec<_>>>()?;
            let normalized = name.strip_prefix("root.").unwrap_or(name.as_str());
            if args.is_empty() && type_param_scope.contains(normalized) {
                return Ok(TypeRefIr::TypeParam {
                    name: normalized.to_string(),
                });
            }
            if is_builtin_type(normalized) || is_builtin_generic_type(normalized) {
                return Ok(TypeRefIr::Native {
                    name: normalized.to_string(),
                    args: resolved_args,
                });
            }
            if let Some(symbol) =
                source_type_symbol(module_path, normalized, source_types, types_by_bare)?
            {
                return Ok(TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: symbol.module_path().to_string(),
                        symbol: symbol.symbol().to_string(),
                    },
                });
            }
            if let Some(symbol) = source_symbol_from_qualified(normalized) {
                return Ok(TypeRefIr::ServiceSymbol {
                    symbol: ServiceSymbolRef {
                        module_path: symbol.module_path().to_string(),
                        symbol: symbol.symbol().to_string(),
                    },
                });
            }
            TypeRefIr::Native {
                name: normalized.to_string(),
                args: resolved_args,
            }
        }
        TypeExpr::Nullable(inner) => TypeRefIr::Nullable {
            inner: Box::new(resolve_type_expr(
                module_path,
                inner,
                type_param_scope,
                source_types,
                types_by_bare,
                interfaces,
                interfaces_by_bare,
            )?),
        },
        TypeExpr::Union(items) => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| {
                    resolve_type_expr(
                        module_path,
                        item,
                        type_param_scope,
                        source_types,
                        types_by_bare,
                        interfaces,
                        interfaces_by_bare,
                    )
                })
                .collect::<Result<Vec<_>>>()?,
        },
        TypeExpr::AnyInterface { interface } => {
            let interface = resolve_interface_instantiation_expr(
                module_path,
                interface,
                type_param_scope,
                source_types,
                types_by_bare,
                interfaces,
                interfaces_by_bare,
            )?;
            TypeRefIr::AnyInterface {
                interface: interface_instantiation_ref(
                    interface_symbol_type_ref(&interface.symbol),
                    interface.args,
                ),
            }
        }
        TypeExpr::Record(fields) => TypeRefIr::Record {
            fields: fields
                .iter()
                .map(|field| {
                    Ok((
                        field.name.clone(),
                        resolve_type_expr(
                            module_path,
                            &field.ty,
                            type_param_scope,
                            source_types,
                            types_by_bare,
                            interfaces,
                            interfaces_by_bare,
                        )?,
                    ))
                })
                .collect::<Result<BTreeMap<_, _>>>()?,
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
                        ty: resolve_type_expr(
                            module_path,
                            &param.ty,
                            type_param_scope,
                            source_types,
                            types_by_bare,
                            interfaces,
                            interfaces_by_bare,
                        )?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            return_type: Box::new(resolve_type_expr(
                module_path,
                return_type,
                type_param_scope,
                source_types,
                types_by_bare,
                interfaces,
                interfaces_by_bare,
            )?),
        },
    })
}

fn resolve_interface_instantiation_expr(
    module_path: &str,
    expr: &TypeExpr,
    type_param_scope: &BTreeSet<String>,
    source_types: &BTreeMap<SourceSymbolKey, SourceTypeFact>,
    types_by_bare: &BTreeMap<String, Vec<SourceSymbolKey>>,
    interfaces: &BTreeMap<SourceSymbolKey, InterfaceDeclFact>,
    interfaces_by_bare: &BTreeMap<String, Vec<SourceSymbolKey>>,
) -> Result<InterfaceInstantiation> {
    let TypeExpr::Named { name, args } = expr else {
        return Err(CompileError::Semantic(format!(
            "interface selector `{}` must be a named interface type",
            expr.to_type_string()
        )));
    };
    let symbol = source_interface_symbol(
        module_path,
        name,
        source_types,
        types_by_bare,
        interfaces,
        interfaces_by_bare,
    )?;
    let fact = interfaces.get(&symbol).ok_or_else(|| {
        CompileError::Semantic(format!(
            "interface selector `{}` references unknown interface {symbol}",
            expr.to_type_string()
        ))
    })?;
    if fact.type_params.len() != args.len() {
        return Err(CompileError::Semantic(format!(
            "interface selector `{}` targets interface {}, which expects {} type arguments, found {}",
            expr.to_type_string(),
            symbol,
            fact.type_params.len(),
            args.len()
        )));
    }
    let args = args
        .iter()
        .map(|arg| {
            resolve_type_expr(
                module_path,
                arg,
                type_param_scope,
                source_types,
                types_by_bare,
                interfaces,
                interfaces_by_bare,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(InterfaceInstantiation { symbol, args })
}

fn source_type_symbol(
    module_path: &str,
    name: &str,
    source_types: &BTreeMap<SourceSymbolKey, SourceTypeFact>,
    types_by_bare: &BTreeMap<String, Vec<SourceSymbolKey>>,
) -> Result<Option<SourceSymbolKey>> {
    let local = SourceSymbolKey::new(module_path, name);
    if source_types.contains_key(&local) {
        return Ok(Some(local));
    }
    if let Some(symbol) = source_symbol_from_qualified(name) {
        if source_types.contains_key(&symbol) {
            return Ok(Some(symbol));
        }
    }
    match types_by_bare.get(name).map(Vec::as_slice) {
        Some([symbol]) => Ok(Some(symbol.clone())),
        Some(symbols) if symbols.len() > 1 => Err(CompileError::Semantic(format!(
            "bare type `{name}` is ambiguous across {}; use a qualified type name",
            symbols
                .iter()
                .map(SourceSymbolKey::to_source_symbol)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        _ => Ok(None),
    }
}

fn source_interface_symbol(
    module_path: &str,
    name: &str,
    source_types: &BTreeMap<SourceSymbolKey, SourceTypeFact>,
    types_by_bare: &BTreeMap<String, Vec<SourceSymbolKey>>,
    interfaces: &BTreeMap<SourceSymbolKey, InterfaceDeclFact>,
    interfaces_by_bare: &BTreeMap<String, Vec<SourceSymbolKey>>,
) -> Result<SourceSymbolKey> {
    let normalized = name.strip_prefix("root.").unwrap_or(name);
    if let Some(symbol) = source_symbol_from_qualified(normalized) {
        if interfaces.contains_key(&symbol) {
            return Ok(symbol);
        }
        if source_types.contains_key(&symbol) {
            return Err(CompileError::Semantic(format!(
                "interface selector `{name}` resolves to type {symbol}, not an interface"
            )));
        }
        return Err(CompileError::Semantic(format!(
            "interface selector `{name}` does not resolve to an interface"
        )));
    }
    let local = SourceSymbolKey::new(module_path, normalized);
    if interfaces.contains_key(&local) {
        return Ok(local);
    }
    if source_types.contains_key(&local) {
        return Err(CompileError::Semantic(format!(
            "interface selector `{name}` resolves to type {local}, not an interface"
        )));
    }
    match interfaces_by_bare.get(normalized).map(Vec::as_slice) {
        Some([symbol]) => Ok(symbol.clone()),
        Some(symbols) if symbols.len() > 1 => Err(CompileError::Semantic(format!(
            "bare interface selector `{name}` is ambiguous across {}; use a qualified interface name",
            symbols
                .iter()
                .map(SourceSymbolKey::to_source_symbol)
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        _ if types_by_bare
            .get(normalized)
            .is_some_and(|symbols| !symbols.is_empty()) =>
        {
            Err(CompileError::Semantic(format!(
                "interface selector `{name}` resolves to a type name, not an interface"
            )))
        }
        _ => Err(CompileError::Semantic(format!(
            "interface selector `{name}` does not resolve to an interface"
        ))),
    }
}

fn source_symbol_from_qualified(name: &str) -> Option<SourceSymbolKey> {
    let (module_path, symbol) = name.rsplit_once('.')?;
    if module_path.is_empty() || symbol.is_empty() {
        return None;
    }
    Some(SourceSymbolKey::new(module_path, symbol))
}

fn is_builtin_type(name: &str) -> bool {
    matches!(
        name,
        "string"
            | "String"
            | "integer"
            | "number"
            | "bool"
            | "Bool"
            | "null"
            | "void"
            | "never"
            | "Json"
            | "JsonObject"
            | "Date"
            | "Duration"
            | "Instant"
            | "bytes"
    )
}

fn is_builtin_generic_type(name: &str) -> bool {
    matches!(
        name.rsplit('.').next().unwrap_or(name),
        "Array" | "Map" | "Set" | "Result" | "Stream" | "ActorRef"
    )
}

fn is_probable_external_interface(name: &str) -> bool {
    !name.starts_with("root.") && name.contains('.')
}

pub fn actor_interface_symbol_key() -> SourceSymbolKey {
    SourceSymbolKey::new(ACTOR_INTERFACE_MODULE, ACTOR_INTERFACE_SYMBOL)
}

pub fn interface_instantiation_display(interface: &InterfaceInstantiation) -> String {
    if interface.args.is_empty() {
        return interface.symbol.to_source_symbol();
    }
    format!(
        "{}<{}>",
        interface.symbol,
        interface
            .args
            .iter()
            .map(type_ref_display)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub fn type_instantiation_pattern_display(pattern: &TypeInstantiationPattern) -> String {
    if pattern.args.is_empty() {
        return pattern.symbol.to_source_symbol();
    }
    format!(
        "{}<{}>",
        pattern.symbol,
        pattern
            .args
            .iter()
            .map(type_ref_display)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub fn object_safety_diagnostics_display(
    diagnostics: &[InterfaceObjectSafetyDiagnostic],
) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| match diagnostic {
            InterfaceObjectSafetyDiagnostic::MarkerInterface { interface } => {
                format!("interface {interface} is a marker interface")
            }
            InterfaceObjectSafetyDiagnostic::MissingSelfReceiver { method_name } => {
                format!("method {method_name} must declare `self: Self` as its first parameter")
            }
            InterfaceObjectSafetyDiagnostic::UnsupportedMethodRequirement {
                method_name,
                message,
            } => {
                format!("method {method_name} {message}")
            }
            InterfaceObjectSafetyDiagnostic::InvalidSelfUsage {
                method_name,
                message,
            } => {
                format!("method {method_name}: {message}")
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn type_ref_display(ty: &TypeRefIr) -> String {
    match ty {
        TypeRefIr::Native { name, args } if args.is_empty() => name.clone(),
        TypeRefIr::Native { name, args } => format!(
            "{name}<{}>",
            args.iter()
                .map(type_ref_display)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::LocalType { type_index } => format!("$localType{type_index}"),
        TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
            symbol.symbol_path()
        }
        TypeRefIr::PackageSymbol { symbol } => symbol.symbol_path.clone(),
        TypeRefIr::Record { fields } => format!(
            "{{ {} }}",
            fields
                .iter()
                .map(|(name, ty)| format!("{name}: {}", type_ref_display(ty)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TypeRefIr::Union { items } => items
            .iter()
            .map(type_ref_display)
            .collect::<Vec<_>>()
            .join(" | "),
        TypeRefIr::Nullable { inner } => format!("{}?", type_ref_display(inner)),
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
                        .map(type_ref_display)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        }
        TypeRefIr::Literal { value } => match value {
            LiteralIr::Null => "null".to_string(),
            LiteralIr::Bool { value } => value.to_string(),
            LiteralIr::Number { value } => value.to_string(),
            LiteralIr::String { value } => serde_json::to_string(value).unwrap_or_default(),
        },
        TypeRefIr::TypeParam { name } => name.clone(),
        TypeRefIr::Function {
            params,
            return_type,
        } => format!(
            "fn({}) -> {}",
            params
                .iter()
                .map(|param| format!("{}: {}", param.name, type_ref_display(&param.ty)))
                .collect::<Vec<_>>()
                .join(", "),
            type_ref_display(return_type)
        ),
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

fn interface_type_arg_substitutions(
    fact: &InterfaceDeclFact,
    interface: &InterfaceInstantiation,
) -> Result<BTreeMap<String, TypeRefIr>> {
    if fact.type_params.len() != interface.args.len() {
        return Err(CompileError::Semantic(format!(
            "interface {} expects {} type arguments, found {}",
            fact.symbol,
            fact.type_params.len(),
            interface.args.len()
        )));
    }
    Ok(fact
        .type_params
        .iter()
        .cloned()
        .zip(interface.args.iter().cloned())
        .collect())
}

fn instantiate_conformance_for_receiver(
    fact: &InterfaceConformanceFact,
    receiver: &TypeInstantiationPattern,
) -> Option<InterfaceConformanceFact> {
    if fact.receiver.symbol != receiver.symbol
        || fact.receiver_type_params.len() != receiver.args.len()
    {
        return None;
    }
    let substitutions = fact
        .receiver_type_params
        .iter()
        .cloned()
        .zip(receiver.args.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    Some(InterfaceConformanceFact {
        receiver_type_params: Vec::new(),
        receiver: receiver.clone(),
        interface: InterfaceInstantiation {
            symbol: fact.interface.symbol.clone(),
            args: fact
                .interface
                .args
                .iter()
                .map(|arg| substitute_type_params_in_type_ref_ref(arg, &substitutions))
                .collect(),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::{
        semantic::{SemanticPublication, SemanticSource, SourceOrigin},
        shared::parser::parse_source,
    };

    use super::*;

    fn build_semantics(source: &str) -> InterfaceSemantics {
        let ast = parse_source(source).expect("test source should parse");
        let aliases = BTreeMap::new();
        let publication = SemanticPublication::new(vec![SemanticSource::new(
            "test.skiff",
            "test",
            SourceOrigin::Service,
            &ast,
            &aliases,
        )]);
        InterfaceSemantics::build(&publication).expect("interface semantics should build")
    }

    fn inst(symbol: &str, args: Vec<TypeRefIr>) -> InterfaceInstantiation {
        InterfaceInstantiation {
            symbol: SourceSymbolKey::new("test", symbol),
            args,
        }
    }

    #[test]
    fn marker_interface_is_not_object_safe_for_dynamic_any_interface() {
        let semantics = build_semantics("interface Marker {}\n");
        let interface = inst("Marker", Vec::new());

        assert_eq!(
            semantics.object_safety_diagnostics(&interface).unwrap(),
            vec![InterfaceObjectSafetyDiagnostic::MarkerInterface {
                interface: SourceSymbolKey::new("test", "Marker"),
            }]
        );
        assert!(!semantics.is_object_safe_interface(&interface).unwrap());
    }

    #[test]
    fn object_safety_reports_self_outside_receiver() {
        let semantics = build_semantics(
            r#"
            interface CloneLike {
                function clone() -> Self
            }
            "#,
        );
        let interface = inst("CloneLike", Vec::new());
        let diagnostics = semantics.object_safety_diagnostics(&interface).unwrap();

        assert!(matches!(
            diagnostics.as_slice(),
            [InterfaceObjectSafetyDiagnostic::InvalidSelfUsage { method_name, message }]
                if method_name == "clone"
                    && message.contains("can only use Self in the first receiver parameter")
        ));
    }

    #[test]
    fn object_safety_reports_missing_self_receiver() {
        let semantics = build_semantics(
            r#"
            interface Reader {
                function read() -> string
            }
            "#,
        );
        let interface = inst("Reader", Vec::new());

        assert_eq!(
            semantics.object_safety_diagnostics(&interface).unwrap(),
            vec![InterfaceObjectSafetyDiagnostic::MissingSelfReceiver {
                method_name: "read".to_string(),
            }]
        );
    }

    #[test]
    fn interface_method_level_generics_fail_closed_in_existing_parser_diagnostic() {
        let error = parse_source(
            r#"
            interface GenericMethod {
                function get<T>() -> T
            }
            "#,
        )
        .expect_err("interface method type params must be rejected")
        .to_string();

        assert!(
            error.contains("interface method requirements cannot declare type parameters"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn method_slots_substitute_interface_type_args_without_reparsing_text() {
        let semantics = build_semantics(
            r#"
            interface Reader<T> {
                function read(self: Self, fallback: T) -> Array<T>
            }
            "#,
        );
        let interface = inst("Reader", vec![TypeRefIr::native("string")]);

        let slots = semantics.method_slots_for_interface(&interface).unwrap();

        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].slot, 0);
        assert_eq!(slots[0].name, "read");
        assert_eq!(
            slots[0].params,
            vec![
                FunctionTypeParamIr {
                    name: "self".to_string(),
                    ty: TypeRefIr::native("Self"),
                },
                FunctionTypeParamIr {
                    name: "fallback".to_string(),
                    ty: TypeRefIr::native("string"),
                },
            ]
        );
        assert_eq!(
            slots[0].return_type,
            TypeRefIr::Native {
                name: "Array".to_string(),
                args: vec![TypeRefIr::native("string")],
            }
        );
        assert_eq!(
            slots[0].method_abi_id,
            canonical_interface_method_abi_id(
                &semantics.canonical_interface_instantiation_ref(&interface),
                "read",
            )
        );
    }

    #[test]
    fn local_conformance_lookup_substitutes_generic_receiver_args() {
        let semantics = build_semantics(
            r#"
            interface Reader<T> {
                function read(self: Self) -> T
            }
            type Box<T> implements Reader<T> {}
            impl Box {
                function read() -> T { return }
            }
            "#,
        );
        let receiver = TypeInstantiationPattern {
            symbol: SourceSymbolKey::new("test", "Box"),
            args: vec![TypeRefIr::native("string")],
        };
        let interface = inst("Reader", vec![TypeRefIr::native("string")]);

        let conformance = semantics
            .local_conformance_for_receiver_instantiation(&receiver, &interface)
            .expect("generic conformance should instantiate for concrete receiver");

        assert_eq!(conformance.receiver, receiver);
        assert_eq!(conformance.interface, interface);
    }
}
