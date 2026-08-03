use std::collections::BTreeMap;

use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_boundary::{
    binary::{decode_payload_plan, encode_payload_plan},
    http::HttpBoundaryResponseParts,
    payload::{PayloadBoundary, PayloadBoundaryKind},
};
use skiff_runtime_capability_context::{RequestPayloadContext, RequestPayloadEncoding};
use skiff_runtime_linked_program::{CallIr, LinkedCallTarget};
use skiff_runtime_linked_type_plan::{PlanContext, RuntimeTypePlanLinkedExt};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue},
    type_plan::RuntimeTypePlan,
};

use super::{dispatch_in_process_boundary, InProcessBoundaryDispatchOrigin};
use crate::{
    binary_http_boundary::{
        binary_http_request_parameter_value, binary_http_response_from_runtime_value,
    },
    env::Env,
    error::{Result, RuntimeError},
    eval_context::EvalContext,
    heap_access::HeapAccess,
    program_execution::ProgramExecutionContext,
    program_invocation::executable_request_payload_plan,
    program_ir::executable_has_explicit_self_binding,
    Interpreter, RuntimeAssemblyServiceCallTarget,
};

/// Minimal top-level adapter that enters the same resolved in-process dispatcher as an internal
/// service call. It performs no provider lookup or boundary materialization of its own.
pub async fn dispatch_ingress_via_in_process_boundary(
    interpreter: &Interpreter,
    context: ProgramExecutionContext<'_>,
    heap: &mut HeapAccess,
    target: RuntimeAssemblyServiceCallTarget,
    request: &RequestPayloadContext<'_>,
) -> Result<InProcessBoundaryIngressResponse> {
    let addr = target.executable_addr().clone();
    let projection = super::RuntimeExecutionProjection::for_context(interpreter, &context)?;
    let resolved = projection.resolve_executable(&addr)?;
    let canonical_addr = resolved.addr.clone();
    let args = adapt_ingress_arguments(
        request,
        &projection,
        &canonical_addr,
        resolved.executable,
        heap.heap_mut(),
    )?;
    let mut env = Env::new();
    let call = CallIr {
        target: LinkedCallTarget::Executable {
            addr: canonical_addr.clone(),
        },
        args: Vec::new(),
        type_args: BTreeMap::new(),
        metadata: BTreeMap::new(),
        actor_metadata: None,
        site: InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::RuntimeBoundaryDispatch,
        },
    };
    let value = {
        let mut eval_context = EvalContext::new(
            interpreter,
            context,
            heap,
            &mut env,
            &canonical_addr,
            resolved.file,
            resolved.executable,
        )?;
        dispatch_in_process_boundary(
            &mut eval_context,
            &call,
            target,
            args,
            InProcessBoundaryDispatchOrigin::Ingress,
        )
        .await?
    };
    if request.has_binary_http() {
        return Ok(InProcessBoundaryIngressResponse::BinaryHttp(
            binary_http_response_from_runtime_value(
                &value,
                resolved.executable.return_type.as_ref(),
                projection.type_view(),
                &canonical_addr,
                heap.heap_mut(),
            )?,
        ));
    }
    let return_plan = resolved.executable.return_type.as_ref().map_or_else(
        || Ok(RuntimeTypePlan::json_value_plan()),
        |return_type| {
            RuntimeTypePlan::from_linked(
                return_type,
                &PlanContext::from_type_view(projection.type_view(), &canonical_addr),
            )
            .map_err(RuntimeError::from)
        },
    )?;
    Ok(InProcessBoundaryIngressResponse::RuntimePayload(
        encode_payload_plan(
            &value,
            &return_plan,
            &PayloadBoundary::external_untrusted(PayloadBoundaryKind::ServiceResponse),
            heap.heap_mut(),
        )?,
    ))
}

#[derive(Debug)]
pub enum InProcessBoundaryIngressResponse {
    RuntimePayload(Vec<u8>),
    BinaryHttp(HttpBoundaryResponseParts),
}

fn adapt_ingress_arguments(
    request: &RequestPayloadContext<'_>,
    projection: &super::RuntimeExecutionProjection<'_>,
    addr: &skiff_runtime_linked_program::ExecutableAddr,
    executable: &skiff_runtime_linked_program::LinkedExecutable,
    heap: &mut RequestHeap,
) -> Result<Vec<RuntimeValue>> {
    let parameters = executable
        .params
        .iter()
        .skip(usize::from(executable_has_explicit_self_binding(
            executable,
        )))
        .collect::<Vec<_>>();
    if let Some(binary_http) = request.binary_http() {
        return parameters
            .into_iter()
            .map(|parameter| {
                binary_http_request_parameter_value(
                    request.target(),
                    executable.symbol.as_str(),
                    parameter.name.as_str(),
                    Some(&parameter.ty),
                    projection.type_view(),
                    addr,
                    binary_http,
                    heap,
                )
            })
            .collect();
    }
    if request.payload_encoding() != RequestPayloadEncoding::RuntimeBinary {
        return Err(RuntimeError::InvalidArtifact(
            "canonical assembly ingress does not accept recoverable task dispatch payloads".to_string(),
        ));
    }
    if parameters.is_empty() && request.payload_bytes().is_empty() {
        return Ok(Vec::new());
    }
    let args_plan = executable_request_payload_plan(projection.type_view(), addr, executable)?;
    let decoded = decode_payload_plan(
        request.payload_bytes(),
        &args_plan,
        &PayloadBoundary::external_untrusted(PayloadBoundaryKind::InboundServiceCall),
        heap,
    )?;
    let RuntimeValue::Heap(handle) = decoded else {
        return Err(RuntimeError::Decode(
            "decoded canonical ingress payload must be an args record".to_string(),
        ));
    };
    let HeapNode::Object(object) = heap.get(handle)? else {
        return Err(RuntimeError::Decode(
            "decoded canonical ingress payload must be an args record".to_string(),
        ));
    };
    parameters
        .into_iter()
        .map(|parameter| {
            object
                .fields()
                .get(&parameter.name)
                .cloned()
                .ok_or_else(|| RuntimeError::Protocol {
                    target: request.target().to_string(),
                    message: format!(
                        "missing required canonical ingress parameter {}",
                        parameter.name
                    ),
                })
        })
        .collect()
}

#[cfg(test)]
mod tests;
