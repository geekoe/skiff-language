use skiff_runtime_model::{service_error::NamedUnionOwnerIdentity, type_plan::RuntimeTypePlan};

use super::{NativeBindingKey, NativeRequiredContext};

#[derive(Clone, Debug)]
pub struct NativeCallPlan {
    pub binding_key: NativeBindingKey,
    pub arg_plans: Vec<RuntimeTypePlan>,
    pub return_plan: RuntimeTypePlan,
    pub required_context: NativeRequiredContext,
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
}

#[cfg(test)]
mod tests {
    use skiff_runtime_model::{
        addr::{FileAddr, TypeAddr, UnitAddr},
        service_error::{LocalExecutionTypeIdentity, NamedUnionOwnerIdentity},
        type_plan::{RuntimeTypeNode, RuntimeTypePlan},
    };

    use super::{NativeBindingKey, NativeCallPlan, NativeRequiredContext};

    fn owner(type_index: usize) -> NamedUnionOwnerIdentity {
        NamedUnionOwnerIdentity::LocalExecution(LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Package(0),
                file: FileAddr::loaded_file(0),
                type_index,
            },
            type_arguments: Vec::new(),
        })
    }

    #[test]
    fn websocket_request_plan_alone_carries_exact_linked_error_owner() {
        let exact_owner = owner(7);
        let plan = NativeCallPlan::new(
            NativeBindingKey::from_static("std.websocket.requestJsonToConnection"),
            Vec::new(),
            RuntimeTypePlan::new("null", None, RuntimeTypeNode::Null),
            NativeRequiredContext::Websocket,
        )
        .with_named_union_error_owner(exact_owner.clone())
        .expect("request binding should admit its linked named-union owner");

        assert_eq!(plan.named_union_error_owner(), Some(&exact_owner));

        let other = NativeCallPlan::new(
            NativeBindingKey::from_static("std.websocket.sendTextToConnection"),
            Vec::new(),
            RuntimeTypePlan::new("null", None, RuntimeTypeNode::Null),
            NativeRequiredContext::Websocket,
        )
        .with_named_union_error_owner(owner(8))
        .expect_err("unrelated natives must not acquire a pseudo owner");
        assert!(other.contains("requestJsonToConnection"));
    }
}
