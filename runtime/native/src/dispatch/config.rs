use skiff_runtime_boundary::{
    contract::RuntimeBoundaryContract, json::encode_untyped_wire_json, plan::BoundaryUse,
};
use skiff_runtime_model::addr::ExecutableAddr;

use crate::capability::NativeConfigCapability;
use crate::error::{Result, RuntimeError};
use crate::runtime_value_facade::{RequestHeap, RuntimeTypeNode, RuntimeTypePlan, RuntimeValue};

pub(super) struct ConfigNativeDispatch;

impl ConfigNativeDispatch {
    pub(super) fn matches(target: &str) -> bool {
        canonical_target(target).is_some()
    }

    pub(super) fn dispatch_native_call(
        config_context: &impl NativeConfigCapability,
        invocation: crate::dispatch::RuntimeNativeInvocation,
        args: Vec<RuntimeValue>,
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let target = canonical_target(invocation.binding_key()).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "{} resolved config call has an unsupported target",
                invocation.target_name()
            ))
        })?;
        let type_arg_plan = config_type_arg_plan(target, &invocation)?;
        Self::dispatch_builtin(
            config_context,
            invocation.current_addr()?,
            target,
            type_arg_plan,
            &args,
            heap,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch_builtin(
        config_context: &impl NativeConfigCapability,
        current_addr: &ExecutableAddr,
        target: &str,
        type_arg_plan: Option<RuntimeTypePlan>,
        args: &[RuntimeValue],
        heap: &mut RequestHeap,
    ) -> Result<RuntimeValue> {
        let type_arg_plan = match target {
            "config.require" | "config.optional" => Some(type_arg_plan.ok_or_else(|| {
                RuntimeError::InvalidArtifact(format!("{target} call is missing typeArgs[0]"))
            })?),
            "config.has" => None,
            _ => return Err(super::unsupported_native_target(target)),
        };
        let args = args
            .iter()
            .map(|value| Ok(encode_untyped_wire_json(value, heap)?))
            .collect::<Result<Vec<_>>>()?;
        let value = config_context.read_config_target(
            current_addr,
            target,
            &args,
            type_arg_plan.as_ref(),
        )?;
        match target {
            "config.has" => {
                let bool_plan = RuntimeTypePlan::synthetic_named_builtin(
                    "bool",
                    RuntimeTypeNode::Bool,
                    Vec::new(),
                );
                Ok(RuntimeBoundaryContract::default()
                    .codec_for_expected(&bool_plan, BoundaryUse::TypedJson, "config.has response")
                    .from_wire_json(&value, heap)?)
            }
            "config.optional" => {
                let return_plan = type_arg_plan
                    .as_ref()
                    .map(|plan| RuntimeTypePlan::synthetic_nullable(plan.clone()));
                let return_plan = return_plan.as_ref().ok_or_else(|| {
                    RuntimeError::InvalidArtifact(
                        "config.optional response boundary is missing expected type descriptor"
                            .to_string(),
                    )
                })?;
                RuntimeBoundaryContract::default()
                    .codec_for_expected(
                        return_plan,
                        BoundaryUse::TypedJson,
                        "config.optional response",
                    )
                    .from_wire_json(&value, heap)
                    .map_err(RuntimeError::from)
            }
            _ => {
                let plan = type_arg_plan.as_ref().ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "{target} response boundary is missing expected type descriptor"
                    ))
                })?;
                RuntimeBoundaryContract::default()
                    .codec_for_expected(plan, BoundaryUse::TypedJson, format!("{target} response"))
                    .from_wire_json(&value, heap)
                    .map_err(RuntimeError::from)
            }
        }
    }
}

fn canonical_target(target: &str) -> Option<&'static str> {
    match target {
        "config.require" | "std.config.require" => Some("config.require"),
        "config.optional" | "std.config.optional" => Some("config.optional"),
        "config.has" | "std.config.has" => Some("config.has"),
        _ => None,
    }
}

fn config_type_arg_plan(
    target: &str,
    invocation: &crate::dispatch::RuntimeNativeInvocation,
) -> Result<Option<RuntimeTypePlan>> {
    match target {
        "config.require" => Ok(Some(invocation.return_plan()?.clone())),
        "config.optional" => Ok(Some(nullable_inner(invocation.return_plan()?)?.clone())),
        "config.has" => Ok(None),
        _ => Err(RuntimeError::Unsupported(format!(
            "unsupported config native target {target}"
        ))),
    }
}

fn nullable_inner(plan: &RuntimeTypePlan) -> Result<&RuntimeTypePlan> {
    let mut plan = plan;
    loop {
        match &plan.node {
            RuntimeTypeNode::Alias(inner) => plan = inner,
            RuntimeTypeNode::Nullable(inner) => return Ok(inner),
            _ => {
                return Err(RuntimeError::InvalidArtifact(
                    "config.optional response boundary is missing its nullable type descriptor"
                        .to_string(),
                ))
            }
        }
    }
}
