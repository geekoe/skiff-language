use std::collections::BTreeMap;

use super::{
    object_literal_key_text, span_label, type_ir_is_null, type_ref_debug_text, ExpressionKey,
    ExpressionSourceMap, ResolvedTypeRef, TypeResolutionContext, TypeResolutionModel,
};
use skiff_artifact_model::{FunctionTypeParamIr, PackageRefIr, TypeDescriptorIr, TypeRefIr};

use crate::{
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
}

pub(super) struct ExpressionAssignability<'a, 'ctx> {
    diagnostic_path: &'a str,
    expression_sources: &'a ExpressionSourceMap,
    type_resolution: &'a TypeResolutionModel,
    type_context: &'a TypeResolutionContext<'ctx>,
}

impl<'a, 'ctx> ExpressionAssignability<'a, 'ctx> {
    pub(super) fn new(
        diagnostic_path: &'a str,
        expression_sources: &'a ExpressionSourceMap,
        type_resolution: &'a TypeResolutionModel,
        type_context: &'a TypeResolutionContext<'ctx>,
    ) -> Self {
        Self {
            diagnostic_path,
            expression_sources,
            type_resolution,
            type_context,
        }
    }

    pub(super) fn value_assignable_to_expected(
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
            TypeRefIr::Native { name, args } => builtin_object_literal_targets(name, args)
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
                let expected = ResolvedTypeRef {
                    ir: expected_ty.clone(),
                    source_text: type_ref_debug_text(expected_ty),
                };
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
        let actual = ResolvedTypeRef {
            ir: actual_ty.clone(),
            source_text: type_ref_debug_text(actual_ty),
        };
        self.type_resolution
            .assignable_in_context(&actual, expected, self.type_context)
            || matches!(actual_ty, TypeRefIr::Record { .. })
                && self.object_record_ir_assignable_to_resolved_expected(actual_ty, expected)
    }

    fn object_record_fields_assignable_to_resolved_expected(
        &self,
        actual_fields: &BTreeMap<String, TypeRefIr>,
        expected: &ResolvedTypeRef,
    ) -> bool {
        self.type_resolution
            .resolve_constructor_target_text(&expected.source_text, self.type_context)
            .ok()
            .is_some_and(|target| {
                self.object_fields_assignable_to_target(actual_fields, &target.fields)
            })
            || self
                .type_resolution
                .type_shape_ir(expected, self.type_context)
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
                        ty: fields.get(&name).map(|ty| ResolvedTypeRef {
                            ir: ty.clone(),
                            source_text: type_ref_debug_text(ty),
                        }),
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
                    label: target.ty.source_text,
                    fields: target.fields,
                });
            }
        }
        if candidates.is_empty() {
            if let Ok(target) = self
                .type_resolution
                .resolve_constructor_target_text(&expected.source_text, self.type_context)
            {
                candidates.push(ObjectLiteralTargetCandidate {
                    label: target.ty.source_text,
                    fields: target.fields,
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
                ));
            }
        }
        if candidates.is_empty() {
            candidates.extend(object_literal_target_candidates_from_ir(
                &expected.source_text,
                &expected.ir,
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
    let decl = lower_prelude_type_decl(decl);
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
        TypeDescriptorIr::Alias { target } => {
            object_literal_targets_from_type_ref(target, substitutions)
        }
        TypeDescriptorIr::Union { variants } => variants
            .iter()
            .map(|variant| object_literal_targets_from_type_ref(variant, substitutions))
            .collect::<Option<Vec<_>>>()
            .map(|items| items.into_iter().flatten().collect()),
        TypeDescriptorIr::Native { .. } => None,
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
        TypeRefIr::Native { name, args } => TypeRefIr::Native {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_std_type_params_in_ir(arg, substitutions))
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
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::PackageSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. } => ty.clone(),
    }
}

pub(super) fn record_type_fields<const N: usize>(
    fields: [(&str, TypeRefIr); N],
) -> BTreeMap<String, TypeRefIr> {
    fields
        .into_iter()
        .map(|(name, ty)| (name.to_string(), ty))
        .collect()
}

fn object_literal_target_candidates_from_ir(
    label: &str,
    ty: &TypeRefIr,
) -> Vec<ObjectLiteralTargetCandidate> {
    match ty {
        TypeRefIr::Record { fields } => vec![ObjectLiteralTargetCandidate {
            label: label.to_string(),
            fields: resolved_fields_from_ir(fields),
        }],
        TypeRefIr::Union { items } => items
            .iter()
            .flat_map(|item| {
                object_literal_target_candidates_from_ir(&type_ref_debug_text(item), item)
            })
            .collect(),
        TypeRefIr::Native { name, args } => builtin_object_literal_targets(name, args)
            .into_iter()
            .map(|fields| ObjectLiteralTargetCandidate {
                label: label.to_string(),
                fields: resolved_fields_from_ir(&fields),
            })
            .collect(),
        TypeRefIr::PackageSymbol { symbol }
            if matches!(
                &symbol.package,
                PackageRefIr::PackageId { package_id } if package_id == SKIFF_STD_PUBLICATION_ID
            ) =>
        {
            standard_library_object_literal_targets(&symbol.symbol_path, &[])
                .unwrap_or_default()
                .into_iter()
                .map(|fields| ObjectLiteralTargetCandidate {
                    label: label.to_string(),
                    fields: resolved_fields_from_ir(&fields),
                })
                .collect()
        }
        _ => Vec::new(),
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
                ResolvedTypeRef {
                    ir: ty.clone(),
                    source_text: type_ref_debug_text(ty),
                },
            )
        })
        .collect()
}

fn type_ir_is_nullable(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Nullable { .. })
        || matches!(ty, TypeRefIr::Union { items } if items.iter().any(type_ir_is_null))
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
