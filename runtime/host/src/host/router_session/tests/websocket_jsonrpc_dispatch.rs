use std::time::Duration;

use serde_json::{json, Value};
use skiff_artifact_model::WebSocketEntryId;
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::{
        decode_typed_binary_frame, encode_binary_frame, RequestCancelFrameHeader, TypedEnvelope,
        RUNTIME_FRAME_SCHEMA_VERSION,
    },
    runtime_assembly_request::{
        decode_runtime_assembly_websocket_jsonrpc_response_end_frame,
        RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestDeadlineFrameHeader,
        RuntimeAssemblyRequestStartFrameWireHeader, RuntimeAssemblyRequestTraceFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressProtocol,
        RuntimeAssemblyWebSocketJsonRpcIngressFrameHeader, RuntimeAssemblyWebSocketJsonRpcProfile,
        RuntimeAssemblyWebSocketJsonRpcRequestFrameHeader,
        RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome,
        RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader,
    },
    websocket_generation_lifecycle::{
        WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleOperation,
        WebSocketGenerationLifecycleSender, WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
    },
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::{
    sync::mpsc,
    time::{sleep, timeout},
};

use crate::{host::RuntimeHost, loader::assembly_admission::ActiveAssemblyRoute};

use super::runtime_assembly_request::fixture;

const ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:test-session";
const OTHER_ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:other-session";
const CONNECTION_ID: &str = "host-websocket-jsonrpc-connection";

#[tokio::test]
async fn websocket_jsonrpc_host_maps_typed_outcomes_and_preserves_success_payloads() {
    let fixture::ReloadedWebSocketGatewayHost {
        host,
        physical_a,
        methods_a,
        physical_b,
        ..
    } = pinned_host().await;
    assert_eq!(physical_a.generation(), 1);
    assert_eq!(physical_b.generation(), 2);

    let (outcome, payload) = invoke(
        &host,
        &physical_a,
        &methods_a["result.record"],
        "host-jsonrpc-record",
        br#"{"value":"record-value"}"#,
        None,
        Some("business-record"),
    )
    .await;
    assert_eq!(
        outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&payload).unwrap(),
        json!({"value": "record-value", "accepted": true})
    );

    let (outcome, payload) = invoke(
        &host,
        &physical_a,
        &methods_a["params.array"],
        "host-jsonrpc-array",
        br#"["first","second"]"#,
        None,
        None,
    )
    .await;
    assert_eq!(
        outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&payload).unwrap(),
        json!(["first", "second"])
    );

    let (outcome, payload) = invoke(
        &host,
        &physical_a,
        &methods_a["result.void"],
        "host-jsonrpc-void",
        br#"{"value":"ignored"}"#,
        None,
        None,
    )
    .await;
    assert_eq!(
        outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
    );
    assert_eq!(payload, b"null");

    let (outcome, payload) = invoke(
        &host,
        &physical_a,
        &methods_a["result.expectedFailure"],
        "host-jsonrpc-business-failure",
        br#"{"value":"expected-reason"}"#,
        None,
        None,
    )
    .await;
    assert_eq!(
        outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&payload).unwrap(),
        json!({"tag": "expectedFailure", "reason": "expected-reason"})
    );

    let (outcome, payload) = invoke(
        &host,
        &physical_a,
        &methods_a["status.get"],
        "host-jsonrpc-invalid-params",
        br#"{"value":7}"#,
        None,
        None,
    )
    .await;
    assert_eq!(
        outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::InvalidParams
    );
    assert!(payload.is_empty());

    let (outcome, payload) = invoke(
        &host,
        &physical_a,
        &methods_a["result.throw"],
        "host-jsonrpc-private-throw",
        br#"{"value":"throw"}"#,
        None,
        None,
    )
    .await;
    assert_eq!(
        outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::InternalError
    );
    assert!(payload.is_empty());

    let (outcome, payload) = invoke(
        &host,
        &physical_a,
        &methods_a["result.slow"],
        "host-jsonrpc-deadline",
        br#"{"value":"late"}"#,
        Some(deadline(10, 5_000)),
        None,
    )
    .await;
    assert_eq!(
        outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::DeadlineExceeded
    );
    assert!(payload.is_empty());

    assert_host_drained(&host).await;
}

#[tokio::test]
async fn websocket_jsonrpc_host_uses_header_identity_not_peer_params() {
    let fixture::ReloadedWebSocketGatewayHost {
        host,
        physical_a,
        methods_a,
        ..
    } = pinned_host().await;
    let (outcome, payload) = invoke(
        &host,
        &physical_a,
        &methods_a["identity.read"],
        "host-jsonrpc-business-identity",
        br#"{"connectionId":"peer-spoofed-connection","businessIdentity":"peer-spoofed-business"}"#,
        None,
        Some("trusted-header-business"),
    )
    .await;
    assert_eq!(
        outcome,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&payload).unwrap(),
        json!({
            "connectionId": CONNECTION_ID,
            "businessIdentity": "trusted-header-business",
            "peerConnectionId": "peer-spoofed-connection",
            "peerBusinessIdentity": "peer-spoofed-business",
        })
    );
    assert_host_drained(&host).await;
}

#[tokio::test]
async fn websocket_jsonrpc_host_rejects_wrong_pinned_tuple_before_eval() {
    let fixture::ReloadedWebSocketGatewayHost {
        host,
        physical_a,
        methods_a,
        physical_b,
        ..
    } = pinned_host().await;
    let status = &methods_a["status.get"];
    let base = jsonrpc_header(&physical_a, status, "host-jsonrpc-reject-base", None, None);

    let mut wrong_connection = base.clone();
    wrong_connection.request_id = "host-jsonrpc-reject-connection".to_string();
    wrong_connection.websocket_json_rpc.connection_id = "missing-connection".to_string();
    assert_wire_rejection(&host, wrong_connection).await;

    let mut wrong_identity = base.clone();
    wrong_identity.request_id = "host-jsonrpc-reject-assembly".to_string();
    wrong_identity.routing.assembly_identity = physical_b.assembly_identity().clone();
    assert_wire_rejection(&host, wrong_identity).await;

    let mut wrong_generation = base.clone();
    wrong_generation.request_id = "host-jsonrpc-reject-generation".to_string();
    wrong_generation.routing.assembly_generation += 1;
    assert_wire_rejection(&host, wrong_generation).await;

    let wrong_websocket_entry_id = WebSocketEntryId::parse(format!(
        "skiff-websocket-entry-v1:sha256:{}",
        "e".repeat(64)
    ))
    .unwrap();
    let mut wrong_physical = base.clone();
    wrong_physical.request_id = "host-jsonrpc-reject-physical".to_string();
    wrong_physical.websocket_json_rpc.websocket_entry_id = wrong_websocket_entry_id;
    assert_wire_rejection(&host, wrong_physical).await;

    let mut wrong_deployment = base.clone();
    wrong_deployment.request_id = "host-jsonrpc-reject-deployment".to_string();
    wrong_deployment.routing.deployment.service_id =
        "example.com/other-websocket-service".to_string();
    assert_wire_rejection(&host, wrong_deployment).await;

    let mut wrong_path = base.clone();
    wrong_path.request_id = "host-jsonrpc-reject-path".to_string();
    wrong_path.routing.ingress.path = "/wrong".to_string();
    assert_wire_rejection(&host, wrong_path).await;

    let mut wrong_method = base.clone();
    wrong_method.request_id = "host-jsonrpc-reject-method".to_string();
    wrong_method.routing.ingress.method = "status.missing".to_string();
    assert_wire_rejection(&host, wrong_method).await;

    let record_identity = methods_a["result.record"].gateway_entry_identity().clone();
    let mut wrong_method_identity = base.clone();
    wrong_method_identity.request_id = "host-jsonrpc-reject-method-identity".to_string();
    wrong_method_identity.routing.gateway_entry_identity = record_identity.clone();
    wrong_method_identity
        .websocket_json_rpc
        .gateway_entry_identity = record_identity;
    assert_wire_rejection(&host, wrong_method_identity).await;

    let wrong_session = with_request_id(&base, "host-jsonrpc-reject-session");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let bootstrap = super::super::test_connection_bootstrap("jsonrpc-dispatch").unwrap();
    host.spawn_runtime_assembly_request(
        OTHER_ROUTER_SESSION,
        RuntimeAssemblyRequestStartFrameWireHeader::WebSocketJsonRpc(wrong_session),
        br#"{"value":"must-not-run"}"#.to_vec(),
        &bootstrap,
        sender,
    )
    .await;
    assert_ordinary_rejection(&mut receiver).await;

    let mut wrong_profile =
        serde_json::to_value(with_request_id(&base, "host-jsonrpc-reject-profile")).unwrap();
    wrong_profile["websocketJsonRpc"]["profile"] = Value::String("jsonrpc-1.0".to_string());
    let frame = encode_binary_frame(&wrong_profile, br#"{"value":"must-not-run"}"#).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let error = dispatch(&host, &frame, &sender)
        .await
        .expect_err("strict decoder must reject a non-canonical profile");
    assert!(
        error.to_string().contains("jsonrpc-1.0"),
        "wrong profile rejection: {error}"
    );
    assert!(receiver.try_recv().is_err());

    assert_host_drained(&host).await;
}

#[tokio::test]
async fn websocket_jsonrpc_host_cancel_is_silent_and_late_completion_cannot_write() {
    let fixture::ReloadedWebSocketGatewayHost {
        host,
        physical_a,
        methods_a,
        ..
    } = pinned_host().await;

    for (suffix, deadline_header, reason) in [
        ("peer", None, "peer_disconnect"),
        ("deadline-race", Some(deadline(50, 5_000)), "caller_cancel"),
    ] {
        let request_id = format!("host-jsonrpc-cancel-{suffix}");
        let header = jsonrpc_header(
            &physical_a,
            &methods_a["result.slow"],
            &request_id,
            deadline_header,
            None,
        );
        let frame = encode_binary_frame(&header, br#"{"value":"late-completion"}"#).unwrap();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        dispatch(&host, &frame, &sender).await.unwrap();
        wait_for_active_requests(&host, 1).await;

        let cancel = encode_binary_frame(
            &RequestCancelFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
                envelope_type: "request.cancel".to_string(),
                request_id,
                reason: reason.to_string(),
            },
            &[],
        )
        .unwrap();
        dispatch(&host, &cancel, &sender).await.unwrap();
        wait_for_active_requests(&host, 0).await;
        assert_no_frame(&mut receiver, Duration::from_millis(300)).await;
        assert_host_drained(&host).await;
    }
}

#[tokio::test]
async fn websocket_jsonrpc_host_send_failure_and_session_close_leave_no_leases() {
    let fixture::ReloadedWebSocketGatewayHost {
        host,
        physical_a,
        methods_a,
        ..
    } = pinned_host().await;
    let header = jsonrpc_header(
        &physical_a,
        &methods_a["result.slow"],
        "host-jsonrpc-session-close",
        None,
        None,
    );
    let frame = encode_binary_frame(&header, br#"{"value":"send-fails"}"#).unwrap();
    let (sender, receiver) = mpsc::unbounded_channel();
    dispatch(&host, &frame, &sender).await.unwrap();
    wait_for_active_requests(&host, 1).await;

    drop(receiver);
    drop(sender);
    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .expect("captured Router session disconnect");
    wait_for_active_requests(&host, 0).await;

    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
    assert_host_drained(&host).await;
}

#[test]
fn websocket_jsonrpc_host_dispatch_source_has_no_current_assembly_lookup() {
    let wire = include_str!("../../request_entry/assembly_wire.rs");
    let start = wire
        .find("fn websocket_jsonrpc_request_from_wire(")
        .expect("Host WebSocket JSON-RPC wire admission");
    let end = wire[start..]
        .find("#[cfg(test)]")
        .map(|offset| start + offset)
        .expect("wire admission terminator");
    let admission = &wire[start..end];
    assert!(admission.contains(".websocket_jsonrpc_execution_route("));
    for forbidden in [
        "lookup_active_assembly",
        "assembly_admission",
        "active_runtime_assembly_route",
    ] {
        assert!(
            !admission.contains(forbidden),
            "pinned JSON-RPC admission must not query current assembly: {forbidden}"
        );
    }

    let dispatch = include_str!("../../request_entry/websocket_jsonrpc.rs");
    assert!(dispatch.contains("execute_runtime_websocket_jsonrpc("));
    assert!(dispatch.contains("complete_success("));
    assert!(dispatch.contains("complete_cancelled("));

    let assembly = include_str!("../../request_entry/assembly.rs");
    let context_start = assembly
        .find("fn runtime_assembly_eval_adapter_context(")
        .expect("shared assembly execution context input");
    let context_end = assembly[context_start..]
        .find("fn websocket_connect_telemetry_context(")
        .map(|offset| context_start + offset)
        .expect("shared context input terminator");
    let context = &assembly[context_start..context_end];
    for required in [
        "route.activation()",
        "route.execution_image()",
        "route.service_protocol_identity()",
        ".db_source()",
    ] {
        assert!(
            context.contains(required),
            "shared execution context must use old method route fact: {required}"
        );
    }
    assert!(!context.contains("unavailable()"));
}

async fn pinned_host() -> fixture::ReloadedWebSocketGatewayHost {
    let loaded = fixture::reloaded_websocket_gateway_host().await;
    loaded
        .host
        .websocket_generations
        .connect(ROUTER_SESSION)
        .unwrap();
    let websocket_entry_id = websocket_entry_id(&loaded.physical_a);
    let acquire = loaded
        .host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            loaded.physical_a.clone(),
            websocket_entry_id.as_str().to_string(),
            CONNECTION_ID.to_string(),
        )
        .unwrap();
    loaded
        .host
        .websocket_generations
        .handle_acquire_response(&acquire_ack(&acquire))
        .unwrap();
    loaded
}

fn acquire_ack(
    acquire: &WebSocketGenerationLifecycleControl,
) -> WebSocketGenerationLifecycleControl {
    let WebSocketGenerationLifecycleControl::Acquire {
        request_id, tuple, ..
    } = acquire
    else {
        panic!("expected acquire")
    };
    WebSocketGenerationLifecycleControl::Ack {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
        operation: WebSocketGenerationLifecycleOperation::Acquire,
        request_id: request_id.clone(),
        sender: WebSocketGenerationLifecycleSender::Router,
        tuple: tuple.clone(),
    }
}

fn websocket_entry_id(route: &ActiveAssemblyRoute) -> WebSocketEntryId {
    skiff_artifact_identity::websocket_entry_id(
        &route.entry().owner().service_id,
        route.gateway_entry_key(),
    )
    .unwrap()
}

fn jsonrpc_header(
    physical: &ActiveAssemblyRoute,
    method: &ActiveAssemblyRoute,
    request_id: &str,
    deadline: Option<RuntimeAssemblyRequestDeadlineFrameHeader>,
    business_identity: Option<&str>,
) -> RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader {
    let selector = method.selector();
    RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: physical.assembly_identity().clone(),
            assembly_generation: physical.generation(),
            deployment: method.deployment().clone(),
            build_id: None,
            gateway_entry_identity: method.gateway_entry_identity().clone(),
            ingress: RuntimeAssemblyWebSocketJsonRpcIngressFrameHeader {
                protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                method: selector.method.clone().expect("JSON-RPC method selector"),
                path: selector.path.clone(),
            },
        },
        client_session: None,
        deadline,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: format!("span-{request_id}"),
            parent_span_id: None,
            sampled: None,
        },
        websocket_json_rpc: RuntimeAssemblyWebSocketJsonRpcRequestFrameHeader {
            profile: RuntimeAssemblyWebSocketJsonRpcProfile::JsonRpc2_0Text,
            connection_id: CONNECTION_ID.to_string(),
            websocket_entry_id: websocket_entry_id(physical),
            gateway_entry_identity: method.gateway_entry_identity().clone(),
            business_identity: business_identity.map(str::to_string),
        },
        test_effects_enabled: false,
    }
}

fn with_request_id(
    header: &RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
    request_id: &str,
) -> RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader {
    let mut header = header.clone();
    header.request_id = request_id.to_string();
    header.trace.trace_id = format!("trace-{request_id}");
    header.trace.span_id = format!("span-{request_id}");
    header
}

async fn invoke(
    host: &RuntimeHost,
    physical: &ActiveAssemblyRoute,
    method: &ActiveAssemblyRoute,
    request_id: &str,
    params: &[u8],
    deadline: Option<RuntimeAssemblyRequestDeadlineFrameHeader>,
    business_identity: Option<&str>,
) -> (RuntimeAssemblyWebSocketJsonRpcResponseOutcome, Vec<u8>) {
    let header = jsonrpc_header(physical, method, request_id, deadline, business_identity);
    let frame = encode_binary_frame(&header, params).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(host, &frame, &sender).await.unwrap();
    let RouterWriterMessage::Binary(frame) = timeout(Duration::from_secs(10), receiver.recv())
        .await
        .expect("WebSocket JSON-RPC Host response timeout")
        .expect("WebSocket JSON-RPC Host response channel")
    else {
        panic!("WebSocket JSON-RPC response must use binary wire")
    };
    let (response, payload) = decode_runtime_assembly_websocket_jsonrpc_response_end_frame(&frame)
        .expect("typed WebSocket JSON-RPC response.end");
    assert_eq!(response.request_id, request_id);
    assert_no_frame(&mut receiver, Duration::from_millis(25)).await;
    (response.websocket_json_rpc.outcome, payload)
}

async fn assert_wire_rejection(
    host: &RuntimeHost,
    header: RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
) {
    let frame = encode_binary_frame(&header, br#"{"value":"must-not-run"}"#).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(host, &frame, &sender).await.unwrap();
    assert_ordinary_rejection(&mut receiver).await;
}

async fn assert_ordinary_rejection(receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>) {
    let RouterWriterMessage::Binary(frame) = timeout(Duration::from_secs(2), receiver.recv())
        .await
        .expect("Host admission rejection timeout")
        .expect("Host admission rejection channel")
    else {
        panic!("Host admission rejection must use binary wire")
    };
    let (typed, _): (TypedEnvelope, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("ordinary Host rejection frame");
    assert_eq!(typed.envelope_type, "response.error");
    assert_no_frame(receiver, Duration::from_millis(25)).await;
}

async fn dispatch(
    host: &RuntimeHost,
    frame: &[u8],
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
) -> crate::error::Result<()> {
    let mut control = None;
    let mut fingerprint = None;
    super::dispatch_router_binary_frame(host, frame, sender, &mut control, &mut fingerprint).await
}

async fn wait_for_active_requests(host: &RuntimeHost, expected: usize) {
    timeout(Duration::from_secs(3), async {
        loop {
            if host.request_supervisor.active_count().await == expected {
                break;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Host request supervisor did not reach {expected} active requests"));
}

async fn assert_no_frame(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    wait: Duration,
) {
    match timeout(wait, receiver.recv()).await {
        Err(_) | Ok(None) => {}
        Ok(Some(message)) => panic!("unexpected second/ordinary response frame: {message:?}"),
    }
}

async fn assert_host_drained(host: &RuntimeHost) {
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_eq!(host.connection_requests.pending_count(), 0);
    assert_eq!(host.connection_requests.active_lease_count(), 0);
    assert_eq!(host.connection_requests.active_timer_count(), 0);
    assert_eq!(host.outbound_requests.pending_count(), 0);
    assert_eq!(host.outbound_requests.active_lease_count(), 0);
}

fn deadline(timeout_ms: u64, expires_in_ms: i64) -> RuntimeAssemblyRequestDeadlineFrameHeader {
    RuntimeAssemblyRequestDeadlineFrameHeader {
        timeout_ms,
        expires_at: (OffsetDateTime::now_utc() + time::Duration::milliseconds(expires_in_ms))
            .format(&Rfc3339)
            .unwrap(),
    }
}
