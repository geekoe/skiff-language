use std::collections::BTreeMap;

use skiff_artifact_model::ValueTransferPlan;

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
