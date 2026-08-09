use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{LiteralIr, NamedUnionBranchIr, NominalTypeRefBaseIr, TypeRefIr};
use skiff_compiler_core::{
    prelude_registry::{compiler_builtin_type, CompilerBuiltinType, CompilerBuiltinTypeKind},
    type_ref::{substitute_type_params_in_type_ref_ref, BuiltinShape},
};

use super::{
    ResolvedNamedType, ResolvedTypeRef, SourceTypeKind, TypeResolutionContext, TypeResolutionModel,
};
use crate::{
    runtime_type_projection::lower_prelude_type_decl,
    shared::{
        id::SKIFF_STD_PUBLICATION_ID, prelude_registry::prelude_registry, type_expr::TypeExpr,
    },
};

/// The exact nominal leaves accepted by source `throw`, `catch` and
/// `rethrow`. Anonymous unions contribute their branches directly; named
/// unions retain both their nominal owner and the frozen branch input.
#[derive(Clone, Debug, PartialEq)]
pub struct CatchLeaves {
    leaves: Vec<CatchLeafIdentity>,
}

impl CatchLeaves {
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    pub fn identities(&self) -> &[CatchLeafIdentity] {
        &self.leaves
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CatchLeafIdentity {
    Nominal {
        nominal_type: TypeRefIr,
    },
    NamedUnionBranch {
        union_type: TypeRefIr,
        branch: NamedUnionBranchIr,
    },
}

struct NominalInstantiation<'a> {
    named: ResolvedNamedType<'a>,
    substitutions: BTreeMap<String, TypeRefIr>,
}

impl TypeResolutionModel {
    /// Computes the reusable source-language CatchLeaves set.
    ///
    /// The resolved IR is the sole identity source. Applied nominal arguments
    /// are carried positionally by `TypeRefIr::AppliedNominal`.
    pub fn catch_leaves(
        &self,
        ty: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CatchLeaves, String> {
        let leaves = self.collect_catch_leaves(&ty.ir, context)?;
        if leaves.is_empty() {
            return Err(format!("`{ty}` has no catch leaves"));
        }
        Ok(CatchLeaves { leaves })
    }

    pub fn exception_catch_leaves(
        &self,
        exception: &ResolvedTypeRef,
        context: &TypeResolutionContext<'_>,
    ) -> Result<CatchLeaves, String> {
        let TypeRefIr::Builtin { name, args } = &exception.ir else {
            return Err(format!(
                "rethrow operand must be Exception<E>, found `{exception}`"
            ));
        };
        let [payload] = args.as_slice() else {
            return Err(format!(
                "rethrow operand must be Exception<E>, found `{exception}`"
            ));
        };
        if name != BuiltinShape::Exception.name() {
            return Err(format!(
                "rethrow operand must be Exception<E>, found `{exception}`"
            ));
        }

        let leaves = self.collect_catch_leaves(payload, context)?;
        if leaves.is_empty() {
            return Err("Exception<E> payload has no catch leaves".to_string());
        }
        Ok(CatchLeaves { leaves })
    }

    fn collect_catch_leaves(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Vec<CatchLeafIdentity>, String> {
        if let Some(leaves) = self.collect_prelude_catch_leaves(ty, context)? {
            return Ok(leaves);
        }
        if let Some(instantiation) = self
            .resolved_named_type(ty, context)
            .map(|named| self.nominal_instantiation_for(named, ty, context))
            .transpose()?
        {
            return self.collect_nominal_catch_leaves(ty, instantiation, context);
        }

        match ty {
            TypeRefIr::PackageSchema { .. } => Ok(vec![CatchLeafIdentity::Nominal {
                nominal_type: ty.clone(),
            }]),
            TypeRefIr::Union { items } => {
                let mut leaves = Vec::new();
                for item in items {
                    leaves.extend(self.collect_catch_leaves(item, context)?);
                }
                if leaves.is_empty() {
                    return Err("anonymous union has no catch leaves".to_string());
                }
                Ok(leaves)
            }
            TypeRefIr::Nullable { .. } => {
                Err("nullable types include a null branch without catch identity".to_string())
            }
            TypeRefIr::TypeParam { name } => Err(format!(
                "unconstrained type parameter `{name}` has no catch identity"
            )),
            TypeRefIr::Builtin { name, .. } => Err(format!(
                "unwrapped primitive or container `{name}` has no catch identity"
            )),
            TypeRefIr::Literal { .. } => Err("unwrapped literal has no catch identity".to_string()),
            TypeRefIr::Record { .. } => Err("anonymous record has no catch identity".to_string()),
            TypeRefIr::AnyInterface { .. } => {
                Err("interface values have no catch identity".to_string())
            }
            TypeRefIr::Function { .. } => Err("function values have no catch identity".to_string()),
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::AppliedNominal { .. }
            | TypeRefIr::DbObjectSymbol { .. } => {
                Err("nominal catch type cannot be resolved to its declaration".to_string())
            }
        }
    }

    fn collect_prelude_catch_leaves(
        &self,
        nominal_type: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<Option<Vec<CatchLeafIdentity>>, String> {
        if let TypeRefIr::Builtin { name, args } = nominal_type {
            if let Some(builtin) = compiler_builtin_type(name) {
                if builtin.kind != CompilerBuiltinTypeKind::Error {
                    return Ok(None);
                }
                if args.len() != builtin.arity {
                    return Err(format!(
                        "compiler nominal `{name}` expects {} type arguments, found {}",
                        builtin.arity,
                        args.len()
                    ));
                }
                for argument in args {
                    self.validate_instantiated_type_argument(argument, context)?;
                }
                return Ok(Some(vec![CatchLeafIdentity::Nominal {
                    nominal_type: nominal_type.clone(),
                }]));
            }
        }

        let Some((symbol, type_params)) = prelude_nominal_type(nominal_type) else {
            return Ok(None);
        };
        let arguments = nominal_arguments(nominal_type);
        if arguments.len() != type_params.len() {
            return Err(format!(
                "prelude nominal `{symbol}` requires {} fully-instantiated type arguments, found {}",
                type_params.len(),
                arguments.len()
            ));
        }
        let substitutions = type_params
            .iter()
            .zip(arguments)
            .map(|(name, argument)| {
                self.validate_instantiated_type_argument(argument, context)?;
                Ok((name.clone(), argument.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;

        let registry = prelude_registry();
        if let Some(alias) = registry.type_alias(&symbol) {
            let module_path = symbol
                .rsplit_once('.')
                .map_or(context.module_path, |(module, _)| module);
            let alias_context = TypeResolutionContext::source(module_path);
            let expanded =
                self.resolve_type_expr(&TypeExpr::parse(&alias.target_type.name), &alias_context)?;
            return self
                .collect_catch_leaves(&expanded, &alias_context)
                .map(Some);
        }
        let declaration = registry
            .type_decl(&symbol)
            .ok_or_else(|| format!("prelude nominal `{symbol}` has no declaration"))?;
        let lowered = lower_prelude_type_decl(declaration)?;
        let leaves = match lowered.descriptor {
            skiff_artifact_model::TypeDescriptorIr::Record { .. }
            | skiff_artifact_model::TypeDescriptorIr::Representation { .. } => {
                vec![CatchLeafIdentity::Nominal {
                    nominal_type: nominal_type.clone(),
                }]
            }
            skiff_artifact_model::TypeDescriptorIr::Union { branches } => {
                let branches = branches
                    .iter()
                    .map(|branch| substitute_named_union_branch(branch, &substitutions))
                    .collect::<Vec<_>>();
                if branches.is_empty() {
                    return Err(format!("prelude named union `{symbol}` has no branches"));
                }
                let branch_context = TypeResolutionContext::source(
                    symbol
                        .rsplit_once('.')
                        .map_or(context.module_path, |(module, _)| module),
                );
                for branch in &branches {
                    self.validate_named_union_branch(branch, &branch_context)?;
                }
                branches
                    .into_iter()
                    .map(|branch| CatchLeafIdentity::NamedUnionBranch {
                        union_type: nominal_type.clone(),
                        branch,
                    })
                    .collect()
            }
            skiff_artifact_model::TypeDescriptorIr::Alias { .. } => {
                return Err(format!(
                    "prelude nominal `{symbol}` unexpectedly lowered as a transparent alias"
                ));
            }
            skiff_artifact_model::TypeDescriptorIr::Interface => {
                return Err(format!(
                    "prelude interface `{symbol}` has no catch identity"
                ));
            }
        };
        Ok(Some(leaves))
    }

    fn collect_nominal_catch_leaves(
        &self,
        nominal_type: &TypeRefIr,
        instantiation: NominalInstantiation<'_>,
        caller_context: &TypeResolutionContext<'_>,
    ) -> Result<Vec<CatchLeafIdentity>, String> {
        match &instantiation.named.resolution.kind {
            SourceTypeKind::Record { .. } => Ok(vec![CatchLeafIdentity::Nominal {
                nominal_type: nominal_type.clone(),
            }]),
            SourceTypeKind::Representation {
                target,
                named_union_branches,
                discriminator,
            } => {
                let branches = if let Some(branches) = named_union_branches {
                    branches
                        .iter()
                        .map(|branch| {
                            substitute_named_union_branch(branch, &instantiation.substitutions)
                        })
                        .collect::<Vec<_>>()
                } else {
                    let target = TypeExpr::parse(target);
                    let TypeExpr::Union(branches) = target else {
                        return Ok(vec![CatchLeafIdentity::Nominal {
                            nominal_type: nominal_type.clone(),
                        }]);
                    };
                    self.source_named_union_branches(
                        &instantiation.named,
                        &branches,
                        discriminator.as_deref(),
                        &instantiation.substitutions,
                        caller_context,
                    )?
                };
                if branches.is_empty() {
                    return Err("named union has no branches".to_string());
                }
                let branch_context = TypeResolutionContext::with_type_params(
                    &instantiation.named.source_module_path,
                    BTreeSet::new(),
                );
                for branch in &branches {
                    self.validate_named_union_branch(branch, &branch_context)?;
                }
                Ok(branches
                    .into_iter()
                    .map(|branch| CatchLeafIdentity::NamedUnionBranch {
                        union_type: nominal_type.clone(),
                        branch,
                    })
                    .collect())
            }
            SourceTypeKind::Alias { .. } => {
                let expanded = self.expand_alias_type_ref(nominal_type, caller_context)?;
                if expanded == *nominal_type {
                    return Err("transparent alias could not be expanded".to_string());
                }
                self.collect_catch_leaves(&expanded, caller_context)
            }
            SourceTypeKind::Actor { .. } => {
                Err("actor handles are not user `type` catch payloads".to_string())
            }
            SourceTypeKind::External => Err("interfaces have no catch identity".to_string()),
        }
    }

    fn nominal_instantiation_for<'a>(
        &'a self,
        named: ResolvedNamedType<'a>,
        nominal_type: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<NominalInstantiation<'a>, String> {
        let arguments = nominal_arguments(nominal_type);
        let expected = named.resolution.type_params.len();
        if arguments.len() != expected {
            return Err(format!(
                "generic nominal `{}` requires {expected} fully-instantiated type arguments, found {}",
                named.resolution.name,
                arguments.len()
            ));
        }
        let mut substitutions = BTreeMap::new();
        for (name, argument) in named.resolution.type_params.iter().zip(arguments) {
            self.validate_instantiated_type_argument(argument, context)?;
            substitutions.insert(name.clone(), argument.clone());
        }
        if expected == 0 && !substitutions.is_empty() {
            return Err(format!(
                "non-generic nominal `{}` received type arguments",
                named.resolution.name
            ));
        }
        if matches!(nominal_type, TypeRefIr::TypeParam { .. }) {
            return Err("unconstrained type parameter has no nominal identity".to_string());
        }
        Ok(NominalInstantiation {
            named,
            substitutions,
        })
    }

    fn validate_instantiated_type_argument(
        &self,
        ty: &TypeRefIr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<(), String> {
        match ty {
            TypeRefIr::TypeParam { name } => {
                Err(format!("type argument `{name}` is not fully instantiated"))
            }
            TypeRefIr::Builtin { name, .. } if name == BuiltinShape::Unknown.name() => {
                Err("type argument `unknown` has no runtime identity".to_string())
            }
            TypeRefIr::Builtin { args, .. } => {
                for argument in args {
                    self.validate_instantiated_type_argument(argument, context)?;
                }
                Ok(())
            }
            TypeRefIr::AppliedNominal { arguments, .. } => {
                let named = self
                    .resolved_named_type(ty, context)
                    .ok_or_else(|| "applied nominal declaration cannot be resolved".to_string())?;
                if named.resolution.type_params.len() != arguments.len() {
                    return Err(format!(
                        "nested generic nominal `{}` expects {} type arguments, found {}",
                        named.resolution.name,
                        named.resolution.type_params.len(),
                        arguments.len()
                    ));
                }
                for argument in arguments {
                    self.validate_instantiated_type_argument(argument, context)?;
                }
                Ok(())
            }
            TypeRefIr::Record { fields } => {
                for field in fields.values() {
                    self.validate_instantiated_type_argument(field, context)?;
                }
                Ok(())
            }
            TypeRefIr::Union { items } => {
                for item in items {
                    self.validate_instantiated_type_argument(item, context)?;
                }
                Ok(())
            }
            TypeRefIr::Nullable { inner } => {
                self.validate_instantiated_type_argument(inner, context)
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.validate_instantiated_type_argument(&param.ty, context)?;
                }
                self.validate_instantiated_type_argument(return_type, context)
            }
            TypeRefIr::AnyInterface { interface } => {
                for argument in &interface.canonical_type_args {
                    self.validate_instantiated_type_argument(argument, context)?;
                }
                Ok(())
            }
            TypeRefIr::LocalType { .. }
            | TypeRefIr::PublicationType { .. }
            | TypeRefIr::ServiceSymbol { .. }
            | TypeRefIr::PackageSymbol { .. }
            | TypeRefIr::DbObjectSymbol { .. } => {
                let named = self.resolved_named_type(ty, context).ok_or_else(|| {
                    "type argument nominal declaration cannot be resolved".to_string()
                })?;
                if named.resolution.type_params.is_empty() {
                    Ok(())
                } else {
                    Err(format!(
                        "nested generic nominal `{}` lost its instantiated type arguments",
                        named.resolution.name
                    ))
                }
            }
            TypeRefIr::PackageSchema { .. } | TypeRefIr::Literal { .. } => Ok(()),
        }
    }

    fn source_named_union_branches(
        &self,
        owner: &ResolvedNamedType<'_>,
        branches: &[TypeExpr],
        discriminator: Option<&str>,
        substitutions: &BTreeMap<String, TypeRefIr>,
        caller_context: &TypeResolutionContext<'_>,
    ) -> Result<Vec<NamedUnionBranchIr>, String> {
        let declaration_context = TypeResolutionContext::with_type_params(
            &owner.source_module_path,
            owner
                .resolution
                .type_params
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>(),
        );
        branches
            .iter()
            .map(|branch| match branch {
                TypeExpr::StringLiteral(value) => Ok(NamedUnionBranchIr::Literal {
                    value: LiteralIr::String {
                        value: value.clone(),
                    },
                }),
                TypeExpr::Record(fields) => {
                    let discriminator = discriminator.ok_or_else(|| {
                        "anonymous named-union branch has no discriminator".to_string()
                    })?;
                    let discriminator_value = fields
                        .iter()
                        .find(|field| field.name == discriminator)
                        .and_then(|field| match &field.ty {
                            TypeExpr::StringLiteral(value) => Some(value.clone()),
                            _ => None,
                        })
                        .ok_or_else(|| {
                            format!(
                                "anonymous named-union branch discriminator `{discriminator}` is not a string literal"
                            )
                        })?;
                    let payload_type = self.resolve_type_expr(branch, &declaration_context)?;
                    let payload_type =
                        substitute_type_params_in_type_ref_ref(&payload_type, substitutions);
                    Ok(NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type,
                        discriminator_field: discriminator.to_string(),
                        discriminator_value,
                    })
                }
                TypeExpr::Named { .. } => {
                    let nominal_type = self.resolve_type_expr(branch, &declaration_context)?;
                    let nominal_type =
                        substitute_type_params_in_type_ref_ref(&nominal_type, substitutions);
                    let named = self
                        .resolved_named_type(&nominal_type, caller_context)
                        .or_else(|| {
                            self.resolved_named_type(&nominal_type, &declaration_context)
                        })
                        .ok_or_else(|| {
                            format!(
                                "named-union branch `{}` is not a concrete nominal type",
                                branch.to_type_string()
                            )
                        })?;
                    if matches!(
                        named.resolution.kind,
                        SourceTypeKind::Alias { .. }
                            | SourceTypeKind::Actor { .. }
                            | SourceTypeKind::External
                    ) {
                        return Err(format!(
                            "named-union branch `{}` is not a concrete nominal type",
                            branch.to_type_string()
                        ));
                    }
                    if source_kind_is_named_union(&named.resolution.kind) {
                        return Err(format!(
                            "named-union branch `{}` cannot nest another named union",
                            branch.to_type_string()
                        ));
                    }
                    Ok(NamedUnionBranchIr::ConcreteNominal { nominal_type })
                }
                _ => Err(format!(
                    "named-union branch `{}` has no deterministic branch identity",
                    branch.to_type_string()
                )),
            })
            .collect()
    }

    fn validate_named_union_branch(
        &self,
        branch: &NamedUnionBranchIr,
        context: &TypeResolutionContext<'_>,
    ) -> Result<(), String> {
        match branch {
            NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                let (name, type_params) =
                    if let Some(named) = self.resolved_named_type(nominal_type, context) {
                        if matches!(
                            named.resolution.kind,
                            SourceTypeKind::Alias { .. }
                                | SourceTypeKind::Actor { .. }
                                | SourceTypeKind::External
                        ) {
                            return Err("concrete named-union branch is not a user nominal type"
                                .to_string());
                        }
                        (
                            named.resolution.name.clone(),
                            named.resolution.type_params.as_slice(),
                        )
                    } else if let Some((symbol, type_params)) = prelude_nominal_type(nominal_type) {
                        (symbol, type_params)
                    } else if compiler_builtin_type_for_catch(nominal_type)
                        .is_some_and(|builtin| builtin.arity == 0)
                    {
                        ("compiler error nominal".to_string(), &[] as &[String])
                    } else {
                        return Err(
                            "concrete named-union branch cannot resolve its nominal declaration"
                                .to_string(),
                        );
                    };
                let arguments = nominal_arguments(nominal_type);
                if arguments.len() != type_params.len() {
                    return Err(format!(
                        "concrete named-union branch `{name}` is not fully instantiated",
                    ));
                }
                for argument in arguments {
                    self.validate_instantiated_type_argument(argument, context)?;
                }
                Ok(())
            }
            NamedUnionBranchIr::SyntheticDiscriminator {
                payload_type,
                discriminator_field,
                discriminator_value,
            } => {
                let TypeRefIr::Record { fields } = payload_type else {
                    return Err(
                        "synthetic named-union branch payload must be an anonymous record"
                            .to_string(),
                    );
                };
                if discriminator_field.is_empty() || discriminator_value.is_empty() {
                    return Err(
                        "synthetic named-union branch discriminator must be non-empty".to_string(),
                    );
                }
                let Some(TypeRefIr::Literal {
                    value: LiteralIr::String { value },
                }) = fields.get(discriminator_field)
                else {
                    return Err(
                        "synthetic named-union branch payload is missing its discriminator literal"
                            .to_string(),
                    );
                };
                if value != discriminator_value {
                    return Err(
                        "synthetic named-union branch discriminator value does not match payload"
                            .to_string(),
                    );
                }
                Ok(())
            }
            NamedUnionBranchIr::Literal { .. } => Ok(()),
        }
    }
}

fn substitute_named_union_branch(
    branch: &NamedUnionBranchIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> NamedUnionBranchIr {
    match branch {
        NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type: substitute_type_params_in_type_ref_ref(nominal_type, substitutions),
            }
        }
        NamedUnionBranchIr::SyntheticDiscriminator {
            payload_type,
            discriminator_field,
            discriminator_value,
        } => NamedUnionBranchIr::SyntheticDiscriminator {
            payload_type: substitute_type_params_in_type_ref_ref(payload_type, substitutions),
            discriminator_field: discriminator_field.clone(),
            discriminator_value: discriminator_value.clone(),
        },
        NamedUnionBranchIr::Literal { value } => NamedUnionBranchIr::Literal {
            value: value.clone(),
        },
    }
}

fn source_kind_is_named_union(kind: &SourceTypeKind) -> bool {
    matches!(
        kind,
        SourceTypeKind::Representation {
            named_union_branches: Some(_),
            ..
        }
    ) || matches!(
        kind,
        SourceTypeKind::Representation {
            target,
            named_union_branches: None,
            ..
        } if matches!(TypeExpr::parse(target), TypeExpr::Union(_))
    )
}

fn compiler_builtin_type_for_catch(ty: &TypeRefIr) -> Option<&'static CompilerBuiltinType> {
    let TypeRefIr::Builtin { name, .. } = ty else {
        return None;
    };
    compiler_builtin_type(name).filter(|builtin| builtin.kind == CompilerBuiltinTypeKind::Error)
}

fn nominal_arguments(ty: &TypeRefIr) -> &[TypeRefIr] {
    match ty {
        TypeRefIr::AppliedNominal { arguments, .. } => arguments,
        TypeRefIr::Builtin { args, .. } => args,
        _ => &[],
    }
}

fn prelude_nominal_type(ty: &TypeRefIr) -> Option<(String, &'static [String])> {
    let symbol = match ty {
        TypeRefIr::Builtin { name, .. } if compiler_builtin_type(name).is_none() => {
            prelude_registry().known_type_symbol(name)?
        }
        TypeRefIr::PackageSymbol { symbol }
            if matches!(
                &symbol.package,
                skiff_artifact_model::PackageRefIr::PackageId { package_id }
                    if package_id == SKIFF_STD_PUBLICATION_ID
            ) =>
        {
            symbol.symbol_path.clone()
        }
        TypeRefIr::AppliedNominal {
            base: NominalTypeRefBaseIr::PackageSymbol { symbol },
            ..
        } if matches!(
            &symbol.package,
            skiff_artifact_model::PackageRefIr::PackageId { package_id }
                if package_id == SKIFF_STD_PUBLICATION_ID
        ) =>
        {
            symbol.symbol_path.clone()
        }
        _ => return None,
    };
    let registry = prelude_registry();
    if let Some(declaration) = registry.type_decl(&symbol) {
        return Some((symbol, declaration.type_params.as_slice()));
    }
    registry
        .type_alias(&symbol)
        .map(|_| (symbol, &[] as &'static [String]))
}
