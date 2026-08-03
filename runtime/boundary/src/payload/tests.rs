use super::*;

#[test]
fn payload_trust_is_derived_from_kind_and_target_service() {
    assert_eq!(
        PayloadBoundary::owner_internal(PayloadBoundaryKind::TaskDispatchPayload).trust(),
        PayloadTrust::OwnerInternal
    );
    assert_eq!(
        PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload).trust(),
        PayloadTrust::OwnerInternal
    );
    assert_eq!(
        PayloadBoundary::owner_internal(PayloadBoundaryKind::RuntimeWirePayload)
            .with_target_service(PayloadServiceRef::new("skiff.run/account"))
            .trust(),
        PayloadTrust::CrossService
    );
    assert_eq!(
        PayloadBoundary::cross_service(
            PayloadBoundaryKind::OutboundServiceCall,
            PayloadServiceRef::new("skiff.run/account")
        )
        .trust(),
        PayloadTrust::CrossService
    );
    assert_eq!(
        PayloadBoundary::external_untrusted(PayloadBoundaryKind::ServiceResponse).trust(),
        PayloadTrust::ExternalUntrusted
    );
}

#[test]
fn payload_boundary_diagnostic_label_contains_kind_trust_and_services() {
    let boundary = PayloadBoundary::cross_service(
        PayloadBoundaryKind::OutboundServiceCall,
        PayloadServiceRef::new("skiff.run/registry").with_version("0.1.0"),
    )
    .with_origin_service(PayloadServiceRef::new("skiff.run/account").with_build_id("build-1"));

    let label = boundary.diagnostic_label();
    assert!(label.contains("kind=OutboundServiceCall"));
    assert!(label.contains("trust=CrossService"));
    assert!(label.contains("origin=skiff.run/account#build-1"));
    assert!(label.contains("target=skiff.run/registry@0.1.0"));
}
