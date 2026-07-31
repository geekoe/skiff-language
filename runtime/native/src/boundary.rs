use serde_json::Value;

use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{RequestHeap, RuntimeTypePlan, RuntimeValue};

use skiff_runtime_boundary::{
    contract::RuntimeBoundaryContract,
    plan::{BoundaryConversionPlan, BoundaryDirection, BoundaryUse},
};
use skiff_runtime_native_contract::{NativeCallPlan, NativeRequiredContext};

pub(crate) struct NativeBoundaryAdapter<'a> {
    target: &'a str,
    plan: &'a NativeCallPlan,
    contract: RuntimeBoundaryContract,
}

impl<'a> NativeBoundaryAdapter<'a> {
    pub(crate) fn new(target: &'a str, plan: &'a NativeCallPlan) -> Self {
        Self {
            target,
            plan,
            contract: RuntimeBoundaryContract::default(),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn binding_key(&self) -> &'static str {
        self.plan.binding_key.as_str()
    }

    #[allow(dead_code)]
    pub(crate) fn required_context(&self) -> NativeRequiredContext {
        self.plan.required_context
    }

    #[allow(dead_code)]
    pub(crate) fn arg_count(&self) -> usize {
        self.plan.arg_plans.len()
    }

    #[allow(dead_code)]
    pub(crate) fn arg_plans(&self) -> &[RuntimeTypePlan] {
        &self.plan.arg_plans
    }

    pub(crate) fn arg_plan(&self, index: usize) -> Result<&RuntimeTypePlan> {
        self.plan.arg_plans.get(index).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "{} resolved native call is missing argument plan {index}",
                self.target
            ))
        })
    }

    #[allow(dead_code)]
    pub(crate) fn return_plan(&self) -> &RuntimeTypePlan {
        &self.plan.return_plan
    }

    pub(crate) fn coerce_arg(
        &self,
        index: usize,
        value: &RuntimeValue,
        label: &str,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let plan = self.arg_conversion_plan(index, BoundaryDirection::Coerce)?;
        Ok(self
            .contract
            .codec(&plan, label)
            .coerce_runtime_value(value, heap)?)
    }

    pub(crate) fn to_wire_arg(
        &self,
        index: usize,
        value: &RuntimeValue,
        label: &str,
        heap: &mut RequestHeap,
    ) -> Result<Value> {
        let plan = self.arg_conversion_plan(index, BoundaryDirection::Encode)?;
        Ok(self
            .contract
            .codec(&plan, label)
            .to_wire_json(value, heap)?)
    }

    pub(crate) fn coerce_return(
        &self,
        value: &RuntimeValue,
        label: &str,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let plan = self.return_conversion_plan(BoundaryDirection::Coerce);
        Ok(self
            .contract
            .codec(&plan, label)
            .coerce_runtime_value(value, heap)?)
    }

    pub(crate) fn from_wire_return(
        &self,
        value: &Value,
        label: &str,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let plan = self.return_conversion_plan(BoundaryDirection::Decode);
        Ok(self
            .contract
            .codec(&plan, label)
            .from_wire_json(value, heap)?)
    }

    pub(crate) fn from_wire_internal_handle_return(
        &self,
        value: &Value,
        label: &str,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let plan = self.return_conversion_plan(BoundaryDirection::Decode);
        Ok(self
            .contract
            .codec(&plan, label)
            .from_wire_json_internal_handle(value, heap)?)
    }

    fn arg_conversion_plan(
        &self,
        index: usize,
        direction: BoundaryDirection,
    ) -> Result<BoundaryConversionPlan> {
        Ok(self.contract.conversion_plan(
            self.arg_plan(index)?.clone(),
            BoundaryUse::NativeArg,
            direction,
        ))
    }

    fn return_conversion_plan(&self, direction: BoundaryDirection) -> BoundaryConversionPlan {
        self.contract.conversion_plan(
            self.plan.return_plan.clone(),
            BoundaryUse::NativeReturn,
            direction,
        )
    }
}

#[cfg(test)]
mod tests;
