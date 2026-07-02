use skiff_runtime_boundary::http::{self, HttpBoundaryResponseParts};
use skiff_runtime_capability_context::{binary_http_request_parts, BinaryHttpRequestContext};
use skiff_runtime_linked_program::{ExecutableAddr, LinkedTypeRef};
use skiff_runtime_linked_type_plan::{self as linked_type_plan, ProgramTypeView, RuntimeTypePlan};
use skiff_runtime_model::{request_heap::RequestHeap, runtime_value::RuntimeValue};

use crate::error::Result;

pub fn binary_http_request_parameter_plan<'p>(
    target: &str,
    executable_symbol: &str,
    parameter_name: &str,
    expected_type: Option<&LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
) -> Result<RuntimeTypePlan> {
    Ok(linked_type_plan::binary_http_request_parameter_plan(
        target,
        executable_symbol,
        parameter_name,
        expected_type,
        program,
        executable_addr,
    )?)
}

pub fn binary_http_request_parameter_value<'p>(
    target: &str,
    executable_symbol: &str,
    parameter_name: &str,
    expected_type: Option<&LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
    binary_http: &BinaryHttpRequestContext<'_>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let plan = binary_http_request_parameter_plan(
        target,
        executable_symbol,
        parameter_name,
        expected_type,
        program,
        executable_addr,
    )?;
    binary_http_request_parameter_value_with_plan(parameter_name, &plan, binary_http, heap)
}

pub fn binary_http_request_parameter_value_with_plan(
    parameter_name: &str,
    plan: &RuntimeTypePlan,
    binary_http: &BinaryHttpRequestContext<'_>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let boundary_plan = http::direct_http_request_coerce_plan(plan.clone());
    Ok(http::direct_http_request_runtime_value(
        &binary_http_request_parts(binary_http),
        &boundary_plan,
        format!("binary HTTP request parameter {parameter_name}"),
        heap,
    )?)
}

pub fn binary_http_response_plan<'p>(
    expected_type: Option<&LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
) -> Result<RuntimeTypePlan> {
    Ok(linked_type_plan::binary_http_response_plan(
        expected_type,
        program,
        executable_addr,
    )?)
}

pub fn binary_http_response_from_runtime_value<'p>(
    value: &RuntimeValue,
    expected_type: Option<&LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
    heap: &mut RequestHeap,
) -> Result<HttpBoundaryResponseParts> {
    let response_plan = binary_http_response_plan(expected_type, program, executable_addr)?;
    let boundary_plan = http::direct_http_response_coerce_plan(response_plan);
    Ok(http::direct_http_response_from_runtime_value(
        value,
        &boundary_plan,
        "binary HTTP response",
        heap,
    )?)
}

pub fn linked_http_response_stream_item_type<'a, 'p>(
    return_type: Option<&'a LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
) -> Result<Option<&'a LinkedTypeRef>> {
    Ok(linked_type_plan::linked_http_response_stream_item_type(
        return_type,
        program,
        executable_addr,
    )?)
}
