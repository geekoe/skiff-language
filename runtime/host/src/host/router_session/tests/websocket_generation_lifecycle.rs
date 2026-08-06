use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::{
    protocol::{encode_binary_frame, RUNTIME_FRAME_SCHEMA_VERSION},
    runtime_assembly_request::{
        decode_runtime_assembly_websocket_connect_response_end_frame,
        decode_runtime_assembly_websocket_jsonrpc_response_end_frame,
        RuntimeAssemblyRequestCallerFrameHeader, RuntimeAssemblyRequestTraceFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressFrameHeader,
        RuntimeAssemblyWebSocketConnectIngressProtocol,
        RuntimeAssemblyWebSocketConnectRequestFrameHeader,
        RuntimeAssemblyWebSocketConnectRequestStartFrameHeader,
        RuntimeAssemblyWebSocketConnectResponseFrameHeader,
        RuntimeAssemblyWebSocketConnectRoutingFrameHeader,
        RuntimeAssemblyWebSocketJsonRpcIngressFrameHeader, RuntimeAssemblyWebSocketJsonRpcProfile,
        RuntimeAssemblyWebSocketJsonRpcRequestFrameHeader,
        RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader,
        RuntimeAssemblyWebSocketJsonRpcResponseOutcome,
        RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader,
    },
    websocket_generation_lifecycle::{
        decode_websocket_generation_lifecycle_frame, encode_websocket_generation_lifecycle_frame,
        WebSocketGenerationLifecycleControl, WebSocketGenerationLifecycleDirection,
        WebSocketGenerationLifecycleOperation, WebSocketGenerationLifecycleRejectionCode,
        WebSocketGenerationLifecycleSender, WebSocketGenerationLifecycleTuple,
        WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE,
    },
};
use std::sync::Arc;

use tokio::sync::mpsc;

use super::runtime_assembly_request::fixture;

const ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:test-session";
const OTHER_ROUTER_SESSION: &str = "skiff-router-session-v1:opaque:other-session";
const CONNECTION_ID: &str = "connection-a";
const WEBSOCKET_ENTRY_ID: &str =
    "skiff-websocket-entry-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

#[tokio::test]
async fn websocket_generation_old_route_survives_reload_until_disconnect_without_artifact_io() {
    let (host, generation_a, generation_b) = fixture::reloaded_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    assert_eq!(generation_a.generation(), 1);
    // M2 loaded set is append-only per buildId: re-admitting the same
    // deployment resolves to the same loaded image.
    assert_eq!(generation_b.generation(), 1);

    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.queue_websocket_generation_acquire_for_test(
        &generation_a,
        ROUTER_SESSION,
        WEBSOCKET_ENTRY_ID,
        CONNECTION_ID,
        &sender,
    )
    .expect("generation A connect should queue an acquire");
    let acquire = receive_lifecycle(&mut receiver).await;
    assert!(matches!(
        &acquire,
        WebSocketGenerationLifecycleControl::Acquire { .. }
    ));
    let ack_frame = encode_websocket_generation_lifecycle_frame(
        WebSocketGenerationLifecycleDirection::RouterToRuntime,
        &acquire_ack(&acquire),
    )
    .expect("exact acquire ack should encode");
    let mut control = None;
    let mut fingerprint = None;
    super::super::dispatch_router_binary_frame(
        &host,
        &ack_frame,
        &sender,
        &mut control,
        &mut fingerprint,
    )
    .await
    .expect("binary acquire ack should correlate");

    let duplicate_acquire = host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            generation_a.clone(),
            WEBSOCKET_ENTRY_ID.to_string(),
            CONNECTION_ID.to_string(),
        )
        .expect("the same connection tuple should acquire idempotently");
    host.websocket_generations
        .handle_acquire_response(&acquire_ack(&duplicate_acquire))
        .expect("duplicate exact acquire ack should correlate");
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 1);

    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .expect("session disconnect should release all pins");
    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .expect("duplicate session disconnect should be idempotent");
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
    assert!(host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            generation_b,
            WEBSOCKET_ENTRY_ID.to_string(),
            "late-connection".to_string(),
        )
        .is_err());
}

#[tokio::test]
async fn websocket_generation_release_is_exact_idempotent_and_fail_closed() {
    let (host, generation_a, _) = fixture::reloaded_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let retired_context = Arc::downgrade(generation_a.context_set());
    let acquire = host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            generation_a,
            WEBSOCKET_ENTRY_ID.to_string(),
            CONNECTION_ID.to_string(),
        )
        .expect("connect should pin");
    host.websocket_generations
        .handle_acquire_response(&acquire_ack(&acquire))
        .expect("exact acquire ack should correlate");
    let tuple = lifecycle_tuple(&acquire).clone();

    let mut mismatched_tuple = tuple.clone();
    mismatched_tuple.websocket_entry_id =
        format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64));
    assert!(matches!(
        host.websocket_generations
            .handle_release(
                ROUTER_SESSION,
                release_control("release-wrong-tuple", mismatched_tuple),
            )
            .expect("wrong tuple should receive a typed rejection"),
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::TupleMismatch,
            ..
        }
    ));
    assert!(matches!(
        host.websocket_generations
            .handle_release(
                OTHER_ROUTER_SESSION,
                release_control("release-wrong-session", tuple.clone()),
            )
            .expect("wrong session should receive a typed rejection"),
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::SenderMismatch,
            ..
        }
    ));
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 1);

    let release = release_control("release-1", tuple.clone());
    let first = host
        .websocket_generations
        .handle_release(ROUTER_SESSION, release.clone())
        .expect("exact release should apply");
    assert!(matches!(
        first,
        WebSocketGenerationLifecycleControl::Ack {
            operation: WebSocketGenerationLifecycleOperation::Release,
            ..
        }
    ));
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
    // M2 loaded set is append-only: the deployment image stays strongly held
    // by the loaded registry, so release only reclaims the generation pin.
    assert!(
        retired_context.upgrade().is_some(),
        "loaded registry must keep the deployment image alive after release"
    );

    assert_eq!(
        host.websocket_generations
            .handle_release(ROUTER_SESSION, release)
            .expect("duplicate exact release should be idempotent"),
        first
    );
    assert!(matches!(
        host.websocket_generations
            .handle_release(
                OTHER_ROUTER_SESSION,
                release_control("release-1", lifecycle_tuple(&acquire).clone()),
            )
            .expect("cross-session replay should receive a typed rejection"),
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::SenderMismatch,
            ..
        }
    ));

    let mut conflicting_tuple = tuple;
    conflicting_tuple.websocket_entry_id =
        format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64));
    let conflict = release_control("release-1", conflicting_tuple);
    assert!(matches!(
        host.websocket_generations
            .handle_release(ROUTER_SESSION, conflict)
            .expect("request-id conflict should receive a typed rejection"),
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::RequestConflict,
            ..
        }
    ));

    let mut valid_unacquired_tuple = lifecycle_tuple(&acquire).clone();
    valid_unacquired_tuple.service_id = "example.com/service".to_string();
    valid_unacquired_tuple.connection_id = "unacquired-connection".to_string();
    let unacquired = release_control("release-unacquired", valid_unacquired_tuple);
    let unacquired_frame = encode_websocket_generation_lifecycle_frame(
        WebSocketGenerationLifecycleDirection::RouterToRuntime,
        &unacquired,
    )
    .expect("valid unacquired release should encode");
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut fingerprint = None;
    super::super::dispatch_router_binary_frame(
        &host,
        &unacquired_frame,
        &sender,
        &mut control,
        &mut fingerprint,
    )
    .await
    .expect("unacquired release should receive a typed rejection");
    assert!(matches!(
        receive_lifecycle(&mut receiver).await,
        WebSocketGenerationLifecycleControl::Reject {
            operation: WebSocketGenerationLifecycleOperation::Release,
            code: WebSocketGenerationLifecycleRejectionCode::NotAcquired,
            ..
        }
    ));
}

#[tokio::test]
async fn websocket_generation_acquire_rejection_rolls_back_the_route_pin() {
    let (host, generation_a, _) = fixture::reloaded_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let acquire = host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            generation_a,
            WEBSOCKET_ENTRY_ID.to_string(),
            CONNECTION_ID.to_string(),
        )
        .expect("connect should tentatively pin");
    let reject = acquire_rejection(&acquire);
    host.websocket_generations
        .handle_acquire_response(&reject)
        .expect("typed acquire rejection should be isolated to the connection");
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
}

mod websocket_jsonrpc_target {
    use super::*;

    #[tokio::test]
    async fn handlerless_method_websocket_eager_pins_before_accept_without_user_connect() {
        let (host, physical, _) = fixture::admitted_websocket_gateway_host().await;
        host.websocket_generations.connect(ROUTER_SESSION).unwrap();
        assert!(physical.entry().optional_handler().is_none());
        assert!(physical.has_websocket_jsonrpc_methods().unwrap());

        let header = handlerless_connect_header(&physical, "handlerless-method-connect");
        let frame = encode_binary_frame(&header, &[]).unwrap();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut control = None;
        let mut fingerprint = None;
        super::super::super::dispatch_router_binary_frame(
            &host,
            &frame,
            &sender,
            &mut control,
            &mut fingerprint,
        )
        .await
        .expect("handlerless method-bearing connect should enter Host admission");

        let acquire = receive_lifecycle(&mut receiver).await;
        assert!(
            receiver.try_recv().is_err(),
            "synthetic accept must wait for the exact acquire receipt"
        );
        host.websocket_generations
            .handle_acquire_response(&acquire_ack(&acquire))
            .expect("exact acquire receipt");

        let RouterWriterMessage::Binary(response) = receiver.recv().await.unwrap() else {
            panic!("synthetic accept must use the binary response wire")
        };
        let response = decode_runtime_assembly_websocket_connect_response_end_frame(&response)
            .expect("synthetic connect response");
        assert_eq!(response.request_id, header.request_id);
        assert!(matches!(
            response.websocket_connect,
            RuntimeAssemblyWebSocketConnectResponseFrameHeader::Accept {
                business_identity: None,
                connection_policy: None,
                admission_rank: None,
            }
        ));
        assert_eq!(host.websocket_generations.pin_count().unwrap(), 1);

        let release = release_control("handlerless-close", lifecycle_tuple(&acquire).clone());
        assert!(matches!(
            host.websocket_generations
                .handle_release(ROUTER_SESSION, release)
                .unwrap(),
            WebSocketGenerationLifecycleControl::Ack {
                operation: WebSocketGenerationLifecycleOperation::Release,
                ..
            }
        ));
        assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
    }

    #[tokio::test]
    async fn websocket_jsonrpc_host_dispatches_pinned_method_instead_of_unsupported() {
        let fixture::ReloadedWebSocketGatewayHost {
            host,
            physical_a,
            method_a,
            physical_b,
            method_b,
            ..
        } = fixture::reloaded_websocket_gateway_host().await;
        host.websocket_generations.connect(ROUTER_SESSION).unwrap();
        let websocket_entry_id = websocket_entry_id(&physical_a);
        acquire_generation(
            &host,
            &physical_a,
            &websocket_entry_id,
            "host-dispatch-old-connection",
        );

        assert_eq!(physical_b.generation(), 0);
        assert!(!Arc::ptr_eq(
            method_a.execution_image(),
            method_b.execution_image()
        ));

        let selector = method_a.selector();
        let header = RuntimeAssemblyWebSocketJsonRpcRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id: "host-websocket-jsonrpc-pinned-dispatch".to_string(),
            mode: "unary".to_string(),
            caller: RuntimeAssemblyRequestCallerFrameHeader {
                kind: "gateway".to_string(),
            },
            routing: RuntimeAssemblyWebSocketJsonRpcRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: None,
                assembly_generation: None,
                deployment: method_a.deployment().clone(),
                build_id: Some(
                    physical_a
                        .deployment()
                        .deployment_artifact_identity
                        .as_str()
                        .to_string(),
                ),
                gateway_entry_identity: method_a.gateway_entry_identity().clone(),
                ingress: RuntimeAssemblyWebSocketJsonRpcIngressFrameHeader {
                    protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                    method: selector.method.clone().expect("JSON-RPC method"),
                    path: selector.path.clone(),
                },
            },
            client_session: None,
            deadline: None,
            trace: RuntimeAssemblyRequestTraceFrameHeader {
                trace_id: "trace-host-websocket-jsonrpc-pinned-dispatch".to_string(),
                span_id: "span-host-websocket-jsonrpc".to_string(),
                parent_span_id: None,
                sampled: None,
            },
            websocket_json_rpc: RuntimeAssemblyWebSocketJsonRpcRequestFrameHeader {
                profile: RuntimeAssemblyWebSocketJsonRpcProfile::JsonRpc2_0Text,
                connection_id: "host-dispatch-old-connection".to_string(),
                websocket_entry_id,
                gateway_entry_identity: method_a.gateway_entry_identity().clone(),
                business_identity: Some("trusted-business".to_string()),
            },
            test_effects_enabled: false,
        };
        let frame = encode_binary_frame(&header, br#"{"value":"old"}"#).unwrap();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let mut control = None;
        let mut fingerprint = None;
        super::super::super::dispatch_router_binary_frame(
            &host,
            &frame,
            &sender,
            &mut control,
            &mut fingerprint,
        )
        .await
        .expect("strict JSON-RPC request should enter Host dispatch");

        let RouterWriterMessage::Binary(response) =
            tokio::time::timeout(std::time::Duration::from_secs(10), receiver.recv())
                .await
                .expect("Host dispatch response timeout")
                .expect("Host response channel")
        else {
            panic!("Host JSON-RPC response must use the binary wire")
        };
        let (response, payload) =
            decode_runtime_assembly_websocket_jsonrpc_response_end_frame(&response)
                .expect("Host must emit typed websocketJsonRpc response.end");
        assert_eq!(
            response.websocket_json_rpc.outcome,
            RuntimeAssemblyWebSocketJsonRpcResponseOutcome::Success
        );
        assert_eq!(payload, br#""old""#);
        assert!(receiver.try_recv().is_err());
    }
}

#[derive(Clone, Copy)]
struct JsonRpcExecutionRouteLookup<'a> {
    router_session_id: &'a str,
    connection_id: &'a str,
    build_id: &'a str,
    websocket_entry_id: &'a skiff_artifact_model::WebSocketEntryId,
    path: &'a str,
    method: &'a str,
    gateway_entry_identity: &'a skiff_artifact_model::GatewayEntryIdentity,
    profile: skiff_artifact_model::GatewayWebSocketRpcProfile,
}

impl JsonRpcExecutionRouteLookup<'_> {
    fn resolve(
        self,
        host: &crate::host::RuntimeHost,
    ) -> crate::error::Result<crate::host::websocket_generation::ResolvedWebSocketJsonRpcExecution>
    {
        host.websocket_generations
            .websocket_jsonrpc_execution_route(
                self.router_session_id,
                self.connection_id,
                Some(self.build_id),
                self.websocket_entry_id,
                self.path,
                self.method,
                self.gateway_entry_identity,
                self.profile,
            )
    }
}

fn execution_route_lookup<'a>(
    connection_id: &'a str,
    physical: &'a crate::loader::assembly_admission::ActiveAssemblyRoute,
    method: &'a crate::loader::assembly_admission::ActiveAssemblyRoute,
    websocket_entry_id: &'a skiff_artifact_model::WebSocketEntryId,
) -> JsonRpcExecutionRouteLookup<'a> {
    JsonRpcExecutionRouteLookup {
        router_session_id: ROUTER_SESSION,
        connection_id,
        build_id: physical.deployment().deployment_artifact_identity.as_str(),
        websocket_entry_id,
        path: &method.selector().path,
        method: method.selector().method.as_deref().unwrap(),
        gateway_entry_identity: method.gateway_entry_identity(),
        profile: skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text,
    }
}

fn websocket_entry_id(
    route: &crate::loader::assembly_admission::ActiveAssemblyRoute,
) -> skiff_artifact_model::WebSocketEntryId {
    skiff_artifact_identity::websocket_entry_id(
        &route.entry().owner().service_id,
        route.gateway_entry_key(),
    )
    .unwrap()
}

fn acquire_generation(
    host: &crate::host::RuntimeHost,
    route: &crate::loader::assembly_admission::ActiveAssemblyRoute,
    websocket_entry_id: &skiff_artifact_model::WebSocketEntryId,
    connection_id: &str,
) -> WebSocketGenerationLifecycleControl {
    let acquire = host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            route.clone(),
            websocket_entry_id.as_str().to_string(),
            connection_id.to_string(),
        )
        .unwrap();
    host.websocket_generations
        .handle_acquire_response(&acquire_ack(&acquire))
        .unwrap();
    acquire
}

fn db_source_marker(route: &crate::loader::assembly_admission::ActiveAssemblyRoute) -> String {
    let result = route
        .db_source()
        .expect("route DB source")
        .context_for_request("route-owner", "route-request")
        .require_store("route-target", "route source must be configured");
    match result {
        Err(skiff_runtime_capability_context::DbCapabilityError::Decode(marker)) => marker,
        Err(error) => panic!("unexpected route DB marker error: {error}"),
        Ok(_) => panic!("marker DB source must not create a real store"),
    }
}

fn assert_resolved_execution_owners(
    resolved: &crate::host::websocket_generation::ResolvedWebSocketJsonRpcExecution,
    expected: &crate::loader::assembly_admission::ActiveAssemblyRoute,
) {
    assert!(Arc::ptr_eq(
        resolved.method_route.context_set(),
        expected.context_set()
    ));
    assert!(Arc::ptr_eq(
        resolved.method_route.activation(),
        expected.activation()
    ));
    assert!(Arc::ptr_eq(
        resolved.method_route.execution_image(),
        expected.execution_image()
    ));
    assert!(Arc::ptr_eq(resolved.method_route.entry(), expected.entry()));
    assert!(Arc::ptr_eq(
        resolved.method_route.activation(),
        resolved.target.eval().activation_context()
    ));
    assert!(Arc::ptr_eq(
        resolved.method_route.execution_image(),
        resolved.target.eval().execution_image()
    ));
    assert_eq!(resolved.method_route.selector(), expected.selector());
    assert_eq!(
        resolved.method_route.gateway_entry_key(),
        resolved.target.gateway_entry_key()
    );
    assert_eq!(
        resolved.method_route.gateway_entry_identity(),
        resolved.target.gateway_entry_identity()
    );
    assert_eq!(
        resolved.method_route.entry().owner(),
        resolved.target.owner()
    );
    assert_eq!(
        resolved
            .method_route
            .activation()
            .implementation_package_build_id(),
        resolved.target.implementation_package_build_id()
    );
    assert_eq!(
        resolved.target.profile(),
        skiff_artifact_model::GatewayWebSocketRpcProfile::JsonRpc2_0Text
    );
}

fn assert_resolved_physical_route(
    resolved: &crate::host::websocket_generation::ResolvedWebSocketJsonRpcExecution,
    physical: &crate::loader::assembly_admission::ActiveAssemblyRoute,
    websocket_entry_id: &skiff_artifact_model::WebSocketEntryId,
) {
    assert!(Arc::ptr_eq(
        resolved.method_route.context_set(),
        physical.context_set()
    ));
    assert!(Arc::ptr_eq(
        resolved.method_route.activation(),
        physical.activation()
    ));
    assert!(Arc::ptr_eq(
        resolved.method_route.execution_image(),
        physical.execution_image()
    ));
    assert_eq!(
        resolved.target.physical_route().selector(),
        physical.selector()
    );
    assert_eq!(
        resolved.target.physical_route().gateway_entry_key(),
        physical.gateway_entry_key()
    );
    assert_eq!(
        resolved.target.physical_route().gateway_entry_identity(),
        physical.gateway_entry_identity()
    );
    assert_eq!(resolved.target.websocket_entry_id(), websocket_entry_id);
}

fn assert_websocket_jsonrpc_targets_equivalent(
    actual: &skiff_runtime_request::RuntimeAssemblyWebSocketJsonRpcTarget,
    expected: &skiff_runtime_request::RuntimeAssemblyWebSocketJsonRpcTarget,
) {
    assert!(Arc::ptr_eq(
        actual.eval().activation_context(),
        expected.eval().activation_context()
    ));
    assert!(Arc::ptr_eq(
        actual.eval().execution_image(),
        expected.eval().execution_image()
    ));
    assert_eq!(actual.assembly_identity(), expected.assembly_identity());
    assert_eq!(actual.assembly_generation(), expected.assembly_generation());
    assert_eq!(actual.owner(), expected.owner());
    assert_eq!(
        actual.implementation_package_build_id(),
        expected.implementation_package_build_id()
    );
    assert_eq!(actual.selector(), expected.selector());
    assert_eq!(actual.gateway_entry_key(), expected.gateway_entry_key());
    assert_eq!(
        actual.gateway_entry_identity(),
        expected.gateway_entry_identity()
    );
    assert_eq!(
        actual.physical_route().selector(),
        expected.physical_route().selector()
    );
    assert_eq!(
        actual.physical_route().gateway_entry_key(),
        expected.physical_route().gateway_entry_key()
    );
    assert_eq!(
        actual.physical_route().gateway_entry_identity(),
        expected.physical_route().gateway_entry_identity()
    );
    assert_eq!(actual.websocket_entry_id(), expected.websocket_entry_id());
    assert_eq!(actual.profile(), expected.profile());
    assert_eq!(actual.protocol_surface(), expected.protocol_surface());
    assert_eq!(actual.adapter_plan(), expected.adapter_plan());
    assert_eq!(actual.handler_callable_id(), expected.handler_callable_id());
    assert_eq!(actual.handler_signature(), expected.handler_signature());
    assert_eq!(actual.handler_addr(), expected.handler_addr());
}

#[tokio::test]
async fn websocket_jsonrpc_target_matches_websocket_jsonrpc_execution_route_for_old_context() {
    let fixture::ReloadedWebSocketGatewayHost {
        host,
        physical_a,
        method_a,
        physical_b,
        method_b,
        ..
    } = fixture::reloaded_websocket_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let websocket_entry_id_a = websocket_entry_id(&physical_a);
    let websocket_entry_id_b = websocket_entry_id(&physical_b);
    acquire_generation(
        &host,
        &physical_a,
        &websocket_entry_id_a,
        "old-generation-connection",
    );
    acquire_generation(
        &host,
        &physical_b,
        &websocket_entry_id_b,
        "current-generation-connection",
    );

    assert_ne!(db_source_marker(&method_a), db_source_marker(&method_b));
    assert_ne!(
        method_a.service_protocol_identity(),
        method_b.service_protocol_identity()
    );
    assert!(!Arc::ptr_eq(method_a.activation(), method_b.activation()));
    assert!(!Arc::ptr_eq(
        method_a.execution_image(),
        method_b.execution_image()
    ));
    assert_ne!(
        method_a.activation().implementation_package_build_id(),
        method_b.activation().implementation_package_build_id()
    );
    assert_ne!(
        method_a.entry().owner(),
        method_b.entry().owner(),
        "replacement fixture must distinguish deployment ownership"
    );

    let lookup_a = execution_route_lookup(
        "old-generation-connection",
        &physical_a,
        &method_a,
        &websocket_entry_id_a,
    );
    let lookup_b = execution_route_lookup(
        "current-generation-connection",
        &physical_b,
        &method_b,
        &websocket_entry_id_b,
    );
    let resolved_a = lookup_a
        .resolve(&host)
        .expect("old connection resolves only from its pinned generation A");
    let resolved_b = lookup_b
        .resolve(&host)
        .expect("new connection resolves from generation B");

    assert_resolved_execution_owners(&resolved_a, &method_a);
    assert_resolved_execution_owners(&resolved_b, &method_b);
    assert_resolved_physical_route(&resolved_a, &physical_a, &websocket_entry_id_a);
    assert_resolved_physical_route(&resolved_b, &physical_b, &websocket_entry_id_b);
    assert_eq!(
        db_source_marker(&resolved_a.method_route),
        db_source_marker(&method_a)
    );
    assert_ne!(
        db_source_marker(&resolved_a.method_route),
        db_source_marker(&method_b)
    );
    assert_eq!(
        resolved_a.method_route.service_protocol_identity(),
        method_a.service_protocol_identity()
    );
    assert_ne!(
        resolved_a.method_route.service_protocol_identity(),
        method_b.service_protocol_identity()
    );
    assert_eq!(resolved_a.target.assembly_generation(), 0);
    assert_eq!(resolved_b.target.assembly_generation(), 0);
    assert_ne!(
        resolved_a.method_route.deployment(),
        resolved_b.method_route.deployment(),
        "distinct deployment build ids must keep distinct pinned routes"
    );

    let target_only_a = host
        .websocket_generations
        .websocket_jsonrpc_target(
            lookup_a.router_session_id,
            lookup_a.connection_id,
            Some(lookup_a.build_id),
            lookup_a.websocket_entry_id,
            lookup_a.path,
            lookup_a.method,
            lookup_a.gateway_entry_identity,
            lookup_a.profile,
        )
        .expect("target-only API delegates the same pinned join");
    assert_websocket_jsonrpc_targets_equivalent(&target_only_a, &resolved_a.target);

    let wrong_websocket_entry_id = skiff_artifact_model::WebSocketEntryId::parse(format!(
        "skiff-websocket-entry-v1:sha256:{}",
        "e".repeat(64)
    ))
    .unwrap();
    for wrong in [
        JsonRpcExecutionRouteLookup {
            router_session_id: OTHER_ROUTER_SESSION,
            ..lookup_a
        },
        JsonRpcExecutionRouteLookup {
            connection_id: "missing-connection",
            ..lookup_a
        },
        JsonRpcExecutionRouteLookup {
            build_id: physical_b.deployment().deployment_artifact_identity.as_str(),
            ..lookup_a
        },
        JsonRpcExecutionRouteLookup {
            websocket_entry_id: &wrong_websocket_entry_id,
            ..lookup_a
        },
        JsonRpcExecutionRouteLookup {
            path: "/wrong",
            ..lookup_a
        },
        JsonRpcExecutionRouteLookup {
            method: "status.missing",
            ..lookup_a
        },
        JsonRpcExecutionRouteLookup {
            gateway_entry_identity: physical_a.gateway_entry_identity(),
            ..lookup_a
        },
    ] {
        assert!(
            wrong.resolve(&host).is_err(),
            "mismatched pinned route tuple must fail closed"
        );
    }
    assert!(
        serde_json::from_str::<skiff_artifact_model::GatewayWebSocketRpcProfile>(
            "\"wrong-profile\""
        )
        .is_err(),
        "the typed profile has no representable non-canonical resolver input"
    );

    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .unwrap();
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
}

#[tokio::test]
async fn websocket_jsonrpc_execution_route_rejects_tentative_and_released_pin_and_reclaims_old() {
    let fixture::ReloadedWebSocketGatewayHost {
        host,
        physical_a,
        method_a,
        physical_b,
        method_b,
        ..
    } = fixture::reloaded_websocket_gateway_host().await;
    drop((physical_b, method_b));
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let websocket_entry_id_a = websocket_entry_id(&physical_a);
    let retired_context = Arc::downgrade(physical_a.context_set());
    let acquire = host
        .websocket_generations
        .begin_acquire(
            ROUTER_SESSION,
            physical_a.clone(),
            websocket_entry_id_a.as_str().to_string(),
            "release-connection".to_string(),
        )
        .unwrap();
    let lookup = execution_route_lookup(
        "release-connection",
        &physical_a,
        &method_a,
        &websocket_entry_id_a,
    );
    assert!(
        lookup.resolve(&host).is_err(),
        "tentative pin without exact acquire receipt must expose no route"
    );
    host.websocket_generations
        .handle_acquire_response(&acquire_ack(&acquire))
        .unwrap();
    lookup
        .resolve(&host)
        .expect("exact receipt exposes the pinned route");
    let release = release_control("route-release", lifecycle_tuple(&acquire).clone());
    assert!(matches!(
        host.websocket_generations
            .handle_release(ROUTER_SESSION, release)
            .unwrap(),
        WebSocketGenerationLifecycleControl::Ack {
            operation: WebSocketGenerationLifecycleOperation::Release,
            ..
        }
    ));
    assert!(
        lookup.resolve(&host).is_err(),
        "released pin must expose no stale route"
    );
    drop(acquire);
    drop(method_a);
    drop(physical_a);
    // M2 loaded set is append-only: release reclaims the generation pin but
    // the deployment image stays strongly held by the loaded registry.
    assert!(
        retired_context.upgrade().is_some(),
        "loaded registry must keep the deployment image alive after release"
    );
}

#[tokio::test]
async fn websocket_jsonrpc_execution_route_rejects_disconnected_pin_and_reclaims_old() {
    let fixture::ReloadedWebSocketGatewayHost {
        host,
        physical_a,
        method_a,
        physical_b,
        method_b,
        ..
    } = fixture::reloaded_websocket_gateway_host().await;
    drop((physical_b, method_b));
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let websocket_entry_id_a = websocket_entry_id(&physical_a);
    let retired_context = Arc::downgrade(physical_a.context_set());
    acquire_generation(
        &host,
        &physical_a,
        &websocket_entry_id_a,
        "disconnect-connection",
    );
    let lookup = execution_route_lookup(
        "disconnect-connection",
        &physical_a,
        &method_a,
        &websocket_entry_id_a,
    );
    lookup
        .resolve(&host)
        .expect("acquired pin resolves before disconnect");
    host.websocket_generations
        .disconnect(ROUTER_SESSION)
        .unwrap();
    assert!(
        lookup.resolve(&host).is_err(),
        "disconnected pin must expose no stale route"
    );
    drop(method_a);
    drop(physical_a);
    // M2 loaded set is append-only: disconnect reclaims the generation pin
    // but the deployment image stays strongly held by the loaded registry.
    assert!(
        retired_context.upgrade().is_some(),
        "loaded registry must keep the deployment image alive after disconnect"
    );
}

#[test]
fn websocket_jsonrpc_execution_route_source_uses_only_the_pinned_route_join() {
    let source = include_str!("../../websocket_generation.rs");
    let start = source
        .find("fn acquired_physical_route(")
        .expect("pinned physical resolver source");
    let end = source[start..]
        .find("pub(super) fn handle_release(")
        .map(|offset| start + offset)
        .expect("resolver source terminator");
    let resolver = &source[start..end];
    let acquired = resolver
        .find("acquired_physical_route(")
        .expect("acquired pin join");
    let method = resolver
        .find(".websocket_jsonrpc_method_route(")
        .expect("old sibling method join");
    let target = resolver
        .find(".websocket_jsonrpc_target(&physical_route)")
        .expect("target projection from the same old route");
    assert!(acquired < method && method < target);
    for forbidden in [
        "lookup_active_assembly",
        "assembly_admission",
        "resolve_runtime_assembly",
        "artifact_store",
        "FilesystemRuntimeAssembly",
    ] {
        assert!(
            !resolver.contains(forbidden),
            "generation resolver must not use current assembly or artifact lookup: {forbidden}"
        );
    }
}

#[tokio::test]
async fn websocket_jsonrpc_target_path_only_no_method_keeps_zero_acquire() {
    let (host, physical) = fixture::admitted_path_only_websocket_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    assert!(physical.entry().optional_handler().is_none());
    assert!(!physical.has_websocket_jsonrpc_methods().unwrap());

    let header = handlerless_connect_header(&physical, "path-only-connect");
    let frame = encode_binary_frame(&header, &[]).unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut control = None;
    let mut fingerprint = None;
    super::super::dispatch_router_binary_frame(
        &host,
        &frame,
        &sender,
        &mut control,
        &mut fingerprint,
    )
    .await
    .expect("path-only runtime request should fail closed at Host admission");
    let RouterWriterMessage::Binary(_) = receiver.recv().await.unwrap() else {
        panic!("path-only admission rejection must use binary response.error")
    };
    assert!(
        receiver.try_recv().is_err(),
        "path-only admission must not queue a generation acquire"
    );
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
}

#[tokio::test]
async fn websocket_generation_acquire_receipt_mismatch_fails_and_cleans_before_attach() {
    let (host, physical, _) = fixture::admitted_websocket_gateway_host().await;
    host.websocket_generations.connect(ROUTER_SESSION).unwrap();
    let websocket_entry_id = skiff_artifact_identity::websocket_entry_id(
        &physical.entry().owner().service_id,
        physical.gateway_entry_key(),
    )
    .unwrap();
    let (acquire, receipt) = host
        .websocket_generations
        .begin_acquire_with_receipt(
            ROUTER_SESSION,
            physical,
            websocket_entry_id.as_str().to_string(),
            "receipt-mismatch".to_string(),
        )
        .unwrap();
    let mut mismatch = acquire_ack(&acquire);
    let WebSocketGenerationLifecycleControl::Ack { tuple, .. } = &mut mismatch else {
        unreachable!()
    };
    tuple.websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "f".repeat(64));
    assert!(host
        .websocket_generations
        .handle_acquire_response(&mismatch)
        .is_err());
    assert!(receipt.wait().await.is_err());
    assert_eq!(host.websocket_generations.pin_count().unwrap(), 0);
}

fn handlerless_connect_header(
    route: &crate::loader::assembly_admission::ActiveAssemblyRoute,
    request_id: &str,
) -> RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
    let selector = route.selector();
    let websocket_entry_id = skiff_artifact_identity::websocket_entry_id(
        &route.entry().owner().service_id,
        route.gateway_entry_key(),
    )
    .unwrap();
    RuntimeAssemblyWebSocketConnectRequestStartFrameHeader {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: "request.start".to_string(),
        request_id: request_id.to_string(),
        mode: "unary".to_string(),
        caller: RuntimeAssemblyRequestCallerFrameHeader {
            kind: "gateway".to_string(),
        },
        routing: RuntimeAssemblyWebSocketConnectRoutingFrameHeader {
            kind: "runtimeAssembly".to_string(),
            assembly_identity: None,
            assembly_generation: None,
            deployment: route.deployment().clone(),
            build_id: Some(
                route
                    .deployment()
                    .deployment_artifact_identity
                    .as_str()
                    .to_string(),
            ),
            gateway_entry_identity: route.gateway_entry_identity().clone(),
            ingress: RuntimeAssemblyWebSocketConnectIngressFrameHeader {
                protocol: RuntimeAssemblyWebSocketConnectIngressProtocol::WebSocket,
                method: (),
                path: selector.path.clone(),
            },
        },
        client_session: None,
        deadline: None,
        trace: RuntimeAssemblyRequestTraceFrameHeader {
            trace_id: format!("trace-{request_id}"),
            span_id: "span-handlerless-websocket".to_string(),
            parent_span_id: None,
            sampled: None,
        },
        websocket_connect: RuntimeAssemblyWebSocketConnectRequestFrameHeader {
            connection_id: "handlerless-connection".to_string(),
            url: format!("ws://websocket.test{}", selector.path),
            query: Vec::new(),
            headers: Vec::new(),
            cookies: Vec::new(),
            version: None,
            websocket_entry_id,
            gateway_entry_identity: route.gateway_entry_identity().clone(),
        },
        test_effects_enabled: false,
    }
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

fn acquire_rejection(
    acquire: &WebSocketGenerationLifecycleControl,
) -> WebSocketGenerationLifecycleControl {
    let WebSocketGenerationLifecycleControl::Acquire {
        request_id, tuple, ..
    } = acquire
    else {
        panic!("expected acquire")
    };
    WebSocketGenerationLifecycleControl::Reject {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
        operation: WebSocketGenerationLifecycleOperation::Acquire,
        request_id: request_id.clone(),
        sender: WebSocketGenerationLifecycleSender::Router,
        tuple: tuple.clone(),
        code: WebSocketGenerationLifecycleRejectionCode::GenerationUnavailable,
        reason: "generation unavailable".to_string(),
    }
}

fn lifecycle_tuple(
    control: &WebSocketGenerationLifecycleControl,
) -> &WebSocketGenerationLifecycleTuple {
    match control {
        WebSocketGenerationLifecycleControl::Acquire { tuple, .. } => tuple,
        _ => panic!("expected acquire"),
    }
}

fn release_control(
    suffix: &str,
    tuple: WebSocketGenerationLifecycleTuple,
) -> WebSocketGenerationLifecycleControl {
    WebSocketGenerationLifecycleControl::Release {
        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
        frame_type: WEBSOCKET_GENERATION_LIFECYCLE_FRAME_TYPE.to_string(),
        request_id: format!("skiff-websocket-lifecycle-request-v1:opaque:{suffix}"),
        sender: WebSocketGenerationLifecycleSender::Router,
        tuple,
    }
}

async fn receive_lifecycle(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
) -> WebSocketGenerationLifecycleControl {
    let RouterWriterMessage::Binary(frame) = receiver
        .recv()
        .await
        .expect("lifecycle response should queue")
    else {
        panic!("lifecycle response must be binary")
    };
    decode_websocket_generation_lifecycle_frame(
        WebSocketGenerationLifecycleDirection::RuntimeToRouter,
        &frame,
    )
    .expect("lifecycle response should decode")
}
