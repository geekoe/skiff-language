use super::*;

#[test]
fn canonical_http_shapes_are_closed_and_exact() {
    let ContractTypeRef::Record { fields } =
        canonical_http_boundary_type(HTTP_REQUEST_TYPE).expect("request shape")
    else {
        panic!("request must be a record")
    };
    assert_eq!(
        fields.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["body", "headers", "method", "path", "query", "url"]
    );

    let ContractTypeRef::StructuralUnion { variants } =
        canonical_http_boundary_type(HTTP_RESPONSE_STREAM_EVENT_TYPE).expect("stream shape")
    else {
        panic!("stream event must be a union")
    };
    assert_eq!(variants.len(), 3);
    assert!(canonical_http_boundary_type("std.http.HttpClientRequest").is_none());
}

#[test]
fn canonical_imported_http_identity_requires_the_official_package_and_symbol() {
    let symbol = |package: &str, symbol_path: &str| PackageSymbolRef {
        package: PackageRefIr::PackageId {
            package_id: package.to_string(),
        },
        symbol_path: symbol_path.to_string(),
        abi_expectation: None,
    };
    assert_eq!(
        canonical_http_boundary_symbol(&symbol(HTTP_BOUNDARY_PACKAGE_ID, HTTP_REQUEST_TYPE)),
        Some(HTTP_REQUEST_TYPE)
    );
    assert!(
        canonical_http_boundary_symbol(&symbol("example.com/std", HTTP_REQUEST_TYPE)).is_none()
    );
    assert!(canonical_http_boundary_symbol(&symbol(
        HTTP_BOUNDARY_PACKAGE_ID,
        "std.http.HttpClientRequest"
    ))
    .is_none());
}
