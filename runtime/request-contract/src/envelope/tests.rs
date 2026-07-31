use serde_json::json;

use super::RequestEnvelope;

#[test]
fn request_start_text_json_deserialize_fails_closed() {
    let error = serde_json::from_value::<RequestEnvelope>(json!({
            "requestId": "request-1",
            "mode": "unary",
            "target": "service.example~com~~service-a.Api.hello",
            "buildId": "skiff-service-build-v1:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            "args": {}
        }))
        .expect_err("text protocol request.start should fail closed");

    assert!(error
        .to_string()
        .contains("text protocol messages are not supported on runtime WebSocket"));
}
