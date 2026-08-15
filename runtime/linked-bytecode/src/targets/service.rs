use skiff_artifact_model::{
    BoundaryDropPlan, BoundaryErrorPlan, BoundaryTransfer, BoundaryValuePlan, ContractTypeRef,
    ValueProvenance,
};

use crate::TypeIndex;

/// Linked form of one compiler-emitted service boundary value. The canonical
/// contract type remains available for provider-side matching; the caller
/// runtime tag is the exact caller-image type row selected by the linker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedServiceBoundaryValue {
    contract_type: ContractTypeRef,
    value_plan: BoundaryValuePlan,
    transfer: BoundaryTransfer,
    drop: BoundaryDropPlan,
    source: ValueProvenance,
    caller_type: TypeIndex,
}

impl LinkedServiceBoundaryValue {
    pub fn new(
        contract_type: ContractTypeRef,
        value_plan: BoundaryValuePlan,
        transfer: BoundaryTransfer,
        drop: BoundaryDropPlan,
        source: ValueProvenance,
        caller_type: TypeIndex,
    ) -> Self {
        Self {
            contract_type,
            value_plan,
            transfer,
            drop,
            source,
            caller_type,
        }
    }

    pub const fn contract_type(&self) -> &ContractTypeRef {
        &self.contract_type
    }

    pub const fn value_plan(&self) -> &BoundaryValuePlan {
        &self.value_plan
    }

    pub const fn transfer(&self) -> BoundaryTransfer {
        self.transfer
    }

    pub const fn drop(&self) -> &BoundaryDropPlan {
        &self.drop
    }

    pub const fn source(&self) -> &ValueProvenance {
        &self.source
    }

    pub const fn caller_type(&self) -> TypeIndex {
        self.caller_type
    }
}

/// Linked ordinary-error plan with the exact compiler-emitted fallback policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedServiceBoundaryErrorPlan {
    plan: BoundaryErrorPlan,
    fallback: LinkedServiceBoundaryValue,
}

impl LinkedServiceBoundaryErrorPlan {
    pub fn new(plan: BoundaryErrorPlan, fallback: LinkedServiceBoundaryValue) -> Self {
        Self { plan, fallback }
    }

    pub const fn plan(&self) -> &BoundaryErrorPlan {
        &self.plan
    }

    pub const fn fallback(&self) -> &LinkedServiceBoundaryValue {
        &self.fallback
    }
}

/// Callback surface accepted by the linked service boundary table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkedServiceCallbackPlan {
    None,
}

/// Canonical linked service boundary table consumed by cross-owner
/// materialization. The linker copies every fact from the compiler-emitted
/// artifact plan; it never derives argument/result/error behavior from names
/// or type shapes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedServiceBoundaryPlan {
    arguments: Box<[LinkedServiceBoundaryValue]>,
    results: Box<[LinkedServiceBoundaryValue]>,
    error: LinkedServiceBoundaryErrorPlan,
    stream_item: Option<Box<LinkedServiceBoundaryValue>>,
    callbacks: LinkedServiceCallbackPlan,
}

impl LinkedServiceBoundaryPlan {
    pub fn new(
        arguments: Vec<LinkedServiceBoundaryValue>,
        results: Vec<LinkedServiceBoundaryValue>,
        error: LinkedServiceBoundaryErrorPlan,
        stream_item: Option<LinkedServiceBoundaryValue>,
        callbacks: LinkedServiceCallbackPlan,
    ) -> Self {
        Self {
            arguments: arguments.into_boxed_slice(),
            results: results.into_boxed_slice(),
            error,
            stream_item: stream_item.map(Box::new),
            callbacks,
        }
    }

    pub fn arguments(&self) -> &[LinkedServiceBoundaryValue] {
        &self.arguments
    }

    pub fn results(&self) -> &[LinkedServiceBoundaryValue] {
        &self.results
    }

    pub const fn error(&self) -> &LinkedServiceBoundaryErrorPlan {
        &self.error
    }

    pub fn stream_item(&self) -> Option<&LinkedServiceBoundaryValue> {
        self.stream_item.as_deref()
    }

    pub const fn callbacks(&self) -> LinkedServiceCallbackPlan {
        self.callbacks
    }
}
