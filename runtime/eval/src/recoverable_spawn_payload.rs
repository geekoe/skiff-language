use skiff_runtime_boundary::{
    binary::{decode_recoverable_payload_with_behavior, encode_recoverable_payload_with_behavior},
    payload::PayloadBoundary,
    recoverable::RecoverableBehaviorHooks,
};
use skiff_runtime_linked_program::{ExecutableAddr, LinkedExecutable};
use skiff_runtime_linked_type_plan::{
    PlanContext, ProgramTypeView, RuntimeRecoverableExpectedTypePlanLinkedExt,
};
use skiff_runtime_model::{
    recoverable::{
        RuntimeRecoverableExpectedRecordFieldPlan, RuntimeRecoverableExpectedTypeNode,
        RuntimeRecoverableExpectedTypePlan,
    },
    request_heap::RequestHeap,
    runtime_value::RuntimeValue,
};

use crate::{error::Result, program_ir::executable_has_explicit_self_binding};

pub fn executable_request_recoverable_expected_plan<'p>(
    program: impl Into<ProgramTypeView<'p>>,
    addr: &ExecutableAddr,
    executable: &LinkedExecutable,
) -> Result<RuntimeRecoverableExpectedTypePlan> {
    let program = program.into();
    let explicit_self_param = executable_has_explicit_self_binding(executable);
    let ctx = PlanContext::from_type_view(program, addr);
    let mut fields = Vec::new();
    for parameter in executable
        .params
        .iter()
        .skip(usize::from(explicit_self_param))
    {
        let ty = RuntimeRecoverableExpectedTypePlan::from_linked(&parameter.ty, &ctx)?;
        let required = !matches!(
            &ty.node,
            RuntimeRecoverableExpectedTypeNode::Nullable { .. }
        );
        fields.push(RuntimeRecoverableExpectedRecordFieldPlan {
            name: parameter.name.clone(),
            ty,
            required,
        });
    }
    Ok(RuntimeRecoverableExpectedTypePlan {
        label: "record".to_string(),
        identity: None,
        node: RuntimeRecoverableExpectedTypeNode::Record {
            fields,
            boundary_record_kind: None,
        },
    })
}

pub fn encode_spawn_args_payload(
    value: &RuntimeValue,
    expected: &RuntimeRecoverableExpectedTypePlan,
    boundary: &PayloadBoundary,
    heap: &RequestHeap,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
) -> Result<Vec<u8>> {
    encode_recoverable_payload_with_behavior(value, expected, boundary, heap, behavior_hooks)
        .map_err(Into::into)
}

pub fn decode_spawn_args_payload(
    bytes: &[u8],
    expected: &RuntimeRecoverableExpectedTypePlan,
    boundary: &PayloadBoundary,
    heap: &mut RequestHeap,
    behavior_hooks: &dyn RecoverableBehaviorHooks,
) -> Result<RuntimeValue> {
    decode_recoverable_payload_with_behavior(bytes, expected, boundary, heap, behavior_hooks)
        .map_err(Into::into)
}
