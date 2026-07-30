mod path;
mod source;

use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{NamedUnionBranchIr, TypeDescriptorIr, TypeRefIr};

use crate::type_ref::{substitute_type_params_in_type_ref_ref, type_ref_children};

pub use path::{
    NoTypeClosureGuards, RepresentationIndirectionGuards, TypeClosureGuardPolicy, TypeClosureTrace,
    TypeClosureTraceSegment,
};
pub use source::{
    is_nominal_type_ref, type_decl_for_symbol_in_unit, ArtifactNominalTypeSource, NominalTypeKey,
    NominalTypeResolver, PackageTypeSource, ResolvedNominalType,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypeClosureControl {
    Continue,
    Prune,
}

#[derive(Clone, Copy, Debug)]
pub struct TypeClosureVisit<'a> {
    pub module_path: &'a str,
    pub ty: &'a TypeRefIr,
    pub trace: &'a TypeClosureTrace,
    pub guarded: bool,
}

pub trait TypeClosurePolicy {
    type Error;

    fn visit_type_ref(
        &mut self,
        _visit: TypeClosureVisit<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        Ok(TypeClosureControl::Continue)
    }

    fn enter_nominal(
        &mut self,
        _visit: TypeClosureVisit<'_>,
        _resolved: &ResolvedNominalType<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        Ok(TypeClosureControl::Continue)
    }

    fn unresolved_nominal(&mut self, _visit: TypeClosureVisit<'_>) -> Result<(), Self::Error> {
        Ok(())
    }

    fn nominal_cycle(
        &mut self,
        _visit: TypeClosureVisit<'_>,
        _resolved: &ResolvedNominalType<'_>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeClosureFailure<E> {
    pub trace: TypeClosureTrace,
    pub error: E,
}

pub struct TypeClosureWalker<'a, R, G> {
    resolver: &'a R,
    guard_policy: &'a G,
}

impl<'a, R, G> TypeClosureWalker<'a, R, G>
where
    R: NominalTypeResolver,
    G: TypeClosureGuardPolicy,
{
    pub fn new(resolver: &'a R, guard_policy: &'a G) -> Self {
        Self {
            resolver,
            guard_policy,
        }
    }

    pub fn walk<P>(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        policy: &mut P,
    ) -> Result<(), TypeClosureFailure<P::Error>>
    where
        P: TypeClosurePolicy,
    {
        self.walk_type_ref(
            module_path,
            ty,
            &TypeClosureTrace::empty(),
            false,
            &mut BTreeSet::new(),
            policy,
        )
    }

    pub fn walk_declaration<P>(
        &self,
        resolved: &ResolvedNominalType<'_>,
        policy: &mut P,
    ) -> Result<(), TypeClosureFailure<P::Error>>
    where
        P: TypeClosurePolicy,
    {
        let trace = TypeClosureTrace::empty().child(TypeClosureTraceSegment::Nominal {
            module_path: resolved.key.module_path.clone(),
            name: resolved.key.name.clone(),
        });
        let mut active = BTreeSet::from([resolved.key.clone()]);
        self.walk_descriptor(
            &resolved.key,
            &resolved.declaration.descriptor,
            &BTreeMap::new(),
            &trace,
            false,
            &mut active,
            policy,
        )
    }

    fn walk_type_ref<P>(
        &self,
        module_path: &str,
        ty: &TypeRefIr,
        trace: &TypeClosureTrace,
        guarded: bool,
        active: &mut BTreeSet<NominalTypeKey>,
        policy: &mut P,
    ) -> Result<(), TypeClosureFailure<P::Error>>
    where
        P: TypeClosurePolicy,
    {
        let visit = TypeClosureVisit {
            module_path,
            ty,
            trace,
            guarded,
        };
        if self.call(trace, policy.visit_type_ref(visit))? == TypeClosureControl::Prune {
            return Ok(());
        }

        if is_nominal_type_ref(ty) {
            if let TypeRefIr::AppliedNominal { arguments, .. } = ty {
                for (index, argument) in arguments.iter().enumerate() {
                    self.walk_child(
                        module_path,
                        ty,
                        argument,
                        trace,
                        TypeClosureTraceSegment::AppliedNominalArgument { index },
                        guarded,
                        active,
                        policy,
                    )?;
                }
            }
            return self.walk_nominal(visit, active, policy);
        }

        for child in type_ref_children(ty) {
            self.walk_child(
                module_path,
                ty,
                child.ty,
                trace,
                child.segment.into(),
                guarded,
                active,
                policy,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_child<P>(
        &self,
        module_path: &str,
        parent: &TypeRefIr,
        child: &TypeRefIr,
        trace: &TypeClosureTrace,
        segment: TypeClosureTraceSegment,
        guarded: bool,
        active: &mut BTreeSet<NominalTypeKey>,
        policy: &mut P,
    ) -> Result<(), TypeClosureFailure<P::Error>>
    where
        P: TypeClosurePolicy,
    {
        let child_guarded = self
            .guard_policy
            .child_is_guarded(parent, &segment, guarded);
        self.walk_type_ref(
            module_path,
            child,
            &trace.child(segment),
            child_guarded,
            active,
            policy,
        )
    }

    fn walk_nominal<P>(
        &self,
        visit: TypeClosureVisit<'_>,
        active: &mut BTreeSet<NominalTypeKey>,
        policy: &mut P,
    ) -> Result<(), TypeClosureFailure<P::Error>>
    where
        P: TypeClosurePolicy,
    {
        let Some(resolved) = self.resolver.resolve(visit.module_path, visit.ty) else {
            return self.call(visit.trace, policy.unresolved_nominal(visit));
        };
        let trace = visit.trace.child(TypeClosureTraceSegment::Nominal {
            module_path: resolved.key.module_path.clone(),
            name: resolved.key.name.clone(),
        });
        let resolved_visit = TypeClosureVisit {
            trace: &trace,
            ..visit
        };
        if active.contains(&resolved.key) {
            return self.call(&trace, policy.nominal_cycle(resolved_visit, &resolved));
        }
        if self.call(&trace, policy.enter_nominal(resolved_visit, &resolved))?
            == TypeClosureControl::Prune
        {
            return Ok(());
        }
        active.insert(resolved.key.clone());
        let substitutions = match visit.ty {
            TypeRefIr::AppliedNominal { arguments, .. } => resolved
                .declaration
                .type_params
                .iter()
                .cloned()
                .zip(arguments.iter().cloned())
                .collect(),
            _ => BTreeMap::new(),
        };
        let result = self.walk_descriptor(
            &resolved.key,
            &resolved.declaration.descriptor,
            &substitutions,
            &trace,
            visit.guarded,
            active,
            policy,
        );
        active.remove(&resolved.key);
        result
    }

    fn walk_descriptor<P>(
        &self,
        nominal: &NominalTypeKey,
        descriptor: &TypeDescriptorIr,
        substitutions: &BTreeMap<String, TypeRefIr>,
        trace: &TypeClosureTrace,
        guarded: bool,
        active: &mut BTreeSet<NominalTypeKey>,
        policy: &mut P,
    ) -> Result<(), TypeClosureFailure<P::Error>>
    where
        P: TypeClosurePolicy,
    {
        match descriptor {
            TypeDescriptorIr::Alias { target } => {
                let target = substitute_type_params_in_type_ref_ref(target, substitutions);
                self.walk_type_ref(
                    &nominal.module_path,
                    &target,
                    &trace.child(TypeClosureTraceSegment::AliasTarget),
                    guarded,
                    active,
                    policy,
                )
            }
            TypeDescriptorIr::Record { fields } => {
                for (name, field_ty) in fields {
                    let field_ty = substitute_type_params_in_type_ref_ref(field_ty, substitutions);
                    self.walk_type_ref(
                        &nominal.module_path,
                        &field_ty,
                        &trace.child(TypeClosureTraceSegment::DeclarationField {
                            name: name.clone(),
                        }),
                        guarded,
                        active,
                        policy,
                    )?;
                }
                Ok(())
            }
            TypeDescriptorIr::Representation { representation } => {
                let representation =
                    substitute_type_params_in_type_ref_ref(representation, substitutions);
                self.walk_type_ref(
                    &nominal.module_path,
                    &representation,
                    &trace.child(TypeClosureTraceSegment::RepresentationTarget),
                    guarded,
                    active,
                    policy,
                )
            }
            TypeDescriptorIr::Union { branches } => {
                for (index, branch) in branches.iter().enumerate() {
                    let branch_trace =
                        trace.child(TypeClosureTraceSegment::NamedUnionBranch { index });
                    match branch {
                        NamedUnionBranchIr::ConcreteNominal { nominal_type } => {
                            let nominal_type =
                                substitute_type_params_in_type_ref_ref(nominal_type, substitutions);
                            self.walk_type_ref(
                                &nominal.module_path,
                                &nominal_type,
                                &branch_trace
                                    .child(TypeClosureTraceSegment::NamedUnionConcreteType),
                                guarded,
                                active,
                                policy,
                            )?;
                        }
                        NamedUnionBranchIr::SyntheticDiscriminator { payload_type, .. } => {
                            let payload_type =
                                substitute_type_params_in_type_ref_ref(payload_type, substitutions);
                            self.walk_type_ref(
                                &nominal.module_path,
                                &payload_type,
                                &branch_trace
                                    .child(TypeClosureTraceSegment::NamedUnionSyntheticPayload),
                                guarded,
                                active,
                                policy,
                            )?;
                        }
                        NamedUnionBranchIr::Literal { .. } => {}
                    }
                }
                Ok(())
            }
            TypeDescriptorIr::Interface => Ok(()),
        }
    }

    fn call<T, E>(
        &self,
        trace: &TypeClosureTrace,
        result: Result<T, E>,
    ) -> Result<T, TypeClosureFailure<E>> {
        result.map_err(|error| TypeClosureFailure {
            trace: trace.clone(),
            error,
        })
    }
}

#[cfg(test)]
mod tests;
