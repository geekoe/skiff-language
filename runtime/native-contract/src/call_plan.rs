use skiff_artifact_model::DbOperationReference;
use skiff_runtime_model::{service_error::NamedUnionOwnerIdentity, type_plan::RuntimeTypePlan};

use super::{NativeBindingKey, NativeRequiredContext};

#[derive(Clone, Debug)]
pub struct NativeCallPlan {
    pub binding_key: NativeBindingKey,
    pub arg_plans: Vec<RuntimeTypePlan>,
    pub return_plan: RuntimeTypePlan,
    pub required_context: NativeRequiredContext,
    pub db_operation: Option<DbOperationReference>,
    named_union_error_owner: Option<NamedUnionOwnerIdentity>,
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
            db_operation: None,
            named_union_error_owner: None,
        }
    }

    pub fn with_named_union_error_owner(
        mut self,
        owner: NamedUnionOwnerIdentity,
    ) -> Result<Self, String> {
        if self.binding_key.as_str() != "std.websocket.requestJsonToConnection" {
            return Err(
                "only std.websocket.requestJsonToConnection may carry a linked named-union error owner"
                    .to_string(),
            );
        }
        self.named_union_error_owner = Some(owner);
        Ok(self)
    }

    pub fn named_union_error_owner(&self) -> Option<&NamedUnionOwnerIdentity> {
        self.named_union_error_owner.as_ref()
    }

    pub fn with_db_operation(mut self, operation: DbOperationReference) -> Self {
        self.db_operation = Some(operation);
        self
    }

    pub fn db_operation(&self) -> Option<&DbOperationReference> {
        self.db_operation.as_ref()
    }
}

#[cfg(test)]
mod tests;
