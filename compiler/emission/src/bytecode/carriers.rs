//! Compiler-owned producer-specific machine value carrier facts.

mod constraints;
mod model;
mod policy;
mod solver;

#[cfg(test)]
mod tests;

pub(crate) use constraints::analyze_machine_carriers;
pub(crate) use model::{
    FunctionMachineCarrierFacts, MachineDefaultValueFact, MachineDefaultValueKind,
    MachineShapeCarrierFact, MachineWritableStepFact, PackageMachineCarrierFacts,
};
pub(crate) use policy::may_share_scalar_machine_carrier;
