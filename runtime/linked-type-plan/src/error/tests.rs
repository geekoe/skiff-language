use super::*;

#[test]
fn linked_type_plan_protocol_payload_and_catch_projection_are_service_protocol() {
    let error = Error::Protocol {
        target: "svc.account".to_string(),
        message: "bad request payload".to_string(),
    };

    let payload = error.payload();
    assert_eq!(payload.code, "std.service.ProtocolError");
    assert_eq!(
        payload.details,
        Some(json!({
            "target": "svc.account",
            "message": "bad request payload",
        }))
    );
    assert_eq!(
        error.catch_projection(),
        Some((
            PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
            json!({
                "target": "svc.account",
                "message": "bad request payload",
            })
        ))
    );
}

#[test]
fn linked_type_plan_boundary_delegates_payload_and_catch_projection() {
    let boundary = skiff_runtime_boundary::error::RuntimeError::file_error("std.file denied");
    let expected_payload = boundary.payload();
    let expected_catch_projection = boundary.catch_projection();
    let error = Error::from(boundary);

    assert_eq!(error.payload(), expected_payload);
    assert_eq!(error.catch_projection(), expected_catch_projection);
}

#[test]
fn linked_type_plan_diagnostics_remain_uncatchable() {
    let invalid_artifact = Error::InvalidArtifact("missing linked type".to_string());
    assert_eq!(invalid_artifact.catch_projection(), None);

    let diagnostic = Error::from(skiff_runtime_boundary::error::RuntimeError::Decode(
        "ordinary boundary diagnostic".to_string(),
    ));
    assert_eq!(diagnostic.catch_projection(), None);
}
