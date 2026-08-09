use std::collections::BTreeMap;

use skiff_artifact_model::ValueTransferPlan;

/// Explicit source-owned transfer facts for every bytecode function.
///
/// The emitter never derives a plan from a MIR slot kind or type. Function
/// keys use the canonical `"{module_path}::{symbol}"` image key, and the
/// emitter requires this map to cover the MIR function set exactly.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BytecodeValueTransferPlans {
    pub functions: BTreeMap<String, FunctionValueTransferPlans>,
}

/// Dense transfer plans for one function frame.
///
/// `slot_plans` is indexed by MIR slot. `result_plans` is in result order
/// (zero entries for `void`, one for every other Phase 2 return type). The
/// emitter rejects missing, extra or differently-sized vectors rather than
/// defaulting any entry to `SnapshotShare`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionValueTransferPlans {
    pub slot_plans: Vec<ValueTransferPlan>,
    pub result_plans: Vec<ValueTransferPlan>,
}
