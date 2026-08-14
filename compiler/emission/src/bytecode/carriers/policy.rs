use std::collections::BTreeMap;

use skiff_artifact_model::{LiteralIr, TypeDescriptorIr, TypeRefIr};
use skiff_compiler_lowering::mir::MirUnit;

use super::model::SemanticRole;
use crate::bytecode::BytecodeEmissionError;

pub(crate) fn literal_carrier_type(literal: &LiteralIr) -> TypeRefIr {
    TypeRefIr::builtin(match literal {
        LiteralIr::Null => "null",
        LiteralIr::Bool { .. } => "bool",
        LiteralIr::Number { .. } => "number",
        LiteralIr::String { .. } => "string",
    })
}

/// Selects the exact producer emitted by the catch-slot default initializer.
///
/// This is deliberately not a global type rewrite. Native `integer` values
/// remain Integer carriers; only this compiler-owned zero literal is a Number
/// producer. Nullable defaults likewise materialize the explicit Null branch.
pub(super) fn catch_default_literal(ty: &TypeRefIr) -> Option<(LiteralIr, TypeRefIr)> {
    let value = match ty {
        TypeRefIr::Literal { value } => value.clone(),
        TypeRefIr::Builtin { name, args } if name == "string" && args.is_empty() => {
            LiteralIr::String {
                value: String::new(),
            }
        }
        TypeRefIr::Builtin { name, args } if name == "null" && args.is_empty() => LiteralIr::Null,
        TypeRefIr::Nullable { .. } => LiteralIr::Null,
        TypeRefIr::Builtin { name, args }
            if matches!(name.as_str(), "integer" | "number") && args.is_empty() =>
        {
            LiteralIr::Number {
                value: serde_json::Number::from(0_u64),
            }
        }
        TypeRefIr::Builtin { name, args } if name == "bool" && args.is_empty() => {
            LiteralIr::Bool { value: false }
        }
        _ => return None,
    };
    let carrier = literal_carrier_type(&value);
    Some((value, carrier))
}

/// Admission-side precondition for a later exact carrier join.
///
/// This does not choose a carrier and is intentionally weaker than the graph
/// result: it only says that two source types have a single identical scalar
/// physical face. The complete writer graph must still prove that exact face
/// before an admitted artifact can be emitted.
pub(crate) fn may_share_scalar_machine_carrier(left: &TypeRefIr, right: &TypeRefIr) -> bool {
    left == right
        || scalar_semantic_carrier(left)
            .zip(scalar_semantic_carrier(right))
            .is_some_and(|(left, right)| left == right)
}

pub(super) fn semantic_accepts_carrier(
    semantic: &TypeRefIr,
    carrier: &TypeRefIr,
    role: SemanticRole,
) -> bool {
    if semantic == carrier {
        return true;
    }
    match semantic {
        TypeRefIr::Literal { value } => &literal_carrier_type(value) == carrier,
        TypeRefIr::Builtin { name, args } if name == "integer" && args.is_empty() => {
            carrier == &TypeRefIr::builtin("number")
        }
        TypeRefIr::Builtin { name, args } => {
            let TypeRefIr::Builtin {
                name: carrier_name,
                args: carrier_args,
            } = carrier
            else {
                return false;
            };
            name == carrier_name
                && args.len() == carrier_args.len()
                && args.iter().zip(carrier_args).all(|(semantic, carrier)| {
                    semantic_accepts_carrier(semantic, carrier, SemanticRole::Position)
                })
        }
        TypeRefIr::Record { fields } => {
            let explicit_nominal = matches!(
                carrier,
                TypeRefIr::LocalType { .. }
                    | TypeRefIr::PublicationType { .. }
                    | TypeRefIr::ServiceSymbol { .. }
                    | TypeRefIr::PackageSymbol { .. }
                    | TypeRefIr::PackageSchema { .. }
                    | TypeRefIr::DbObjectSymbol { .. }
            );
            let generated_record = matches!(
                carrier,
                TypeRefIr::Builtin { name, args }
                    if matches!(name.as_str(), "CatchResult" | "Exception")
                        && !args.is_empty()
            );
            if (matches!(role, SemanticRole::ConstructExpression) && explicit_nominal)
                || generated_record
            {
                // The explicit Construct.type_ref is the runtime nominal
                // identity. Its separately checked exact field graph is the
                // only authority for this structural source expression.
                return true;
            }
            let TypeRefIr::Record {
                fields: carrier_fields,
            } = carrier
            else {
                return false;
            };
            fields.len() == carrier_fields.len()
                && fields.iter().all(|(name, semantic)| {
                    carrier_fields.get(name).is_some_and(|carrier| {
                        semantic_accepts_carrier(semantic, carrier, SemanticRole::Position)
                    })
                })
        }
        TypeRefIr::Union { items } if matches!(role, SemanticRole::ConstructExpression) => {
            items.iter().any(|item| item == carrier)
        }
        TypeRefIr::Union { items } => {
            let mut collapsed = None;
            for item in items {
                let Some(item_carrier) = scalar_semantic_carrier(item) else {
                    return false;
                };
                if collapsed
                    .as_ref()
                    .is_some_and(|collapsed| collapsed != &item_carrier)
                {
                    return false;
                }
                collapsed = Some(item_carrier);
            }
            collapsed.as_ref() == Some(carrier)
        }
        TypeRefIr::Nullable { inner }
            if matches!(
                role,
                SemanticRole::ShapeField | SemanticRole::CatchPosition | SemanticRole::DefaultValue
            ) =>
        {
            // A concrete record construction publishes its physical null
            // or inner field row and plan. Nullable frame/call/return
            // positions do not take this path and remain fail closed without
            // an explicit representation fact.
            carrier == &TypeRefIr::builtin("null")
                || semantic_accepts_carrier(inner, carrier, SemanticRole::Position)
        }
        // Nullable/nominal/representation identity is never implicitly
        // replaced by a concrete branch or payload.
        _ => false,
    }
}

fn scalar_semantic_carrier(ty: &TypeRefIr) -> Option<TypeRefIr> {
    match ty {
        TypeRefIr::Literal { value } => Some(literal_carrier_type(value)),
        TypeRefIr::Builtin { name, args } if name == "integer" && args.is_empty() => {
            Some(TypeRefIr::builtin("number"))
        }
        TypeRefIr::Builtin { args, .. } if args.is_empty() => Some(ty.clone()),
        _ => None,
    }
}

pub(super) fn declared_record_fields(
    units: &[MirUnit],
    unit_index: usize,
    ty: &TypeRefIr,
) -> Option<BTreeMap<String, TypeRefIr>> {
    let unit = units.get(unit_index)?;
    match ty {
        TypeRefIr::Record { fields } => Some(fields.clone()),
        TypeRefIr::LocalType { type_index } => unit
            .type_table
            .get(*type_index as usize)
            .and_then(record_descriptor_fields),
        TypeRefIr::PublicationType {
            module_path,
            type_index,
        } => units
            .iter()
            .find(|unit| &unit.module_path == module_path)?
            .type_table
            .get(*type_index as usize)
            .and_then(record_descriptor_fields),
        TypeRefIr::PackageSymbol { symbol } => {
            let package_id = match &symbol.package {
                skiff_artifact_model::PackageRefIr::PackageId { package_id } => package_id,
                skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } => dependency_ref,
            };
            unit.package_type_records
                .get(&(package_id.clone(), symbol.symbol_path.clone()))
                .cloned()
        }
        TypeRefIr::Builtin { name, args } if name == "Exception" && args.len() == 1 => {
            Some(BTreeMap::from([("error".to_string(), args[0].clone())]))
        }
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
            Some(BTreeMap::from([
                (
                    "exception".to_string(),
                    TypeRefIr::Builtin {
                        name: "Exception".to_string(),
                        args: vec![args[1].clone()],
                    },
                ),
                ("tag".to_string(), TypeRefIr::builtin("string")),
            ]))
        }
        _ => None,
    }
}

fn record_descriptor_fields(
    declaration: &skiff_artifact_model::TypeDeclIr,
) -> Option<BTreeMap<String, TypeRefIr>> {
    match &declaration.descriptor {
        TypeDescriptorIr::Record { fields } if declaration.type_params.is_empty() => {
            Some(fields.clone())
        }
        _ => None,
    }
}

pub(super) fn carrier_error(
    function_key: &str,
    detail: impl Into<String>,
) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "exact machine carrier facts",
        location: format!(" {}", detail.into()),
    }
}

pub(super) fn is_void(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if name == "void" && args.is_empty())
}
