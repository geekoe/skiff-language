use std::{collections::HashMap, sync::Arc, time::Duration};

use serde_json::{json, Value};
use skiff_artifact_model::{AssemblyIdentity, ContractOperationId, IngressProtocol};
use skiff_runtime_request::{OutboundControlMessage, RouterWriterMessage};
use skiff_runtime_transport::{
    protocol::{
        decode_typed_binary_frame, encode_binary_frame, RequestCancelFrameHeader,
        ResponseEndFrameHeader, ResponseEndFrameMetadata, ResponseErrorFrameHeader,
        SpawnSubmitResponseFrameHeader, TypedEnvelope, BINARY_FRAME_HEADER_ENCODING_JSON,
        BINARY_FRAME_MAGIC, BINARY_FRAME_VERSION, RUNTIME_FRAME_SCHEMA_VERSION,
    },
    runtime_assembly_request::{
        RuntimeAssemblyHttpAdapterCallableFrameHeader, RuntimeAssemblyHttpAdapterFrameHeader,
        RuntimeAssemblyHttpAdapterKindFrameHeader, RuntimeAssemblyHttpRequestFrameHeader,
        RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestIngressFrameHeader,
        RuntimeAssemblyRequestIngressProtocol, RuntimeAssemblyRequestRoutingFrameHeader,
        RuntimeAssemblyRequestStartFrameHeader, RuntimeAssemblyRequestTestEffectDoubleFrameHeader,
        RuntimeAssemblyRequestTraceFrameHeader,
    },
};
use tokio::{sync::mpsc, time::timeout};

use crate::{host::RuntimeHost, loader::assembly_admission::ActiveAssemblyRoute};

pub(super) mod fixture;

#[tokio::test]
async fn runtime_assembly_spawn_continuation_resumes_on_correlated_submitted_receipt() {
    let (host, route) = fixture::admitted_spawn_host().await;
    let header = canonical_header(&route, "runtime-assembly-spawn-continuation");
    let frame = encode_binary_frame(&header, &[]).expect("canonical request.start should encode");
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender)
        .await
        .expect("canonical spawn request.start should dispatch");

    let outbound = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("spawn.submit request timeout")
        .expect("router writer channel closed");
    let RouterWriterMessage::Control(OutboundControlMessage::SpawnSubmit {
        request,
        payload: _,
    }) = outbound
    else {
        panic!("canonical eval must emit spawn.submit.request, got {outbound:?}")
    };
    assert_eq!(
        request.activation_identity.assembly_identity,
        *route.assembly_identity()
    );
    assert_eq!(request.activation_identity.generation, route.generation());
    assert_eq!(
        request.activation_identity.runtime_replica_id,
        route.activation().identity().runtime_replica_id
    );
    assert_eq!(
        request.activation_identity.deployment_revision,
        route.activation().identity().deployment.deployment_revision
    );
    assert_eq!(
        request.caller_request_id.as_deref(),
        Some(header.request_id.as_str())
    );
    assert_eq!(host.outbound_requests.pending_count(), 1);
    assert_eq!(host.outbound_requests.active_lease_count(), 1);

    let wrong_receipt = spawn_submitted_receipt("rpc:wrong-f50a");
    let wrong_frame =
        encode_binary_frame(&wrong_receipt, &[]).expect("wrong-rpc receipt should encode");
    dispatch(&host, &wrong_frame, &sender)
        .await
        .expect("wrong-rpc receipt should be ignored by the same dispatcher");
    assert!(
        timeout(Duration::from_millis(25), receiver.recv())
            .await
            .is_err(),
        "wrong rpcId must not resume the request"
    );
    assert_eq!(host.outbound_requests.pending_count(), 1);
    assert_eq!(host.request_supervisor.active_count().await, 1);

    let receipt = spawn_submitted_receipt(&request.rpc_id);
    let receipt_frame =
        encode_binary_frame(&receipt, &[]).expect("correlated submitted receipt should encode");
    dispatch(&host, &receipt_frame, &sender)
        .await
        .expect("correlated receipt should dispatch");

    let Terminal::End(response, payload) = recv_terminal(&mut receiver).await else {
        panic!("spawn continuation must complete with response.end")
    };
    assert_eq!(response.request_id, header.request_id);
    let decoded = skiff_runtime_boundary::binary::decode_payload(
        &payload,
        &json!({ "kind": "builtin", "name": "void", "args": [] }),
        &mut skiff_runtime_model::request_heap::RequestHeap::default(),
    )
    .expect("fixture business payload should decode");
    assert_eq!(decoded, skiff_runtime_model::value::RuntimeValue::Null);
    assert_eq!(host.outbound_requests.pending_count(), 0);
    assert_eq!(host.outbound_requests.active_lease_count(), 0);
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_no_second_terminal(&mut receiver).await;
}

#[tokio::test]
async fn runtime_assembly_request_executes_zero_payload_unary_with_nested_provider() {
    let (host, route) = fixture::admitted_nested_host().await;
    let header = canonical_header(&route, "runtime-assembly-nested-provider");
    let frame = encode_binary_frame(&header, &[]).expect("canonical request.start should encode");
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender)
        .await
        .expect("canonical request.start should dispatch");

    let terminal = recv_terminal(&mut receiver).await;
    let Terminal::End(response, payload) = terminal else {
        panic!("nested-provider unary must end successfully, got {terminal:?}")
    };
    assert_eq!(response.request_id, header.request_id);
    assert!(response.payload_present);
    let decoded = skiff_runtime_boundary::binary::decode_payload(
        &payload,
        &json!({ "kind": "builtin", "name": "bool", "args": [] }),
        &mut skiff_runtime_model::request_heap::RequestHeap::default(),
    )
    .expect("nested provider bool response should decode");
    assert_eq!(
        decoded,
        skiff_runtime_model::value::RuntimeValue::Bool(true)
    );
    assert_eq!(response.metadata, ResponseEndFrameMetadata::None);
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_no_second_terminal(&mut receiver).await;
}

#[tokio::test]
async fn runtime_assembly_http_response_ceiling_uses_bootstrap_and_releases_request() {
    let (host, route) = fixture::admitted_nested_host().await;
    let header = canonical_header(&route, "runtime-assembly-http-over-limit");
    let frame = encode_binary_frame(&header, &[]).expect("canonical request.start should encode");
    let (sender, mut receiver) = mpsc::unbounded_channel();

    super::super::dispatch_router_binary_frame_with_http_response_max(&host, &frame, &sender, 1)
        .await
        .expect("oversize request should dispatch to its response terminal");

    let Terminal::Error(response) = recv_terminal(&mut receiver).await else {
        panic!("oversize HTTP response must terminate with response.error")
    };
    assert_eq!(response.request_id, header.request_id);
    assert_eq!(response.error.code, "ResourceLimitExceeded");
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_no_second_terminal(&mut receiver).await;
}

#[tokio::test]
async fn runtime_assembly_request_executes_zero_payload_zero_arg_void_unary() {
    let (host, route) = fixture::admitted_void_host(false).await;
    let header = canonical_header(&route, "runtime-assembly-void");
    let frame = encode_binary_frame(&header, &[]).expect("canonical request.start should encode");
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &frame, &sender)
        .await
        .expect("canonical void request.start should dispatch");

    let terminal = recv_terminal(&mut receiver).await;
    let Terminal::End(response, payload) = terminal else {
        panic!("zero-arg void unary must end successfully, got {terminal:?}")
    };
    assert_eq!(response.request_id, header.request_id);
    assert!(response.payload_present);
    let decoded = skiff_runtime_boundary::binary::decode_payload(
        &payload,
        &json!({ "kind": "builtin", "name": "void", "args": [] }),
        &mut skiff_runtime_model::request_heap::RequestHeap::default(),
    )
    .expect("void response should decode on the ordinary unary lane");
    assert_eq!(decoded, skiff_runtime_model::value::RuntimeValue::Null);
    assert_eq!(response.metadata, ResponseEndFrameMetadata::None);
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_no_second_terminal(&mut receiver).await;
}

#[tokio::test]
async fn runtime_assembly_request_rejects_wrong_tuple_http_effects_adapter_and_stream() {
    let (host, route) = fixture::admitted_void_host(false).await;
    let exact = canonical_header(&route, "runtime-assembly-reject-base");
    let mut cases = Vec::new();

    let mut wrong_identity = exact.clone();
    wrong_identity.routing.assembly_identity = AssemblyIdentity::new(format!(
        "skiff-runtime-assembly-v1:sha256:{}",
        "f".repeat(64)
    ));
    cases.push(("identity", wrong_identity));

    let mut wrong_generation = exact.clone();
    wrong_generation.routing.assembly_generation += 1;
    cases.push(("generation", wrong_generation));

    let mut wrong_operation = exact.clone();
    wrong_operation.routing.contract_operation_id = ContractOperationId::new(format!(
        "skiff-contract-operation-v1:sha256:{}",
        "f".repeat(64)
    ));
    cases.push(("operation", wrong_operation));

    let mut wrong_http_method = exact.clone();
    wrong_http_method
        .http_request
        .as_mut()
        .expect("HTTP metadata")
        .method = "GET".to_string();
    cases.push(("http-method-cross-check", wrong_http_method));

    let mut wrong_http_url = exact.clone();
    wrong_http_url
        .http_request
        .as_mut()
        .expect("HTTP metadata")
        .url = "https://other.test/consume".to_string();
    cases.push(("http-url-cross-check", wrong_http_url));

    let mut wrong_route_host = exact.clone();
    wrong_route_host.routing.ingress.host = "other.test".to_string();
    wrong_route_host
        .http_request
        .as_mut()
        .expect("HTTP metadata")
        .url = "https://other.test/consume".to_string();
    cases.push(("http-route", wrong_route_host));

    let mut effects_enabled = exact.clone();
    effects_enabled.test_effects_enabled = true;
    cases.push(("test-effects-enabled", effects_enabled));

    let mut effect_double = exact.clone();
    effect_double.test_effect_doubles = HashMap::from([(
        "effect".to_string(),
        vec![RuntimeAssemblyRequestTestEffectDoubleFrameHeader {
            expect_request: None,
            response: Value::Null,
        }],
    )]);
    cases.push(("test-effect-double", effect_double));

    let mut adapter = exact.clone();
    adapter.http_adapter = Some(RuntimeAssemblyHttpAdapterFrameHeader {
        kind: RuntimeAssemblyHttpAdapterKindFrameHeader::TypedJson,
        handler: RuntimeAssemblyHttpAdapterCallableFrameHeader::ServiceFunction {
            module_path: "handler".to_string(),
            symbol: "run".to_string(),
        },
        guard: None,
        pre: None,
        adapter_args: Vec::new(),
    });
    cases.push(("http-adapter", adapter));

    let mut stream = exact.clone();
    stream.mode = "serverStream".to_string();
    cases.push(("server-stream", stream));

    let mut websocket_without_adapter = exact;
    websocket_without_adapter.routing.ingress.protocol =
        RuntimeAssemblyRequestIngressProtocol::WebSocket;
    websocket_without_adapter.routing.ingress.method = None;
    websocket_without_adapter.http_request = None;

    for (name, mut header) in cases {
        header.request_id = format!("runtime-assembly-reject-{name}");
        let frame = encode_binary_frame(&header, &[])
            .unwrap_or_else(|error| panic!("{name} request.start should encode: {error}"));
        let (sender, mut receiver) = mpsc::unbounded_channel();
        dispatch(&host, &frame, &sender)
            .await
            .unwrap_or_else(|error| panic!("{name} should reach narrow bridge: {error}"));
        let Terminal::Error(response) = recv_terminal(&mut receiver).await else {
            panic!("{name} must fail closed with response.error")
        };
        assert_eq!(response.request_id, header.request_id, "{name}");
        assert_no_second_terminal(&mut receiver).await;
    }

    let mut trusted_test = canonical_header(&route, "runtime-assembly-trusted-test-effects");
    trusted_test.caller.target = "__skiff.runtime-assembly-test-dispatch".to_string();
    trusted_test.test_effects_enabled = true;
    trusted_test.test_effect_doubles = HashMap::from([(
        "unused.effect".to_string(),
        vec![RuntimeAssemblyRequestTestEffectDoubleFrameHeader {
            expect_request: None,
            response: Value::Null,
        }],
    )]);
    let frame = encode_binary_frame(&trusted_test, &[]).expect("trusted test request encodes");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &frame, &sender)
        .await
        .expect("trusted test request should reach canonical execution");
    let Terminal::End(response, _) = recv_terminal(&mut receiver).await else {
        panic!("trusted canonical test controls should execute")
    };
    assert_eq!(response.request_id, trusted_test.request_id);
    assert_no_second_terminal(&mut receiver).await;

    websocket_without_adapter.request_id =
        "runtime-assembly-reject-websocket-without-adapter".to_string();
    let frame = encode_binary_frame(&websocket_without_adapter, &[])
        .expect("structurally encodable WebSocket request");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let error = dispatch(&host, &frame, &sender)
        .await
        .expect_err("shared strict decoder must reject WebSocket metadata before the bridge");
    assert!(error
        .to_string()
        .contains("canonical WebSocket ingress requires websocketAdapter metadata"));
    assert!(
        receiver.try_recv().is_err(),
        "decoder rejection must not enter request terminal ownership"
    );
}

#[tokio::test]
async fn runtime_assembly_request_reload_rejects_stale_generation_and_retains_route_arc_pin() {
    let (host, pinned, current) = fixture::reloaded_nested_host().await;
    let stale_header = canonical_header(&pinned, "runtime-assembly-stale-generation");
    assert_eq!(pinned.generation(), 1);
    assert_eq!(current.generation(), 2);
    assert!(!Arc::ptr_eq(pinned.context_set(), current.context_set()));

    let frame = encode_binary_frame(&stale_header, &[]).expect("stale frame should encode");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    dispatch(&host, &frame, &sender)
        .await
        .expect("stale tuple should produce a terminal rejection");
    let Terminal::Error(response) = recv_terminal(&mut receiver).await else {
        panic!("stale generation must fail closed")
    };
    assert_eq!(response.request_id, stale_header.request_id);
    assert_no_second_terminal(&mut receiver).await;
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_assembly_request_cancel_preserves_same_request_single_terminal_ownership() {
    let (host, route) = fixture::admitted_void_host(true).await;
    let header = canonical_header(&route, "runtime-assembly-cancel");
    let start = encode_binary_frame(&header, &[]).expect("request.start should encode");
    let cancel = encode_binary_frame(
        &RequestCancelFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.cancel".to_string(),
            request_id: header.request_id.clone(),
            reason: "caller_cancel".to_string(),
        },
        &[],
    )
    .expect("request.cancel should encode");
    let (sender, mut receiver) = mpsc::unbounded_channel();

    dispatch(&host, &start, &sender)
        .await
        .expect("request.start should dispatch");
    dispatch(&host, &cancel, &sender)
        .await
        .expect("request.cancel should dispatch on the same session");

    let Terminal::Error(response) = recv_terminal(&mut receiver).await else {
        panic!("cancelled request must have one response.error terminal")
    };
    assert_eq!(response.request_id, header.request_id);
    assert_eq!(response.error.code, "CancelError");
    assert_eq!(host.request_supervisor.active_count().await, 0);
    assert_no_second_terminal(&mut receiver).await;
}

#[tokio::test]
async fn runtime_assembly_request_session_rejects_legacy_flat_unknown_and_duplicate_headers() {
    let host = super::test_host();
    let canonical = canonical_value("runtime-assembly-raw");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let accepted = encode_binary_frame(&canonical, &[]).expect("canonical raw frame should encode");
    dispatch(&host, &accepted, &sender)
        .await
        .expect("strict canonical bytes should reach active-route lookup");
    let Terminal::Error(response) = recv_terminal(&mut receiver).await else {
        panic!("missing active route should be one response.error")
    };
    assert_eq!(response.request_id, "runtime-assembly-raw");

    let legacy = json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": "request.start",
        "requestId": "legacy-request-baseline",
        "mode": "unary",
        "caller": { "kind": "gateway", "target": "legacy-gateway" },
        "target": "legacy-target",
        "operationAbiId": "legacy-operation",
        "serviceId": "example.com/legacy",
        "version": "1.0.0",
        "buildId": format!("skiff-service-build-v1:sha256:{}", "9".repeat(64)),
        "serviceProtocolIdentity": format!("skiff-protocol-v1:sha256:{}", "8".repeat(64)),
        "trace": { "traceId": "legacy-trace", "spanId": "legacy-span" }
    });
    let mut flat = canonical.clone();
    flat.as_object_mut().expect("canonical object").insert(
        "assemblyIdentity".to_string(),
        json!(format!(
            "skiff-runtime-assembly-v1:sha256:{}",
            "a".repeat(64)
        )),
    );
    let mut unknown = canonical.clone();
    unknown
        .as_object_mut()
        .expect("canonical object")
        .insert("unknownField".to_string(), Value::Bool(true));
    let canonical_json = serde_json::to_string(&canonical).expect("canonical JSON");
    let duplicate_json = canonical_json.replacen(
        "\"requestId\":\"runtime-assembly-raw\",",
        "\"requestId\":\"runtime-assembly-raw\",\"requestId\":\"duplicate\",",
        1,
    );
    assert_ne!(
        duplicate_json, canonical_json,
        "duplicate insertion must apply"
    );

    let invalid = [
        ("legacy", encode_binary_frame(&legacy, &[]).unwrap()),
        ("flat", encode_binary_frame(&flat, &[]).unwrap()),
        ("unknown", encode_binary_frame(&unknown, &[]).unwrap()),
        (
            "duplicate",
            raw_binary_frame(duplicate_json.as_bytes(), &[]),
        ),
    ];
    for (name, frame) in invalid {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let error = dispatch(&host, &frame, &sender)
            .await
            .expect_err("strict decoder must reject non-canonical request.start");
        assert!(error.to_string().contains("invalid"), "{name}: {error}");
        assert!(
            receiver.try_recv().is_err(),
            "{name} must not emit a terminal"
        );
    }
}

fn spawn_submitted_receipt(rpc_id: &str) -> SpawnSubmitResponseFrameHeader {
    SpawnSubmitResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "spawn.submit.response".to_string(),
        rpc_id: rpc_id.to_string(),
        spawn_id: "spawn-f50a".to_string(),
        item_id: "item-f50a".to_string(),
        status: "submitted".to_string(),
    }
}

fn canonical_header(
    route: &ActiveAssemblyRoute,
    request_id: &str,
) -> RuntimeAssemblyRequestStartFrameHeader {
    let selector = &route.binding().selector;
    assert_eq!(selector.protocol, IngressProtocol::Http);
    let method = selector.method.clone().expect("HTTP route method");
    RuntimeAssemblyRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
            target: "canonical-gateway".to_string(),
        },
        routing: RuntimeAssemblyRequestRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: route.assembly_identity().clone(),
            assembly_generation: route.generation(),
            contract_operation_id: route.binding().contract_operation_id.clone(),
            ingress: RuntimeAssemblyRequestIngressFrameHeader {
                protocol: RuntimeAssemblyRequestIngressProtocol::Http,
                host: selector.host.clone(),
                method: Some(method.clone()),
                path: selector.path.clone(),
            },
        },
        // The bridge must derive its internal activation from the admitted route, not this value.
        activation_identity: Some(
            "skiff-runtime-activation-v1:opaque:untrusted-wire-activation".to_string(),
        ),
        gateway_entry_identity: None,
        business_identity: None,
        websocket_entry_id: None,
        client_session: None,
        deadline: None,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: "span-canonical-request".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        http_request: Some(RuntimeAssemblyHttpRequestFrameHeader {
            method,
            url: format!("https://{}{}", selector.host, selector.path),
            path: selector.path.clone(),
            query: Vec::new(),
            headers: Vec::new(),
        }),
        http_adapter: None,
        websocket_adapter: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
    }
}

fn canonical_value(request_id: &str) -> Value {
    json!({
        "schemaVersion": RUNTIME_FRAME_SCHEMA_VERSION,
        "type": "request.start",
        "requestId": request_id,
        "mode": "unary",
        "caller": { "kind": "gateway", "target": "canonical-gateway" },
        "routing": {
            "kind": "runtimeAssembly",
            "assemblyIdentity": format!("skiff-runtime-assembly-v1:sha256:{}", "a".repeat(64)),
            "assemblyGeneration": 1,
            "contractOperationId": format!("skiff-contract-operation-v1:sha256:{}", "b".repeat(64)),
            "ingress": {
                "protocol": "http",
                "host": "canonical.test",
                "method": "POST",
                "path": "/invoke"
            }
        },
        "trace": { "traceId": "trace-raw", "spanId": "span-raw" },
        "httpRequest": {
            "method": "POST",
            "url": "https://canonical.test/invoke",
            "path": "/invoke",
            "query": [],
            "headers": []
        }
    })
}

async fn dispatch(
    host: &RuntimeHost,
    frame: &[u8],
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
) -> crate::error::Result<()> {
    let mut control = None;
    let mut artifact_fingerprint = None;
    super::super::dispatch_router_binary_frame(
        host,
        frame,
        sender,
        &mut control,
        &mut artifact_fingerprint,
    )
    .await
}

#[derive(Debug)]
enum Terminal {
    End(ResponseEndFrameHeader, Vec<u8>),
    Error(ResponseErrorFrameHeader),
}

async fn recv_terminal(receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>) -> Terminal {
    let message = timeout(Duration::from_secs(5), receiver.recv())
        .await
        .expect("request terminal timeout")
        .expect("request terminal channel closed");
    let RouterWriterMessage::Binary(frame) = message else {
        panic!("request terminal must be a binary frame")
    };
    let (typed, _): (TypedEnvelope, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("terminal envelope should decode");
    match typed.envelope_type.as_str() {
        "response.end" => {
            let (header, payload) =
                decode_typed_binary_frame(&frame).expect("response.end terminal should decode");
            Terminal::End(header, payload)
        }
        "response.error" => {
            let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
                decode_typed_binary_frame(&frame).expect("response.error terminal should decode");
            assert!(payload.is_empty());
            Terminal::Error(header)
        }
        other => panic!("unexpected request terminal {other}"),
    }
}

async fn assert_no_second_terminal(receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>) {
    assert!(
        timeout(Duration::from_millis(25), receiver.recv())
            .await
            .is_err(),
        "request must emit exactly one terminal"
    );
}

fn raw_binary_frame(header: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(14 + header.len() + payload.len());
    frame.extend_from_slice(&BINARY_FRAME_MAGIC);
    frame.push(BINARY_FRAME_VERSION);
    frame.push(BINARY_FRAME_HEADER_ENCODING_JSON);
    frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(header);
    frame.extend_from_slice(payload);
    frame
}
