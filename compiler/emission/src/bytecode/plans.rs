use std::collections::BTreeMap;

use skiff_artifact_model::{
    native_value_lifecycle_registry, NativeValueDropPlan, NativeValueLifecycleConcrete,
    ValueDropPlan, ValueTransferPlan,
};
use skiff_compiler_lowering::mir::{MirSlot, MirUnit};

use super::{
    inputs::{canonical_function_key, is_void},
    BytecodeEmissionError,
};

/// Derives explicit transfer plans from MIR and the pinned native lifecycle
/// registry.
///
/// Constants retain their owner-qualified `FromType` plan. Function slots and
/// results are resolved to concrete `SnapshotShare` plans; missing or
/// non-snapshot lifecycle facts fail closed instead of being inferred.
pub fn derive_bytecode_value_transfer_plans(
    units: &[MirUnit],
) -> Result<BytecodeValueTransferPlans, BytecodeEmissionError> {
    let mut functions = BTreeMap::new();
    for unit in units {
        for function in &unit.functions {
            let function_key = canonical_function_key(&unit.module_path, &function.symbol)?;
            let mut slot_plans = Vec::with_capacity(function.slots.len());
            for slot in &function.slots {
                let ty = slot
                    .ty
                    .as_ref()
                    .ok_or_else(|| unsupported_slot_type(&function_key, slot))?;
                slot_plans.push(concrete_snapshot_plan(
                    &function_key,
                    &format!("slot `{}`", slot.name),
                    ty,
                )?);
            }
            let result_plans = if is_void(&function.return_type) {
                Vec::new()
            } else {
                vec![concrete_snapshot_plan(
                    &function_key,
                    "return value",
                    &function.return_type,
                )?]
            };
            functions.insert(
                function_key,
                FunctionValueTransferPlans {
                    slot_plans,
                    result_plans,
                },
            );
        }
    }
    let constants = units
        .iter()
        .flat_map(|unit| &unit.constants)
        .map(|constant| {
            (
                constant.symbol.clone(),
                ValueTransferPlan::FromType {
                    ty: constant.ty.clone(),
                },
            )
        })
        .collect();
    Ok(BytecodeValueTransferPlans::new(functions, constants))
}

fn concrete_snapshot_plan(
    function_key: &str,
    location: &str,
    ty: &skiff_artifact_model::TypeRefIr,
) -> Result<ValueTransferPlan, BytecodeEmissionError> {
    let resolution = native_value_lifecycle_registry()
        .lookup(ty)
        .map_err(|error| BytecodeEmissionError::UnsupportedConstruct {
            function_key: function_key.to_string(),
            construct: "value lifecycle lookup",
            location: format!(" {location}: {error}"),
        })?;
    let NativeValueLifecycleConcrete::SnapshotShare { drop } = resolution.lifecycle else {
        return Err(BytecodeEmissionError::UnsupportedConstruct {
            function_key: function_key.to_string(),
            construct: "non-snapshot value lifecycle",
            location: format!(" {location}"),
        });
    };
    let drop = match drop {
        NativeValueDropPlan::Trivial => ValueDropPlan::Trivial,
        NativeValueDropPlan::SnapshotRelease => ValueDropPlan::SnapshotRelease,
        NativeValueDropPlan::NativeAdapter { .. } => {
            return Err(BytecodeEmissionError::UnsupportedConstruct {
                function_key: function_key.to_string(),
                construct: "native adapter value drop",
                location: format!(" {location}"),
            })
        }
    };
    Ok(ValueTransferPlan::SnapshotShare { drop })
}

fn unsupported_slot_type(function_key: &str, slot: &MirSlot) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct: "slot without an exact type",
        location: format!(" slot `{}`", slot.name),
    }
}

/// Explicit source-owned transfer facts for every bytecode function and
/// implementation constant.
///
/// The emitter never derives a plan from a MIR slot kind or type. Function
/// Keys use the canonical `"{module_path}::{declaration}"` image spelling:
/// the emitter first requires MIR `symbol` to start with the exact
/// `"{module_path}."` owner prefix, strips that prefix once, and rejects an
/// empty declaration. It never appends the still-qualified MIR symbol to the
/// module a second time. This map must cover that canonical MIR function set
/// exactly. Constant keys use the canonical `"{module_path}.{declaration}"`
/// spelling retained by [`skiff_compiler_lowering::mir::MirConst`].
#[derive(Debug, Clone, PartialEq)]
pub struct BytecodeValueTransferPlans {
    functions: BTreeMap<String, FunctionValueTransferPlans>,
    constants: BTreeMap<String, ValueTransferPlan>,
}

impl BytecodeValueTransferPlans {
    /// Creates one explicit, package-wide plan bundle.
    ///
    /// Both maps are exact-coverage inputs. Emission rejects missing and
    /// extra rows; this constructor never fills a plan from a type or slot
    /// kind.
    pub fn new(
        functions: BTreeMap<String, FunctionValueTransferPlans>,
        constants: BTreeMap<String, ValueTransferPlan>,
    ) -> Self {
        Self {
            functions,
            constants,
        }
    }

    /// Explicit empty coverage for a package with no functions or constants.
    pub fn empty() -> Self {
        Self::new(BTreeMap::new(), BTreeMap::new())
    }

    pub fn functions(&self) -> &BTreeMap<String, FunctionValueTransferPlans> {
        &self.functions
    }

    pub fn function(&self, function_key: &str) -> Option<&FunctionValueTransferPlans> {
        self.functions.get(function_key)
    }

    pub fn constants(&self) -> &BTreeMap<String, ValueTransferPlan> {
        &self.constants
    }

    pub fn constant(&self, symbol: &str) -> Option<&ValueTransferPlan> {
        self.constants.get(symbol)
    }
}

/// Dense transfer plans for one function frame.
///
/// `slot_plans` is indexed by MIR slot. `result_plans` is in result order
/// (zero entries for `void`, one for every other Phase 2 return type). The
/// emitter rejects missing, extra or differently-sized vectors rather than
/// defaulting any entry to `SnapshotShare`.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionValueTransferPlans {
    pub slot_plans: Vec<ValueTransferPlan>,
    pub result_plans: Vec<ValueTransferPlan>,
}
