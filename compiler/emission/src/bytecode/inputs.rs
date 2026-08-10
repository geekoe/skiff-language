use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::{TypeRefIr, ValueTransferPlan};
use skiff_compiler_lowering::{
    mir::{liveness::compute_liveness, MirConst, MirFunction, MirUnit},
    FrozenConstantBundle,
};

use super::{BytecodeEmissionError, BytecodeValueTransferPlans, FunctionValueTransferPlans};

pub(crate) struct ValidatedEmissionInputs<'a> {
    pub(crate) constants: BTreeMap<String, ValidatedConstant<'a>>,
    pub(crate) functions: BTreeMap<String, &'a MirFunction>,
    pub(crate) function_plans: BTreeMap<String, &'a FunctionValueTransferPlans>,
    pub(crate) units: BTreeMap<String, &'a MirUnit>,
}

pub(crate) struct ValidatedConstant<'a> {
    pub(crate) module_path: &'a str,
    pub(crate) type_count: usize,
    pub(crate) constant: &'a MirConst,
    pub(crate) bundle: &'a FrozenConstantBundle,
    pub(crate) plan: &'a ValueTransferPlan,
}

impl<'a> ValidatedEmissionInputs<'a> {
    pub(crate) fn validate(
        units: &'a [MirUnit],
        bundles: &'a [FrozenConstantBundle],
        transfer_plans: &'a BytecodeValueTransferPlans,
    ) -> Result<Self, BytecodeEmissionError> {
        let mut units_by_module = BTreeMap::new();
        for unit in units {
            if units_by_module
                .insert(unit.module_path.clone(), unit)
                .is_some()
            {
                return Err(BytecodeEmissionError::DuplicateMirModule {
                    module_path: unit.module_path.clone(),
                });
            }
            unit.validate_executable_indices()?;
            unit.validate_constants()?;
        }

        let mut bundles_by_module = BTreeMap::new();
        for bundle in bundles {
            let module_path = bundle.module_path().to_string();
            if bundles_by_module
                .insert(module_path.clone(), bundle)
                .is_some()
            {
                return Err(BytecodeEmissionError::DuplicateConstantBundle { module_path });
            }
        }
        for module_path in units_by_module.keys() {
            if !bundles_by_module.contains_key(module_path) {
                return Err(BytecodeEmissionError::MissingConstantBundle {
                    module_path: module_path.clone(),
                });
            }
        }
        for module_path in bundles_by_module.keys() {
            if !units_by_module.contains_key(module_path) {
                return Err(BytecodeEmissionError::UnexpectedConstantBundle {
                    module_path: module_path.clone(),
                });
            }
        }

        for (module_path, unit) in &units_by_module {
            let bundle = bundles_by_module
                .get(module_path)
                .expect("bundle coverage was checked above");
            validate_constant_coverage(unit, bundle)?;
        }

        let mut constants = BTreeMap::new();
        for (module_path, unit) in &units_by_module {
            let bundle = *bundles_by_module
                .get(module_path)
                .expect("bundle coverage was checked above");
            for constant in &unit.constants {
                validate_constant_symbol(module_path, &constant.symbol)?;
                let plan = transfer_plans.constant(&constant.symbol).ok_or_else(|| {
                    BytecodeEmissionError::MissingConstantValueTransferPlan {
                        symbol: constant.symbol.clone(),
                    }
                })?;
                if matches!(plan, ValueTransferPlan::FromType { ty } if ty != &constant.ty) {
                    return Err(
                        BytecodeEmissionError::ConstantValueTransferPlanTypeMismatch {
                            symbol: constant.symbol.clone(),
                        },
                    );
                }
                if constants
                    .insert(
                        constant.symbol.clone(),
                        ValidatedConstant {
                            module_path: unit.module_path.as_str(),
                            type_count: unit.type_table.len(),
                            constant,
                            bundle,
                            plan,
                        },
                    )
                    .is_some()
                {
                    return Err(BytecodeEmissionError::DuplicateConstantSymbol {
                        symbol: constant.symbol.clone(),
                    });
                }
            }
        }
        for symbol in transfer_plans.constants().keys() {
            if !constants.contains_key(symbol) {
                return Err(BytecodeEmissionError::UnexpectedConstantValueTransferPlan {
                    symbol: symbol.clone(),
                });
            }
        }

        let mut functions = BTreeMap::new();
        let mut function_plans = BTreeMap::new();
        for (module_path, unit) in &units_by_module {
            for function in &unit.functions {
                function.validate_expression_indices()?;
                function.validate_slot_types()?;
                let function_key = canonical_function_key(module_path, &function.symbol)?;
                validate_mir_function(&function_key, function)?;

                if functions.contains_key(&function_key) {
                    return Err(BytecodeEmissionError::DuplicateFunctionKey { function_key });
                }

                let plans = transfer_plans.function(&function_key).ok_or_else(|| {
                    BytecodeEmissionError::MissingValueTransferPlans {
                        function_key: function_key.clone(),
                    }
                })?;
                validate_plan_counts(&function_key, function, plans)?;
                function_plans.insert(function_key.clone(), plans);
                functions.insert(function_key, function);
            }
        }
        for function_key in transfer_plans.functions().keys() {
            if !functions.contains_key(function_key) {
                return Err(BytecodeEmissionError::UnexpectedValueTransferPlans {
                    function_key: function_key.clone(),
                });
            }
        }

        Ok(Self {
            constants,
            functions,
            function_plans,
            units: units_by_module,
        })
    }
}

fn validate_constant_symbol(module_path: &str, symbol: &str) -> Result<(), BytecodeEmissionError> {
    let prefix = format!("{module_path}.");
    let canonical = symbol
        .strip_prefix(&prefix)
        .is_some_and(|declaration| !declaration.is_empty())
        && symbol.split('.').all(|segment| {
            !segment.is_empty()
                && !segment.chars().any(char::is_whitespace)
                && !segment.chars().any(char::is_control)
        });
    if !canonical {
        return Err(BytecodeEmissionError::InvalidConstantSymbol {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        });
    }
    Ok(())
}

pub(crate) fn canonical_function_key(
    module_path: &str,
    symbol: &str,
) -> Result<String, BytecodeEmissionError> {
    let prefix = format!("{module_path}.");
    let Some(declaration) = symbol
        .strip_prefix(&prefix)
        .filter(|declaration| !declaration.is_empty())
    else {
        return Err(BytecodeEmissionError::InvalidFunctionSymbol {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        });
    };
    Ok(format!("{module_path}::{declaration}"))
}

pub(crate) fn is_void(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::Builtin { name, args } if name == "void" && args.is_empty())
}

fn validate_constant_coverage(
    unit: &MirUnit,
    bundle: &FrozenConstantBundle,
) -> Result<(), BytecodeEmissionError> {
    let expected = unit
        .constants
        .iter()
        .map(|constant| constant.symbol.as_str())
        .collect::<BTreeSet<_>>();
    for symbol in &expected {
        if !bundle.graphs().contains_key(*symbol) {
            return Err(BytecodeEmissionError::MissingConstantGraph {
                symbol: (*symbol).to_string(),
            });
        }
        if bundle.graph(symbol)?.nodes.is_empty() {
            return Err(BytecodeEmissionError::EmptyConstantGraph {
                symbol: (*symbol).to_string(),
            });
        }
    }
    for symbol in bundle.graphs().keys() {
        if !expected.contains(symbol.as_str()) {
            return Err(BytecodeEmissionError::UnexpectedConstantGraph {
                symbol: symbol.clone(),
            });
        }
    }
    Ok(())
}

fn validate_mir_function(
    function_key: &str,
    function: &MirFunction,
) -> Result<(), BytecodeEmissionError> {
    let recomputed = compute_liveness(function)?;
    if recomputed != function.liveness {
        return Err(BytecodeEmissionError::LivenessMismatch {
            function_key: function_key.to_string(),
        });
    }

    let flattened = function
        .blocks
        .iter()
        .flat_map(|block| block.statements.iter());
    let mut position = 0usize;
    for (statement, entry) in flattened.zip(function.statements.iter()) {
        if statement.statement_index != entry.statement_index || statement.span != entry.span {
            return Err(BytecodeEmissionError::StatementTableMismatch {
                function_key: function_key.to_string(),
                position,
            });
        }
        position += 1;
    }
    let flattened_count = function
        .blocks
        .iter()
        .map(|block| block.statements.len())
        .sum::<usize>();
    if flattened_count != function.statements.len() {
        return Err(BytecodeEmissionError::StatementTableMismatch {
            function_key: function_key.to_string(),
            position,
        });
    }

    let mut parameter_slots = BTreeSet::new();
    for parameter in &function.params {
        let slot_type = function.slot_type(parameter.slot)?;
        if !parameter_slots.insert(parameter.slot) {
            return Err(BytecodeEmissionError::DuplicateParameterSlot {
                function_key: function_key.to_string(),
                slot: parameter.slot,
            });
        }
        if slot_type != &parameter.ty {
            return Err(BytecodeEmissionError::ParameterSlotTypeMismatch {
                function_key: function_key.to_string(),
                parameter: parameter.name.clone(),
                slot: parameter.slot,
            });
        }
    }
    Ok(())
}

fn validate_plan_counts(
    function_key: &str,
    function: &MirFunction,
    plans: &FunctionValueTransferPlans,
) -> Result<(), BytecodeEmissionError> {
    if plans.slot_plans.len() != function.slots.len() {
        return Err(BytecodeEmissionError::SlotPlanCountMismatch {
            function_key: function_key.to_string(),
            slot_count: function.slots.len(),
            plan_count: plans.slot_plans.len(),
        });
    }
    let result_count = usize::from(!is_void(&function.return_type));
    if plans.result_plans.len() != result_count {
        return Err(BytecodeEmissionError::ResultPlanCountMismatch {
            function_key: function_key.to_string(),
            result_count,
            plan_count: plans.result_plans.len(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::canonical_function_key;

    #[test]
    fn canonical_function_key_strips_the_exact_module_owner_once() {
        assert_eq!(
            canonical_function_key("alpha.beta", "alpha.beta.worker").unwrap(),
            "alpha.beta::worker"
        );
        assert!(canonical_function_key("alpha.beta", "other.worker").is_err());
        assert!(canonical_function_key("alpha.beta", "alpha.beta.").is_err());
    }
}
