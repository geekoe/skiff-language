use skiff_artifact_model::TypeRefIr;
use skiff_compiler_core::type_closure::{
    NativeDescriptorVisit, NoTypeClosureGuards, NominalTypeResolver, ResolvedNominalType,
    TypeClosureControl, TypeClosurePolicy, TypeClosureTraceSegment, TypeClosureVisit,
    TypeClosureWalker,
};
use skiff_compiler_core::type_ref::{TypeRefVisit, TypeRefVisitPath};

use crate::contract::{static_type_ref_boundary_policy, BoundaryTypePolicyDecision};
use crate::type_closure_diagnostics::{type_closure_trace_segments, type_closure_trace_suffix};

use super::{
    display_native_type_ref, recoverable_closure_error, recoverable_nominal_type_label,
    BoundaryKind, ProjectionError, RecoverableMetadataBuilder,
};

pub(super) fn validate_recoverable_type_closure(
    builder: &RecoverableMetadataBuilder,
    module_path: &str,
    ty: &TypeRefIr,
    boundary_kind: BoundaryKind,
    context: &str,
) -> Result<(), ProjectionError> {
    let resolver = RecoverableNominalResolver { builder };
    let guards = NoTypeClosureGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let mut policy = RecoverableClosurePolicy {
        builder,
        boundary_kind,
        context,
    };
    match walker.walk(module_path, ty, &mut policy) {
        Ok(()) => Ok(()),
        Err(failure) => match failure.error {
            RecoverableClosurePolicyError::Message(message) => Err(recoverable_closure_error(
                context,
                &type_closure_trace_segments(&failure.trace),
                message,
            )),
            RecoverableClosurePolicyError::Existing(error) => Err(error),
        },
    }
}

pub(super) fn recoverable_closure_contains_any_interface(
    builder: &RecoverableMetadataBuilder,
    module_path: &str,
    ty: &TypeRefIr,
) -> bool {
    let resolver = RecoverableNominalResolver { builder };
    let guards = NoTypeClosureGuards;
    let walker = TypeClosureWalker::new(&resolver, &guards);
    let mut policy = ContainsAnyInterfacePolicy {
        builder,
        found: false,
    };
    let result = walker.walk(module_path, ty, &mut policy);
    if result.is_err() {
        return true;
    }
    policy.found
}

struct RecoverableNominalResolver<'a> {
    builder: &'a RecoverableMetadataBuilder,
}

impl NominalTypeResolver for RecoverableNominalResolver<'_> {
    fn resolve<'a>(
        &'a self,
        current_module: &str,
        ty: &TypeRefIr,
    ) -> Option<ResolvedNominalType<'a>> {
        self.builder
            .type_decl_for_type_ref_with_module(current_module, ty)
            .map(|(module_path, declaration)| {
                ResolvedNominalType::new(module_path.to_string(), declaration)
            })
    }
}

enum RecoverableClosurePolicyError {
    Message(String),
    Existing(ProjectionError),
}

struct RecoverableClosurePolicy<'a> {
    builder: &'a RecoverableMetadataBuilder,
    boundary_kind: BoundaryKind,
    context: &'a str,
}

impl TypeClosurePolicy for RecoverableClosurePolicy<'_> {
    type Error = RecoverableClosurePolicyError;

    fn visit_type_ref(
        &mut self,
        visit: TypeClosureVisit<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        match static_type_ref_boundary_policy(
            self.boundary_kind,
            TypeRefVisit {
                ty: visit.ty,
                path: TypeRefVisitPath::empty(),
            },
        ) {
            BoundaryTypePolicyDecision::Accept => {}
            BoundaryTypePolicyDecision::Reject(message) => {
                return Err(RecoverableClosurePolicyError::Message(message));
            }
        }

        let TypeRefIr::Native { name, args } = visit.ty else {
            return Ok(TypeClosureControl::Continue);
        };
        match name.as_str() {
            "string" | "integer" | "number" | "bool" | "boolean" | "null" | "void" | "Date"
            | "Duration" | "Bytes" | "Json" | "JsonObject" => {
                if args.is_empty() {
                    Ok(TypeClosureControl::Continue)
                } else {
                    Err(RecoverableClosurePolicyError::Message(format!(
                        "plain native type `{name}` cannot have type arguments"
                    )))
                }
            }
            "Array" => {
                if args.len() == 1 {
                    Ok(TypeClosureControl::Continue)
                } else {
                    Err(RecoverableClosurePolicyError::Message(
                        "Array<T> must have exactly one type argument".to_string(),
                    ))
                }
            }
            "Map" => {
                let [key, _value] = args.as_slice() else {
                    return Err(RecoverableClosurePolicyError::Message(
                        "Map<K,V> must have exactly two type arguments".to_string(),
                    ));
                };
                let key_trace = visit.trace.child(TypeClosureTraceSegment::NativeArg {
                    name: name.clone(),
                    index: 0,
                });
                self.builder
                    .validate_recoverable_map_key_type(
                        visit.module_path,
                        key,
                        self.context,
                        &type_closure_trace_segments(&key_trace),
                    )
                    .map_err(RecoverableClosurePolicyError::Existing)?;
                Ok(TypeClosureControl::Continue)
            }
            _ => {
                let Some((plan_key, plan)) =
                    self.builder.native_adapter_plan_for_type_ref(visit.ty)
                else {
                    return Err(RecoverableClosurePolicyError::Message(format!(
                        "native type `{}` requires RecoverableNativeAdapterPlan",
                        display_native_type_ref(name, args)
                    )));
                };
                self.builder
                    .validate_expected_type_plan_closure(
                        &self.builder.metadata,
                        &plan.durable_state_type_plan,
                        self.boundary_kind,
                        &format!(
                            "{}: native adapter plan {plan_key} durable state type{}",
                            self.context,
                            type_closure_trace_suffix(visit.trace)
                        ),
                    )
                    .map_err(RecoverableClosurePolicyError::Existing)?;
                Ok(TypeClosureControl::Continue)
            }
        }
    }

    fn enter_nominal(
        &mut self,
        visit: TypeClosureVisit<'_>,
        resolved: &ResolvedNominalType<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        let owned_resolved = (
            resolved.key.module_path.clone(),
            resolved.declaration.clone(),
        );
        let identity_ref = self.builder.nominal_type_identity_ref(
            visit.module_path,
            visit.ty,
            Some(&owned_resolved),
        );
        let Some((plan_key, plan)) = self.builder.custom_restore_plan_for_identity(&identity_ref)
        else {
            return Ok(TypeClosureControl::Continue);
        };
        self.builder
            .validate_expected_type_plan_closure(
                &self.builder.metadata,
                &plan.durable_state_type_plan,
                self.boundary_kind,
                &format!(
                    "{}: custom restore plan {plan_key} durable state type{}",
                    self.context,
                    type_closure_trace_suffix(visit.trace)
                ),
            )
            .map_err(RecoverableClosurePolicyError::Existing)?;
        Ok(TypeClosureControl::Prune)
    }

    fn unresolved_nominal(&mut self, visit: TypeClosureVisit<'_>) -> Result<(), Self::Error> {
        Err(RecoverableClosurePolicyError::Message(format!(
            "{} cannot be resolved for recoverable closure validation",
            recoverable_nominal_type_label(visit.ty)
        )))
    }

    fn native_descriptor(&mut self, visit: NativeDescriptorVisit<'_>) -> Result<(), Self::Error> {
        let Some((plan_key, plan)) = self.builder.native_adapter_plan_for_symbol(visit.symbol)
        else {
            return Err(RecoverableClosurePolicyError::Message(format!(
                "native descriptor `{}` requires RecoverableNativeAdapterPlan",
                visit.symbol
            )));
        };
        self.builder
            .validate_expected_type_plan_closure(
                &self.builder.metadata,
                &plan.durable_state_type_plan,
                self.boundary_kind,
                &format!(
                    "{}: native adapter plan {plan_key} durable state type{}",
                    self.context,
                    type_closure_trace_suffix(visit.trace)
                ),
            )
            .map_err(RecoverableClosurePolicyError::Existing)
    }
}

struct ContainsAnyInterfacePolicy<'a> {
    builder: &'a RecoverableMetadataBuilder,
    found: bool,
}

impl TypeClosurePolicy for ContainsAnyInterfacePolicy<'_> {
    type Error = ();

    fn visit_type_ref(
        &mut self,
        visit: TypeClosureVisit<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        if matches!(visit.ty, TypeRefIr::AnyInterface { .. }) {
            self.found = true;
            return Ok(TypeClosureControl::Prune);
        }
        Ok(TypeClosureControl::Continue)
    }

    fn enter_nominal(
        &mut self,
        visit: TypeClosureVisit<'_>,
        resolved: &ResolvedNominalType<'_>,
    ) -> Result<TypeClosureControl, Self::Error> {
        let owned_resolved = (
            resolved.key.module_path.clone(),
            resolved.declaration.clone(),
        );
        let identity_ref = self.builder.nominal_type_identity_ref(
            visit.module_path,
            visit.ty,
            Some(&owned_resolved),
        );
        if let Some((_, plan)) = self.builder.custom_restore_plan_for_identity(&identity_ref) {
            self.found |= self
                .builder
                .expected_type_plan_contains_any_interface(&plan.durable_state_type_plan);
            return Ok(TypeClosureControl::Prune);
        }
        Ok(TypeClosureControl::Continue)
    }

    fn unresolved_nominal(&mut self, _visit: TypeClosureVisit<'_>) -> Result<(), Self::Error> {
        self.found = true;
        Ok(())
    }
}
