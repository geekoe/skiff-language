// B1a/EVAL5 temporary adapter for legacy descriptor/type-plan imports.
//
// Owner: B1 / EVAL5-type-plan-contract.
// Deletion/narrowing point: after remaining runtime-root non-eval users import
// `skiff_runtime_boundary::type_descriptor` and
// `skiff_runtime_linked_type_plan` directly.
pub(crate) use skiff_runtime_boundary::type_descriptor::*;
#[cfg(test)]
pub(crate) use skiff_runtime_linked_program::ResolvedSymbol;
#[allow(unused_imports)]
pub(crate) use skiff_runtime_linked_type_plan::{
    Error as RuntimeError, PlanContext, ProgramTypeView, RuntimeTypePlanLinkedExt,
};

#[cfg(test)]
mod tests;
