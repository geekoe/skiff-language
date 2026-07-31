use super::*;

#[test]
fn generated_service_deployment_authoring_accepts_path_only_websocket() {
    let websocket = serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(
        r#"
path: /chat
"#,
    )
    .unwrap();
    assert_eq!(websocket.path, "/chat");
    assert!(websocket.connect.is_none());
    assert!(websocket.json_rpc.is_empty());
}

#[test]
fn generated_service_deployment_rejects_legacy_websocket_operation_ingress() {
    let error = serde_yaml::from_str::<WebSocketGatewayDocumentAuthoring>(
        r#"
routes:
  - path: /chat
    operation: receive
"#,
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("unknown field `routes`"), "{message}");
}
