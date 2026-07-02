use skiff_runtime_model::type_plan::RuntimeTypePlan;

use super::{NativeBindingKey, NativeRequiredContext};

#[derive(Clone, Debug)]
pub struct NativeCallPlan {
    pub binding_key: NativeBindingKey,
    pub arg_plans: Vec<RuntimeTypePlan>,
    pub return_plan: RuntimeTypePlan,
    pub required_context: NativeRequiredContext,
}

impl NativeCallPlan {
    pub fn new(
        binding_key: NativeBindingKey,
        arg_plans: Vec<RuntimeTypePlan>,
        return_plan: RuntimeTypePlan,
        required_context: NativeRequiredContext,
    ) -> Self {
        Self {
            binding_key,
            arg_plans,
            return_plan,
            required_context,
        }
    }
}
