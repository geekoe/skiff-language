#![allow(dead_code)]

use skiff_runtime_boundary::http::{
    self, HttpBoundaryNameValue, HttpBoundaryRequestParts, HttpBoundaryResponseParts,
};
use skiff_runtime_capability_context::{
    binary_http_request_parts, http_name_value_contexts, BinaryHttpRequestContext,
    HttpNameValueContext,
};
use skiff_runtime_linked_program::{ExecutableAddr, LinkedTypeRef};
use skiff_runtime_linked_type_plan::{self as linked_type_plan, ProgramTypeView, RuntimeTypePlan};

use crate::{error::Result, request_heap::RequestHeap, runtime_value::RuntimeValue};

#[cfg(test)]
use crate::type_descriptor::{PlanContext, RuntimeTypePlanLinkedExt};
#[cfg(test)]
use skiff_runtime_boundary::type_descriptor::RuntimeTypeNode;
#[cfg(test)]
use skiff_runtime_linked_program::{LinkedTypeDescriptor, PackageRefIr, UnitAddr};

#[cfg(test)]
const HTTP_REQUEST_TYPE: &str = "std.http.HttpRequest";
#[cfg(test)]
const HTTP_RESPONSE_TYPE: &str = "std.http.HttpResponse";
#[cfg(test)]
const HTTP_RESPONSE_STREAM_EVENT_TYPE: &str = "std.http.HttpResponseStreamEvent";

pub(crate) fn binary_http_request_parameter_value<'p>(
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
    let boundary_plan = http::direct_http_request_coerce_plan(plan);
    Ok(http::direct_http_request_runtime_value(
        &boundary_request_parts_from_binary_http(binary_http),
        &boundary_plan,
        format!("binary HTTP request parameter {parameter_name}"),
        heap,
    )?)
}

pub(crate) fn binary_http_request_parameter_value_with_plan(
    parameter_name: &str,
    plan: &RuntimeTypePlan,
    binary_http: &BinaryHttpRequestContext<'_>,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let boundary_plan = http::direct_http_request_coerce_plan(plan.clone());
    Ok(http::direct_http_request_runtime_value(
        &boundary_request_parts_from_binary_http(binary_http),
        &boundary_plan,
        format!("binary HTTP request parameter {parameter_name}"),
        heap,
    )?)
}

pub(crate) fn binary_http_request_parameter_plan<'p>(
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

pub(crate) fn binary_http_response_from_runtime_value<'p>(
    value: &RuntimeValue,
    expected_type: Option<&LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
    heap: &mut RequestHeap,
) -> Result<HttpBoundaryResponseParts> {
    let response_plan = binary_http_response_plan(expected_type, program, executable_addr)?;
    let boundary_plan = http::direct_http_response_coerce_plan(response_plan);
    let parts = http::direct_http_response_from_runtime_value(
        value,
        &boundary_plan,
        "binary HTTP response",
        heap,
    )?;
    Ok(parts)
}

pub(crate) fn binary_http_response_plan<'p>(
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

pub(crate) fn boundary_request_parts_from_binary_http(
    binary_http: &BinaryHttpRequestContext<'_>,
) -> HttpBoundaryRequestParts {
    binary_http_request_parts(binary_http)
}

pub(crate) fn boundary_name_values_from_request(
    items: &[HttpNameValueContext<'_>],
) -> Vec<HttpBoundaryNameValue> {
    http_name_value_contexts(items)
}

pub(crate) fn linked_http_response_stream_item_type<'a, 'p>(
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

pub(crate) fn linked_type_ref_is_http_response_stream<'p>(
    return_type: Option<&LinkedTypeRef>,
    program: impl Into<ProgramTypeView<'p>>,
    executable_addr: &'p ExecutableAddr,
) -> bool {
    linked_type_plan::linked_type_ref_is_http_response_stream(return_type, program, executable_addr)
}

#[cfg(test)]
fn nullable_plan_inner(plan: &RuntimeTypePlan) -> Option<&RuntimeTypePlan> {
    match plan.node() {
        RuntimeTypeNode::Nullable(inner) => Some(inner),
        _ => None,
    }
}

#[cfg(test)]
fn plan_matches_nominal_type(plan: &RuntimeTypePlan, expected: &str) -> bool {
    if plan.named_type_name() == Some(expected) || plan.boundary_record_kind() == Some(expected) {
        return true;
    }
    match plan.node() {
        RuntimeTypeNode::Alias(target) => plan_matches_nominal_type(target, expected),
        _ => false,
    }
}

#[cfg(test)]
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

#[cfg(test)]
fn linked_std_http_package_type_matches(linked: &LinkedTypeRef, expected: &str) -> bool {
    let LinkedTypeRef::PackageSymbol { symbol } = linked else {
        return false;
    };
    matches!(
        &symbol.package,
        PackageRefIr::PackageId { package_id } if package_id == "skiff.run/std"
    ) && symbol.symbol_path == expected
}

#[cfg(test)]
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
    if package.package_id != "skiff.run/std" {
        return false;
    }
    let Some(declaration) = program.types.declaration(addr) else {
        return false;
    };
    if declaration.name == http_type_short_name(expected) {
        return true;
    }
    matches!(
        &declaration.descriptor,
        LinkedTypeDescriptor::Native { symbol } if symbol == expected
    )
}

#[cfg(test)]
fn http_type_short_name(expected: &str) -> &str {
    expected.rsplit('.').next().unwrap_or(expected)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        sync::Arc,
    };

    use super::*;
    use crate::{
        program::{
            anonymous_type_decl, ExecutableAddr, LinkedProgramImage, LinkedTypeDescriptor,
            LinkedTypeRef, PackageUnit, RuntimeTypeContext, TypeAddr, UnitAddr,
        },
        runtime_value::{RuntimeObject, RuntimeObjectFields},
    };

    #[test]
    fn binary_http_response_requires_linked_std_response_nominal_type() {
        let (program, addr, response_ref, spoof_ref) = http_boundary_program();
        let mut heap = RequestHeap::default();
        let headers_handle = heap
            .alloc_array(Vec::new())
            .expect("headers should allocate");
        let body_handle = heap.alloc_bytes(Vec::new()).expect("body should allocate");
        let response_handle = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                ("status".to_string(), RuntimeValue::Number(204.0)),
                ("headers".to_string(), RuntimeValue::Heap(headers_handle)),
                ("body".to_string(), RuntimeValue::Heap(body_handle)),
            ])))
            .expect("response should allocate");

        let response = binary_http_response_from_runtime_value(
            &RuntimeValue::Heap(response_handle),
            Some(&response_ref),
            &program,
            &addr,
            &mut heap,
        )
        .expect("linked std HttpResponse should be accepted");
        assert_eq!(response.status, 204);

        let error = match binary_http_response_from_runtime_value(
            &RuntimeValue::Heap(response_handle),
            Some(&spoof_ref),
            &program,
            &addr,
            &mut heap,
        ) {
            Ok(_) => panic!("structurally matching non-std response must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("std.http.HttpResponse"));
        assert_std_package_http_request_refs_are_accepted();
    }

    fn assert_std_package_http_request_refs_are_accepted() {
        let std_request = LinkedTypeRef::PackageSymbol {
            symbol: crate::program::PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "skiff.run/std".to_string(),
                },
                symbol_path: HTTP_REQUEST_TYPE.to_string(),
                abi_expectation: None,
            },
        };
        let spoof_request = LinkedTypeRef::PackageSymbol {
            symbol: crate::program::PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "example.com/std-lookalike".to_string(),
                },
                symbol_path: HTTP_REQUEST_TYPE.to_string(),
                abi_expectation: None,
            },
        };

        let (program, package_request_ref, spoof_address_ref) = package_address_program();
        assert!(linked_or_planned_type_matches_http_nominal_type(
            &std_request,
            &RuntimeTypePlan::from_linked(
                &package_request_ref,
                &PlanContext::new(&program, &ExecutableAddr::service(0, 0))
            )
            .expect("std package address should plan"),
            &program,
            HTTP_REQUEST_TYPE
        ));
        assert!(!linked_or_planned_type_matches_http_nominal_type(
            &spoof_request,
            &RuntimeTypePlan::from_linked(
                &spoof_address_ref,
                &PlanContext::new(&program, &ExecutableAddr::service(0, 0))
            )
            .expect("spoof address should plan"),
            &program,
            HTTP_REQUEST_TYPE
        ));
        assert!(linked_or_planned_type_matches_http_nominal_type(
            &package_request_ref,
            &RuntimeTypePlan::from_linked(
                &package_request_ref,
                &PlanContext::new(&program, &ExecutableAddr::service(0, 0))
            )
            .expect("std package address should plan"),
            &program,
            HTTP_REQUEST_TYPE
        ));
        assert!(!linked_or_planned_type_matches_http_nominal_type(
            &spoof_address_ref,
            &RuntimeTypePlan::from_linked(
                &spoof_address_ref,
                &PlanContext::new(&program, &ExecutableAddr::service(0, 0))
            )
            .expect("spoof address should plan"),
            &program,
            HTTP_REQUEST_TYPE
        ));
    }

    #[test]
    fn binary_http_response_reads_erased_payloads() {
        let (program, addr, expected_type, _) = http_boundary_program();
        let mut heap = RequestHeap::default();
        let header_handle = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                (
                    "name".to_string(),
                    RuntimeValue::String("x-test".to_string()),
                ),
                ("value".to_string(), RuntimeValue::String("ok".to_string())),
            ])))
            .expect("header should allocate");
        let headers_handle = heap
            .alloc_array(vec![RuntimeValue::Heap(header_handle)])
            .expect("headers should allocate");
        let body_handle = heap
            .alloc_bytes(vec![1, 2, 3, 4])
            .expect("body should allocate");
        let response_handle = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                ("status".to_string(), RuntimeValue::Number(204.0)),
                ("headers".to_string(), RuntimeValue::Heap(headers_handle)),
                ("body".to_string(), RuntimeValue::Heap(body_handle)),
            ])))
            .expect("response should allocate");

        let response = binary_http_response_from_runtime_value(
            &RuntimeValue::Heap(response_handle),
            Some(&expected_type),
            &program,
            &addr,
            &mut heap,
        )
        .expect("response boundary should read erased payloads");

        assert_eq!(response.status, 204);
        assert_eq!(
            response.headers,
            vec![HttpBoundaryNameValue {
                name: "x-test".to_string(),
                value: "ok".to_string(),
            }]
        );
        assert_eq!(response.body, vec![1, 2, 3, 4]);
    }

    #[test]
    fn binary_http_response_rejects_legacy_skiff_type_metadata() {
        let (program, addr, expected_type, _) = http_boundary_program();
        let mut heap = RequestHeap::default();
        let headers_handle = heap
            .alloc_array(Vec::new())
            .expect("headers should allocate");
        let body_handle = heap.alloc_bytes(Vec::new()).expect("body should allocate");
        let response_handle = heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                ("status".to_string(), RuntimeValue::Number(200.0)),
                ("headers".to_string(), RuntimeValue::Heap(headers_handle)),
                ("body".to_string(), RuntimeValue::Heap(body_handle)),
                (
                    "__skiffType".to_string(),
                    RuntimeValue::String("std.http.HttpResponse".to_string()),
                ),
            ])))
            .expect("response should allocate");

        let error = match binary_http_response_from_runtime_value(
            &RuntimeValue::Heap(response_handle),
            Some(&expected_type),
            &program,
            &addr,
            &mut heap,
        ) {
            Ok(_) => panic!("HTTP response boundary should reject legacy metadata"),
            Err(error) => error,
        };

        assert!(error
            .to_string()
            .contains("reserved Skiff metadata field __skiffType"));
    }

    fn http_boundary_program() -> (
        LinkedProgramImage,
        ExecutableAddr,
        LinkedTypeRef,
        LinkedTypeRef,
    ) {
        let response_addr = TypeAddr {
            unit: UnitAddr::Service,
            file: crate::program::FileAddr::LoadedFileIndex(0),
            type_index: 0,
        };
        let spoof_addr = TypeAddr {
            unit: UnitAddr::Service,
            file: crate::program::FileAddr::LoadedFileIndex(0),
            type_index: 1,
        };
        let header_addr = TypeAddr {
            unit: UnitAddr::Service,
            file: crate::program::FileAddr::LoadedFileIndex(0),
            type_index: 2,
        };
        let mut types = RuntimeTypeContext::default();
        types.descriptors.insert(
            header_addr.clone(),
            anonymous_type_decl(
                "std.http.HttpHeader",
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::from([
                        (
                            "name".to_string(),
                            LinkedTypeRef::Native {
                                name: "string".to_string(),
                                args: Vec::new(),
                            },
                        ),
                        (
                            "value".to_string(),
                            LinkedTypeRef::Native {
                                name: "string".to_string(),
                                args: Vec::new(),
                            },
                        ),
                    ]),
                },
            ),
        );
        let response_fields = BTreeMap::from([
            (
                "status".to_string(),
                LinkedTypeRef::Native {
                    name: "integer".to_string(),
                    args: Vec::new(),
                },
            ),
            (
                "headers".to_string(),
                LinkedTypeRef::Native {
                    name: "Array".to_string(),
                    args: vec![LinkedTypeRef::Address { addr: header_addr }],
                },
            ),
            (
                "body".to_string(),
                LinkedTypeRef::Native {
                    name: "bytes".to_string(),
                    args: Vec::new(),
                },
            ),
        ]);
        types.descriptors.insert(
            response_addr.clone(),
            anonymous_type_decl(
                "std.http.HttpResponse",
                LinkedTypeDescriptor::Record {
                    fields: response_fields.clone(),
                },
            ),
        );
        types.descriptors.insert(
            spoof_addr.clone(),
            anonymous_type_decl(
                "local.HttpResponse",
                LinkedTypeDescriptor::Record {
                    fields: response_fields,
                },
            ),
        );
        (
            LinkedProgramImage {
                service_files: Vec::new(),
                packages: Vec::new(),
                package_files: Vec::new(),
                routes: HashMap::new(),
                spawn_routes: HashMap::new(),
                operations: HashMap::new(),
                operation_receivers: HashMap::new(),
                link_overlay: Default::default(),
                types,
            },
            ExecutableAddr::service(0, 0),
            LinkedTypeRef::Address {
                addr: response_addr,
            },
            LinkedTypeRef::Address { addr: spoof_addr },
        )
    }

    fn package_address_program() -> (LinkedProgramImage, LinkedTypeRef, LinkedTypeRef) {
        let request_addr = TypeAddr {
            unit: UnitAddr::Package(0),
            file: crate::program::FileAddr::LoadedFileIndex(0),
            type_index: 0,
        };
        let spoof_addr = TypeAddr {
            unit: UnitAddr::Package(1),
            file: crate::program::FileAddr::LoadedFileIndex(0),
            type_index: 0,
        };
        let mut types = RuntimeTypeContext::default();
        types.descriptors.insert(
            request_addr.clone(),
            anonymous_type_decl(
                "HttpRequest",
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
            ),
        );
        types.descriptors.insert(
            spoof_addr.clone(),
            anonymous_type_decl(
                "HttpRequest",
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::new(),
                },
            ),
        );
        (
            LinkedProgramImage {
                service_files: Vec::new(),
                packages: vec![
                    Arc::new(PackageUnit::empty(
                        "skiff.run/std",
                        "1.0.0",
                        "build:std",
                        "abi:std",
                    )),
                    Arc::new(PackageUnit::empty(
                        "example.com/std-lookalike",
                        "1.0.0",
                        "build:spoof",
                        "abi:spoof",
                    )),
                ],
                package_files: vec![Vec::new(), Vec::new()],
                routes: HashMap::new(),
                spawn_routes: HashMap::new(),
                operations: HashMap::new(),
                operation_receivers: HashMap::new(),
                link_overlay: Default::default(),
                types,
            },
            LinkedTypeRef::Address { addr: request_addr },
            LinkedTypeRef::Address { addr: spoof_addr },
        )
    }
}
