use std::collections::BTreeMap;

use super::{
    object_literal_key_text,
    object_materialization::{ObjectMaterializationKind, ObjectMaterializationPlan},
    resolved_type_from_ir, span_label, transparent_value_target, ExpressionKey,
    ExpressionSourceMap, ResolvedTypeRef, TypeResolutionContext, TypeResolutionModel,
};
use skiff_artifact_model::{
    FunctionTypeParamIr, PackageRefIr, PackageTypeRef, TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_core::type_ref::{debug_text, is_null_type};

use crate::{
    dependency_analysis::SourceDependencyAnalysisInput,
    expression_type_model::contract_call_typing::{
        contract_source_assignability, local_ir_json_compatible, package_type_target_assignable,
    },
    runtime_type_projection::lower_prelude_type_decl,
    shared::ast::{Expr, TypeRef},
    shared::error::SourceSpan,
    shared::id::SKIFF_STD_PUBLICATION_ID,
    shared::prelude_registry::prelude_registry,
};

#[derive(Clone, Debug)]
struct ObjectLiteralActualField {
    name: String,
    ty: Option<ResolvedTypeRef>,
    name_span: SourceSpan,
    value_span: SourceSpan,
}

#[derive(Clone, Debug)]
struct ObjectLiteralTargetCandidate {
    label: String,
    fields: BTreeMap<String, ResolvedTypeRef>,
    kind: ObjectMaterializationKind,
}

pub(super) struct ExpressionAssignability<'a, 'ctx> {
    diagnostic_path: &'a str,
    expression_sources: &'a ExpressionSourceMap,
    type_resolution: &'a TypeResolutionModel,
    type_context: &'a TypeResolutionContext<'ctx>,
    dependency_analysis: Option<&'a SourceDependencyAnalysisInput>,
    package_json_context: bool,
}

impl<'a, 'ctx> ExpressionAssignability<'a, 'ctx> {
    pub(super) fn new(
        diagnostic_path: &'a str,
        expression_sources: &'a ExpressionSourceMap,
        type_resolution: &'a TypeResolutionModel,
        type_context: &'a TypeResolutionContext<'ctx>,
        dependency_analysis: Option<&'a SourceDependencyAnalysisInput>,
    ) -> Self {
        Self {
            diagnostic_path,
            expression_sources,
            type_resolution,
            type_context,
            dependency_analysis,
            package_json_context: false,
        }
    }

    pub(super) fn with_package_json_context(mut self) -> Self {
        self.package_json_context = true;
        self
    }

    pub(super) fn value_assignable_to_expected(
        &self,
        annotation: Option<&TypeRef>,
        value: &Expr,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        actual_projected: Option<&PackageTypeRef>,
    ) -> Result<bool, String> {
        let value = transparent_value_target(value);
        if self.package_json_context {
            if let (Some(actual), Some(dependencies)) = (actual_projected, self.dependency_analysis)
            {
                if let TypeRefIr::Builtin { name, args } = &expected.ir {
                    if args.is_empty()
                        && matches!(name.as_str(), "Json" | "JsonObject")
                        && package_type_target_assignable(
                            actual,
                            &PackageTypeRef::Container {
                                name: name.clone(),
                                arguments: Vec::new(),
                            },
                            dependencies,
                        )
                    {
                        return Ok(true);
                    }
                }
            }
        }
        if self.package_json_context
            && self.dependency_analysis.is_some_and(|dependencies| {
                matches!(&expected.ir, TypeRefIr::Builtin { name, args }
                    if args.is_empty() && matches!(name.as_str(), "Json" | "JsonObject"))
                    && {
                        let object_only = matches!(&expected.ir, TypeRefIr::Builtin { name, .. } if name == "JsonObject");
                        local_ir_json_compatible(&actual.ir, dependencies, object_only)
                            || matches!(&actual.ir, TypeRefIr::PackageSchema { .. })
                                && self
                                    .type_resolution
                                    .type_shape_ir(actual, self.type_context)
                                    .is_some_and(|shape| {
                                        local_ir_json_compatible(
                                            &shape,
                                            dependencies,
                                            object_only,
                                        )
                                    })
                    }
            })
        {
            return Ok(true);
        }
        if let Some(assignable) = contract_source_assignability(
            actual,
            actual_projected,
            expected,
            self.type_resolution,
            self.dependency_analysis,
            self.type_context,
        )? {
            return Ok(assignable);
        }
        Ok(self.value_assignable_without_contract_projection(annotation, value, actual, expected))
    }

    pub(super) fn value_assignable_without_contract_projection(
        &self,
        annotation: Option<&TypeRef>,
        value: &Expr,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
    ) -> bool {
        self.value_assignable_to_resolved_expected(value, actual, expected)
            || annotation.is_some_and(|annotation| {
                self.target_typed_object_literal_assignable(annotation, value, actual, expected)
            })
    }

    fn value_assignable_to_resolved_expected(
        &self,
        value: &Expr,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
    ) -> bool {
        self.type_resolution
            .assignable_in_context(actual, expected, self.type_context)
            || self.object_literal_assignable_to_resolved_expected(value, actual, expected)
    }

    fn target_typed_object_literal_assignable(
        &self,
        annotation: &TypeRef,
        value: &Expr,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
    ) -> bool {
        let Some(fields) = object_literal_record_fields(value, actual) else {
            return false;
        };

        self.type_resolution
            .resolve_constructor_target_text(&annotation.name, self.type_context)
            .ok()
            .is_some_and(|target| self.object_fields_assignable_to_target(fields, &target.fields))
            || self.object_fields_assignable_to_expected(fields, &expected.ir)
    }

    fn object_literal_assignable_to_resolved_expected(
        &self,
        value: &Expr,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
    ) -> bool {
        object_literal_record_fields(value, actual).is_some_and(|fields| {
            self.object_record_fields_assignable_to_resolved_expected(fields, expected)
        })
    }

    fn object_fields_assignable_to_target(
        &self,
        actual_fields: &BTreeMap<String, TypeRefIr>,
        target_fields: &BTreeMap<String, ResolvedTypeRef>,
    ) -> bool {
        actual_fields.iter().all(|(name, actual_ty)| {
            target_fields.get(name).is_some_and(|expected_ty| {
                self.type_ir_assignable_to_resolved_expected(actual_ty, expected_ty)
            })
        }) && target_fields.iter().all(|(name, expected_ty)| {
            actual_fields.contains_key(name) || self.type_resolution.is_nullable(expected_ty)
        })
    }

    fn object_fields_assignable_to_expected(
        &self,
        actual_fields: &BTreeMap<String, TypeRefIr>,
        expected: &TypeRefIr,
    ) -> bool {
        match expected {
            TypeRefIr::Record { fields } => {
                self.object_fields_assignable_to_ir_fields(actual_fields, fields)
            }
            TypeRefIr::Union { items } => items
                .iter()
                .any(|item| self.object_fields_assignable_to_expected(actual_fields, item)),
            TypeRefIr::Builtin { name, args } => builtin_object_literal_targets(name, args)
                .iter()
                .any(|fields| self.object_fields_assignable_to_ir_fields(actual_fields, fields)),
            TypeRefIr::PackageSymbol { symbol }
                if matches!(
                    &symbol.package,
                    PackageRefIr::PackageId { package_id }
                        if package_id == SKIFF_STD_PUBLICATION_ID
                ) =>
            {
                standard_library_object_literal_targets(&symbol.symbol_path, &[])
                    .unwrap_or_default()
                    .iter()
                    .any(|fields| self.object_fields_assignable_to_ir_fields(actual_fields, fields))
            }
            _ => false,
        }
    }

    fn object_fields_assignable_to_ir_fields(
        &self,
        actual_fields: &BTreeMap<String, TypeRefIr>,
        target_fields: &BTreeMap<String, TypeRefIr>,
    ) -> bool {
        actual_fields.iter().all(|(name, actual_ty)| {
            target_fields.get(name).is_some_and(|expected_ty| {
                let expected = ResolvedTypeRef::new(expected_ty.clone());
                self.type_ir_assignable_to_resolved_expected(actual_ty, &expected)
            })
        }) && target_fields.iter().all(|(name, expected_ty)| {
            actual_fields.contains_key(name) || type_ir_is_nullable(expected_ty)
        })
    }

    fn type_ir_assignable_to_resolved_expected(
        &self,
        actual_ty: &TypeRefIr,
        expected: &ResolvedTypeRef,
    ) -> bool {
        let actual = ResolvedTypeRef::new(actual_ty.clone());
        self.type_resolution
            .assignable_in_context(&actual, expected, self.type_context)
            || self.package_json_context
                && self.dependency_analysis.is_some_and(|dependencies| {
                    matches!(&expected.ir, TypeRefIr::Builtin { name, args }
                        if args.is_empty() && matches!(name.as_str(), "Json" | "JsonObject"))
                        && local_ir_json_compatible(
                            actual_ty,
                            dependencies,
                            matches!(&expected.ir, TypeRefIr::Builtin { name, .. } if name == "JsonObject"),
                        )
                })
            || matches!(actual_ty, TypeRefIr::Record { .. })
                && self.object_record_ir_assignable_to_resolved_expected(actual_ty, expected)
    }

    fn object_record_fields_assignable_to_resolved_expected(
        &self,
        actual_fields: &BTreeMap<String, TypeRefIr>,
        expected: &ResolvedTypeRef,
    ) -> bool {
        let expected = non_nullable_object_target(expected);
        self.type_resolution
            .resolve_constructor_target_resolved(&expected, self.type_context)
            .ok()
            .is_some_and(|target| {
                self.object_fields_assignable_to_target(actual_fields, &target.fields)
            })
            || self
                .type_resolution
                .type_shape_ir(&expected, self.type_context)
                .is_some_and(|shape| {
                    self.object_fields_assignable_to_expected(actual_fields, &shape)
                })
            || self.object_fields_assignable_to_expected(actual_fields, &expected.ir)
    }

    fn object_record_ir_assignable_to_resolved_expected(
        &self,
        actual_ty: &TypeRefIr,
        expected: &ResolvedTypeRef,
    ) -> bool {
        let TypeRefIr::Record { fields } = actual_ty else {
            return false;
        };
        self.object_record_fields_assignable_to_resolved_expected(fields, expected)
    }

    pub(super) fn object_literal_assignability_diagnostics(
        &self,
        annotation: Option<&TypeRef>,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        context: &str,
    ) -> Option<Vec<String>> {
        let actual_fields = self.object_literal_actual_fields(value, value_key, actual)?;
        let candidates = self.object_literal_target_candidates(annotation, expected);
        candidates
            .into_iter()
            .map(|candidate| {
                self.object_literal_candidate_diagnostics(
                    context,
                    &candidate,
                    &actual_fields,
                    self.expression_span(value_key),
                )
            })
            .min_by_key(|diagnostics| diagnostics.len())
    }

    pub(super) fn object_literal_materialization_plan(
        &self,
        annotation: Option<&TypeRef>,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
        expected: &ResolvedTypeRef,
        context: &str,
    ) -> Result<ObjectMaterializationPlan, Vec<String>> {
        let Some(actual_fields) = self.object_literal_actual_fields(value, value_key, actual)
        else {
            return Err(vec![format!(
                "{}: {context} object materialization requires an object literal at {}",
                self.diagnostic_path,
                span_label(self.expression_span(value_key))
            )]);
        };
        let target = non_nullable_object_target(expected);
        if let Some(value_target) = map_object_value_target(&target) {
            let fields = actual_fields
                .iter()
                .map(|field| (field.name.clone(), value_target.clone()))
                .collect::<BTreeMap<_, _>>();
            let candidate = ObjectLiteralTargetCandidate {
                label: target.source_text.clone(),
                fields: fields.clone(),
                kind: ObjectMaterializationKind::Map,
            };
            let diagnostics =
                self.object_literal_map_candidate_diagnostics(context, &candidate, &actual_fields);
            if diagnostics.is_empty() {
                return Ok(ObjectMaterializationPlan {
                    resolved_target: expected.clone(),
                    kind: candidate.kind,
                    fields,
                });
            }
            return Err(diagnostics);
        }

        let candidates = self.object_literal_target_candidates(annotation, &target);
        if candidates.is_empty() {
            return Err(vec![format!(
                "{}: {context} object literal target {} is not a record, discriminated union, Map<string, T>, JsonObject, or Json at {}",
                self.diagnostic_path,
                expected.source_text,
                span_label(self.expression_span(value_key))
            )]);
        }
        let evaluated = candidates
            .into_iter()
            .map(|candidate| {
                let diagnostics = self.object_literal_candidate_diagnostics(
                    context,
                    &candidate,
                    &actual_fields,
                    self.expression_span(value_key),
                );
                (candidate, diagnostics)
            })
            .collect::<Vec<_>>();
        let matching = evaluated
            .iter()
            .filter(|(_, diagnostics)| diagnostics.is_empty())
            .collect::<Vec<_>>();
        match matching.as_slice() {
            [(candidate, _)] => Ok(ObjectMaterializationPlan {
                resolved_target: expected.clone(),
                kind: candidate.kind.clone(),
                fields: candidate.fields.clone(),
            }),
            [] => Err(evaluated
                .into_iter()
                .min_by_key(|(_, diagnostics)| diagnostics.len())
                .map(|(_, diagnostics)| diagnostics)
                .unwrap_or_default()),
            many => Err(vec![format!(
                "{}: {context} ambiguous object literal branch for {} at {}; matching branches: {}",
                self.diagnostic_path,
                expected.source_text,
                span_label(self.expression_span(value_key)),
                many.iter()
                    .map(|(candidate, _)| candidate.label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )]),
        }
    }

    fn object_literal_map_candidate_diagnostics(
        &self,
        context: &str,
        target: &ObjectLiteralTargetCandidate,
        actual_fields: &[ObjectLiteralActualField],
    ) -> Vec<String> {
        let mut diagnostics = Vec::new();
        let mut provided = BTreeMap::<String, &ObjectLiteralActualField>::new();
        for field in actual_fields {
            if provided.insert(field.name.clone(), field).is_some() {
                diagnostics.push(format!(
                    "{}: {context} duplicate object literal field `{}` at {}",
                    self.diagnostic_path,
                    field.name,
                    span_label(field.name_span)
                ));
                continue;
            }
            let expected = target
                .fields
                .get(&field.name)
                .expect("map materialization candidate must cover every provided field");
            if let Some(actual) = &field.ty {
                if !self.type_ir_assignable_to_resolved_expected(&actual.ir, expected) {
                    diagnostics.push(format!(
                        "{}: {context} object literal value for key `{}` type mismatch at {}: expected {}, found {}",
                        self.diagnostic_path,
                        field.name,
                        span_label(field.value_span),
                        expected.source_text,
                        actual.source_text
                    ));
                }
            }
        }
        diagnostics
    }

    fn object_literal_actual_fields(
        &self,
        value: &Expr,
        value_key: &ExpressionKey,
        actual: &ResolvedTypeRef,
    ) -> Option<Vec<ObjectLiteralActualField>> {
        let Expr::ObjectLiteral { entries } = value else {
            return None;
        };
        let TypeRefIr::Record { fields } = &actual.ir else {
            return None;
        };
        let source_fact = self.expression_sources.fact(value_key);
        Some(
            entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    let name = object_literal_key_text(&entry.key)?;
                    let field_spans = source_fact.and_then(|fact| fact.record_fields.get(index));
                    Some(ObjectLiteralActualField {
                        ty: fields
                            .get(&name)
                            .map(|ty| ResolvedTypeRef::new(ty.clone())),
                        name,
                        name_span: field_spans
                            .map(|field| field.name_span)
                            .or(entry.key_span)
                            .unwrap_or_else(SourceSpan::synthetic),
                        value_span: field_spans
                            .map(|field| field.value_span)
                            .unwrap_or_else(SourceSpan::synthetic),
                    })
                })
                .collect(),
        )
    }

    fn object_literal_target_candidates(
        &self,
        annotation: Option<&TypeRef>,
        expected: &ResolvedTypeRef,
    ) -> Vec<ObjectLiteralTargetCandidate> {
        let mut candidates = Vec::new();
        if let Some(annotation) = annotation {
            if let Ok(target) = self
                .type_resolution
                .resolve_constructor_target_text(&annotation.name, self.type_context)
            {
                candidates.push(ObjectLiteralTargetCandidate {
                    label: target.ty.source_text.clone(),
                    fields: target.fields,
                    kind: ObjectMaterializationKind::Record {
                        construct_target: target.ty,
                    },
                });
            }
        }
        if candidates.is_empty() {
            if let Ok(target) = self
                .type_resolution
                .resolve_constructor_target_resolved(expected, self.type_context)
            {
                candidates.push(ObjectLiteralTargetCandidate {
                    label: target.ty.source_text.clone(),
                    fields: target.fields,
                    kind: ObjectMaterializationKind::Record {
                        construct_target: target.ty,
                    },
                });
            }
        }
        if candidates.is_empty() {
            if let Some(shape) = self
                .type_resolution
                .type_shape_ir(expected, self.type_context)
            {
                candidates.extend(object_literal_target_candidates_from_ir(
                    &expected.source_text,
                    &shape,
                    expected,
                    false,
                ));
            }
        }
        if candidates.is_empty() {
            candidates.extend(object_literal_target_candidates_from_ir(
                &expected.source_text,
                &expected.ir,
                expected,
                false,
            ));
        }
        candidates
    }

    fn object_literal_candidate_diagnostics(
        &self,
        context: &str,
        target: &ObjectLiteralTargetCandidate,
        actual_fields: &[ObjectLiteralActualField],
        object_span: SourceSpan,
    ) -> Vec<String> {
        let mut diagnostics = Vec::new();
        let mut provided = BTreeMap::<String, &ObjectLiteralActualField>::new();
        for field in actual_fields {
            if provided.insert(field.name.clone(), field).is_some() {
                diagnostics.push(format!(
                    "{}: {context} duplicate object literal field `{}` at {}",
                    self.diagnostic_path,
                    field.name,
                    span_label(field.name_span)
                ));
            }
        }
        for field in actual_fields {
            let Some(expected) = target.fields.get(&field.name) else {
                diagnostics.push(format!(
                    "{}: {context} unknown object literal field `{}` for {} at {}",
                    self.diagnostic_path,
                    field.name,
                    target.label,
                    span_label(field.name_span)
                ));
                continue;
            };
            if let Some(actual) = &field.ty {
                if !self.type_ir_assignable_to_resolved_expected(&actual.ir, expected) {
                    diagnostics.push(format!(
                        "{}: {context} object literal field `{}` type mismatch at {}: expected {}, found {}",
                        self.diagnostic_path,
                        field.name,
                        span_label(field.value_span),
                        expected.source_text,
                        actual.source_text
                    ));
                }
            }
        }
        for (name, expected) in &target.fields {
            if !provided.contains_key(name) && !self.type_resolution.is_nullable(expected) {
                diagnostics.push(format!(
                    "{}: {context} missing required object literal field `{name}` for {} at {}",
                    self.diagnostic_path,
                    target.label,
                    span_label(object_span)
                ));
            }
        }
        diagnostics
    }

    fn expression_span(&self, key: &ExpressionKey) -> SourceSpan {
        self.expression_sources
            .fact(key)
            .map(|fact| fact.span)
            .unwrap_or_else(SourceSpan::synthetic)
    }
}

fn builtin_object_literal_targets(
    name: &str,
    args: &[TypeRefIr],
) -> Vec<BTreeMap<String, TypeRefIr>> {
    standard_library_object_literal_targets(name, args).unwrap_or_default()
}

fn standard_library_object_literal_targets(
    name: &str,
    args: &[TypeRefIr],
) -> Option<Vec<BTreeMap<String, TypeRefIr>>> {
    let registry = prelude_registry();
    let symbol = registry.known_type_symbol(name)?;
    let decl = registry.type_decl(&symbol)?;
    let decl = lower_prelude_type_decl(decl).ok()?;
    if decl.type_params.len() != args.len() {
        return None;
    }
    let substitutions = decl
        .type_params
        .iter()
        .cloned()
        .zip(args.iter().cloned())
        .collect::<BTreeMap<_, _>>();
    object_literal_targets_from_descriptor(&decl.descriptor, &substitutions)
}

fn object_literal_targets_from_descriptor(
    descriptor: &TypeDescriptorIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> Option<Vec<BTreeMap<String, TypeRefIr>>> {
    match descriptor {
        TypeDescriptorIr::Record { fields } => Some(vec![substitute_std_type_params_in_fields(
            fields,
            substitutions,
        )]),
        TypeDescriptorIr::Alias { target }
        | TypeDescriptorIr::Representation {
            representation: target,
        } => object_literal_targets_from_type_ref(target, substitutions),
        TypeDescriptorIr::Union { branches } => {
            let targets = branches
                .iter()
                .filter_map(|branch| match branch {
                    skiff_artifact_model::NamedUnionBranchIr::SyntheticDiscriminator {
                        payload_type,
                        ..
                    } => object_literal_targets_from_type_ref(payload_type, substitutions),
                    skiff_artifact_model::NamedUnionBranchIr::ConcreteNominal { .. }
                    | skiff_artifact_model::NamedUnionBranchIr::Literal { .. } => None,
                })
                .flatten()
                .collect::<Vec<_>>();
            (!targets.is_empty()).then_some(targets)
        }
        TypeDescriptorIr::Interface => None,
    }
}

fn object_literal_targets_from_type_ref(
    ty: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> Option<Vec<BTreeMap<String, TypeRefIr>>> {
    let ty = substitute_std_type_params_in_ir(ty, substitutions);
    match ty {
        TypeRefIr::Record { fields } => Some(vec![fields]),
        TypeRefIr::Union { items } => items
            .iter()
            .map(|item| object_literal_targets_from_type_ref(item, substitutions))
            .collect::<Option<Vec<_>>>()
            .map(|items| items.into_iter().flatten().collect()),
        _ => None,
    }
}

fn substitute_std_type_params_in_fields(
    fields: &BTreeMap<String, TypeRefIr>,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> BTreeMap<String, TypeRefIr> {
    fields
        .iter()
        .map(|(name, ty)| {
            (
                name.clone(),
                substitute_std_type_params_in_ir(ty, substitutions),
            )
        })
        .collect()
}

fn substitute_std_type_params_in_ir(
    ty: &TypeRefIr,
    substitutions: &BTreeMap<String, TypeRefIr>,
) -> TypeRefIr {
    match ty {
        TypeRefIr::TypeParam { name } => substitutions
            .get(name)
            .cloned()
            .unwrap_or_else(|| ty.clone()),
        TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_std_type_params_in_ir(arg, substitutions))
                .collect(),
        },
        TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
            base: base.clone(),
            arguments: arguments
                .iter()
                .map(|argument| substitute_std_type_params_in_ir(argument, substitutions))
                .collect(),
        },
        TypeRefIr::Record { fields } => TypeRefIr::Record {
            fields: substitute_std_type_params_in_fields(fields, substitutions),
        },
        TypeRefIr::Union { items } => TypeRefIr::Union {
            items: items
                .iter()
                .map(|item| substitute_std_type_params_in_ir(item, substitutions))
                .collect(),
        },
        TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
            inner: Box::new(substitute_std_type_params_in_ir(inner, substitutions)),
        },
        TypeRefIr::AnyInterface { interface } => TypeRefIr::AnyInterface {
            interface: skiff_artifact_model::InterfaceInstantiationRef {
                interface_abi_id: interface.interface_abi_id.clone(),
                canonical_type_args: interface
                    .canonical_type_args
                    .iter()
                    .map(|arg| substitute_std_type_params_in_ir(arg, substitutions))
                    .collect(),
            },
        },
        TypeRefIr::Function {
            params,
            return_type,
        } => TypeRefIr::Function {
            params: params
                .iter()
                .map(|param| FunctionTypeParamIr {
                    name: param.name.clone(),
                    ty: substitute_std_type_params_in_ir(&param.ty, substitutions),
                })
                .collect(),
            return_type: Box::new(substitute_std_type_params_in_ir(return_type, substitutions)),
        },
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::PackageSchema { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. } => ty.clone(),
    }
}

fn object_literal_target_candidates_from_ir(
    label: &str,
    ty: &TypeRefIr,
    construct_target: &ResolvedTypeRef,
    union_branch: bool,
) -> Vec<ObjectLiteralTargetCandidate> {
    match ty {
        TypeRefIr::Record { fields } => vec![ObjectLiteralTargetCandidate {
            label: label.to_string(),
            fields: resolved_fields_from_ir(fields),
            kind: if union_branch {
                ObjectMaterializationKind::DiscriminatedUnionBranch {
                    branch: resolved_type_from_ir(ty),
                }
            } else {
                ObjectMaterializationKind::Record {
                    construct_target: construct_target.clone(),
                }
            },
        }],
        TypeRefIr::Union { items } => items
            .iter()
            .flat_map(|item| {
                object_literal_target_candidates_from_ir(
                    &debug_text(item),
                    item,
                    construct_target,
                    true,
                )
            })
            .collect(),
        TypeRefIr::Builtin { name, args } => {
            let targets = builtin_object_literal_targets(name, args);
            let is_union = union_branch || targets.len() > 1;
            targets
                .into_iter()
                .map(|fields| {
                    let branch = ResolvedTypeRef::new(TypeRefIr::Record {
                        fields: fields.clone(),
                    });
                    ObjectLiteralTargetCandidate {
                        label: label.to_string(),
                        fields: resolved_fields_from_ir(&fields),
                        kind: if is_union {
                            ObjectMaterializationKind::DiscriminatedUnionBranch { branch }
                        } else {
                            ObjectMaterializationKind::Record {
                                construct_target: construct_target.clone(),
                            }
                        },
                    }
                })
                .collect()
        }
        TypeRefIr::PackageSymbol { symbol }
            if matches!(
                &symbol.package,
                PackageRefIr::PackageId { package_id } if package_id == SKIFF_STD_PUBLICATION_ID
            ) =>
        {
            let targets = standard_library_object_literal_targets(&symbol.symbol_path, &[])
                .unwrap_or_default();
            let is_union = union_branch || targets.len() > 1;
            targets
                .into_iter()
                .map(|fields| {
                    let branch = resolved_type_from_ir(&TypeRefIr::Record {
                        fields: fields.clone(),
                    });
                    ObjectLiteralTargetCandidate {
                        label: label.to_string(),
                        fields: resolved_fields_from_ir(&fields),
                        kind: if is_union {
                            ObjectMaterializationKind::DiscriminatedUnionBranch { branch }
                        } else {
                            ObjectMaterializationKind::Record {
                                construct_target: construct_target.clone(),
                            }
                        },
                    }
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn non_nullable_object_target(target: &ResolvedTypeRef) -> ResolvedTypeRef {
    match &target.ir {
        TypeRefIr::Nullable { inner } => resolved_type_from_ir(inner),
        TypeRefIr::Union { items } => {
            let non_null = items
                .iter()
                .filter(|item| !is_null_type(item))
                .collect::<Vec<_>>();
            match non_null.as_slice() {
                [only] => resolved_type_from_ir(only),
                _ => target.clone(),
            }
        }
        _ => target.clone(),
    }
}

fn map_object_value_target(target: &ResolvedTypeRef) -> Option<ResolvedTypeRef> {
    match &target.ir {
        TypeRefIr::Builtin { name, args }
            if name == "Map"
                && matches!(
                    args.as_slice(),
                    [TypeRefIr::Builtin { name, args: key_args }, _]
                        if name == "string" && key_args.is_empty()
                ) =>
        {
            args.get(1).map(resolved_type_from_ir)
        }
        TypeRefIr::Builtin { name, args }
            if args.is_empty() && matches!(name.as_str(), "Json" | "JsonObject") =>
        {
            Some(ResolvedTypeRef::with_text(
                TypeRefIr::Builtin {
                    name: "Json".to_string(),
                    args: Vec::new(),
                },
                "Json".to_string(),
            ))
        }
        _ => None,
    }
}

fn resolved_fields_from_ir(
    fields: &BTreeMap<String, TypeRefIr>,
) -> BTreeMap<String, ResolvedTypeRef> {
    fields
        .iter()
        .map(|(name, ty)| {
            (
                name.clone(),
                ResolvedTypeRef::new(ty.clone()),
            )
        })
        .collect()
}

fn type_ir_is_nullable(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Nullable { .. })
        || matches!(ty, TypeRefIr::Union { items } if items.iter().any(is_null_type))
}

fn object_literal_record_fields<'a>(
    value: &Expr,
    actual: &'a ResolvedTypeRef,
) -> Option<&'a BTreeMap<String, TypeRefIr>> {
    let Expr::ObjectLiteral { .. } = value else {
        return None;
    };
    let TypeRefIr::Record { fields } = &actual.ir else {
        return None;
    };
    Some(fields)
}
