use serde::{Deserialize, Serialize};

use crate::{
    bytecode::HostEffectSignature, CallableMayEffects, NativeSignatureDef, NativeSignatureTypeExpr,
    NominalTypeRefBaseIr, PackageRefIr, ParamModeIr, PositionalTypeEnvironment, TypeRefIr,
    ValueLifecycleFactResolver, ValueLifecyclePolicyBudget, ValueLifecyclePolicyError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CallableRegistryTypeExpression {
    TypeParameter {
        ordinal: u32,
    },
    Builtin {
        name: String,
        arguments: Vec<CallableRegistryTypeExpression>,
    },
    PackageSymbol {
        package_id: String,
        symbol_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CallableRegistryPlanExpression {
    FromType { ty: CallableRegistryTypeExpression },
}

/// Versioned callable ABI template. It contains verifier-relevant claims but
/// no runtime handler, route, provider identity, or capability object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CallableRegistrySignature {
    pub type_parameter_count: u32,
    pub parameter_types: Vec<CallableRegistryTypeExpression>,
    pub parameter_modes: Vec<ParamModeIr>,
    pub parameter_plans: Vec<CallableRegistryPlanExpression>,
    pub result_types: Vec<CallableRegistryTypeExpression>,
    pub result_plans: Vec<CallableRegistryPlanExpression>,
    pub effects: CallableMayEffects,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallableRegistryMatch {
    /// Instantiations in declaration ordinal order, never lexical name order.
    pub type_arguments: Vec<TypeRefIr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CallableRegistryMatchError {
    Arity {
        position: &'static str,
        expected: usize,
        actual: usize,
    },
    ModeMismatch,
    EffectMismatch,
    TypeMismatch {
        position: &'static str,
        ordinal: usize,
        message: String,
    },
    UnboundTypeParameter {
        ordinal: usize,
    },
    Lifecycle {
        position: &'static str,
        ordinal: usize,
        source: ValueLifecyclePolicyError,
    },
}

pub fn callable_signature_from_native(
    signature: &NativeSignatureDef,
    effects: CallableMayEffects,
) -> CallableRegistrySignature {
    let parameter_types = signature
        .params
        .iter()
        .map(type_expression_from_native)
        .collect::<Vec<_>>();
    let result_types = if matches!(
        signature.return_type,
        NativeSignatureTypeExpr::Builtin("void")
    ) {
        Vec::new()
    } else {
        vec![type_expression_from_native(&signature.return_type)]
    };
    CallableRegistrySignature {
        type_parameter_count: u32::try_from(signature.type_param_count)
            .expect("native signature generic arity fits u32"),
        parameter_modes: vec![ParamModeIr::Value; parameter_types.len()],
        parameter_plans: parameter_types
            .iter()
            .cloned()
            .map(|ty| CallableRegistryPlanExpression::FromType { ty })
            .collect(),
        result_plans: result_types
            .iter()
            .cloned()
            .map(|ty| CallableRegistryPlanExpression::FromType { ty })
            .collect(),
        parameter_types,
        result_types,
        effects,
    }
}

pub fn match_callable_registry_signature<R: ValueLifecycleFactResolver>(
    expected: &CallableRegistrySignature,
    actual: &HostEffectSignature,
    resolver: &mut R,
    budget: &mut ValueLifecyclePolicyBudget,
) -> Result<CallableRegistryMatch, CallableRegistryMatchError> {
    check_len(
        "parameters",
        expected.parameter_types.len(),
        actual.parameter_types.len(),
    )?;
    check_len(
        "parameterModes",
        expected.parameter_modes.len(),
        actual.parameter_modes.len(),
    )?;
    check_len(
        "parameterPlans",
        expected.parameter_plans.len(),
        actual.parameter_plans.len(),
    )?;
    check_len(
        "results",
        expected.result_types.len(),
        actual.result_types.len(),
    )?;
    check_len(
        "resultPlans",
        expected.result_plans.len(),
        actual.result_plans.len(),
    )?;
    if expected.parameter_modes != actual.parameter_modes {
        return Err(CallableRegistryMatchError::ModeMismatch);
    }
    if expected.effects != actual.effects {
        return Err(CallableRegistryMatchError::EffectMismatch);
    }
    let type_parameter_count = expected.type_parameter_count as usize;
    let mut arguments = vec![None; type_parameter_count];
    for (ordinal, (template, ty)) in expected
        .parameter_types
        .iter()
        .zip(&actual.parameter_types)
        .enumerate()
    {
        match_type_expression(template, ty, &mut arguments).map_err(|message| {
            CallableRegistryMatchError::TypeMismatch {
                position: "parameter",
                ordinal,
                message,
            }
        })?;
    }
    for (ordinal, (template, ty)) in expected
        .result_types
        .iter()
        .zip(&actual.result_types)
        .enumerate()
    {
        match_type_expression(template, ty, &mut arguments).map_err(|message| {
            CallableRegistryMatchError::TypeMismatch {
                position: "result",
                ordinal,
                message,
            }
        })?;
    }
    let type_arguments = arguments
        .into_iter()
        .enumerate()
        .map(|(ordinal, argument)| {
            argument.ok_or(CallableRegistryMatchError::UnboundTypeParameter { ordinal })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let environment = PositionalTypeEnvironment::empty();
    for (ordinal, (ty, plan)) in actual
        .parameter_types
        .iter()
        .zip(&actual.parameter_plans)
        .enumerate()
    {
        crate::verify_value_transfer_plan(ty, plan, &environment, resolver, budget).map_err(
            |source| CallableRegistryMatchError::Lifecycle {
                position: "parameter",
                ordinal,
                source,
            },
        )?;
    }
    for (ordinal, (ty, plan)) in actual
        .result_types
        .iter()
        .zip(&actual.result_plans)
        .enumerate()
    {
        crate::verify_value_transfer_plan(ty, plan, &environment, resolver, budget).map_err(
            |source| CallableRegistryMatchError::Lifecycle {
                position: "result",
                ordinal,
                source,
            },
        )?;
    }
    Ok(CallableRegistryMatch { type_arguments })
}

pub(crate) fn validate_callable_registry_signature(
    signature: &CallableRegistrySignature,
) -> Result<(), &'static str> {
    if signature.parameter_types.len() != signature.parameter_modes.len()
        || signature.parameter_types.len() != signature.parameter_plans.len()
        || signature.result_types.len() != signature.result_plans.len()
    {
        return Err("parallel type/mode/plan vectors differ in length");
    }
    if signature.result_types.len() > 1 {
        return Err("registry signature has more than one result");
    }
    if signature
        .parameter_modes
        .iter()
        .any(|mode| *mode != ParamModeIr::Value)
    {
        return Err("host and intrinsic parameters must use Value mode");
    }
    if signature
        .parameter_types
        .iter()
        .zip(&signature.parameter_plans)
        .chain(signature.result_types.iter().zip(&signature.result_plans))
        .any(|(ty, plan)| {
            !matches!(plan, CallableRegistryPlanExpression::FromType { ty: plan_ty } if plan_ty == ty)
        })
    {
        return Err("plan expression differs from its type expression");
    }
    let count = signature.type_parameter_count as usize;
    let mut seen = vec![false; count];
    for ty in signature
        .parameter_types
        .iter()
        .chain(&signature.result_types)
    {
        visit_type_parameters(ty, &mut seen)?;
    }
    if seen.iter().any(|seen| !seen) {
        return Err("declared type parameter is unbound");
    }
    Ok(())
}

fn visit_type_parameters(
    ty: &CallableRegistryTypeExpression,
    seen: &mut [bool],
) -> Result<(), &'static str> {
    match ty {
        CallableRegistryTypeExpression::TypeParameter { ordinal } => {
            let Some(slot) = seen.get_mut(*ordinal as usize) else {
                return Err("type parameter ordinal is outside declared arity");
            };
            *slot = true;
        }
        CallableRegistryTypeExpression::Builtin { arguments, .. } => {
            for argument in arguments {
                visit_type_parameters(argument, seen)?;
            }
        }
        CallableRegistryTypeExpression::PackageSymbol { .. } => {}
    }
    Ok(())
}

fn check_len(
    position: &'static str,
    expected: usize,
    actual: usize,
) -> Result<(), CallableRegistryMatchError> {
    if expected != actual {
        return Err(CallableRegistryMatchError::Arity {
            position,
            expected,
            actual,
        });
    }
    Ok(())
}

fn type_expression_from_native(
    expression: &NativeSignatureTypeExpr,
) -> CallableRegistryTypeExpression {
    match expression {
        NativeSignatureTypeExpr::TypeParam(ordinal) => {
            CallableRegistryTypeExpression::TypeParameter {
                ordinal: u32::try_from(*ordinal).expect("native type parameter ordinal fits u32"),
            }
        }
        NativeSignatureTypeExpr::Builtin(name) => CallableRegistryTypeExpression::Builtin {
            name: (*name).to_string(),
            arguments: Vec::new(),
        },
        NativeSignatureTypeExpr::Package {
            package_id,
            public_path,
        } => CallableRegistryTypeExpression::PackageSymbol {
            package_id: (*package_id).to_string(),
            symbol_path: (*public_path).to_string(),
        },
        NativeSignatureTypeExpr::Array(item) => CallableRegistryTypeExpression::Builtin {
            name: "Array".to_string(),
            arguments: vec![type_expression_from_native(item)],
        },
        NativeSignatureTypeExpr::Map(key, value) => CallableRegistryTypeExpression::Builtin {
            name: "Map".to_string(),
            arguments: vec![
                type_expression_from_native(key),
                type_expression_from_native(value),
            ],
        },
        NativeSignatureTypeExpr::Nullable(inner) => CallableRegistryTypeExpression::Builtin {
            name: "Nullable".to_string(),
            arguments: vec![type_expression_from_native(inner)],
        },
        NativeSignatureTypeExpr::Stream(inner) => CallableRegistryTypeExpression::Builtin {
            name: "Stream".to_string(),
            arguments: vec![type_expression_from_native(inner)],
        },
    }
}

fn match_type_expression(
    template: &CallableRegistryTypeExpression,
    actual: &TypeRefIr,
    arguments: &mut [Option<TypeRefIr>],
) -> Result<(), String> {
    match template {
        CallableRegistryTypeExpression::TypeParameter { ordinal } => {
            let slot = arguments
                .get_mut(*ordinal as usize)
                .ok_or_else(|| "type parameter ordinal is outside declared arity".to_string())?;
            if let Some(previous) = slot {
                if previous != actual {
                    return Err("type parameter has inconsistent instantiations".to_string());
                }
            } else {
                *slot = Some(actual.clone());
            }
            Ok(())
        }
        CallableRegistryTypeExpression::Builtin {
            name,
            arguments: expected_arguments,
        } => {
            let (actual_name, actual_arguments): (&str, &[TypeRefIr]) = match actual {
                TypeRefIr::Builtin { name, args } => (name, args),
                TypeRefIr::Nullable { inner } if name == "Nullable" => {
                    ("Nullable", std::slice::from_ref(inner.as_ref()))
                }
                _ => return Err(format!("expected builtin {name}")),
            };
            if actual_name != name || actual_arguments.len() != expected_arguments.len() {
                return Err(format!("expected builtin {name} with exact arity"));
            }
            for (expected, actual) in expected_arguments.iter().zip(actual_arguments) {
                match_type_expression(expected, actual, arguments)?;
            }
            Ok(())
        }
        CallableRegistryTypeExpression::PackageSymbol {
            package_id,
            symbol_path,
        } => {
            let symbol = match actual {
                TypeRefIr::PackageSymbol { symbol } => symbol,
                TypeRefIr::AppliedNominal {
                    base: NominalTypeRefBaseIr::PackageSymbol { symbol },
                    arguments,
                } if arguments.is_empty() => symbol,
                _ => return Err("expected exact package symbol".to_string()),
            };
            let PackageRefIr::PackageId {
                package_id: actual_package_id,
            } = &symbol.package
            else {
                return Err("package symbol retains an unresolved dependency alias".to_string());
            };
            if actual_package_id != package_id || symbol.symbol_path != *symbol_path {
                return Err("package symbol owner/path mismatch".to_string());
            }
            if symbol.abi_expectation.as_deref().is_none_or(str::is_empty) {
                return Err("package symbol lacks exact ABI identity".to_string());
            }
            Ok(())
        }
    }
}
