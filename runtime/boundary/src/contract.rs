use std::borrow::Cow;

use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

use crate::{
    error::Result,
    json::RuntimeBoundaryCodec,
    plan::{BoundaryConversionPlan, BoundaryDirection, BoundaryUse},
};

#[derive(Debug, Clone, Default)]
pub struct RuntimeBoundaryContract {
    policy: BoundaryContractPolicy,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoundaryContractPolicy {
    #[allow(dead_code)]
    reject_runtime_owned_handles_by_default: bool,
}

impl RuntimeBoundaryContract {
    pub fn new(policy: BoundaryContractPolicy) -> Self {
        Self { policy }
    }

    pub fn policy(&self) -> BoundaryContractPolicy {
        self.policy
    }

    pub fn conversion_plan(
        &self,
        expected: RuntimeTypePlan,
        use_case: BoundaryUse,
        direction: BoundaryDirection,
    ) -> BoundaryConversionPlan {
        let _ = self.policy;
        BoundaryConversionPlan::new(expected, use_case, direction)
    }

    pub fn codec<'a>(
        &self,
        plan: &'a BoundaryConversionPlan,
        label: impl Into<Cow<'a, str>>,
    ) -> RuntimeBoundaryCodec<'a> {
        let _ = self.policy;
        self.codec_for_expected(plan.expected(), plan.use_case(), label)
    }

    pub fn codec_for_expected<'a>(
        &self,
        expected: &'a RuntimeTypePlan,
        use_case: BoundaryUse,
        label: impl Into<Cow<'a, str>>,
    ) -> RuntimeBoundaryCodec<'a> {
        let _ = self.policy;
        RuntimeBoundaryCodec::new(expected, use_case, label)
    }

    pub fn encode_json_text<'a>(
        &self,
        value: &RuntimeValue,
        expected_type: Option<&'a RuntimeTypePlan>,
        use_case: BoundaryUse,
        label: impl Into<Cow<'a, str>>,
        heap: &mut RequestHeap,
    ) -> Result<String> {
        let _ = self.policy;
        RuntimeBoundaryCodec::encode_json_text(value, expected_type, use_case, label, heap)
    }
}
