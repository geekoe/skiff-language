use skiff_runtime_model::type_plan::{RuntimeRecordFieldPlan, RuntimeTypeNode, RuntimeTypePlan};

use super::*;
use crate::env::PreparedTailCall;

fn executable_return_plan(
    projection: RuntimeExecutionProjection<'_>,
    addr: &ExecutableAddr,
    executable: &LinkedExecutable,
    env: &Env,
) -> Result<Option<RuntimeTypePlan>> {
    executable
        .return_type
        .as_ref()
        .map(|return_type| {
            EvalTypeProjection::from_execution_projection(projection)
                .plan_from_linked_nested_ref_with_substitutions(
                    return_type,
                    addr,
                    &env.type_substitutions,
                )
        })
        .transpose()
}

fn runtime_record_fields_equivalent(
    left: &[RuntimeRecordFieldPlan],
    right: &[RuntimeRecordFieldPlan],
) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.name == right.name
                && left.required == right.required
                && left.identity == right.identity
                && runtime_type_plans_equivalent(&left.ty, &right.ty)
        })
}

fn runtime_type_nodes_equivalent(left: &RuntimeTypeNode, right: &RuntimeTypeNode) -> bool {
    match (left, right) {
        (RuntimeTypeNode::Alias(left), RuntimeTypeNode::Alias(right))
        | (RuntimeTypeNode::Nullable(left), RuntimeTypeNode::Nullable(right))
        | (RuntimeTypeNode::Stream(left), RuntimeTypeNode::Stream(right))
        | (RuntimeTypeNode::Array(left), RuntimeTypeNode::Array(right)) => {
            runtime_type_plans_equivalent(left, right)
        }
        (RuntimeTypeNode::Union(left), RuntimeTypeNode::Union(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| runtime_type_plans_equivalent(left, right))
        }
        (
            RuntimeTypeNode::Representation {
                type_name: left_name,
                payload: left_payload,
            },
            RuntimeTypeNode::Representation {
                type_name: right_name,
                payload: right_payload,
            },
        ) => left_name == right_name && runtime_type_plans_equivalent(left_payload, right_payload),
        (
            RuntimeTypeNode::Map {
                key: left_key,
                value: left_value,
            },
            RuntimeTypeNode::Map {
                key: right_key,
                value: right_value,
            },
        ) => {
            runtime_type_plans_equivalent(left_key, right_key)
                && runtime_type_plans_equivalent(left_value, right_value)
        }
        (
            RuntimeTypeNode::Record {
                fields: left_fields,
                boundary_record_kind: left_kind,
            },
            RuntimeTypeNode::Record {
                fields: right_fields,
                boundary_record_kind: right_kind,
            },
        ) => left_kind == right_kind && runtime_record_fields_equivalent(left_fields, right_fields),
        (RuntimeTypeNode::LiteralString(left), RuntimeTypeNode::LiteralString(right)) => {
            left == right
        }
        (RuntimeTypeNode::Json, RuntimeTypeNode::Json)
        | (RuntimeTypeNode::JsonObject, RuntimeTypeNode::JsonObject)
        | (RuntimeTypeNode::Bytes, RuntimeTypeNode::Bytes)
        | (RuntimeTypeNode::Date, RuntimeTypeNode::Date)
        | (RuntimeTypeNode::String, RuntimeTypeNode::String)
        | (RuntimeTypeNode::Bool, RuntimeTypeNode::Bool)
        | (RuntimeTypeNode::Number, RuntimeTypeNode::Number)
        | (RuntimeTypeNode::Integer, RuntimeTypeNode::Integer)
        | (RuntimeTypeNode::Null, RuntimeTypeNode::Null)
        | (RuntimeTypeNode::Unknown, RuntimeTypeNode::Unknown) => true,
        _ => false,
    }
}

fn runtime_type_plans_equivalent(left: &RuntimeTypePlan, right: &RuntimeTypePlan) -> bool {
    left.named_type_name == right.named_type_name
        && left.identity == right.identity
        && runtime_type_nodes_equivalent(left.node(), right.node())
}

impl Interpreter {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_tail_call(
        &self,
        projection: RuntimeExecutionProjection<'_>,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        caller_executable: &LinkedExecutable,
        target: &ExecutableAddr,
        type_args: &BTreeMap<String, LinkedTypeRef>,
        args: &[RuntimeValueCarrier],
        tail_site: InstructionSourceSite,
    ) -> Result<Option<Box<PreparedTailCall>>> {
        let resolved = projection.resolve_nested_executable(target)?;
        let target = resolved.addr.clone();
        let callee = resolved.executable;
        let explicit_self_param = executable_has_explicit_self_binding(callee);
        let has_separate_self_arg = matches!(callee.kind, ExecutableKind::ImplMethod)
            && callee.self_type.is_some()
            && !explicit_self_param
            && args.len() == callee.params.len() + 1;
        if !has_separate_self_arg {
            validate_program_call_arg_count(callee, args.len())?;
        }

        let mut env = Env::for_program_executable(
            callee,
            Some(resolved.file.module_path.clone()),
            program_assembly_index(&target),
        )?;
        env.inherit_stream_consumer_supervision_from(caller_env);
        env.stream_sink = caller_env.stream_sink.clone();
        env.current_stream_item_type = caller_env.current_stream_item_type.clone();
        env.response_stream_sink = caller_env.response_stream_sink.clone();
        env.type_substitutions = call_type_substitutions(
            projection.type_view(),
            caller_addr,
            &caller_env.type_substitutions,
            callee,
            type_args,
        );

        let (self_value, args) = if explicit_self_param || has_separate_self_arg {
            let Some((self_value, args)) = args.split_first() else {
                return Err(RuntimeError::Decode(format!(
                    "callable {} missing self argument",
                    callee.symbol
                )));
            };
            (self_value.clone(), args)
        } else {
            (
                caller_env
                    .self_value()
                    .unwrap_or_else(|| RuntimeValue::Null.into()),
                args,
            )
        };
        if explicit_self_param {
            env.declare_program_parameter(callee, "self", self_value)?;
        } else {
            env.declare_program_self(callee, self_value)?;
        }
        for (index, parameter) in callee
            .params
            .iter()
            .skip(usize::from(explicit_self_param))
            .enumerate()
        {
            env.declare_program_parameter(
                callee,
                &parameter.name,
                args.get(index)
                    .cloned()
                    .unwrap_or_else(|| RuntimeValue::Null.into()),
            )?;
        }

        let Ok(caller_return_plan) = executable_return_plan(
            projection.clone(),
            caller_addr,
            caller_executable,
            caller_env,
        ) else {
            return Ok(None);
        };
        let Ok(return_plan) = executable_return_plan(projection.clone(), &target, callee, &env)
        else {
            return Ok(None);
        };
        let equivalent = match (&caller_return_plan, &return_plan) {
            (None, None) => true,
            (Some(caller), Some(callee)) => runtime_type_plans_equivalent(caller, callee),
            _ => false,
        };
        if !equivalent {
            return Ok(None);
        }

        Ok(Some(Box::new(PreparedTailCall {
            target,
            env,
            return_plan,
            tail_site,
        })))
    }
}
