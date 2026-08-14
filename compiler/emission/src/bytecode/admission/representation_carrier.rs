use skiff_artifact_model::{CallTargetIr, ExprIr, TypeRefIr};
use skiff_compiler_lowering::mir::MirUnit;

use super::{host_effects::HostEffectAdmissions, CANONICAL_DURATION_MILLISECONDS_BINDING_KEY};
use crate::bytecode::{
    carriers::PackageMachineCarrierFacts, inputs::canonical_function_key, BytecodeEmissionError,
};

/// Compiler-owned proof that one exact semantic type is materialized through
/// one exact source representation and physical VM carrier.
///
/// The fact is deliberately opaque outside bytecode emission. It is created
/// only from an admitted `Duration.milliseconds(number-literal)` producer that
/// is in the exact value-flow closure of the pinned Sleep parameter.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RepresentationCarrierFact {
    module_path: String,
    owner: TypeRefIr,
    representation: TypeRefIr,
    physical_carrier: TypeRefIr,
}

impl RepresentationCarrierFact {
    pub(crate) fn module_path(&self) -> &str {
        &self.module_path
    }

    pub(crate) fn owner(&self) -> &TypeRefIr {
        &self.owner
    }

    pub(crate) fn representation(&self) -> &TypeRefIr {
        &self.representation
    }

    pub(crate) fn physical_carrier(&self) -> &TypeRefIr {
        &self.physical_carrier
    }
}

pub(super) fn analyze(
    units: &[MirUnit],
    machine_carriers: &PackageMachineCarrierFacts,
) -> Result<Vec<RepresentationCarrierFact>, BytecodeEmissionError> {
    let mut facts = Vec::new();
    for unit in units {
        for function in &unit.functions {
            let function_key = canonical_function_key(&unit.module_path, &function.symbol)?;
            let host_effects = HostEffectAdmissions::analyze(
                function,
                CANONICAL_DURATION_MILLISECONDS_BINDING_KEY,
            )
            .map_err(|error| {
                representation_error(
                    &function_key,
                    format!(
                        "expression {} exact host-effect closure: {}",
                        error.expression_index, error.detail
                    ),
                )
            })?;
            let carriers = machine_carriers.function(&function_key).ok_or_else(|| {
                representation_error(&function_key, "function machine-carrier row is absent")
            })?;

            for expression in &function.expressions {
                let ExprIr::Call { call } = &expression.expression else {
                    continue;
                };
                let CallTargetIr::Native { target } = &call.target else {
                    continue;
                };
                if target.binding_key.as_deref()
                    != Some(CANONICAL_DURATION_MILLISECONDS_BINDING_KEY)
                    || !host_effects.admits_duration_constructor(expression.index, &expression.ty)
                {
                    continue;
                }
                let [argument_ref] = call.args.as_slice() else {
                    return Err(representation_error(
                        &function_key,
                        format!(
                            "expression {} admitted materializer lost its exact argument",
                            expression.index
                        ),
                    ));
                };
                let argument = function.expression(*argument_ref)?;
                if !matches!(
                    &argument.expression,
                    ExprIr::Literal {
                        value: skiff_artifact_model::LiteralIr::Number { .. }
                    }
                ) {
                    return Err(representation_error(
                        &function_key,
                        format!(
                            "expression {} admitted materializer argument is not an exact Number literal",
                            expression.index
                        ),
                    ));
                }
                let physical_carrier = carriers
                    .expression(argument.index)
                    .ok_or_else(|| {
                        representation_error(
                            &function_key,
                            format!(
                                "expression {} materializer argument has no machine carrier",
                                expression.index
                            ),
                        )
                    })?
                    .ty();
                let owner_carrier = carriers.expression(expression.index).ok_or_else(|| {
                    representation_error(
                        &function_key,
                        format!(
                            "expression {} materializer result has no machine carrier",
                            expression.index
                        ),
                    )
                })?;
                if owner_carrier.ty() != &expression.ty {
                    return Err(representation_error(
                        &function_key,
                        format!(
                            "expression {} materializer result carrier {:?} differs from its exact semantic type {:?}",
                            expression.index,
                            owner_carrier.ty(),
                            expression.ty
                        ),
                    ));
                }
                let fact = close_fact(
                    &function_key,
                    &unit.module_path,
                    expression.ty.clone(),
                    argument.ty.clone(),
                    physical_carrier.clone(),
                )?;
                insert_fact(&function_key, &mut facts, fact)?;
            }
        }
    }
    Ok(facts)
}

fn close_fact(
    function_key: &str,
    module_path: &str,
    owner: TypeRefIr,
    representation: TypeRefIr,
    physical_carrier: TypeRefIr,
) -> Result<RepresentationCarrierFact, BytecodeEmissionError> {
    if !matches!(
        &representation,
        TypeRefIr::Builtin { name, args } if name == "integer" && args.is_empty()
    ) {
        return Err(representation_error(
            function_key,
            format!("materializer representation is not exact integer: {representation:?}"),
        ));
    }
    if physical_carrier != TypeRefIr::builtin("number") {
        return Err(representation_error(
            function_key,
            format!("integer literal physical carrier is not exact number: {physical_carrier:?}"),
        ));
    }
    if owner == representation || owner == physical_carrier || representation == physical_carrier {
        return Err(representation_error(
            function_key,
            "owner, representation, and physical carrier types must be distinct",
        ));
    }
    Ok(RepresentationCarrierFact {
        module_path: module_path.to_string(),
        owner,
        representation,
        physical_carrier,
    })
}

fn insert_fact(
    function_key: &str,
    facts: &mut Vec<RepresentationCarrierFact>,
    candidate: RepresentationCarrierFact,
) -> Result<(), BytecodeEmissionError> {
    let existing = facts
        .iter()
        .find(|fact| fact.module_path == candidate.module_path && fact.owner == candidate.owner);
    match existing {
        Some(existing) if existing == &candidate => Ok(()),
        Some(existing) => Err(representation_error(
            function_key,
            format!(
                "conflicting facts for {:?}: {:?} -> {:?} versus {:?} -> {:?}",
                candidate.owner,
                existing.representation,
                existing.physical_carrier,
                candidate.representation,
                candidate.physical_carrier
            ),
        )),
        None => {
            facts.push(candidate);
            Ok(())
        }
    }
}

fn representation_error(function_key: &str, detail: impl Into<String>) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "admitted representation carrier fact",
        location: format!(" {}", detail.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact(physical_carrier: TypeRefIr) -> RepresentationCarrierFact {
        RepresentationCarrierFact {
            module_path: "main".to_string(),
            owner: TypeRefIr::builtin("fixture-duration"),
            representation: TypeRefIr::builtin("integer"),
            physical_carrier,
        }
    }

    #[test]
    fn wrong_physical_carrier_is_a_typed_compiler_rejection() {
        let error = close_fact(
            "main::run",
            "main",
            TypeRefIr::builtin("fixture-duration"),
            TypeRefIr::builtin("integer"),
            TypeRefIr::builtin("integer"),
        )
        .expect_err("integer is not the Number literal carrier");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedConstruct {
                construct: "admitted representation carrier fact",
                location,
                ..
            } if location.contains("physical carrier is not exact number")
        ));
    }

    #[test]
    fn conflicting_materializer_facts_are_a_typed_compiler_rejection() {
        let mut facts = vec![fact(TypeRefIr::builtin("number"))];
        let error = insert_fact(
            "main::run",
            &mut facts,
            fact(TypeRefIr::builtin("different-number-carrier")),
        )
        .expect_err("one owner cannot retain two physical facts");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedConstruct {
                construct: "admitted representation carrier fact",
                location,
                ..
            } if location.contains("conflicting facts")
        ));
    }
}
