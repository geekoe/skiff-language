//! Source-owned value-transfer lifecycle facts.
//!
//! Transfer planning is deliberately independent from writable loans:
//! `InOut` is a parameter mode, not a fifth transfer kind and not a reason to
//! turn an otherwise snapshot-shareable value into `MoveOnly`.
//!
//! Native lifecycle behavior comes only from artifact-model's pinned native
//! lifecycle registry. Source ordinary aggregates are proven recursively and
//! use a root-only snapshot release; this crate never constructs a pool-local
//! recursive-shape drop plan.

use skiff_artifact_model::{
    native_value_lifecycle_registry, NativeValueLifecycleRegistry, ValueTransferPlan,
};

mod classifier;
mod contract;
mod native;

use classifier::Classifier;

pub use contract::{
    SourceValueTransferError, SourceValueTransferFacts, SourceValueTransferNominalFact,
    SourceValueTransferNominalId, SourceValueTransferNominalSemantics,
    SourceValueTransferPackageRef, SourceValueTransferPlanInput, SourceValueTransferPosition,
};

impl SourceValueTransferFacts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_nominal(
        &mut self,
        identity: SourceValueTransferNominalId,
        fact: SourceValueTransferNominalFact,
    ) -> Option<SourceValueTransferNominalFact> {
        self.nominals.insert(identity, fact)
    }

    pub fn nominal(
        &self,
        identity: &SourceValueTransferNominalId,
    ) -> Option<&SourceValueTransferNominalFact> {
        self.nominals.get(identity)
    }

    /// Derives the complete relocatable value-transfer plan for one exact
    /// source type. Native behavior is resolved only through the registry
    /// whose identity is pinned into the bytecode artifact schema.
    pub fn plan(
        &self,
        input: SourceValueTransferPlanInput<'_>,
    ) -> Result<ValueTransferPlan, SourceValueTransferError> {
        self.plan_with_registry(native_value_lifecycle_registry(), input)
    }

    fn plan_with_registry(
        &self,
        registry: &NativeValueLifecycleRegistry,
        input: SourceValueTransferPlanInput<'_>,
    ) -> Result<ValueTransferPlan, SourceValueTransferError> {
        Classifier::new(self, registry, input.relocatable_type_parameters)?
            .plan(input.module_path, input.ty)
    }
}

/// Fallible convenience entrypoint over exact source and nominal facts.
pub fn source_value_transfer_plan(
    facts: &SourceValueTransferFacts,
    input: SourceValueTransferPlanInput<'_>,
) -> Result<ValueTransferPlan, SourceValueTransferError> {
    facts.plan(input)
}

#[cfg(test)]
mod tests;
