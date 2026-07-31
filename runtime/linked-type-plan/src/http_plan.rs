use skiff_runtime_boundary::type_descriptor::bare_type_name;
use skiff_runtime_linked_program::{ExecutableAddr, LinkedTypeRef, PackageRefIr, UnitAddr};
use skiff_runtime_model::type_plan::{RuntimeTypeNode, RuntimeTypePlan};

use crate::{
    error::{Error, Result},
    type_plan::{PlanContext, ProgramTypeView, RuntimeTypePlanLinkedExt},
};

const HTTP_REQUEST_TYPE: &str = "std.http.HttpRequest";
const HTTP_RESPONSE_TYPE: &str = "std.http.HttpResponse";
const HTTP_RESPONSE_STREAM_EVENT_TYPE: &str = "std.http.HttpResponseStreamEvent";

pub fn binary_http_request_parameter_plan<'p>(
    target: &str,
    executable_symbol: &str,
    parameter_name: &str,
    expected_type: Option<&LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
) -> Result<RuntimeTypePlan> {
    let expected_type = expected_type.ok_or_else(|| {
        Error::InvalidArtifact(format!(
            "request parameter {executable_symbol}.{parameter_name} is missing expected type"
        ))
    })?;
    let program = program.into();
    let plan = RuntimeTypePlan::from_linked(
        expected_type,
        &PlanContext::from_type_view(program, executable_addr),
    )?;
    if !linked_or_planned_type_matches_http_nominal_type(
        expected_type,
        &plan,
        program,
        HTTP_REQUEST_TYPE,
    ) {
        return Err(Error::Protocol {
            target: target.to_string(),
            message: format!(
                "binary HTTP request parameter {parameter_name} must be std.http.HttpRequest"
            ),
        });
    }
    Ok(plan)
}

pub fn binary_http_response_plan<'p>(
    expected_type: Option<&LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
) -> Result<RuntimeTypePlan> {
    let expected_type = expected_type.ok_or_else(|| {
        Error::InvalidArtifact("HTTP response boundary is missing return type".to_string())
    })?;
    let program = program.into();
    let plan = RuntimeTypePlan::from_linked(
        expected_type,
        &PlanContext::from_type_view(program, executable_addr),
    )?;
    let response_plan = nullable_plan_inner(&plan).unwrap_or(&plan);
    if !linked_or_planned_type_matches_http_nominal_type(
        expected_type,
        response_plan,
        program,
        HTTP_RESPONSE_TYPE,
    ) {
        return Err(Error::Protocol {
            target: "response.end".to_string(),
            message: "binary HTTP handler must return std.http.HttpResponse".to_string(),
        });
    }
    Ok(response_plan.clone())
}

pub fn linked_http_response_stream_item_type<'a, 'p>(
    return_type: Option<&'a LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
) -> Result<Option<&'a LinkedTypeRef>> {
    let Some(LinkedTypeRef::Native { name, args }) = return_type else {
        return Ok(None);
    };
    if bare_type_name(name) != "Stream" || args.len() != 1 {
        return Ok(None);
    }
    let item_type = &args[0];
    let program = program.into();
    let plan = RuntimeTypePlan::from_linked_nested_ref(
        item_type,
        &PlanContext::from_type_view(program, executable_addr),
    )?;
    Ok(linked_or_planned_type_matches_http_nominal_type(
        item_type,
        &plan,
        program,
        HTTP_RESPONSE_STREAM_EVENT_TYPE,
    )
    .then_some(item_type))
}

pub fn linked_type_ref_is_http_response_stream<'p>(
    return_type: Option<&LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
) -> bool {
    linked_http_response_stream_item_type(return_type, program, executable_addr)
        .ok()
        .flatten()
        .is_some()
}

fn nullable_plan_inner(plan: &RuntimeTypePlan) -> Option<&RuntimeTypePlan> {
    match plan.node() {
        RuntimeTypeNode::Nullable(inner) => Some(inner),
        _ => None,
    }
}

fn plan_matches_nominal_type(plan: &RuntimeTypePlan, expected: &str) -> bool {
    if plan.named_type_name() == Some(expected) || plan.boundary_record_kind() == Some(expected) {
        return true;
    }
    match plan.node() {
        RuntimeTypeNode::Alias(target) => plan_matches_nominal_type(target, expected),
        _ => false,
    }
}

fn linked_or_planned_type_matches_http_nominal_type<'p>(
    linked: &LinkedTypeRef,
    plan: &RuntimeTypePlan,
    program: impl Into<ProgramTypeView<'p>>,
    expected: &str,
) -> bool {
    let program = program.into();
    linked_std_http_package_type_matches(linked, expected)
        || linked_address_resolves_to_std_http_package_type(linked, program, expected)
        || plan_matches_nominal_type(plan, expected)
}

fn linked_std_http_package_type_matches(linked: &LinkedTypeRef, expected: &str) -> bool {
    let LinkedTypeRef::PackageSymbol { symbol } = linked else {
        return false;
    };
    matches!(
        &symbol.package,
        PackageRefIr::PackageId { package_id } if package_id == "skiff.run/std"
    ) && symbol.symbol_path == expected
}

fn linked_address_resolves_to_std_http_package_type(
    linked: &LinkedTypeRef,
    program: ProgramTypeView<'_>,
    expected: &str,
) -> bool {
    let LinkedTypeRef::Address { addr } = linked else {
        return false;
    };
    let UnitAddr::Package(slot) = &addr.unit else {
        return false;
    };
    let Some(package) = program.packages.get(*slot) else {
        return false;
    };
    if package.package_id() != "skiff.run/std" {
        return false;
    }
    let Some(declaration) = program.types.declaration(addr) else {
        return false;
    };
    if declaration.name == http_type_short_name(expected) {
        return true;
    }
    false
}

fn http_type_short_name(expected: &str) -> &str {
    expected.rsplit('.').next().unwrap_or(expected)
}

#[cfg(test)]
mod tests;
