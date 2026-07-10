use serde_json::{json, Value};
use skiff_artifact_model::{ConfigShape, RecoverableArtifactMetadata};
use skiff_runtime_boundary::binary::{decode_payload, encode_payload, encode_recoverable_payload};
use skiff_runtime_boundary::{
    payload::{PayloadBoundary, PayloadBoundaryKind},
    type_descriptor::{RuntimeTypePlan, RuntimeTypePlanDescriptorExt},
};
use skiff_runtime_model::{
    recoverable::{
        RuntimeRecoverableExpectedRecordFieldPlan, RuntimeRecoverableExpectedTypeNode,
        RuntimeRecoverableExpectedTypePlan,
    },
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields, RuntimeValue},
};
use skiff_runtime_request::{
    cancellation::CancellationToken, ActorFindControlRequest, ActorKeyControlMetadata,
    ActorPutControlRequest, ExecutionBudget, OutboundResponse, RequestEnvelope,
};
use skiff_runtime_transport::control_mapper::encode_outbound_control_message;
use skiff_runtime_transport::control_response_mapper::spawn_claim_response_control_payload;
use skiff_runtime_transport::protocol::{
    decode_typed_binary_frame, ActorPutRequestFrameHeader, ActorPutResponseFrameHeader,
    ActorRefFrameMetadata, ConnectionSendFrameHeader, RequestStartFrameHeader,
    ResponseChunkFrameHeader, ResponseEndFrameHeader, ResponseErrorFrameHeader,
    ResponseStartFrameHeader, RouterControlEnvelope, RouterControlPackageConfig,
    RouterControlServiceConfig, RuntimeCallerFrameHeader, RuntimeCapabilitiesFrameHeader,
    RuntimeDispatchModeCapability, RuntimeHttpAdapterArgFrameHeader,
    RuntimeHttpAdapterCallableFrameHeader, RuntimeHttpAdapterFrameHeader,
    RuntimeHttpAdapterKindFrameHeader, RuntimeHttpAdapterSourceFrameHeader,
    RuntimeHttpNameValueFrameHeader, RuntimeHttpRequestFrameHeader, RuntimeHttpResponseFrameHeader,
    RuntimeRegisterFrameHeader, RuntimeTraceContextFrameHeader, SpawnClaimDescriptorFrameMetadata,
    SpawnClaimRequestFrameHeader, SpawnClaimResponseFrameHeader, SpawnCompleteRequestFrameHeader,
    SpawnCompleteResponseFrameHeader, SpawnFailRequestFrameHeader, SpawnFailResponseFrameHeader,
    SpawnRenewRequestFrameHeader, SpawnRenewResponseFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};
use skiff_runtime_transport::request_mapper::request_envelope_from_start_frame;
use std::{
    collections::{BTreeMap, HashMap},
    sync::atomic::AtomicBool,
    sync::Arc,
    time::Instant,
};

use super::{apply_control_config, invocation_context_from_request};
use crate::{
    capability_context::{ActorClient, ActorClientContext},
    config_view::RuntimeConfigView,
    host::{
        RouterWriterMessage, RuntimeConfig, RuntimeHost, RuntimeOperation, RuntimeServiceConfig,
    },
    program::{
        anonymous_type_decl, package_handler_target, ExecutableAddr, ExecutableKind,
        FileDeclarations, FileLinkTargets, LinkOverlay, LinkedCallTarget, LinkedExecutable,
        LinkedExecutableBody, LinkedExprIr, LinkedFileUnit, LinkedTypeDescriptor, LinkedTypeRef,
        LiteralIr, OperationIngressKind, OperationRouteBinding, PackageRefIr, PackageSymbolRef,
        PackageUnit, ParamIr, ResolvedSymbol, RuntimeProgram, RuntimeProgramLayers,
        RuntimeTypeContext, ServiceMeta, ServiceSymbolRef, SlotIr, SlotLayoutIr, TypeAddr,
        UnitAddr,
    },
};
use tokio::{
    sync::mpsc,
    time::{timeout, Duration},
};

const PROTOCOL_A: &str =
    "skiff-protocol-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PROTOCOL_B: &str =
    "skiff-protocol-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BUILD_A: &str = "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BUILD_B: &str = "skiff-service-build-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
#[allow(dead_code)]
const BUILD_C: &str = "skiff-service-build-v1:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

#[derive(Clone)]
struct TestDbCapabilityFactory;

impl skiff_runtime_capability_context::DbCapabilityFactory for TestDbCapabilityFactory {
    fn context_for_request(
        &self,
        _owner: String,
        _request_id: String,
    ) -> skiff_runtime_capability_context::DbCapabilityContext {
        skiff_runtime_capability_context::DbCapabilityContext::unavailable()
    }
}

#[derive(Clone)]
struct TestDbProviderFactory;

impl skiff_runtime_capability_context::DbProviderFactory for TestDbProviderFactory {
    fn build(
        &self,
        _input: skiff_runtime_capability_context::DbProviderBuildInput,
    ) -> skiff_runtime_capability_context::DbCapabilityResult<
        skiff_runtime_capability_context::DbCapabilitySource,
    > {
        Ok(skiff_runtime_capability_context::DbCapabilitySource::new(
            Some(TestDbCapabilityFactory),
        ))
    }
}

fn test_db_provider() -> skiff_runtime_capability_context::DbProviderSource {
    skiff_runtime_capability_context::DbProviderSource::new(TestDbProviderFactory)
}

fn router_binary(message: RouterWriterMessage) -> Vec<u8> {
    match message {
        RouterWriterMessage::Binary(value) => value,
        RouterWriterMessage::Control(command) => encode_outbound_control_message(command)
            .expect("control router writer message should encode"),
    }
}

fn router_binary_error_json(message: RouterWriterMessage) -> Value {
    let frame = router_binary(message);
    let (header, payload): (ResponseErrorFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&frame).expect("binary error response should decode");
    assert!(payload.is_empty());
    serde_json::to_value(header).expect("binary error header should serialize")
}

fn router_binary_end(message: RouterWriterMessage) -> (ResponseEndFrameHeader, Vec<u8>) {
    let frame = router_binary(message);
    decode_typed_binary_frame(&frame).expect("binary response should decode")
}

fn router_binary_start(message: RouterWriterMessage) -> (ResponseStartFrameHeader, Vec<u8>) {
    let frame = router_binary(message);
    decode_typed_binary_frame(&frame).expect("binary response start should decode")
}

fn router_binary_chunk(message: RouterWriterMessage) -> (ResponseChunkFrameHeader, Vec<u8>) {
    let frame = router_binary(message);
    decode_typed_binary_frame(&frame).expect("binary response chunk should decode")
}

fn set_request_string_arg(request: &mut RequestEnvelope, name: &str, value: &str) {
    let args_descriptor = json!({
        "kind": "record",
        "fields": {
            name: { "kind": "builtin", "name": "string", "args": [] }
        }
    });
    let mut heap = RequestHeap::default();
    let args_handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            name.to_string(),
            RuntimeValue::String(value.to_string()),
        )])))
        .expect("args record should allocate");
    request.payload_bytes =
        encode_payload(&RuntimeValue::Heap(args_handle), &args_descriptor, &heap)
            .expect("request args payload should encode");
}

fn set_spawn_request_string_arg(request: &mut RequestEnvelope, name: &str, value: &str) {
    let mut heap = RequestHeap::default();
    let args_handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            name.to_string(),
            RuntimeValue::String(value.to_string()),
        )])))
        .expect("spawn args record should allocate");
    let expected = RuntimeRecoverableExpectedTypePlan {
        label: "record".to_string(),
        identity: None,
        node: RuntimeRecoverableExpectedTypeNode::Record {
            fields: vec![RuntimeRecoverableExpectedRecordFieldPlan {
                name: name.to_string(),
                ty: RuntimeRecoverableExpectedTypePlan {
                    label: "string".to_string(),
                    identity: None,
                    node: RuntimeRecoverableExpectedTypeNode::String,
                },
                required: true,
            }],
            boundary_record_kind: None,
        },
    };
    request.payload_bytes = encode_recoverable_payload(
        &RuntimeValue::Heap(args_handle),
        &expected,
        &PayloadBoundary::owner_internal(PayloadBoundaryKind::SpawnPayload),
        &heap,
    )
    .expect("spawn request args payload should encode as recoverable envelope");
    assert_eq!(&request.payload_bytes[..4], b"SKRE");
}

fn decode_string_response(payload: &[u8]) -> String {
    let mut heap = RequestHeap::default();
    let decoded = decode_payload(
        payload,
        &json!({ "kind": "builtin", "name": "string", "args": [] }),
        &mut heap,
    )
    .expect("string response payload should decode");
    assert!(heap.is_empty());
    match decoded {
        RuntimeValue::String(value) => value,
        other => panic!("expected string response, got {other:?}"),
    }
}

#[allow(dead_code)]
fn decode_json_number_response(payload: &[u8]) -> f64 {
    let mut heap = RequestHeap::default();
    let decoded = decode_payload(
        payload,
        &json!({ "kind": "builtin", "name": "Json", "args": [] }),
        &mut heap,
    )
    .expect("JSON response payload should decode");
    assert!(heap.is_empty());
    match decoded {
        RuntimeValue::Number(value) => value,
        other => panic!("expected number response, got {other:?}"),
    }
}

#[tokio::test]
async fn routes_requests_by_build_id_and_operation_abi_id() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                "shared.target",
            )
            .await,
            service_config(
                "runtime-base:service-b",
                "service-b",
                PROTOCOL_B,
                "shared.target",
            )
            .await,
        ],
    })
    .expect("host should build");

    let route_a = host
        .lookup_operation(&request(BUILD_A, "shared.target"))
        .expect("build-a route should resolve");
    let route_b = host
        .lookup_operation(&request(BUILD_B, "shared.target"))
        .expect("build-b route should resolve");

    assert_eq!(route_a.service.service_id, "service-a");
    assert_eq!(route_a.service.runtime_id, "runtime-base:service-a");
    assert_eq!(route_b.service.service_id, "service-b");
    assert_eq!(route_b.service.runtime_id, "runtime-base:service-b");
}

#[tokio::test]
async fn request_missing_operation_abi_id_fails_closed() {
    let service_a = service_config_with_build(
        "runtime-base:service-a",
        "service-a",
        PROTOCOL_A,
        BUILD_A,
        "shared.target",
    )
    .await;
    let service_b = service_config_with_build(
        "runtime-base:service-b",
        "service-b",
        PROTOCOL_A,
        BUILD_B,
        "shared.target",
    )
    .await;

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![service_a, service_b],
    })
    .expect("host should build");

    let error =
        match host.lookup_operation(&request_without_operation_abi_id(BUILD_A, "shared.target")) {
            Ok(route) => panic!(
                "request without operationAbiId must not route by target {}",
                route.service.runtime_id
            ),
            Err(error) => error,
        };
    assert!(error.to_string().contains("operationAbiId"));
}

#[tokio::test]
async fn runtime_capabilities_registers_without_loaded_services() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: Vec::new(),
    })
    .expect("host should build without services");

    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.queue_registers(sender)
        .expect("runtime capabilities should serialize");
    let capabilities_frame = router_binary(
        receiver
            .recv()
            .await
            .expect("runtime capabilities should be queued"),
    );
    let (capabilities, payload): (RuntimeCapabilitiesFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&capabilities_frame)
            .expect("runtime.capabilities frame should decode");

    assert!(payload.is_empty());
    assert_eq!(capabilities.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
    assert_eq!(capabilities.envelope_type, "runtime.capabilities");
    assert_eq!(capabilities.runtime_id, "runtime-base");
    assert!(capabilities.capabilities.package_test_dispatch);
    assert!(capabilities.capabilities.request_cancel);
    assert!(
        receiver.try_recv().is_err(),
        "empty service snapshot must not queue runtime.register"
    );
}

#[tokio::test]
async fn routes_request_by_operation_abi_id_when_target_does_not_match() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config_with_build(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                BUILD_A,
                "real.target",
            )
            .await,
        ],
    })
    .expect("host should build");
    let mut request = build_request(BUILD_A, "wrong.display.target");
    request.operation_abi_id = Some(operation_abi_id_for_target("real.target"));
    request.selector = Some(format!(
        "operation:{}",
        request.operation_abi_id.as_deref().unwrap()
    ));

    let route = host
        .lookup_operation(&request)
        .expect("operationAbiId route should resolve without target lookup");

    assert_eq!(route.service.service_id, "service-a");
    assert_eq!(route.operation.target, "real.target");
    assert_eq!(
        route.operation.operation_abi_id.as_deref(),
        Some(operation_abi_id_for_target("real.target").as_str())
    );
}

#[tokio::test]
async fn routes_request_selector_mismatch_fails_closed() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config_with_build(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                BUILD_A,
                "real.target",
            )
            .await,
        ],
    })
    .expect("host should build");
    let mut request = build_request(BUILD_A, "real.target");
    request.operation_abi_id = Some(operation_abi_id_for_target("real.target"));
    request.selector = Some("operation:other".to_string());

    let error = match host.lookup_operation(&request) {
        Ok(route) => panic!(
            "selector mismatch must fail closed, got route {}",
            route.operation.target
        ),
        Err(error) => error.to_string(),
    };

    assert!(
        error.contains("selector operation:other is not registered"),
        "unexpected selector mismatch error: {error}"
    );
}

#[test]
fn route_binding_unknown_operation_abi_id_fails_closed_on_load() {
    let mut program = runtime_program_for_service("service-a", BUILD_A, "real.target");
    program
        .operation_route_bindings
        .push(OperationRouteBinding {
            ingress_kind: OperationIngressKind::HttpGateway,
            selector: "GET /missing".to_string(),
            operation_abi_id: operation_abi_id_for_target("missing.target"),
        });

    let error = runtime_host_error_for_program(program);

    assert!(
        error.contains("unknown operationAbiId")
            && error.contains(&operation_abi_id_for_target("missing.target")),
        "unexpected route binding error: {error}"
    );
}

#[test]
fn route_binding_empty_operation_abi_id_fails_closed_on_load() {
    let mut program = runtime_program_for_service("service-a", BUILD_A, "real.target");
    program
        .operation_route_bindings
        .push(OperationRouteBinding {
            ingress_kind: OperationIngressKind::HttpGateway,
            selector: "GET /empty".to_string(),
            operation_abi_id: String::new(),
        });

    let error = runtime_host_error_for_program(program);

    assert!(
        error.contains("empty operationAbiId"),
        "unexpected empty operationAbiId error: {error}"
    );
}

#[test]
fn route_binding_empty_selector_fails_closed_on_load() {
    let mut program = runtime_program_for_service("service-a", BUILD_A, "real.target");
    program
        .operation_route_bindings
        .push(OperationRouteBinding {
            ingress_kind: OperationIngressKind::HttpGateway,
            selector: String::new(),
            operation_abi_id: operation_abi_id_for_target("real.target"),
        });

    let error = runtime_host_error_for_program(program);

    assert!(
        error.contains("empty selector"),
        "unexpected empty selector error: {error}"
    );
}

#[test]
fn service_call_route_binding_selector_mismatch_fails_closed_on_load() {
    let mut program = runtime_program_for_service("service-a", BUILD_A, "real.target");
    program
        .operation_route_bindings
        .push(OperationRouteBinding {
            ingress_kind: OperationIngressKind::ServiceCall,
            selector: "operation:other".to_string(),
            operation_abi_id: operation_abi_id_for_target("real.target"),
        });

    let error = runtime_host_error_for_program(program);

    assert!(
        error.contains("does not match operationAbiId")
            && error.contains(&operation_abi_id_for_target("real.target")),
        "unexpected service-call selector mismatch error: {error}"
    );
}

#[test]
fn route_binding_duplicate_selector_same_operation_fails_closed_on_load() {
    let mut program = runtime_program_for_service("service-a", BUILD_A, "real.target");
    let operation_abi_id = operation_abi_id_for_target("real.target");
    program
        .operation_route_bindings
        .push(OperationRouteBinding {
            ingress_kind: OperationIngressKind::HttpGateway,
            selector: "GET /same".to_string(),
            operation_abi_id: operation_abi_id.clone(),
        });
    program
        .operation_route_bindings
        .push(OperationRouteBinding {
            ingress_kind: OperationIngressKind::HttpGateway,
            selector: "GET /same".to_string(),
            operation_abi_id,
        });

    let error = runtime_host_error_for_program(program);

    assert!(
        error.contains("duplicate route binding selector")
            && error.contains("GET /same")
            && error.contains(&operation_abi_id_for_target("real.target")),
        "unexpected duplicate selector error: {error}"
    );
}

#[tokio::test]
async fn gateway_request_missing_selector_fails_closed() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config_with_build(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                BUILD_A,
                "real.target",
            )
            .await,
        ],
    })
    .expect("host should build");
    let mut request = request(BUILD_A, "real.target");
    request.selector = None;
    request.extra.insert(
        "caller".to_string(),
        json!({"kind": "gateway", "target": "gateway.http"}),
    );

    let error = match host.lookup_operation(&request) {
        Ok(route) => panic!(
            "gateway request without selector must fail closed, got route {}",
            route.operation.target
        ),
        Err(error) => error.to_string(),
    };

    assert!(
        error.contains("selector is required")
            && error.contains(&operation_abi_id_for_target("real.target")),
        "unexpected missing selector error: {error}"
    );
}

#[tokio::test]
async fn internal_service_request_may_omit_selector() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config_with_build(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                BUILD_A,
                "real.target",
            )
            .await,
        ],
    })
    .expect("host should build");
    let mut request = request(BUILD_A, "real.target");
    request.selector = None;
    request.extra.insert(
        "caller".to_string(),
        json!({"kind": "service", "target": "caller.target"}),
    );

    let route = host
        .lookup_operation(&request)
        .expect("internal service-call may omit selector");

    assert_eq!(route.operation.target, "real.target");
}

#[tokio::test]
async fn routes_by_registered_runtime_program_target_only() {
    let service = service_config(
        "runtime-base:service-a",
        "service-a",
        PROTOCOL_A,
        "shared.target",
    )
    .await;

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![service],
    })
    .expect("host should build");

    let route = host
        .lookup_operation(&build_request(BUILD_A, "shared.target"))
        .expect("runtime program target should resolve");
    assert_eq!(route.service.runtime_id, "runtime-base:service-a");
    assert_eq!(
        route.operation.service_protocol_identity.as_deref(),
        Some(PROTOCOL_A)
    );

    let error = match host.lookup_operation(&build_request(BUILD_A, "unregistered.target")) {
        Ok(route) => panic!(
            "target should not be treated as route authority {}",
            route.service.runtime_id
        ),
        Err(error) => error,
    };
    let error = error.to_string();
    assert!(error.contains("selector"));
    assert!(error.contains("not registered for buildId"));
    assert!(error.contains(&operation_abi_id_for_target("unregistered.target")));
}

#[tokio::test]
async fn idle_release_skips_active_build_execution() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                "shared.target",
            )
            .await,
        ],
    })
    .expect("host should build");

    let _guard = host
        .begin_build_execution(BUILD_A)
        .expect("active execution should begin");
    host.loaded_builds
        .force_last_used_for_test(BUILD_A, Instant::now() - Duration::from_secs(3600));

    let report = host
        .release_idle_builds(Duration::from_secs(1))
        .await
        .expect("idle release should run");

    assert!(report.released_builds.is_empty());
    assert_eq!(report.skipped_active_builds, vec![BUILD_A.to_string()]);
    assert_eq!(report.stopped_spawn_workers, 0);
    assert_eq!(host.loaded_builds.active_count(BUILD_A), 1);
    assert!(host.loaded_builds.contains(BUILD_A));
    host.lookup_operation(&request(BUILD_A, "shared.target"))
        .expect("active build should remain routed");
}

#[tokio::test]
async fn idle_release_allows_previously_resolved_context_to_begin_execution() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                "shared.target",
            )
            .await,
        ],
    })
    .expect("host should build");

    let route = host
        .lookup_operation(&request(BUILD_A, "shared.target"))
        .expect("route should resolve before release");
    host.loaded_builds
        .force_last_used_for_test(BUILD_A, Instant::now() - Duration::from_secs(3600));

    let report = host
        .release_idle_builds(Duration::from_secs(1))
        .await
        .expect("idle release should run");

    assert_eq!(report.released_builds, vec![BUILD_A.to_string()]);
    assert_eq!(report.stopped_spawn_workers, 0);
    assert!(!host.loaded_builds.contains(BUILD_A));
    assert!(host
        .lookup_operation(&request(BUILD_A, "shared.target"))
        .is_err());

    let guard = host
        .begin_build_execution(&route.service.build_id)
        .expect("previously resolved context should still be executable");
    drop(guard);
}

#[tokio::test]
async fn applies_runtime_default_http_response_max_bytes_to_services_without_override() {
    let custom_default = 16 * 1024 * 1024;
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: custom_default,
        http_egress_proxy: None,
        services: vec![
            service_config(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                "shared.target",
            )
            .await,
        ],
    })
    .expect("host should build");

    let route = host
        .lookup_operation(&request(BUILD_A, "shared.target"))
        .expect("route should resolve");
    assert_eq!(route.service.http_response_max_bytes, custom_default);
}

#[tokio::test]
async fn request_heap_limit_uses_runtime_memory_budget() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: Vec::new(),
    })
    .expect("host should build");

    assert_eq!(
        host.request_heap_limits().max_estimated_bytes,
        host.artifact_caches.memory_budgets().request_heap_bytes
    );
}

#[tokio::test]
async fn preserves_service_specific_http_response_max_bytes_override() {
    let service_override = 1234;
    let mut service = service_config(
        "runtime-base:service-a",
        "service-a",
        PROTOCOL_A,
        "shared.target",
    )
    .await;
    service.http_response_max_bytes = service_override;
    service.use_runtime_default_http_response_max_bytes = false;

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: 16 * 1024 * 1024,
        http_egress_proxy: None,
        services: vec![service],
    })
    .expect("host should build");

    let route = host
        .lookup_operation(&request(BUILD_A, "shared.target"))
        .expect("route should resolve");
    assert_eq!(route.service.http_response_max_bytes, service_override);
}

#[tokio::test]
async fn routes_multiple_activations_by_activation_identity_and_rejects_ambiguous_missing() {
    let mut activation_a = service_config_with_build(
        "runtime-base:service-a:activation-a",
        "service-a",
        PROTOCOL_A,
        BUILD_A,
        "shared.target",
    )
    .await;
    activation_a.activation_identity = Some("activation-a".to_string());
    activation_a.resolved_config_identity = Some("config-a".to_string());
    activation_a.config =
        RuntimeConfigView::from_resolved_config(json!({"dashscopeModel": "a"}), config_shape())
            .expect("config should build");

    let mut activation_b = service_config_with_build(
        "runtime-base:service-a:activation-b",
        "service-a",
        PROTOCOL_A,
        BUILD_A,
        "shared.target",
    )
    .await;
    activation_b.activation_identity = Some("activation-b".to_string());
    activation_b.resolved_config_identity = Some("config-b".to_string());
    activation_b.config =
        RuntimeConfigView::from_resolved_config(json!({"dashscopeModel": "b"}), config_shape())
            .expect("config should build");

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![activation_a, activation_b],
    })
    .expect("host should build");

    let mut request_b = request(BUILD_A, "shared.target");
    request_b.activation_identity = Some("activation-b".to_string());
    let route_b = host
        .lookup_operation(&request_b)
        .expect("activation-b should resolve");
    assert_eq!(
        route_b
            .service
            .config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("dashscopeModel")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        json!("b")
    );

    let error = match host.lookup_operation(&request(BUILD_A, "shared.target")) {
        Ok(route) => panic!(
            "missing activationIdentity must not route to {}",
            route.service.runtime_id
        ),
        Err(error) => error,
    };
    assert!(error.to_string().contains("activationIdentity"));
}

#[tokio::test]
async fn rejects_missing_build_id_without_protocol_fallback() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                "shared.target",
            )
            .await,
            service_config(
                "runtime-base:service-b",
                "service-b",
                PROTOCOL_B,
                "shared.target",
            )
            .await,
        ],
    })
    .expect("host should build");

    let mut request = request(BUILD_A, "shared.target");
    request.build_id.clear();
    let error = match host.lookup_operation(&request) {
        Ok(route) => panic!(
            "missing buildId must not fall back to protocol route {}",
            route.service.service_id
        ),
        Err(error) => error,
    };

    assert!(error.to_string().contains("buildId"));
}

#[tokio::test]
async fn rejects_invalid_build_id_without_target_fallback() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                "shared.target",
            )
            .await,
            service_config(
                "runtime-base:service-b",
                "service-b",
                PROTOCOL_B,
                "shared.target",
            )
            .await,
        ],
    })
    .expect("host should build");

    let request = request("not-a-build-id", "shared.target");
    let error = match host.lookup_operation(&request) {
        Ok(route) => panic!(
            "invalid buildId must not route to {}",
            route.service.service_id
        ),
        Err(error) => error,
    };

    assert!(error.to_string().contains("buildId"));
    assert!(error.to_string().contains("skiff-service-build-v1"));
}

#[tokio::test]
async fn applies_control_config_by_service_id_and_build_id_without_crossing_builds() {
    let build_a = service_config_with_build(
        "runtime-base:service-a:build-a",
        "example.com/service-a",
        PROTOCOL_A,
        BUILD_A,
        "shared.target",
    )
    .await;
    let build_b = service_config_with_build(
        "runtime-base:service-a:build-b",
        "example.com/service-a",
        PROTOCOL_A,
        BUILD_B,
        "shared.target",
    )
    .await;

    let services = apply_control_config(
        vec![build_a, build_b],
        &[
            control_service_config(
                "example.com/service-a",
                BUILD_A,
                "activation-a",
                "config-a",
                "a",
            ),
            {
                let mut config = control_service_config(
                    "example.com/service-a",
                    BUILD_B,
                    "activation-b",
                    "config-b",
                    "b",
                );
                config.service_db =
                    Some(skiff_runtime_transport::protocol::RouterControlServiceDb {
                        mongo_url: "mongodb://127.0.0.1:27017/?directConnection=true".to_string(),
                        storage_service_id: "example.com/service-a".to_string(),
                        extra: serde_json::Map::new(),
                    });
                config
            },
        ],
    )
    .expect("config activation should apply");

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: test_db_provider(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services,
    })
    .expect("host should build");

    let mut request_a = request(BUILD_A, "shared.target");
    request_a.activation_identity = Some("activation-a".to_string());
    let mut request_b = request(BUILD_B, "shared.target");
    request_b.activation_identity = Some("activation-b".to_string());

    let route_a = host
        .lookup_operation(&request_a)
        .expect("build-a activation should resolve");
    let route_b = host
        .lookup_operation(&request_b)
        .expect("build-b activation should resolve");

    assert_eq!(
        route_a.service.resolved_config_identity.as_deref(),
        Some("config-a")
    );
    assert_eq!(
        route_b.service.resolved_config_identity.as_deref(),
        Some("config-b")
    );
    assert_eq!(
        route_a
            .service
            .config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("dashscopeModel")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        json!("a")
    );
    assert_eq!(
        route_b
            .service
            .config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("dashscopeModel")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        json!("b")
    );
    assert!(route_a.service.service_db.is_none());
    assert!(route_b.service.service_db.is_some());
}

#[tokio::test]
async fn configured_service_db_without_provider_fails_provider_unavailable() {
    let mut service = service_config(
        "runtime-base:service-db-unavailable",
        "service-db-unavailable",
        PROTOCOL_A,
        "service.target",
    )
    .await;
    service.service_db = Some(skiff_runtime_capability_context::DbProviderConfig::opaque(
        json!({
            "mongoUrl": "mongodb://127.0.0.1:27017/?directConnection=true",
            "storageServiceId": "service-db-unavailable",
        }),
    ));

    let error = match RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![service],
    }) {
        Ok(_) => panic!("configured serviceDb should fail when runtime host has no DB provider"),
        Err(error) => error,
    };

    let message = format!("{error:#}");
    assert!(message.contains("provider unavailable for service-db-unavailable"));
    assert!(message.contains("serviceDb provider is not configured for this runtime host"));
}

#[tokio::test]
async fn configured_service_db_requires_explicit_storage_service_id() {
    let mut service = service_config(
        "runtime-base:service-db-missing-storage",
        "service-db-missing-storage",
        PROTOCOL_A,
        "service.target",
    )
    .await;
    service.service_db = Some(skiff_runtime_capability_context::DbProviderConfig::opaque(
        json!({ "mongoUrl": "mongodb://127.0.0.1:27017/?directConnection=true" }),
    ));

    let error = match RuntimeHost::new(RuntimeConfig {
        db_provider: test_db_provider(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![service],
    }) {
        Ok(_) => panic!("configured serviceDb without storageServiceId should fail"),
        Err(error) => error,
    };

    let message = format!("{error:#}");
    assert!(
        message.contains("runtime serviceDb.storageServiceId is required"),
        "unexpected error: {message}"
    );
}

#[tokio::test]
async fn control_config_preserves_package_scoped_config() {
    let mut build_a = service_config_with_build(
        "runtime-base:service-a:build-a",
        "service-a",
        PROTOCOL_A,
        BUILD_A,
        "shared.target",
    )
    .await;
    build_a.package_configs = vec![RuntimeConfigView::from_value(json!({
        "sessionSecret": "package-secret"
    }))];

    let services = apply_control_config(
        vec![build_a],
        &[control_service_config(
            "service-a",
            BUILD_A,
            "activation-a",
            "config-a",
            "service-model",
        )],
    )
    .expect("config activation should apply");

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services,
    })
    .expect("host should build");

    let mut request_a = request(BUILD_A, "shared.target");
    request_a.activation_identity = Some("activation-a".to_string());
    let route_a = host
        .lookup_operation(&request_a)
        .expect("build-a activation should resolve");

    assert_eq!(
        route_a
            .service
            .config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("dashscopeModel")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        json!("service-model")
    );
    assert_eq!(
        route_a.service.package_configs[0]
            .dispatch_typed_config_target(
                "config.require",
                &[json!("sessionSecret")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        json!("package-secret")
    );
}

#[test]
fn control_config_rejects_missing_required_service_config_at_activation_time() {
    let build_a = runtime_program_service_config(
        "runtime-base:service-a:build-a",
        Arc::new(runtime_program_for_service(
            "service-a",
            BUILD_A,
            "shared.target",
        )),
    );
    let mut config = control_service_config(
        "service-a",
        BUILD_A,
        "activation-a",
        "config-a",
        "service-model",
    );
    config.resolved_config = json!({});
    config.config_shape = Some(config_shape_with_entries(json!([
        { "path": "dashscopeModel", "type": "string", "required": true }
    ])));

    let error = match apply_control_config(vec![build_a], &[config]) {
        Ok(_) => panic!("missing required service config should fail while applying activation"),
        Err(error) => error,
    };
    let message = format!("{error:#}");
    assert!(message.contains("dashscopeModel"), "{message}");
    assert!(
        message.contains("required value is missing or null"),
        "{message}"
    );
}

#[test]
fn control_config_rejects_missing_required_package_config_at_activation_time() {
    let mut program = runtime_program_for_service("service-a", BUILD_A, "shared.target");
    program.packages = vec![Arc::new(package_unit("skiff.run/http-session"))];
    program.package_configs = vec![RuntimeConfigView::from_value(json!({}))];
    let build_a =
        runtime_program_service_config("runtime-base:service-a:build-a", Arc::new(program));
    let mut config = control_service_config(
        "service-a",
        BUILD_A,
        "activation-a",
        "config-a",
        "service-model",
    );
    config.package_configs = vec![RouterControlPackageConfig {
        package_id: "skiff.run/http-session".to_string(),
        package_slot: Some(0),
        alias: "httpSession".to_string(),
        resolved_config_identity: "skiff-config-resolved-v1:opaque:http-session-config".to_string(),
        resolved_config: json!({}),
        redacted_resolved_config: Value::Null,
        redaction_projection_identity: None,
        config_shape: Some(config_shape_with_entries(json!([
            { "path": "sessionSecret", "type": "string", "required": true }
        ]))),
        extra: serde_json::Map::new(),
    }];

    let error = match apply_control_config(vec![build_a], &[config]) {
        Ok(_) => panic!("missing required package config should fail while applying activation"),
        Err(error) => error,
    };
    let message = format!("{error:#}");
    assert!(message.contains("sessionSecret"), "{message}");
    assert!(
        message.contains("required value is missing or null"),
        "{message}"
    );
}

#[test]
fn control_config_applies_required_package_config_without_local_placeholder_validation() {
    let mut program = runtime_program_for_service("service-a", BUILD_A, "shared.target");
    program.packages = vec![Arc::new(package_unit("skiff.run/http-session"))];
    program.package_configs = Vec::new();
    let build_a =
        runtime_program_service_config("runtime-base:service-a:build-a", Arc::new(program));
    let mut config = control_service_config(
        "service-a",
        BUILD_A,
        "activation-a",
        "config-a",
        "service-model",
    );
    config.package_configs = vec![RouterControlPackageConfig {
        package_id: "skiff.run/http-session".to_string(),
        package_slot: Some(0),
        alias: "httpSession".to_string(),
        resolved_config_identity: "skiff-config-resolved-v1:opaque:http-session-config".to_string(),
        resolved_config: json!({
            "cookieName": "sid"
        }),
        redacted_resolved_config: Value::Null,
        redaction_projection_identity: None,
        config_shape: Some(config_shape_with_entries(json!([
            { "path": "cookieName", "type": "string", "required": true }
        ]))),
        extra: serde_json::Map::new(),
    }];

    let services = apply_control_config(vec![build_a], &[config])
        .expect("control package config should satisfy required package shape");

    assert_eq!(
        services[0].package_configs[0]
            .dispatch_typed_config_target(
                "config.require",
                &[json!("cookieName")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        json!("sid")
    );
}

#[tokio::test]
async fn control_config_merges_package_scoped_runtime_config_with_artifact_defaults() {
    let mut program = runtime_program_for_service("service-a", BUILD_A, "shared.target");
    program.packages = vec![Arc::new(package_unit("skiff.run/http-session"))];
    program.package_configs = vec![RuntimeConfigView::from_value(json!({
        "dashscopeModel": "qwen-plus",
        "nested": {
            "artifact": "default",
            "remove": "artifact"
        }
    }))];
    let build_a =
        runtime_program_service_config("runtime-base:service-a:build-a", Arc::new(program));
    let mut config = control_service_config(
        "service-a",
        BUILD_A,
        "activation-a",
        "config-a",
        "service-model",
    );
    config.package_configs = vec![RouterControlPackageConfig {
        package_id: "skiff.run/http-session".to_string(),
        package_slot: Some(0),
        alias: "httpSession".to_string(),
        resolved_config_identity: "skiff-config-resolved-v1:opaque:http-session-config"
            .to_string(),
        resolved_config: json!({
            "dashscopeApiKey": "runtime-secret",
            "nested": {
                "runtime": "override",
                "remove": null
            }
        }),
        redacted_resolved_config: json!({
            "dashscopeApiKey": "[REDACTED]",
            "nested": {
                "runtime": "override",
                "remove": null
            }
        }),
        redaction_projection_identity: Some(
            "skiff-config-redaction-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
        ),
        config_shape: Some(config_shape()),
        extra: serde_json::Map::new(),
    }];

    let services = apply_control_config(vec![build_a], &[config])
        .expect("config activation should apply package scoped config");
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services,
    })
    .expect("host should build");

    let mut request_a = request(BUILD_A, "shared.target");
    request_a.activation_identity = Some("activation-a".to_string());
    let route_a = host
        .lookup_operation(&request_a)
        .expect("build-a activation should resolve");
    let package_config = &route_a.service.package_configs[0];

    assert_eq!(
        package_config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("dashscopeModel")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        json!("qwen-plus")
    );
    assert_eq!(
        package_config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("dashscopeApiKey")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        json!("runtime-secret")
    );
    assert_eq!(
        package_config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("nested.runtime")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        json!("override")
    );
    assert_eq!(
        package_config
            .dispatch_typed_config_target("config.has", &[json!("nested.remove")], None)
            .unwrap(),
        json!(false)
    );
}

#[tokio::test]
async fn reload_replaces_same_contract_pointer_for_new_requests() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config(
                "runtime-base:service-a:v1",
                "service-a",
                PROTOCOL_A,
                "shared.target",
            )
            .await,
        ],
    })
    .expect("host should build");

    let first = host
        .lookup_operation(&request(BUILD_A, "shared.target"))
        .expect("first route should resolve");
    assert_eq!(first.service.runtime_id, "runtime-base:service-a:v1");
    assert_eq!(
        first.service.implementation_identity,
        "impl-runtime-base:service-a:v1"
    );

    host.replace_services(vec![
        service_config(
            "runtime-base:service-a:v2",
            "service-a",
            PROTOCOL_A,
            "shared.target",
        )
        .await,
    ])
    .expect("reload should replace services");

    let second = host
        .lookup_operation(&request(BUILD_A, "shared.target"))
        .expect("second route should resolve");
    assert_eq!(second.service.runtime_id, "runtime-base:service-a:v2");
    assert_eq!(
        second.service.implementation_identity,
        "impl-runtime-base:service-a:v2"
    );
    assert_ne!(
        second.service.artifact_identity,
        first.service.artifact_identity
    );
    assert_eq!(first.service.runtime_id, "runtime-base:service-a:v1");
}

#[tokio::test]
async fn request_error_emits_trace_event() {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![
            service_config(
                "runtime-base:service-a",
                "service-a",
                PROTOCOL_A,
                "shared.target",
            )
            .await,
        ],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut request = request(BUILD_A, "shared.target");
    request.mode = "stream".to_string();
    request.extra.insert(
        "trace".to_string(),
        json!({
            "traceId": "trace-1",
            "spanId": "span-1",
            "parentSpanId": "parent-1"
        }),
    );

    host.spawn_request(request, sender).await;
    let response = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("response should not block")
        .expect("response should be present");

    let response = router_binary_error_json(response);
    assert_eq!(response["type"], "response.error");
    assert!(
        response["error"]["details"].get("frames").is_none(),
        "pre-dispatch request errors should not carry executable diagnostic frames"
    );
    let events = host
        .telemetry
        .drain_batches()
        .into_iter()
        .flat_map(|batch| batch.events)
        .collect::<Vec<_>>();
    let error_event = events
        .iter()
        .find(|event| event.name.as_deref() == Some("request.error"))
        .expect("request.error telemetry event should be emitted");
    assert_eq!(error_event.service_id.as_deref(), Some("service-a"));
    assert_eq!(error_event.revision_id.as_deref(), None);
    assert_eq!(
        error_event.runtime_id.as_deref(),
        Some("runtime-base:service-a")
    );
    assert_eq!(error_event.request_id.as_deref(), Some("request-1"));
    assert_eq!(error_event.trace_id.as_deref(), Some("trace-1"));
    assert_eq!(error_event.span_id.as_deref(), Some("span-1"));
    assert_eq!(error_event.parent_span_id.as_deref(), Some("parent-1"));
    assert_eq!(error_event.target.as_deref(), Some("shared.target"));
    assert!(error_event.duration_ms.unwrap_or_default() >= 0.0);
    assert_eq!(
        error_event
            .error
            .as_ref()
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("UnsupportedRuntimeFeature")
    );
}

#[tokio::test]
async fn interpreter_request_error_includes_executable_diagnostic_frame() {
    let program = Arc::new(runtime_program_configured_for_request_path(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program,
        )],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut request = request(BUILD_A, "program.target");
    request.payload_bytes = b"not-runtime-payload".to_vec();

    host.spawn_request(request, sender).await;
    let response = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("response should not block")
        .expect("response should be present");

    let response = router_binary_error_json(response);
    let details = &response["error"]["details"];
    assert_eq!(response["type"], "response.error");
    assert_eq!(details["frames"][0]["sourceId"], Value::Null);
    assert_eq!(details["frames"][0]["operation"], "run");
    assert_eq!(details["frames"][0]["target"], "program.target");
    assert_eq!(details["frames"][0]["buildId"], BUILD_A);
    assert_eq!(details["frames"][0]["runtimeProgram"], true);
    assert_eq!(details["frames"][0]["fileIrIdentity"], "file:program");
    assert_eq!(details["frames"][0]["modulePath"], "program.main");
    assert_eq!(details["frames"][0]["symbol"], "run");

    let events = host
        .telemetry
        .drain_batches()
        .into_iter()
        .flat_map(|batch| batch.events)
        .collect::<Vec<_>>();
    let error_event = events
        .iter()
        .find(|event| event.name.as_deref() == Some("request.error"))
        .expect("request.error telemetry event should be emitted");
    let telemetry_details = error_event
        .error
        .as_ref()
        .and_then(|error| error.get("details"))
        .expect("telemetry error should include details");
    assert_eq!(telemetry_details["frames"][0]["sourceId"].as_u64(), None);
}

#[tokio::test]
async fn telemetry_down_does_not_block_simple_request_path() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral port should bind");
    let endpoint = format!(
        "ws://{}",
        listener.local_addr().expect("listener should have address")
    );
    drop(listener);

    let program = Arc::new(runtime_program_configured_for_request_path(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program,
        )],
    })
    .expect("host should build");
    host.apply_telemetry_control(&RouterControlEnvelope {
        artifact_roots: vec!["/tmp/skiff-artifacts".into()],
        dev_reload: None,
        mode: None,
        generation: None,
        fingerprint: None,
        service_config: Vec::new(),
        telemetry: Some(skiff_runtime_transport::protocol::TelemetryControlConfig {
            endpoint,
            protocol: skiff_runtime_transport::protocol::TelemetryProtocol::SkiffTelemetryV1,
            topics: vec![skiff_runtime_transport::protocol::TelemetryTopic::Trace],
            queue_max_events: 100,
            batch_max_events: 10,
            batch_max_bytes: 262_144,
            flush_interval_ms: 10,
            enabled: true,
        }),
        file_backend: None,
        extra: serde_json::Map::new(),
    })
    .await;
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut request = request(BUILD_A, "program.target");
    set_request_string_arg(&mut request, "input", "Ada");

    host.spawn_request(request, sender).await;
    let response = timeout(Duration::from_millis(500), receiver.recv())
        .await
        .expect("telemetry outage must not block request")
        .expect("response should be present");

    let (header, _payload) = router_binary_end(response);
    assert_eq!(header.envelope_type, "response.end");
    host.shutdown_telemetry().await;
}

#[tokio::test]
async fn runtime_program_service_routes_registers_and_executes() {
    let program = Arc::new(runtime_program_configured_for_request_path(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program.clone(),
        )],
    })
    .expect("host should build");

    let route = host
        .lookup_operation(&request(BUILD_A, "program.target"))
        .expect("runtime program target should route");
    assert_eq!(route.service.service_id, "service-program");
    assert_eq!(
        route.service.runtime_program_identity.dynamic_build_id,
        BUILD_A
    );
    assert_eq!(
        route.service.runtime_program_identity.linked_image_identity,
        BUILD_A
    );
    assert!(route
        .service
        .linked_image
        .routes
        .contains_key("program.target"));
    assert_eq!(
        route.service.runtime_activation.service.id,
        "service-program"
    );
    assert_eq!(route.operation.operation, "run");
    assert_eq!(route.operation.parameters[0].name, "input");

    let (register_sender, mut register_receiver) = mpsc::unbounded_channel();
    host.queue_registers(register_sender)
        .expect("runtime program register should serialize");
    let capabilities_frame = router_binary(
        register_receiver
            .recv()
            .await
            .expect("runtime capabilities should be queued"),
    );
    let (capabilities, capabilities_payload): (RuntimeCapabilitiesFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&capabilities_frame)
            .expect("runtime.capabilities frame should decode");
    assert!(capabilities_payload.is_empty());
    assert_eq!(capabilities.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
    assert_eq!(capabilities.envelope_type, "runtime.capabilities");
    assert_eq!(capabilities.runtime_id, "runtime-base");
    assert!(capabilities.capabilities.package_test_dispatch);
    assert!(capabilities.capabilities.request_cancel);
    let register_frame = router_binary(
        register_receiver
            .recv()
            .await
            .expect("register should be queued"),
    );
    let (register, register_payload): (RuntimeRegisterFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&register_frame).expect("runtime.register frame should decode");
    assert!(register_payload.is_empty());
    assert_eq!(register.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
    assert_eq!(register.envelope_type, "runtime.register");
    assert_eq!(register.service_id, "service-program");
    assert_eq!(register.build_id, BUILD_A);
    assert_eq!(register.targets, vec!["program.target".to_string()]);
    let register_capabilities = register
        .capabilities
        .as_ref()
        .expect("runtime.register capabilities should be present");
    assert!(register_capabilities.runtime_program);
    assert!(register_capabilities.package_test_dispatch);

    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut request = request(BUILD_A, "program.target");
    set_request_string_arg(&mut request, "input", "Ada");

    host.spawn_request(request, sender).await;
    let response = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("runtime program response should not block")
        .expect("response should be present");

    let (header, payload) = router_binary_end(response);
    assert_eq!(header.envelope_type, "response.end");
    assert_eq!(decode_string_response(&payload), "Ada!");
}

#[tokio::test]
async fn test_invocation_sends_websocket_text_as_typed_binary_connection_send() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let frame = test_actor_client_invocation_with_router_sender(sender);

    frame
        .websocket_context()
        .send_connection_text_to_business_identity(
            "user-1".to_string(),
            "hello typed text".to_string(),
        )
        .expect("sendText should send router frame");

    let message = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("router writer should not block")
        .expect("router writer message should exist");
    let (header, payload): (ConnectionSendFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(message))
            .expect("connection.send frame should decode");

    assert_eq!(header.envelope_type, "connection.send");
    assert_eq!(header.service_id, "service-program");
    assert_eq!(header.business_identity.as_deref(), Some("user-1"));
    assert_eq!(header.connection_id, None);
    assert_eq!(header.payload_kind.as_deref(), Some("text"));
    assert_eq!(payload, b"hello typed text");
}

#[tokio::test]
async fn test_invocation_sends_websocket_binary_as_typed_binary_connection_send() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let frame = test_actor_client_invocation_with_router_sender(sender);

    frame
        .websocket_context()
        .send_connection_binary_to_business_identity("user-1".to_string(), vec![0, 1, 255])
        .expect("sendBinary should send router frame");

    let message = timeout(Duration::from_secs(1), receiver.recv())
        .await
        .expect("router writer should not block")
        .expect("router writer message should exist");
    let (header, payload): (ConnectionSendFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(message))
            .expect("connection.send frame should decode");

    assert_eq!(header.envelope_type, "connection.send");
    assert_eq!(header.service_id, "service-program");
    assert_eq!(header.business_identity.as_deref(), Some("user-1"));
    assert_eq!(header.connection_id, None);
    assert_eq!(header.payload_kind.as_deref(), Some("binary"));
    assert_eq!(payload, vec![0, 1, 255]);
}

#[tokio::test]
async fn runtime_binary_http_request_returns_binary_http_response_body() {
    let program = Arc::new(runtime_program_http_echo_body(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config("runtime-base:http", program)],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let body = b"raw \x00 request body".to_vec();
    let request = request_envelope_from_start_frame(
        RequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.start".to_string(),
            request_id: "request-http-1".to_string(),
            mode: "unary".to_string(),
            caller: RuntimeCallerFrameHeader {
                kind: "gateway".to_string(),
                target: "gateway.http.raw".to_string(),
            },
            target: "program.target".to_string(),
            operation_abi_id: Some(operation_abi_id_for_target("program.target")),
            selector: Some(format!(
                "operation:{}",
                operation_abi_id_for_target("program.target")
            )),
            service_id: None,
            version: None,
            build_id: BUILD_A.to_string(),
            service_protocol_identity: PROTOCOL_A.to_string(),
            activation_identity: None,
            gateway_entry_identity: None,
            business_identity: None,
            websocket_entry_id: None,
            client_session: None,
            deadline: None,
            trace: RuntimeTraceContextFrameHeader {
                trace_id: "trace-http".to_string(),
                span_id: "span-http".to_string(),
                parent_span_id: None,
                sampled: None,
            },
            http_adapter: None,
            websocket_adapter: None,
            http_request: Some(RuntimeHttpRequestFrameHeader {
                method: "POST".to_string(),
                url: "https://example.test/echo?x=1".to_string(),
                path: "/echo".to_string(),
                query: vec![RuntimeHttpNameValueFrameHeader {
                    name: "x".to_string(),
                    value: "1".to_string(),
                }],
                headers: vec![RuntimeHttpNameValueFrameHeader {
                    name: "content-type".to_string(),
                    value: "application/octet-stream".to_string(),
                }],
            }),
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
        },
        body.clone(),
    )
    .expect("binary HTTP request should build");

    host.spawn_request(request, sender).await;
    let response = router_binary(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("runtime program response should not block")
            .expect("response should be present"),
    );
    let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&response).expect("binary response should decode");

    assert_eq!(header.envelope_type, "response.end");
    assert_eq!(header.request_id, "request-http-1");
    assert_eq!(
        header.http_response,
        Some(RuntimeHttpResponseFrameHeader {
            status: 201,
            headers: vec![RuntimeHttpNameValueFrameHeader {
                name: "x-runtime".to_string(),
                value: "rust".to_string(),
            }],
        })
    );
    assert_eq!(payload, body);
}

#[tokio::test]
async fn runtime_typed_http_adapter_decodes_body_and_encodes_json_response() {
    let program = Arc::new(runtime_program_typed_http_body_string(BUILD_A));
    let request = typed_http_request(
        "request-http-typed-body",
        br#""Ada""#.to_vec(),
        typed_json_adapter(
            service_http_adapter_callable("run"),
            vec![http_body_adapter_arg("input")],
            None,
            None,
        ),
    );

    let (header, payload) = run_runtime_program_binary_http_request(program, request).await;

    assert_eq!(header.envelope_type, "response.end");
    assert_eq!(header.request_id, "request-http-typed-body");
    assert_eq!(
        header.http_response,
        Some(RuntimeHttpResponseFrameHeader {
            status: 200,
            headers: vec![RuntimeHttpNameValueFrameHeader {
                name: "content-type".to_string(),
                value: "application/json; charset=utf-8".to_string(),
            }],
        })
    );
    assert_eq!(payload, br#""Ada!""#);
}

#[tokio::test]
async fn runtime_typed_http_adapter_dispatches_package_handler_callable() {
    let package_id = "skiff.run/http-session";
    let symbol_path = "issue";
    let package_target = package_handler_target(package_id, symbol_path);
    let program = Arc::new(runtime_program_typed_http_package_body_string(
        BUILD_A,
        package_id,
        symbol_path,
    ));
    let mut request = typed_http_request(
        "request-http-typed-package",
        br#""Ada""#.to_vec(),
        typed_json_adapter(
            RuntimeHttpAdapterCallableFrameHeader::PackageFunction {
                package_id: package_id.to_string(),
                symbol_path: symbol_path.to_string(),
            },
            vec![http_body_adapter_arg("input")],
            None,
            None,
        ),
    );
    let operation_abi_id = operation_abi_id_for_target(&package_target);
    request.target = package_target;
    request.operation_abi_id = Some(operation_abi_id.clone());
    request.selector = Some(format!("operation:{operation_abi_id}"));

    let (_header, payload) = run_runtime_program_binary_http_request(program, request).await;

    assert_eq!(payload, br#""Ada!""#);
}

#[tokio::test]
async fn runtime_typed_http_adapter_passes_pre_context_to_handler() {
    let program = Arc::new(runtime_program_typed_http_pre_context(BUILD_A));
    let request = typed_http_request(
        "request-http-typed-context",
        Vec::new(),
        typed_json_adapter(
            service_http_adapter_callable("run"),
            vec![http_context_adapter_arg("input")],
            None,
            Some(service_http_adapter_callable("pre")),
        ),
    );

    let (_header, payload) = run_runtime_program_binary_http_request(program, request).await;

    assert_eq!(payload, br#""/echo!""#);
}

#[tokio::test]
async fn runtime_typed_http_adapter_guard_short_circuits_before_body_decode() {
    let program = Arc::new(runtime_program_typed_http_guard_short_circuit(BUILD_A));
    let body = b"not json".to_vec();
    let request = typed_http_request(
        "request-http-typed-guard",
        body.clone(),
        typed_json_adapter(
            service_http_adapter_callable("run"),
            vec![http_body_adapter_arg("input")],
            Some(service_http_adapter_callable("guard")),
            None,
        ),
    );

    let (header, payload) = run_runtime_program_binary_http_request(program, request).await;

    assert_eq!(
        header.http_response,
        Some(RuntimeHttpResponseFrameHeader {
            status: 201,
            headers: vec![RuntimeHttpNameValueFrameHeader {
                name: "x-runtime".to_string(),
                value: "rust".to_string(),
            }],
        })
    );
    assert_eq!(payload, body);
}

#[tokio::test]
async fn runtime_http_adapter_rejects_unknown_adapter_arg_param() {
    let program = Arc::new(runtime_program_typed_http_body_string(BUILD_A));
    let request = typed_http_request(
        "request-http-typed-unknown-param",
        br#""Ada""#.to_vec(),
        typed_json_adapter(
            service_http_adapter_callable("run"),
            vec![http_body_adapter_arg("missing")],
            None,
            None,
        ),
    );

    let error = run_runtime_program_request(program, request).await;

    assert_eq!(error["type"], "response.error");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("arg plan references unknown param missing"),
        "unexpected response: {error}"
    );
}

#[tokio::test]
async fn runtime_raw_http_adapter_passes_pre_context_to_handler() {
    let program = Arc::new(runtime_program_raw_http_pre_context(BUILD_A));
    let body = b"raw adapter body".to_vec();
    let request = typed_http_request(
        "request-http-raw-context",
        body.clone(),
        raw_http_adapter(
            service_http_adapter_callable("run"),
            vec![
                http_context_adapter_arg("context"),
                http_request_adapter_arg("request"),
            ],
            None,
            Some(service_http_adapter_callable("pre")),
        ),
    );

    let (header, payload) = run_runtime_program_binary_http_request(program, request).await;

    assert_eq!(
        header.http_response,
        Some(RuntimeHttpResponseFrameHeader {
            status: 203,
            headers: vec![RuntimeHttpNameValueFrameHeader {
                name: "x-context".to_string(),
                value: "/echo".to_string(),
            }],
        })
    );
    assert_eq!(payload, body);
}

#[tokio::test]
async fn runtime_raw_http_adapter_stream_passes_pre_context_to_handler() {
    let program = Arc::new(runtime_program_raw_http_stream_pre_context(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:http-raw-stream-adapter",
            program,
        )],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let body = b"raw stream adapter chunk".to_vec();
    let mut request = typed_http_request(
        "request-http-raw-stream-context",
        body.clone(),
        raw_http_adapter(
            service_http_adapter_callable("run"),
            vec![
                http_request_adapter_arg("request"),
                http_context_adapter_arg("context"),
            ],
            None,
            Some(service_http_adapter_callable("pre")),
        ),
    );
    request.mode = "serverStream".to_string();

    host.spawn_request(request, sender).await;
    let (start, start_payload) = router_binary_start(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("response.start should not block")
            .expect("response.start should be present"),
    );
    let (chunk, chunk_payload) = router_binary_chunk(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("response.chunk should not block")
            .expect("response.chunk should be present"),
    );
    let (end, end_payload) = router_binary_end(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("response.end should not block")
            .expect("response.end should be present"),
    );

    assert_eq!(start.envelope_type, "response.start");
    assert_eq!(start.request_id, "request-http-raw-stream-context");
    assert_eq!(start_payload, Vec::<u8>::new());
    assert_eq!(
        start.http_response,
        RuntimeHttpResponseFrameHeader {
            status: 202,
            headers: vec![RuntimeHttpNameValueFrameHeader {
                name: "x-context".to_string(),
                value: "/echo".to_string(),
            }],
        }
    );
    assert_eq!(chunk.envelope_type, "response.chunk");
    assert_eq!(chunk.request_id, "request-http-raw-stream-context");
    assert_eq!(chunk.seq, 0);
    assert_eq!(chunk_payload, body);
    assert_eq!(end.envelope_type, "response.end");
    assert_eq!(end.request_id, "request-http-raw-stream-context");
    assert!(!end.payload_present);
    assert_eq!(end.http_response, None);
    assert_eq!(end_payload, Vec::<u8>::new());
}

#[tokio::test]
async fn runtime_binary_http_server_stream_sends_start_chunks_and_end() {
    let program = Arc::new(runtime_program_http_stream_body(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:http-stream",
            program,
        )],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let body = b"stream chunk".to_vec();
    let request = binary_http_stream_request("request-http-stream", body.clone());

    host.spawn_request(request, sender).await;
    let (start, start_payload) = router_binary_start(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("response.start should not block")
            .expect("response.start should be present"),
    );
    let (chunk, chunk_payload) = router_binary_chunk(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("response.chunk should not block")
            .expect("response.chunk should be present"),
    );
    let (end, end_payload) = router_binary_end(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("response.end should not block")
            .expect("response.end should be present"),
    );

    assert_eq!(start.envelope_type, "response.start");
    assert_eq!(start.request_id, "request-http-stream");
    assert_eq!(start_payload, Vec::<u8>::new());
    assert_eq!(
        start.http_response,
        RuntimeHttpResponseFrameHeader {
            status: 202,
            headers: vec![RuntimeHttpNameValueFrameHeader {
                name: "content-type".to_string(),
                value: "text/plain".to_string(),
            }],
        }
    );
    assert_eq!(chunk.envelope_type, "response.chunk");
    assert_eq!(chunk.request_id, "request-http-stream");
    assert_eq!(chunk.seq, 0);
    assert_eq!(chunk_payload, body);
    assert_eq!(end.envelope_type, "response.end");
    assert_eq!(end.request_id, "request-http-stream");
    assert!(!end.payload_present);
    assert_eq!(end.http_response, None);
    assert_eq!(end_payload, Vec::<u8>::new());
}

#[tokio::test]
async fn runtime_binary_http_server_stream_missing_sender_precedes_input_plan_validation() {
    let mut program = runtime_program_http_stream_body(BUILD_A);
    Arc::make_mut(&mut program.service_files[0])
        .executables
        .first_mut()
        .expect("HTTP stream executable should exist")
        .params
        .first_mut()
        .expect("HTTP stream executable should have request parameter")
        .ty = builtin_type_ref("string");
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:http-stream-missing-sender",
            Arc::new(program),
        )],
    })
    .expect("host should build");
    let request = binary_http_stream_request(
        "request-http-stream-missing-sender",
        b"stream chunk".to_vec(),
    );
    let execution_budget = Arc::new(ExecutionBudget::for_runtime_request(&request.extra));
    let cancellation = CancellationToken::new();
    let cancelled = cancellation.cancel_flag();
    let route = host
        .lookup_operation(&request)
        .expect("stream route should resolve");

    let error = host
        .execute_runtime_request(
            route.service,
            route.operation,
            route.addr,
            request,
            cancelled,
            cancellation,
            execution_budget,
            None,
        )
        .await
        .expect_err("missing router sender should fail before input plan validation");
    let message = error.to_string();

    assert!(
        message.contains("binary HTTP serverStream request is missing router sender"),
        "unexpected error: {message}"
    );
    assert!(
        !message.contains("binary HTTP request parameter request must be std.http.HttpRequest"),
        "input plan validation should not run before missing sender: {message}"
    );
}

#[tokio::test]
async fn runtime_program_registers_server_stream_dispatch_mode() {
    let program = Arc::new(runtime_program_http_stream_body(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:http-stream",
            program,
        )],
    })
    .expect("host should build");
    let (register_sender, mut register_receiver) = mpsc::unbounded_channel();

    host.queue_registers(register_sender)
        .expect("runtime program register should serialize");
    let _capabilities_frame = register_receiver
        .recv()
        .await
        .expect("runtime capabilities should be queued before service registers");
    let register_frame = router_binary(
        register_receiver
            .recv()
            .await
            .expect("register should be queued"),
    );
    let (register, _payload): (RuntimeRegisterFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&register_frame).expect("runtime.register frame should decode");
    let dispatch_modes = &register
        .capabilities
        .as_ref()
        .expect("dispatchModes should be registered")
        .dispatch_modes;

    assert!(dispatch_modes.contains(&RuntimeDispatchModeCapability::Unary));
    assert!(dispatch_modes.contains(&RuntimeDispatchModeCapability::ServerStream));
}

#[tokio::test]
async fn runtime_binary_http_request_rejects_std_http_service_symbol_types() {
    let mut program = runtime_program_http_echo_body(BUILD_A);
    let file = Arc::make_mut(&mut program.service_files[0]);
    let executable = file
        .executables
        .first_mut()
        .expect("HTTP echo executable should exist");
    executable.params[0].ty = std_http_fallback_service_symbol_type_ref("HttpRequest");
    executable.return_type = Some(std_http_fallback_service_symbol_type_ref("HttpResponse"));

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:http-fallback",
            Arc::new(program),
        )],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = binary_http_request("request-http-fallback", Vec::new());

    host.spawn_request(request, sender).await;
    let error = router_binary_error_json(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("runtime program response should not block")
            .expect("response should be present"),
    );
    assert_eq!(error["type"], "response.error");
    assert!(error["error"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("RuntimeProgram type ref serviceSymbol was not linked before execution"));
}

#[tokio::test]
async fn runtime_binary_http_request_rejects_std_http_package_symbol_types() {
    let mut program = runtime_program_http_echo_body(BUILD_A);
    let file = Arc::make_mut(&mut program.service_files[0]);
    let executable = file
        .executables
        .first_mut()
        .expect("HTTP echo executable should exist");
    executable.params[0].ty = std_http_fallback_package_symbol_type_ref("HttpRequest");
    executable.return_type = Some(std_http_fallback_package_symbol_type_ref("HttpResponse"));

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:http-package-fallback",
            Arc::new(program),
        )],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = binary_http_request("request-http-package-fallback", Vec::new());

    host.spawn_request(request, sender).await;
    let error = router_binary_error_json(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("runtime program response should not block")
            .expect("response should be present"),
    );
    assert_eq!(error["type"], "response.error");
    assert!(error["error"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("RuntimeProgram type ref packageSymbol was not linked before execution"));
}

#[tokio::test]
async fn runtime_binary_http_request_rejects_non_http_fallback_service_symbol_type() {
    let mut program = runtime_program_http_echo_body(BUILD_A);
    let file = Arc::make_mut(&mut program.service_files[0]);
    let executable = file
        .executables
        .first_mut()
        .expect("HTTP echo executable should exist");
    executable.params[0].ty = service_symbol_type_ref("", "std.http.NotHttpRequest");

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:http-reject",
            Arc::new(program),
        )],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let request = binary_http_request("request-http-reject", Vec::new());

    host.spawn_request(request, sender).await;
    let error = router_binary_error_json(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("runtime program response should not block")
            .expect("response should be present"),
    );

    assert_eq!(error["type"], "response.error");
    assert!(error["error"]["message"]
        .as_str()
        .expect("error message should be a string")
        .contains("RuntimeProgram type ref serviceSymbol was not linked before execution"));
}

#[tokio::test]
async fn runtime_binary_operation_decodes_payload_args_and_encodes_response_payload() {
    let program = Arc::new(runtime_program_configured_for_request_path(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program,
        )],
    })
    .expect("host should build");
    let args_descriptor = json!({
        "kind": "record",
        "fields": {
            "input": { "kind": "builtin", "name": "string", "args": [] }
        }
    });
    let mut heap = RequestHeap::default();
    let args_handle = heap
        .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([(
            "input".to_string(),
            RuntimeValue::String("Ada".to_string()),
        )])))
        .expect("args record should allocate");
    let payload = encode_payload(&RuntimeValue::Heap(args_handle), &args_descriptor, &heap)
        .expect("args payload should encode");
    let mut request = request(BUILD_A, "program.target");
    request.payload_bytes = payload;

    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_request(request, sender).await;
    let response = router_binary(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("runtime program response should not block")
            .expect("response should be present"),
    );
    let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&response).expect("binary response should decode");

    assert_eq!(header.envelope_type, "response.end");
    assert_eq!(header.request_id, "request-1");
    assert_eq!(header.http_response, None);
    let mut response_heap = RequestHeap::default();
    let decoded = decode_payload(
        &payload,
        &json!({ "kind": "builtin", "name": "string", "args": [] }),
        &mut response_heap,
    )
    .expect("response payload should decode");
    assert_eq!(decoded, RuntimeValue::String("Ada!".to_string()));
    assert!(response_heap.is_empty());
}

#[tokio::test]
async fn runtime_binary_operation_coerces_map_literal_return_to_union_record_payload() {
    let mut program = runtime_program_configured_for_request_path(BUILD_A);
    let accept_type = LinkedTypeRef::Record {
        fields: BTreeMap::from([
            (
                "tag".to_string(),
                LinkedTypeRef::Literal {
                    value: LiteralIr::String {
                        value: "accept".to_string(),
                    },
                },
            ),
            (
                "context".to_string(),
                LinkedTypeRef::Record {
                    fields: BTreeMap::from([("userId".to_string(), builtin_type_ref("string"))]),
                },
            ),
            ("identity".to_string(), builtin_type_ref("string")),
        ]),
    };
    let reject_type = LinkedTypeRef::Record {
        fields: BTreeMap::from([
            (
                "tag".to_string(),
                LinkedTypeRef::Literal {
                    value: LiteralIr::String {
                        value: "reject".to_string(),
                    },
                },
            ),
            ("reason".to_string(), builtin_type_ref("string")),
        ]),
    };
    let return_type = LinkedTypeRef::Union {
        items: vec![accept_type.clone(), reject_type],
    };
    let executable = Arc::make_mut(&mut program.service_files[0])
        .executables
        .get_mut(0)
        .expect("program executable");
    executable.params.clear();
    executable.return_type = Some(return_type);
    executable.slots = SlotLayoutIr {
        slots: Vec::new(),
        frame_size: 0,
    };
    executable.body = runtime_program_body(json!({
        "blocks": [
            {
                "label": "entry",
                "statements": [
                    { "statement": 0 }
                ]
            }
        ],
        "statements": [
            {
                "kind": "return",
                "value": { "expression": 4 }
            }
        ],
        "expressions": [
            { "kind": "literal", "value": { "kind": "string", "value": "accept" } },
            { "kind": "literal", "value": { "kind": "string", "value": "user-1" } },
            {
                "kind": "mapLiteral",
                "entries": {
                    "userId": { "expression": 1 }
                }
            },
            { "kind": "literal", "value": { "kind": "string", "value": "user-1" } },
            {
                "kind": "mapLiteral",
                "entries": {
                    "context": { "expression": 2 },
                    "identity": { "expression": 3 },
                    "tag": { "expression": 0 }
                }
            }
        ],
    }));

    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            Arc::new(program),
        )],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_request(request(BUILD_A, "program.target"), sender)
        .await;
    let response = router_binary(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("runtime program response should not block")
            .expect("response should be present"),
    );
    let (header, payload): (ResponseEndFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&response).expect("binary response should decode");

    assert_eq!(header.envelope_type, "response.end");
    let mut response_heap = RequestHeap::default();
    let decoded = decode_payload(
        &payload,
        &json!({
            "kind": "union",
            "items": [
                {
                    "kind": "record",
                    "fields": {
                        "tag": {
                            "kind": "literal",
                            "value": { "kind": "string", "value": "accept" }
                        },
                        "context": {
                            "kind": "record",
                            "fields": {
                                "userId": { "kind": "builtin", "name": "string", "args": [] }
                            }
                        },
                        "identity": { "kind": "builtin", "name": "string", "args": [] }
                    }
                },
                {
                    "kind": "record",
                    "fields": {
                        "tag": {
                            "kind": "literal",
                            "value": { "kind": "string", "value": "reject" }
                        },
                        "reason": { "kind": "builtin", "name": "string", "args": [] }
                    }
                }
            ]
        }),
        &mut response_heap,
    )
    .expect("union record response payload should decode");
    let RuntimeValue::Heap(handle) = decoded else {
        panic!("expected decoded response object");
    };
    let HeapNode::Object(object) = response_heap
        .get(handle)
        .expect("decoded object should be present")
    else {
        panic!("expected decoded response object");
    };
    assert_eq!(
        object.fields().get("tag"),
        Some(&RuntimeValue::String("accept".to_string()))
    );
}

#[tokio::test]
async fn runtime_program_request_rejects_unlinked_call_target() {
    let mut program = runtime_program_configured_for_request_path(BUILD_A);
    let executable = Arc::make_mut(&mut program.service_files[0])
        .executables
        .get_mut(0)
        .expect("program executable");
    executable.params.clear();
    executable.slots = SlotLayoutIr::default();
    executable.body = runtime_program_body(json!({
        "blocks": [{ "label": "entry", "statements": [{ "statement": 0 }] }],
        "statements": [{ "kind": "return", "value": { "expression": 0 } }],
        "expressions": [{
            "kind": "call",
            "call": {
                "target": { "kind": "localExecutable", "executableIndex": 0 },
                "args": []
            }
        }]
    }));
    assert!(matches!(
        executable.body.expressions[0],
        LinkedExprIr::Call { ref call }
            if matches!(call.target, LinkedCallTarget::LocalExecutable { .. })
    ));

    let response =
        run_runtime_program_request(Arc::new(program), request(BUILD_A, "program.target")).await;

    assert_eq!(response["type"], "response.error");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("was not linked before execution"),
        "unexpected response: {response}"
    );
}

#[tokio::test]
async fn runtime_program_request_rejects_unlinked_response_type() {
    let mut program = runtime_program_configured_for_request_path(BUILD_A);
    let executable = Arc::make_mut(&mut program.service_files[0])
        .executables
        .get_mut(0)
        .expect("program executable");
    executable.return_type = Some(LinkedTypeRef::LocalType { type_index: 0 });

    let mut request = request(BUILD_A, "program.target");
    set_request_string_arg(&mut request, "input", "Ada");
    let response = run_runtime_program_request(Arc::new(program), request).await;

    assert_eq!(response["type"], "response.error");
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("type ref localType was not linked before execution"),
        "unexpected response: {response}"
    );
}

#[tokio::test]
async fn actor_client_send_failure_removes_pending_response() {
    let (sender, receiver) = mpsc::unbounded_channel();
    drop(receiver);
    let frame = test_actor_client_invocation_with_router_sender(sender);
    let client = ActorClient::new(actor_client_context(&frame));

    let error = client
        .find(ActorFindControlRequest {
            rpc_id: String::new(),
            runtime_id: String::new(),
            actor_key: ActorKeyControlMetadata {
                service_id: "service-program".to_string(),
                actor_type_identity: "internal.ThreadActor".to_string(),
                actor_id_type_identity: "string".to_string(),
                actor_id_encoding_version: "runtime-json-v1".to_string(),
                canonical_actor_id_key_bytes_base64: "InRocmVhZC0xIg==".to_string(),
                actor_id_hash: Some(
                    "sha256:605d0edc19c41397f6f049dad0d7b3bbcc28a8a7dddbf4ebb8eb9f8b6e766b38"
                        .to_string(),
                ),
            },
        })
        .await
        .expect_err("closed router writer should fail");

    assert!(matches!(
        error,
        crate::error::RuntimeError::ProviderUnavailable { .. }
    ));
    assert!(!frame
        .outbound_requests
        .contains_matching(|request_id| request_id.contains("actor.find")));
}

#[tokio::test]
async fn actor_client_put_sends_rpc_and_decodes_response_header() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let frame = test_actor_client_invocation_with_router_sender(sender);
    let actor_key = ActorKeyControlMetadata {
        service_id: "service-program".to_string(),
        actor_type_identity: "internal.ThreadActor".to_string(),
        actor_id_type_identity: "string".to_string(),
        actor_id_encoding_version: "runtime-json-v1".to_string(),
        canonical_actor_id_key_bytes_base64: "InRocmVhZC0xIg==".to_string(),
        actor_id_hash: Some(
            "sha256:605d0edc19c41397f6f049dad0d7b3bbcc28a8a7dddbf4ebb8eb9f8b6e766b38".to_string(),
        ),
    };
    let client = ActorClient::new(actor_client_context(&frame));
    let put = client.put(
        ActorPutControlRequest {
            rpc_id: String::new(),
            runtime_id: String::new(),
            actor_key: actor_key.clone(),
            object_schema_identity: "internal.ThreadActor".to_string(),
            object_encoding_version: "runtime-json-v1".to_string(),
        },
        br#"{"threadId":"thread-1"}"#.to_vec(),
    );
    tokio::pin!(put);

    let message = tokio::select! {
        result = &mut put => panic!("actor put completed before router response: {result:?}"),
        message = receiver.recv() => message.expect("actor put request should be sent"),
    };
    let (request, payload): (ActorPutRequestFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(message))
            .expect("actor.put.request should decode");
    assert_eq!(request.envelope_type, "actor.put.request");
    assert_eq!(request.schema_version, RUNTIME_FRAME_SCHEMA_VERSION);
    assert_eq!(request.runtime_id, "runtime-test");
    assert_eq!(request.actor_key.service_id, actor_key.service_id);
    assert_eq!(
        request.actor_key.actor_type_identity,
        actor_key.actor_type_identity
    );
    assert_eq!(
        request.actor_key.actor_id_type_identity,
        actor_key.actor_id_type_identity
    );
    assert_eq!(
        request.actor_key.actor_id_encoding_version,
        actor_key.actor_id_encoding_version
    );
    assert_eq!(
        request.actor_key.canonical_actor_id_key_bytes_base64,
        actor_key.canonical_actor_id_key_bytes_base64
    );
    assert_eq!(request.actor_key.actor_id_hash, actor_key.actor_id_hash);
    assert_eq!(payload, br#"{"threadId":"thread-1"}"#);

    let response = ActorPutResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "actor.put.response".to_string(),
        rpc_id: request.rpc_id.clone(),
        actor_ref: ActorRefFrameMetadata {
            service_id: request.actor_key.service_id.clone(),
            actor_type_identity: request.actor_key.actor_type_identity.clone(),
            actor_id_type_identity: request.actor_key.actor_id_type_identity.clone(),
            actor_id_encoding_version: request.actor_key.actor_id_encoding_version.clone(),
            canonical_actor_id_key_bytes_base64: request
                .actor_key
                .canonical_actor_id_key_bytes_base64
                .clone(),
            actor_id_hash: request.actor_key.actor_id_hash.clone().unwrap(),
            epoch: Some(1),
        },
    };
    let pending = frame
        .outbound_requests
        .complete(&request.rpc_id)
        .expect("actor put rpc should be pending");
    pending
        .send(OutboundResponse::End {
            payload: serde_json::to_vec(&response).expect("response header serializes"),
        })
        .expect("pending actor put response should deliver");

    let actor_ref = put.await.expect("actor put response should decode");
    assert_eq!(actor_ref.service_id(), "service-program");
    assert_eq!(actor_ref.actor_type_identity(), "internal.ThreadActor");
    assert_eq!(actor_ref.epoch(), Some(1));
}

#[tokio::test]
async fn spawn_worker_claim_executes_function_and_completes_item() {
    let program = Arc::new(runtime_program_configured_for_spawn_path(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program.clone(),
        )],
    })
    .expect("host should build");
    let service = host.service_snapshot()[0].clone();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut spawned_request = request(BUILD_A, "program.target");
    set_spawn_request_string_arg(&mut spawned_request, "input", "Ada");

    let claim_once = super::spawn_worker::claim_once_for_test(
        host.clone(),
        sender,
        service,
        "test-worker".to_string(),
    );
    tokio::pin!(claim_once);

    let claim_request = tokio::select! {
        result = &mut claim_once => panic!("spawn worker completed before claim response: {result:?}"),
        message = receiver.recv() => message.expect("spawn.claim.request should be sent"),
    };
    let (claim_header, payload): (SpawnClaimRequestFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(claim_request))
            .expect("spawn.claim.request should decode");
    assert!(payload.is_empty());
    assert_eq!(claim_header.envelope_type, "spawn.claim.request");
    assert_eq!(claim_header.runtime_id, "runtime-base:program");
    assert_eq!(claim_header.service_id, "service-program");
    assert_eq!(claim_header.service_version, "v1");
    assert_eq!(claim_header.service_protocol_identity, PROTOCOL_A);
    assert_eq!(claim_header.build_id.as_deref(), Some(BUILD_A));
    assert_eq!(
        claim_header.supported_targets,
        vec!["function:program.target".to_string()]
    );

    deliver_spawn_claim_response(
        &host,
        &claim_header.rpc_id,
        spawn_claim_descriptor("function:program.target"),
        spawned_request.payload_bytes,
    );

    let complete_request = tokio::select! {
        result = &mut claim_once => panic!("spawn worker completed before terminal response: {result:?}"),
        message = receiver.recv() => message.expect("spawn.complete.request should be sent"),
    };
    let (complete_header, payload): (SpawnCompleteRequestFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(complete_request))
            .expect("spawn.complete.request should decode");
    assert!(payload.is_empty());
    assert_eq!(complete_header.envelope_type, "spawn.complete.request");
    assert_eq!(complete_header.item_id, "spawn-item-test");
    assert_eq!(complete_header.lease_id, "spawn-lease-test");

    deliver_spawn_complete_response(&host, &complete_header.rpc_id);
    let outcome = claim_once
        .await
        .expect("spawn worker should complete successful claim");
    assert!(matches!(
        outcome,
        super::spawn_worker::ClaimOutcome::Claimed
    ));
}

#[tokio::test]
async fn spawn_worker_rejects_claimed_item_from_different_build_before_execution() {
    let program = Arc::new(runtime_program_configured_for_spawn_path(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program.clone(),
        )],
    })
    .expect("host should build");
    let service = host.service_snapshot()[0].clone();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut spawned_request = request(BUILD_A, "program.target");
    set_spawn_request_string_arg(&mut spawned_request, "input", "Ada");

    let claim_once = super::spawn_worker::claim_once_for_test(
        host.clone(),
        sender,
        service,
        "test-worker".to_string(),
    );
    tokio::pin!(claim_once);

    let claim_request = tokio::select! {
        result = &mut claim_once => panic!("spawn worker completed before claim response: {result:?}"),
        message = receiver.recv() => message.expect("spawn.claim.request should be sent"),
    };
    let (claim_header, _payload): (SpawnClaimRequestFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(claim_request))
            .expect("spawn.claim.request should decode");
    let mut descriptor = spawn_claim_descriptor("function:program.target");
    descriptor.build_id = BUILD_B.to_string();
    deliver_spawn_claim_response(
        &host,
        &claim_header.rpc_id,
        descriptor,
        spawned_request.payload_bytes,
    );

    let fail_request = tokio::select! {
        result = &mut claim_once => panic!("spawn worker completed before fail response: {result:?}"),
        message = receiver.recv() => message.expect("spawn.fail.request should be sent"),
    };
    let (fail_header, payload): (SpawnFailRequestFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(fail_request))
            .expect("spawn.fail.request should decode");
    assert!(payload.is_empty());
    assert_eq!(fail_header.envelope_type, "spawn.fail.request");
    let error = fail_header
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.get("error"))
        .expect("spawn fail diagnostics should include error");
    let error_text = error.to_string();
    assert!(
        error_text.contains("buildId"),
        "unexpected diagnostics: {error_text}"
    );

    deliver_spawn_fail_response(&host, &fail_header.rpc_id);
    let outcome = claim_once
        .await
        .expect("spawn worker should finish failed wrong-build claim");
    assert!(matches!(
        outcome,
        super::spawn_worker::ClaimOutcome::Claimed
    ));
}

#[tokio::test]
async fn spawn_worker_sends_renew_request_and_accepts_response() {
    let program = Arc::new(runtime_program_configured_for_spawn_path(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program.clone(),
        )],
    })
    .expect("host should build");
    let service = host.service_snapshot()[0].clone();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let descriptor = spawn_claim_descriptor("function:program.target");

    let renew_once = super::spawn_worker::renew_once_for_test(
        host.clone(),
        sender,
        service,
        "test-worker".to_string(),
        descriptor,
    );
    tokio::pin!(renew_once);

    let renew_request = tokio::select! {
        result = &mut renew_once => panic!("spawn worker completed before renew response: {result:?}"),
        message = receiver.recv() => message.expect("spawn.renew.request should be sent"),
    };
    let (renew_header, payload): (SpawnRenewRequestFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(renew_request))
            .expect("spawn.renew.request should decode");
    assert!(payload.is_empty());
    assert_eq!(renew_header.envelope_type, "spawn.renew.request");
    assert_eq!(renew_header.runtime_id, "runtime-base:program");
    assert_eq!(renew_header.item_id, "spawn-item-test");
    assert_eq!(renew_header.lease_id, "spawn-lease-test");
    assert_eq!(renew_header.worker_id, "test-worker");

    deliver_spawn_renew_response(&host, &renew_header.rpc_id);
    renew_once
        .await
        .expect("spawn renew response should complete worker renew rpc");
}

#[tokio::test]
async fn spawn_worker_reports_failed_item_when_execution_errors() {
    let program = Arc::new(runtime_program_configured_for_spawn_path(BUILD_A));
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program.clone(),
        )],
    })
    .expect("host should build");
    let service = host.service_snapshot()[0].clone();
    let (sender, mut receiver) = mpsc::unbounded_channel();

    let claim_once = super::spawn_worker::claim_once_for_test(
        host.clone(),
        sender,
        service,
        "test-worker".to_string(),
    );
    tokio::pin!(claim_once);

    let claim_request = tokio::select! {
        result = &mut claim_once => panic!("spawn worker completed before claim response: {result:?}"),
        message = receiver.recv() => message.expect("spawn.claim.request should be sent"),
    };
    let (claim_header, _payload): (SpawnClaimRequestFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(claim_request))
            .expect("spawn.claim.request should decode");

    deliver_spawn_claim_response(
        &host,
        &claim_header.rpc_id,
        spawn_claim_descriptor("missing.spawn.target"),
        Vec::new(),
    );

    let fail_request = tokio::select! {
        result = &mut claim_once => panic!("spawn worker completed before fail response: {result:?}"),
        message = receiver.recv() => message.expect("spawn.fail.request should be sent"),
    };
    let (fail_header, payload): (SpawnFailRequestFrameHeader, Vec<u8>) =
        decode_typed_binary_frame(&router_binary(fail_request))
            .expect("spawn.fail.request should decode");
    assert!(payload.is_empty());
    assert_eq!(fail_header.envelope_type, "spawn.fail.request");
    assert_eq!(fail_header.reason, "failed");
    assert_eq!(fail_header.item_id, "spawn-item-test");
    assert!(fail_header
        .diagnostics
        .as_ref()
        .and_then(|diagnostics| diagnostics.get("error"))
        .is_some());

    deliver_spawn_fail_response(&host, &fail_header.rpc_id);
    let outcome = claim_once
        .await
        .expect("spawn worker should finish failed claim after reporting fail");
    assert!(matches!(
        outcome,
        super::spawn_worker::ClaimOutcome::Claimed
    ));
}

fn spawn_claim_descriptor(target: &str) -> SpawnClaimDescriptorFrameMetadata {
    SpawnClaimDescriptorFrameMetadata {
        item_id: "spawn-item-test".to_string(),
        lease_id: "spawn-lease-test".to_string(),
        spawn_execution_id: "spawn-exec-test".to_string(),
        runtime_request_id: "spawn-request-test".to_string(),
        spawn_id: "spawn-test".to_string(),
        target_kind: "function".to_string(),
        target: target.to_string(),
        service_id: "service-program".to_string(),
        service_version: "v1".to_string(),
        service_protocol_identity: PROTOCOL_A.to_string(),
        build_id: BUILD_A.to_string(),
        payload_schema_identity: Some(format!("skiff-spawn-payload-v1:{PROTOCOL_A}:{target}")),
        lease_expires_at: Some("2026-06-06T10:00:30.000Z".to_string()),
    }
}

fn deliver_spawn_claim_response(
    host: &RuntimeHost,
    rpc_id: &str,
    descriptor: SpawnClaimDescriptorFrameMetadata,
    payload_bytes: Vec<u8>,
) {
    let header = SpawnClaimResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "spawn.claim.response".to_string(),
        rpc_id: rpc_id.to_string(),
        claimed: true,
        item: Some(descriptor),
    };
    let payload = spawn_claim_response_control_payload(header, &payload_bytes)
        .expect("claim response should serialize");
    let pending = host
        .outbound_requests
        .complete(rpc_id)
        .expect("spawn claim rpc should be pending");
    pending
        .send(OutboundResponse::End { payload })
        .expect("spawn claim response should deliver");
}

fn deliver_spawn_complete_response(host: &RuntimeHost, rpc_id: &str) {
    let response = SpawnCompleteResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "spawn.complete.response".to_string(),
        rpc_id: rpc_id.to_string(),
        item_id: "spawn-item-test".to_string(),
        status: "completed".to_string(),
    };
    let pending = host
        .outbound_requests
        .complete(rpc_id)
        .expect("spawn complete rpc should be pending");
    pending
        .send(OutboundResponse::End {
            payload: serde_json::to_vec(&response).expect("complete response serializes"),
        })
        .expect("spawn complete response should deliver");
}

fn deliver_spawn_renew_response(host: &RuntimeHost, rpc_id: &str) {
    let response = SpawnRenewResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "spawn.renew.response".to_string(),
        rpc_id: rpc_id.to_string(),
        item_id: "spawn-item-test".to_string(),
        renewed: true,
        lease_expires_at: Some("2026-06-06T10:00:30.000Z".to_string()),
    };
    let pending = host
        .outbound_requests
        .complete(rpc_id)
        .expect("spawn renew rpc should be pending");
    pending
        .send(OutboundResponse::End {
            payload: serde_json::to_vec(&response).expect("renew response serializes"),
        })
        .expect("spawn renew response should deliver");
}

fn deliver_spawn_fail_response(host: &RuntimeHost, rpc_id: &str) {
    let response = SpawnFailResponseFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        envelope_type: "spawn.fail.response".to_string(),
        rpc_id: rpc_id.to_string(),
        item_id: "spawn-item-test".to_string(),
        status: "failed".to_string(),
    };
    let pending = host
        .outbound_requests
        .complete(rpc_id)
        .expect("spawn fail rpc should be pending");
    pending
        .send(OutboundResponse::End {
            payload: serde_json::to_vec(&response).expect("fail response serializes"),
        })
        .expect("spawn fail response should deliver");
}

async fn service_config(
    runtime_id: &str,
    service_id: &str,
    protocol_identity: &str,
    target: &str,
) -> RuntimeServiceConfig {
    service_config_with_build_and_contract(
        runtime_id,
        service_id,
        protocol_identity,
        build_id_for_protocol(protocol_identity),
        protocol_identity,
        target,
    )
    .await
}

async fn service_config_with_build(
    runtime_id: &str,
    service_id: &str,
    protocol_identity: &str,
    build_id: &str,
    target: &str,
) -> RuntimeServiceConfig {
    service_config_with_build_and_contract(
        runtime_id,
        service_id,
        protocol_identity,
        build_id,
        protocol_identity,
        target,
    )
    .await
}

async fn service_config_with_build_and_contract(
    runtime_id: &str,
    service_id: &str,
    _protocol_identity: &str,
    build_id: &str,
    contract_identity: &str,
    target: &str,
) -> RuntimeServiceConfig {
    let program = Arc::new(runtime_program_for_service(service_id, build_id, target));
    runtime_program_service_config_with_contract(runtime_id, contract_identity, program)
}

fn runtime_program_service_config(
    runtime_id: &str,
    program: Arc<RuntimeProgram>,
) -> RuntimeServiceConfig {
    runtime_program_service_config_with_contract(runtime_id, PROTOCOL_A, program)
}

fn runtime_host_error_for_program(program: RuntimeProgram) -> String {
    let result = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            Arc::new(program),
        )],
    });
    match result {
        Ok(_) => panic!("runtime host should reject malformed route binding"),
        Err(error) => error.to_string(),
    }
}

fn runtime_program_service_config_with_contract(
    runtime_id: &str,
    contract_identity: &str,
    program: Arc<RuntimeProgram>,
) -> RuntimeServiceConfig {
    let package_configs = program.package_configs.clone();
    let runtime_program_layers = Arc::new(RuntimeProgramLayers::new(
        program.runtime_program_identity(),
        Arc::new(program.linked_image()),
        Arc::new(program.activation_view()),
    ));
    RuntimeServiceConfig {
        runtime_program_identity: runtime_program_layers.identity.clone(),
        linked_image: runtime_program_layers.image.clone(),
        runtime_activation: runtime_program_layers.activation.clone(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        use_runtime_default_http_response_max_bytes: true,
        runtime_id: runtime_id.to_string(),
        revision_id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
        contract_identity: contract_identity.to_string(),
        implementation_identity: format!("impl-{runtime_id}"),
        artifact_identity: format!("skiff-runtime-program-v1:sha256:{runtime_id}"),
        activation_identity: None,
        resolved_config_identity: None,
        config: RuntimeConfigView::empty(),
        package_configs,
        service_db: None,
    }
}

fn runtime_program_configured_for_request_path(build_id: &str) -> RuntimeProgram {
    runtime_program_for_service("service-program", build_id, "program.target")
}

fn runtime_program_configured_for_spawn_path(build_id: &str) -> RuntimeProgram {
    let mut program = runtime_program_configured_for_request_path(build_id);
    let addr = ExecutableAddr::service(0, 0);
    program
        .routes
        .insert("function:program.target".to_string(), addr.clone());
    program
        .spawn_routes
        .insert("function:program.target".to_string(), addr);
    program
}

fn runtime_program_typed_http_body_string(build_id: &str) -> RuntimeProgram {
    let mut program = runtime_program_for_service("service-http", build_id, "program.target");
    let file = Arc::make_mut(&mut program.service_files[0]);
    file.executables = vec![runtime_program_echo_executable()];
    link_service_symbol(&mut program, "program.main", "run", 0);
    program
}

fn runtime_program_typed_http_pre_context(build_id: &str) -> RuntimeProgram {
    let mut program = runtime_program_for_service("service-http", build_id, "program.target");
    attach_std_http_types(&mut program);
    let file = Arc::make_mut(&mut program.service_files[0]);
    file.executables = vec![
        runtime_program_echo_executable(),
        runtime_program_http_request_path_executable(),
    ];
    link_service_symbol(&mut program, "program.main", "run", 0);
    link_service_symbol(&mut program, "program.main", "pre", 1);
    program
}

fn runtime_program_raw_http_pre_context(build_id: &str) -> RuntimeProgram {
    let mut program = runtime_program_for_service("service-http", build_id, "program.target");
    attach_std_http_types(&mut program);
    let file = Arc::make_mut(&mut program.service_files[0]);
    file.executables = vec![
        runtime_program_http_context_response_executable(),
        runtime_program_http_request_path_executable(),
    ];
    link_service_symbol(&mut program, "program.main", "run", 0);
    link_service_symbol(&mut program, "program.main", "pre", 1);
    program
}

fn runtime_program_raw_http_stream_pre_context(build_id: &str) -> RuntimeProgram {
    let mut program = runtime_program_for_service("service-http", build_id, "program.target");
    attach_std_http_types(&mut program);
    let file = Arc::make_mut(&mut program.service_files[0]);
    file.executables = vec![
        runtime_program_http_stream_context_executable(),
        runtime_program_http_request_path_executable(),
    ];
    link_service_symbol(&mut program, "program.main", "run", 0);
    link_service_symbol(&mut program, "program.main", "pre", 1);
    program
}

fn runtime_program_typed_http_guard_short_circuit(build_id: &str) -> RuntimeProgram {
    let mut program = runtime_program_for_service("service-http", build_id, "program.target");
    attach_std_http_types(&mut program);
    let mut guard = runtime_program_http_echo_body_executable();
    guard.symbol = "guard".to_string();
    guard.return_type = Some(LinkedTypeRef::Nullable {
        inner: Box::new(std_http_type_ref("HttpResponse")),
    });
    let file = Arc::make_mut(&mut program.service_files[0]);
    file.executables = vec![runtime_program_echo_executable(), guard];
    link_service_symbol(&mut program, "program.main", "run", 0);
    link_service_symbol(&mut program, "program.main", "guard", 1);
    program
}

fn runtime_program_typed_http_package_body_string(
    build_id: &str,
    package_id: &str,
    symbol_path: &str,
) -> RuntimeProgram {
    let package_target = package_handler_target(package_id, symbol_path);
    let package_addr = ExecutableAddr::package(0, 0, 0);
    let mut program = runtime_program_for_service("service-http", build_id, &package_target);
    program.packages = vec![Arc::new(package_unit(package_id))];
    program.package_files = vec![vec![Arc::new(LinkedFileUnit {
        schema_version: "skiff-file-ir-v3".to_string(),
        file_ir_identity: "file:package".to_string(),
        source_ast_hash: "source:package".to_string(),
        module_path: "package.main".to_string(),
        ir_format_version: None,
        opcode_table_version: None,
        source_map: Default::default(),
        declarations: FileDeclarations::default(),
        link_targets: FileLinkTargets::default(),
        types: Vec::new(),
        constants: Vec::new(),
        executables: vec![runtime_program_echo_executable()],
        external_refs: Default::default(),
    })]];
    program
        .link_overlay
        .package_slots_by_id
        .insert(package_id.to_string(), 0);
    program.link_overlay.symbols.insert(
        format!("package[0]::{symbol_path}"),
        ResolvedSymbol::Executable {
            addr: package_addr.clone(),
        },
    );
    let operation_abi_id = operation_abi_id_for_target(&package_target);
    program.routes = HashMap::from([(package_target, package_addr.clone())]);
    program.operations = HashMap::from([(operation_abi_id, package_addr)]);
    program
}

fn runtime_program_http_echo_body(build_id: &str) -> RuntimeProgram {
    let mut program = runtime_program_for_service("service-http", build_id, "program.target");
    attach_std_http_types(&mut program);
    let file = Arc::make_mut(&mut program.service_files[0]);
    file.executables = vec![runtime_program_http_echo_body_executable()];
    program
}

fn runtime_program_http_stream_body(build_id: &str) -> RuntimeProgram {
    let mut program = runtime_program_for_service("service-http", build_id, "program.target");
    attach_std_http_types(&mut program);
    let file = Arc::make_mut(&mut program.service_files[0]);
    file.executables = vec![runtime_program_http_stream_body_executable()];
    program
}

async fn run_runtime_program_request(
    program: Arc<RuntimeProgram>,
    request: RequestEnvelope,
) -> Value {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program,
        )],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_request(request, sender).await;
    router_binary_error_json(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("runtime program response should not block")
            .expect("response should be present"),
    )
}

async fn run_runtime_program_binary_http_request(
    program: Arc<RuntimeProgram>,
    request: RequestEnvelope,
) -> (ResponseEndFrameHeader, Vec<u8>) {
    let host = RuntimeHost::new(RuntimeConfig {
        db_provider: skiff_runtime_capability_context::DbProviderSource::unavailable(),
        router_url: "ws://127.0.0.1:4001/runtime".to_string(),
        base_runtime_id: "runtime-base".to_string(),
        runtime_home: std::env::temp_dir().join("skiff-runtime-test-home"),
        artifact_roots: Vec::new(),
        http_response_max_bytes: crate::config::DEFAULT_HTTP_RESPONSE_MAX_BYTES,
        http_egress_proxy: None,
        services: vec![runtime_program_service_config(
            "runtime-base:program",
            program,
        )],
    })
    .expect("host should build");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_request(request, sender).await;
    router_binary_end(
        timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("runtime program binary HTTP response should not block")
            .expect("response should be present"),
    )
}
fn runtime_program_for_service(service_id: &str, build_id: &str, target: &str) -> RuntimeProgram {
    let addr = ExecutableAddr::service(0, 0);
    let operation_abi_id = operation_abi_id_for_target(target);
    RuntimeProgram {
        service: ServiceMeta {
            id: service_id.to_string(),
            display_name: Some(format!("{service_id} Service")),
            metadata: Default::default(),
        },
        version: "v1".to_string(),
        build_id: build_id.to_string(),
        service_files: vec![Arc::new(LinkedFileUnit {
            schema_version: "skiff-file-ir-v3".to_string(),
            file_ir_identity: "file:program".to_string(),
            source_ast_hash: "source:program".to_string(),
            module_path: "program.main".to_string(),
            ir_format_version: None,
            opcode_table_version: None,
            source_map: Default::default(),
            declarations: FileDeclarations::default(),
            link_targets: FileLinkTargets::default(),
            types: Vec::new(),
            constants: Vec::new(),
            executables: vec![runtime_program_echo_executable()],
            external_refs: Default::default(),
        })],
        packages: Vec::new(),
        package_files: Vec::new(),
        package_configs: Vec::new(),
        service_dependencies: Vec::new(),
        timeout: Default::default(),
        operation_route_bindings: vec![OperationRouteBinding {
            ingress_kind: OperationIngressKind::ServiceCall,
            selector: format!("operation:{operation_abi_id}"),
            operation_abi_id: operation_abi_id.clone(),
        }],
        routes: HashMap::from([(target.to_string(), addr.clone())]),
        spawn_routes: HashMap::new(),
        operations: HashMap::from([(operation_abi_id, addr)]),
        operation_receivers: HashMap::new(),
        db: Vec::new(),
        actors: Vec::new(),
        link_overlay: LinkOverlay::default(),
        gateway: serde_json::from_value(json!({
            "metadata": {
                "webSockets": {
                    "connect": {
                        "gatewayEntryIdentity": "skiff-gateway-v1:sha256:1111111111111111111111111111111111111111111111111111111111111111"
                    }
                }
            }
        }))
        .expect("gateway config fixture should deserialize"),
        types: RuntimeTypeContext::default(),
    }
}

fn attach_std_http_types(program: &mut RuntimeProgram) {
    for (name, descriptor) in [
        (
            "std.http.HttpHeader",
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([
                    ("name".to_string(), builtin_type_ref("string")),
                    ("value".to_string(), builtin_type_ref("string")),
                ]),
            },
        ),
        (
            "std.http.HttpQueryParam",
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([
                    ("name".to_string(), builtin_type_ref("string")),
                    ("value".to_string(), builtin_type_ref("string")),
                ]),
            },
        ),
        (
            "std.http.HttpRequest",
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([
                    ("method".to_string(), builtin_type_ref("string")),
                    ("url".to_string(), builtin_type_ref("string")),
                    ("path".to_string(), builtin_type_ref("string")),
                    (
                        "query".to_string(),
                        array_type_ref(std_http_type_ref("HttpQueryParam")),
                    ),
                    (
                        "headers".to_string(),
                        array_type_ref(std_http_type_ref("HttpHeader")),
                    ),
                    ("body".to_string(), builtin_type_ref("bytes")),
                ]),
            },
        ),
        (
            "std.http.HttpResponse",
            LinkedTypeDescriptor::Record {
                fields: BTreeMap::from([
                    ("status".to_string(), builtin_type_ref("integer")),
                    (
                        "headers".to_string(),
                        array_type_ref(std_http_type_ref("HttpHeader")),
                    ),
                    ("body".to_string(), builtin_type_ref("bytes")),
                ]),
            },
        ),
        (
            "std.http.HttpResponseStreamEvent",
            LinkedTypeDescriptor::Union {
                variants: vec![
                    LinkedTypeRef::Record {
                        fields: BTreeMap::from([
                            (
                                "tag".to_string(),
                                LinkedTypeRef::Literal {
                                    value: LiteralIr::String {
                                        value: "start".to_string(),
                                    },
                                },
                            ),
                            ("status".to_string(), builtin_type_ref("integer")),
                            (
                                "headers".to_string(),
                                array_type_ref(std_http_type_ref("HttpHeader")),
                            ),
                        ]),
                    },
                    LinkedTypeRef::Record {
                        fields: BTreeMap::from([
                            (
                                "tag".to_string(),
                                LinkedTypeRef::Literal {
                                    value: LiteralIr::String {
                                        value: "chunk".to_string(),
                                    },
                                },
                            ),
                            ("value".to_string(), builtin_type_ref("bytes")),
                        ]),
                    },
                    LinkedTypeRef::Record {
                        fields: BTreeMap::from([(
                            "tag".to_string(),
                            LinkedTypeRef::Literal {
                                value: LiteralIr::String {
                                    value: "end".to_string(),
                                },
                            },
                        )]),
                    },
                ],
            },
        ),
    ] {
        let addr = std_http_type_addr(name.rsplit('.').next().unwrap());
        program
            .types
            .descriptors
            .insert(addr.clone(), anonymous_type_decl(name, descriptor));
    }
}

fn link_service_symbol(
    program: &mut RuntimeProgram,
    module_path: &str,
    symbol: &str,
    executable_index: usize,
) {
    program.link_overlay.symbols.insert(
        format!("{module_path}.{symbol}"),
        ResolvedSymbol::Executable {
            addr: ExecutableAddr::service(0, executable_index),
        },
    );
}

fn package_unit(package_id: &str) -> PackageUnit {
    PackageUnit {
        schema_version: "skiff-package-unit-v1".to_string(),
        package_id: package_id.to_string(),
        version: "1.0.0".to_string(),
        build_identity: format!("{package_id}:build"),
        abi_identity: format!("{package_id}:abi"),
        publication_abi: Default::default(),
        files: Vec::new(),
        implementation_links: Default::default(),
        abi_identity_projection: Default::default(),
        dependencies: Vec::new(),
        recoverable_metadata: RecoverableArtifactMetadata::default(),
        config_and_effect_metadata: Default::default(),
    }
}

fn runtime_program_echo_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "input".to_string(),
            slot: 0,
            ty: builtin_type_ref("string"),
        }],
        return_type: Some(builtin_type_ref("string")),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "input".to_string(),
                kind: "param".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: runtime_program_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 2 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "literal", "value": { "kind": "string", "value": "!" } },
                {
                    "kind": "binary",
                    "op": "add",
                    "left": { "expression": 0 },
                    "right": { "expression": 1 }
                },
            ],
        })),
    }
}

fn runtime_program_http_request_path_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "pre".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "request".to_string(),
            slot: 0,
            ty: std_http_type_ref("HttpRequest"),
        }],
        return_type: Some(builtin_type_ref("string")),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "request".to_string(),
                kind: "param".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: runtime_program_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 1 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "field", "object": { "expression": 0 }, "field": "path" }
            ],
        })),
    }
}

fn runtime_program_http_echo_body_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "request".to_string(),
            slot: 0,
            ty: std_http_type_ref("HttpRequest"),
        }],
        return_type: Some(std_http_type_ref("HttpResponse")),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "request".to_string(),
                kind: "param".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: runtime_program_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 7 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "field", "object": { "expression": 0 }, "field": "body" },
                { "kind": "literal", "value": { "kind": "number", "value": 201 } },
                { "kind": "literal", "value": { "kind": "string", "value": "x-runtime" } },
                { "kind": "literal", "value": { "kind": "string", "value": "rust" } },
                {
                    "kind": "construct",
                    "typeRef": { "kind": "address", "addr": { "unit": { "kind": "package", "value": 0 }, "file": { "kind": "loadedFileIndex", "value": 0 }, "typeIndex": 0 } },
                    "fields": {
                        "name": { "expression": 3 },
                        "value": { "expression": 4 }
                    }
                },
                { "kind": "arrayLiteral", "items": [{ "expression": 5 }] },
                {
                    "kind": "construct",
                    "typeRef": { "kind": "address", "addr": { "unit": { "kind": "package", "value": 0 }, "file": { "kind": "loadedFileIndex", "value": 0 }, "typeIndex": 3 } },
                    "fields": {
                        "status": { "expression": 2 },
                        "headers": { "expression": 6 },
                        "body": { "expression": 1 }
                    }
                }
            ]
        })),
    }
}

fn runtime_program_http_context_response_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![
            ParamIr {
                name: "request".to_string(),
                slot: 0,
                ty: std_http_type_ref("HttpRequest"),
            },
            ParamIr {
                name: "context".to_string(),
                slot: 1,
                ty: builtin_type_ref("string"),
            },
        ],
        return_type: Some(std_http_type_ref("HttpResponse")),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![
                SlotIr {
                    index: 0,
                    name: "request".to_string(),
                    kind: "param".to_string(),
                },
                SlotIr {
                    index: 1,
                    name: "context".to_string(),
                    kind: "param".to_string(),
                },
            ],
            frame_size: 2,
        },
        may_suspend: false,
        body: runtime_program_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "return",
                    "value": { "expression": 7 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "field", "object": { "expression": 0 }, "field": "body" },
                { "kind": "literal", "value": { "kind": "number", "value": 203 } },
                { "kind": "literal", "value": { "kind": "string", "value": "x-context" } },
                { "kind": "loadSlot", "slot": 1 },
                {
                    "kind": "construct",
                    "typeRef": { "kind": "address", "addr": { "unit": { "kind": "package", "value": 0 }, "file": { "kind": "loadedFileIndex", "value": 0 }, "typeIndex": 0 } },
                    "fields": {
                        "name": { "expression": 3 },
                        "value": { "expression": 4 }
                    }
                },
                { "kind": "arrayLiteral", "items": [{ "expression": 5 }] },
                {
                    "kind": "construct",
                    "typeRef": { "kind": "address", "addr": { "unit": { "kind": "package", "value": 0 }, "file": { "kind": "loadedFileIndex", "value": 0 }, "typeIndex": 3 } },
                    "fields": {
                        "status": { "expression": 2 },
                        "headers": { "expression": 6 },
                        "body": { "expression": 1 }
                    }
                }
            ]
        })),
    }
}

fn runtime_program_http_stream_body_executable() -> LinkedExecutable {
    LinkedExecutable {
        kind: ExecutableKind::Function,
        symbol: "run".to_string(),
        type_params: Vec::new(),
        params: vec![ParamIr {
            name: "request".to_string(),
            slot: 0,
            ty: std_http_type_ref("HttpRequest"),
        }],
        return_type: Some(LinkedTypeRef::Native {
            name: "Stream".to_string(),
            args: vec![std_http_type_ref("HttpResponseStreamEvent")],
        }),
        self_type: None,
        slots: SlotLayoutIr {
            slots: vec![SlotIr {
                index: 0,
                name: "request".to_string(),
                kind: "param".to_string(),
            }],
            frame_size: 1,
        },
        may_suspend: false,
        body: runtime_program_body(json!({
            "blocks": [
                {
                    "label": "entry",
                    "statements": [
                        { "statement": 0 },
                        { "statement": 1 },
                        { "statement": 2 }
                    ]
                }
            ],
            "statements": [
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 8 }
                },
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 10 }
                },
                {
                    "kind": "emit",
                    "operation": "emit",
                    "value": { "expression": 12 }
                }
            ],
            "expressions": [
                { "kind": "loadSlot", "slot": 0 },
                { "kind": "field", "object": { "expression": 0 }, "field": "body" },
                { "kind": "literal", "value": { "kind": "string", "value": "start" } },
                { "kind": "literal", "value": { "kind": "number", "value": 202 } },
                { "kind": "literal", "value": { "kind": "string", "value": "content-type" } },
                { "kind": "literal", "value": { "kind": "string", "value": "text/plain" } },
                {
                    "entries": {
                        "name": { "expression": 4 },
                        "value": { "expression": 5 }
                    },
                    "kind": "mapLiteral"
                },
                { "kind": "arrayLiteral", "items": [{ "expression": 6 }] },
                {
                    "entries": {
                        "tag": { "expression": 2 },
                        "status": { "expression": 3 },
                        "headers": { "expression": 7 }
                    },
                    "kind": "mapLiteral"
                },
                { "kind": "literal", "value": { "kind": "string", "value": "chunk" } },
                {
                    "entries": {
                        "tag": { "expression": 9 },
                        "value": { "expression": 1 }
                    },
                    "kind": "mapLiteral"
                },
                { "kind": "literal", "value": { "kind": "string", "value": "end" } },
                {
                    "entries": {
                        "tag": { "expression": 11 }
                    },
                    "kind": "mapLiteral"
                }
            ]
        })),
    }
}

fn runtime_program_http_stream_context_executable() -> LinkedExecutable {
    let mut executable = runtime_program_http_stream_body_executable();
    executable.params.push(ParamIr {
        name: "context".to_string(),
        slot: 1,
        ty: builtin_type_ref("string"),
    });
    executable.slots.slots.push(SlotIr {
        index: 1,
        name: "context".to_string(),
        kind: "param".to_string(),
    });
    executable.slots.frame_size = 2;
    executable.body = runtime_program_body(json!({
        "blocks": [
            {
                "label": "entry",
                "statements": [
                    { "statement": 0 },
                    { "statement": 1 },
                    { "statement": 2 }
                ]
            }
        ],
        "statements": [
            {
                "kind": "emit",
                "operation": "emit",
                "value": { "expression": 8 }
            },
            {
                "kind": "emit",
                "operation": "emit",
                "value": { "expression": 10 }
            },
            {
                "kind": "emit",
                "operation": "emit",
                "value": { "expression": 12 }
            }
        ],
        "expressions": [
            { "kind": "loadSlot", "slot": 0 },
            { "kind": "field", "object": { "expression": 0 }, "field": "body" },
            { "kind": "literal", "value": { "kind": "string", "value": "start" } },
            { "kind": "literal", "value": { "kind": "number", "value": 202 } },
            { "kind": "literal", "value": { "kind": "string", "value": "x-context" } },
            { "kind": "loadSlot", "slot": 1 },
            {
                "entries": {
                    "name": { "expression": 4 },
                    "value": { "expression": 5 }
                },
                "kind": "mapLiteral"
            },
            { "kind": "arrayLiteral", "items": [{ "expression": 6 }] },
            {
                "entries": {
                    "tag": { "expression": 2 },
                    "status": { "expression": 3 },
                    "headers": { "expression": 7 }
                },
                "kind": "mapLiteral"
            },
            { "kind": "literal", "value": { "kind": "string", "value": "chunk" } },
            {
                "entries": {
                    "tag": { "expression": 9 },
                    "value": { "expression": 1 }
                },
                "kind": "mapLiteral"
            },
            { "kind": "literal", "value": { "kind": "string", "value": "end" } },
            {
                "entries": {
                    "tag": { "expression": 11 }
                },
                "kind": "mapLiteral"
            }
        ]
    }));
    executable
}

fn request(build_id: &str, target: &str) -> RequestEnvelope {
    let operation_abi_id = operation_abi_id_for_target(target);
    RequestEnvelope {
        request_id: "request-1".to_string(),
        mode: "unary".to_string(),
        target: target.to_string(),
        operation_abi_id: Some(operation_abi_id.clone()),
        selector: Some(format!("operation:{operation_abi_id}")),
        service_id: None,
        build_id: build_id.to_string(),
        service_protocol_identity: String::new(),
        contract_identity: None,
        activation_identity: None,
        http_adapter: None,
        websocket_adapter: None,
        binary_http: None,
        test_effects_enabled: false,
        test_effect_doubles: HashMap::new(),
        payload_bytes: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

fn build_request(build_id: &str, target: &str) -> RequestEnvelope {
    request(build_id, target)
}

fn request_without_operation_abi_id(build_id: &str, target: &str) -> RequestEnvelope {
    let mut request = request(build_id, target);
    request.operation_abi_id = None;
    request.selector = None;
    request
}

fn operation_abi_id_for_target(target: &str) -> String {
    format!("operation:test:{target}")
}

fn build_id_for_protocol(protocol_identity: &str) -> &'static str {
    match protocol_identity {
        PROTOCOL_B => BUILD_B,
        _ => BUILD_A,
    }
}

fn control_service_config(
    service_id: &str,
    build_id: &str,
    activation_identity: &str,
    resolved_config_identity: &str,
    dashscope_model: &str,
) -> RouterControlServiceConfig {
    RouterControlServiceConfig {
        service_id: service_id.to_string(),
        build_id: build_id.to_string(),
        activation_identity: activation_identity.to_string(),
        resolved_config_identity: resolved_config_identity.to_string(),
        resolved_config: json!({ "dashscopeModel": dashscope_model }),
        redacted_resolved_config: Value::Null,
        redaction_projection_identity: None,
        config_shape: Some(config_shape()),
        service_db: None,
        package_configs: Vec::new(),
        extra: serde_json::Map::new(),
    }
}

fn config_shape() -> ConfigShape {
    config_shape_from_json(json!({
        "schemaVersion": "skiff-config-shape-v1",
        "entries": []
    }))
}

fn config_shape_with_entries(entries: Value) -> ConfigShape {
    config_shape_from_json(json!({
        "schemaVersion": "skiff-config-shape-v1",
        "entries": entries
    }))
}

fn config_shape_from_json(value: Value) -> ConfigShape {
    serde_json::from_value(value).expect("test config shape should parse")
}

fn type_plan(name: &str) -> RuntimeTypePlan {
    RuntimeTypePlan::from_descriptor(&json!({ "kind": "builtin", "name": name, "args": [] }))
        .expect("config test type plan should build")
}

fn builtin_type_ref(name: &str) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn array_type_ref(item: LinkedTypeRef) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: "Array".to_string(),
        args: vec![item],
    }
}

fn std_http_type_ref(symbol: &str) -> LinkedTypeRef {
    LinkedTypeRef::Address {
        addr: std_http_type_addr(symbol),
    }
}

fn std_http_type_addr(symbol: &str) -> TypeAddr {
    let type_index = match symbol {
        "HttpHeader" => 0,
        "HttpQueryParam" => 1,
        "HttpRequest" => 2,
        "HttpResponse" => 3,
        "HttpResponseStreamEvent" => 4,
        other => panic!("unknown std HTTP test type {other}"),
    };
    TypeAddr {
        unit: UnitAddr::Package(0),
        file: crate::program::FileAddr::LoadedFileIndex(0),
        type_index,
    }
}

fn std_http_fallback_service_symbol_type_ref(symbol: &str) -> LinkedTypeRef {
    service_symbol_type_ref("", &format!("std.http.{symbol}"))
}

fn std_http_fallback_package_symbol_type_ref(symbol: &str) -> LinkedTypeRef {
    LinkedTypeRef::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "std".to_string(),
            },
            symbol_path: format!("http.{symbol}"),
            abi_expectation: None,
        },
    }
}

fn service_symbol_type_ref(module_path: &str, symbol: &str) -> LinkedTypeRef {
    LinkedTypeRef::ServiceSymbol {
        symbol: ServiceSymbolRef {
            module_path: module_path.to_string(),
            symbol: symbol.to_string(),
        },
    }
}

fn binary_http_request(request_id: &str, body: Vec<u8>) -> RequestEnvelope {
    binary_http_request_with_adapter(request_id, body, None)
}

fn binary_http_request_with_adapter(
    request_id: &str,
    body: Vec<u8>,
    adapter: Option<RuntimeHttpAdapterFrameHeader>,
) -> RequestEnvelope {
    request_envelope_from_start_frame(
        RequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            envelope_type: "request.start".to_string(),
            request_id: request_id.to_string(),
            mode: "unary".to_string(),
            caller: RuntimeCallerFrameHeader {
                kind: "gateway".to_string(),
                target: "gateway.http.raw".to_string(),
            },
            target: "program.target".to_string(),
            operation_abi_id: Some(operation_abi_id_for_target("program.target")),
            selector: Some(format!(
                "operation:{}",
                operation_abi_id_for_target("program.target")
            )),
            service_id: None,
            version: None,
            build_id: BUILD_A.to_string(),
            service_protocol_identity: PROTOCOL_A.to_string(),
            activation_identity: None,
            gateway_entry_identity: None,
            business_identity: None,
            websocket_entry_id: None,
            client_session: None,
            deadline: None,
            trace: RuntimeTraceContextFrameHeader {
                trace_id: format!("trace-{request_id}"),
                span_id: format!("span-{request_id}"),
                parent_span_id: None,
                sampled: None,
            },
            http_adapter: adapter,
            websocket_adapter: None,
            http_request: Some(RuntimeHttpRequestFrameHeader {
                method: "POST".to_string(),
                url: "https://example.test/echo?x=1".to_string(),
                path: "/echo".to_string(),
                query: vec![RuntimeHttpNameValueFrameHeader {
                    name: "x".to_string(),
                    value: "1".to_string(),
                }],
                headers: vec![RuntimeHttpNameValueFrameHeader {
                    name: "content-type".to_string(),
                    value: "application/octet-stream".to_string(),
                }],
            }),
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
        },
        body,
    )
    .expect("binary HTTP request should build")
}

fn typed_http_request(
    request_id: &str,
    body: Vec<u8>,
    adapter: RuntimeHttpAdapterFrameHeader,
) -> RequestEnvelope {
    binary_http_request_with_adapter(request_id, body, Some(adapter))
}

fn typed_json_adapter(
    handler: RuntimeHttpAdapterCallableFrameHeader,
    adapter_args: Vec<RuntimeHttpAdapterArgFrameHeader>,
    guard: Option<RuntimeHttpAdapterCallableFrameHeader>,
    pre: Option<RuntimeHttpAdapterCallableFrameHeader>,
) -> RuntimeHttpAdapterFrameHeader {
    RuntimeHttpAdapterFrameHeader {
        kind: RuntimeHttpAdapterKindFrameHeader::TypedJson,
        handler,
        guard,
        pre,
        adapter_args,
    }
}

fn raw_http_adapter(
    handler: RuntimeHttpAdapterCallableFrameHeader,
    adapter_args: Vec<RuntimeHttpAdapterArgFrameHeader>,
    guard: Option<RuntimeHttpAdapterCallableFrameHeader>,
    pre: Option<RuntimeHttpAdapterCallableFrameHeader>,
) -> RuntimeHttpAdapterFrameHeader {
    RuntimeHttpAdapterFrameHeader {
        kind: RuntimeHttpAdapterKindFrameHeader::RawHttp,
        handler,
        guard,
        pre,
        adapter_args,
    }
}

fn http_request_adapter_arg(param: &str) -> RuntimeHttpAdapterArgFrameHeader {
    http_adapter_arg(param, RuntimeHttpAdapterSourceFrameHeader::HttpRequest)
}

fn http_body_adapter_arg(param: &str) -> RuntimeHttpAdapterArgFrameHeader {
    http_adapter_arg(param, RuntimeHttpAdapterSourceFrameHeader::HttpBody)
}

fn http_context_adapter_arg(param: &str) -> RuntimeHttpAdapterArgFrameHeader {
    http_adapter_arg(param, RuntimeHttpAdapterSourceFrameHeader::HttpContext)
}

fn http_adapter_arg(
    param: &str,
    source: RuntimeHttpAdapterSourceFrameHeader,
) -> RuntimeHttpAdapterArgFrameHeader {
    RuntimeHttpAdapterArgFrameHeader {
        param: param.to_string(),
        source,
    }
}

fn service_http_adapter_callable(symbol: &str) -> RuntimeHttpAdapterCallableFrameHeader {
    RuntimeHttpAdapterCallableFrameHeader::ServiceFunction {
        module_path: "program.main".to_string(),
        symbol: symbol.to_string(),
    }
}

fn binary_http_stream_request(request_id: &str, body: Vec<u8>) -> RequestEnvelope {
    let mut request = binary_http_request(request_id, body);
    request.mode = "serverStream".to_string();
    request
}

fn test_actor_client_invocation_with_router_sender(
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
) -> ActorClientTestInvocation {
    ActorClientTestInvocation {
        request: RequestEnvelope {
            request_id: "request-websocket-send".to_string(),
            mode: "unary".to_string(),
            target: "program.target".to_string(),
            operation_abi_id: Some(operation_abi_id_for_target("program.target")),
            selector: Some(format!(
                "operation:{}",
                operation_abi_id_for_target("program.target")
            )),
            service_id: None,
            build_id: BUILD_A.to_string(),
            service_protocol_identity: PROTOCOL_A.to_string(),
            contract_identity: None,
            activation_identity: None,
            http_adapter: None,
            websocket_adapter: None,
            binary_http: None,
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            payload_bytes: Vec::new(),
            extra: serde_json::Map::new(),
        },
        operation: RuntimeOperation {
            operation_abi_id: Some(operation_abi_id_for_target("program.target")),
            operation: "run".to_string(),
            target: "program.target".to_string(),
            mode: "unary".to_string(),
            parameters: Vec::new(),
            service_protocol_identity: Some(PROTOCOL_A.to_string()),
            extra: serde_json::Map::new(),
        },
        runtime_id: "runtime-test".to_string(),
        service_id: "service-program".to_string(),
        cancelled: Arc::new(AtomicBool::new(false)),
        router_sender: Some(sender),
        outbound_requests: Arc::new(super::OutboundRequestRegistry::default()),
    }
}

struct ActorClientTestInvocation {
    request: RequestEnvelope,
    operation: RuntimeOperation,
    runtime_id: String,
    service_id: String,
    cancelled: Arc<AtomicBool>,
    router_sender: Option<mpsc::UnboundedSender<RouterWriterMessage>>,
    outbound_requests: Arc<super::OutboundRequestRegistry>,
}

impl ActorClientTestInvocation {
    fn websocket_context(&self) -> crate::capability_context::WebsocketCapabilityContext<'_> {
        crate::capability_context::WebsocketCapabilityContext::with_entry_id(
            &self.service_id,
            Some("gateway.websocket.chat"),
            self.router_sender.as_ref(),
        )
    }
}

fn actor_client_context(frame: &ActorClientTestInvocation) -> ActorClientContext<'_> {
    let invocation = invocation_context_from_request(
        &frame.runtime_id,
        &frame.service_id,
        "0.0.0-test",
        &frame.request,
        &frame.operation,
    );
    ActorClientContext::new(
        invocation,
        frame.router_sender.as_ref(),
        frame.outbound_requests.as_ref(),
        frame.cancelled.as_ref(),
    )
}

fn runtime_program_body(value: Value) -> LinkedExecutableBody {
    serde_json::from_value(value).expect("typed executable body should deserialize")
}
