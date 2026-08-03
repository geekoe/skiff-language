use std::{collections::HashMap, sync::Arc, time::Duration};

use skiff_artifact_model::{
    IngressProtocol, IngressSelector, InstructionSourceSite, SyntheticInstructionSiteReason,
};
use skiff_runtime_capability_context::{
    CancellationSource, ConnectionRequestRegistry, ConnectionRequestSession,
    ConnectionRequestTerminal, DbCapabilityContext, FileSourceStreamContext,
    NativeCapabilityContexts, OutboundControlMessage, RouterWriterMessage,
};
use skiff_runtime_eval::{
    capabilities::{HttpRuntimeOptions, TimeCapabilityContext},
    native_capability::project_runtime_native_capability_context,
    program_execution::{ProgramExecutionContext, ProgramExecutionInput},
    EvalRuntimeProgram, EvalRuntimeProgramSource,
};
use skiff_runtime_linked_program::{
    ExecutableAddr, LinkOverlay, LinkedFileUnit, PublicationResourceTable, RuntimeExecutionPackage,
    RuntimeTypeContext,
};
use skiff_runtime_model::request_heap::RequestHeapLimits;
use skiff_runtime_native::capability::NativeWebsocketCapability;
use skiff_runtime_native_contract::NativeRequiredContext;
use skiff_runtime_request::{
    execution_budget::{ExecutionBudget, ExecutionBudgetConfig},
    OutboundRequestRegistry, RequestEnvelope, RuntimeOperation,
};

use super::*;
use crate::{
    capability_context::FileCapabilitySource, config_view::RuntimeConfigView,
    host::file_runtime::FileRuntime,
};

fn request_envelope() -> RequestEnvelope {
    RequestEnvelope {
        request_id: "f445h-i6-websocket-receipt".to_string(),
        mode: "websocket".to_string(),
        target: "websocket.request".to_string(),
        operation_abi_id: None,
        selector: None,
        service_id: Some("skiff.run/f445h-i6-websocket-receipt".to_string()),
        build_id: "build:f445h-i6-websocket-receipt".to_string(),
        service_protocol_identity: "service-protocol:f445h-i6-websocket-receipt".to_string(),
        contract_identity: None,
        activation_identity: None,
        ingress_selector: Some(IngressSelector {
            protocol: IngressProtocol::WebSocket,
            method: None,
            path: "/receipt".to_string(),
        }),
        binary_http: None,
        http_adapter: None,
        test_effects_enabled: false,
        test_effect_doubles: Default::default(),
        payload_bytes: Vec::new(),
        extra: Default::default(),
    }
}

fn runtime_operation() -> RuntimeOperation {
    RuntimeOperation {
        operation_abi_id: None,
        operation: "websocket.request".to_string(),
        target: "websocket.request".to_string(),
        mode: "websocket".to_string(),
        parameters: Vec::new(),
        service_protocol_identity: Some("service-protocol:f445h-i6-websocket-receipt".to_string()),
        extra: Default::default(),
    }
}

#[derive(Default)]
struct EmptyProgramSource {
    service_files: Vec<Arc<LinkedFileUnit>>,
    packages: Vec<Arc<RuntimeExecutionPackage>>,
    service_resources: PublicationResourceTable,
    task_routes: HashMap<String, ExecutableAddr>,
    link_overlay: LinkOverlay,
    types: RuntimeTypeContext,
}

impl EvalRuntimeProgramSource for EmptyProgramSource {
    fn service_id(&self) -> &str {
        "skiff.run/f445h-i6-websocket-receipt"
    }

    fn service_files(&self) -> &[Arc<LinkedFileUnit>] {
        &self.service_files
    }

    fn packages(&self) -> &[Arc<RuntimeExecutionPackage>] {
        &self.packages
    }

    fn service_resources(&self) -> &PublicationResourceTable {
        &self.service_resources
    }

    fn task_routes(&self) -> &HashMap<String, ExecutableAddr> {
        &self.task_routes
    }

    fn link_overlay(&self) -> &LinkOverlay {
        &self.link_overlay
    }

    fn types(&self) -> &RuntimeTypeContext {
        &self.types
    }
}

#[tokio::test]
async fn f445h_i6_websocket_scope_native_projection_reaches_real_pending_and_ancestor_closes_it() {
    let request = request_envelope();
    let operation = runtime_operation();
    let request_cancellation = CancellationSource::new();
    let budget = Arc::new(ExecutionBudget::new(
        ExecutionBudgetConfig::disabled(),
        None,
    ));
    let root =
        skiff_runtime_request::ExecutionControl::new(request_cancellation.token(), &budget).owned();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let current = root
        .derive_scope(
            deadline.into_std(),
            InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
            },
        )
        .expect("derived current invocation scope");
    let scope_observer = current.execution_scope().clone();
    let execution = execution_control(current.borrow());

    let registry = Arc::new(ConnectionRequestRegistry::new(4));
    let session = ConnectionRequestSession::new("router-session:f445h-i6-websocket-receipt")
        .expect("canonical router session");
    let (router_sender, mut router_receiver) = mpsc::unbounded_channel();
    let websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "f".repeat(64));
    let websocket = websocket_from_runtime_request(
        "skiff.run/f445h-i6-websocket-receipt",
        Some(&websocket_entry_id),
        Some(&router_sender),
        Arc::clone(&registry),
        session.clone(),
    );

    let runtime_factory = runtime_factory();
    let stream_runtime = runtime_factory.stream_runtime();
    let test_effect_doubles =
        runtime_factory.reusable_test_effect_doubles(HashMap::new(), &stream_runtime, false);
    let effects = effects(effect_dispatch_context_from_request(
        &request,
        1_048_576,
        execution.cancellation_token(),
        None,
        capability_contract::HttpRuntimeOptions::explicit(false),
    ));
    let db = DbCapabilityContext::unavailable();
    let file = file_source(FileCapabilitySource::new(Arc::new(FileRuntime::new(
        None,
        std::env::temp_dir().join("skiff-f445h-i6-websocket-receipt-unused"),
    ))))
    .context_for_request(db.clone());
    let actor_factory = TestActorCapabilityFactory::default();
    let outbound_requests = Arc::new(OutboundRequestRegistry::default());
    let (actor, request) = actor_factory.actor_from_request(
        "runtime:f445h-i6-websocket-receipt",
        "skiff.run/f445h-i6-websocket-receipt",
        "1.0.0",
        &request,
        &operation,
        Some(&router_sender),
        &outbound_requests,
        execution.cancellation_token(),
    );
    let request_heap_limits = RequestHeapLimits::default();
    let config = RuntimeConfigView::empty();
    let context = ProgramExecutionContext::new(ProgramExecutionInput {
        execution: execution.clone(),
        config: config_context(concrete::ConfigCapabilityContext::new(&config, &[])),
        db,
        file,
        file_source_stream: FileSourceStreamContext::new(stream_runtime.clone(), execution.clone()),
        time: TimeCapabilityContext::new(execution.clone()),
        websocket,
        effects: effects.clone(),
        http_client: effects.http_client_context(
            HttpRuntimeOptions::explicit(false),
            stream_runtime,
            test_effect_doubles.clone(),
        ),
        test_effect_doubles,
        actor: actor.clone(),
        request,
        request_heap_limits,
    });

    let program = EvalRuntimeProgram::from_source(&EmptyProgramSource::default());
    let projected = project_runtime_native_capability_context(
        &context,
        program.projection(),
        skiff_runtime_eval::capabilities::StreamCapabilityContext::default(),
        NativeRequiredContext::Websocket,
    );
    let NativeCapabilityContexts::Websocket(native) = projected else {
        panic!("native WebSocket projection expected");
    };

    let native_request = native.request_json_to_connection(
        "connection:f445h-i6".to_string(),
        "receipt.wait".to_string(),
        br#"{"wait":true}"#.to_vec(),
    );
    tokio::pin!(native_request);
    let queued = tokio::select! {
        message = router_receiver.recv() => message.expect("real connection request frame"),
        terminal = &mut native_request => panic!("native request settled before registry pending: {terminal:?}"),
    };
    let request_id = match queued {
        RouterWriterMessage::Control(OutboundControlMessage::ConnectionRequest {
            request,
            payload,
        }) => {
            assert_eq!(payload, br#"{"wait":true}"#);
            assert!(request.deadline.is_some());
            request.request_id
        }
        other => panic!("unexpected router message: {other:?}"),
    };
    assert_eq!(registry.pending_count(), 1);
    assert_eq!(registry.active_lease_count(), 1);
    assert_eq!(registry.active_timer_count(), 1);
    assert_eq!(
        scope_observer.lifecycle_snapshot(),
        capability_contract::ExecutionScopeLifecycleSnapshot {
            active_leases: 1,
            active_waiters: 1,
            active_timers: 1,
        }
    );

    request_cancellation.cancel();
    assert_eq!(
        native_request.await.expect("native request terminal"),
        ConnectionRequestTerminal::AncestorCancelled
    );
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);
    assert_eq!(registry.active_timer_count(), 0);
    assert_eq!(
        scope_observer.lifecycle_snapshot(),
        capability_contract::ExecutionScopeLifecycleSnapshot::default()
    );
    assert!(!registry.complete(
        &session,
        &request_id,
        ConnectionRequestTerminal::Success(b"late".to_vec())
    ));
    match router_receiver
        .recv()
        .await
        .expect("best-effort internal deadline hint")
    {
        RouterWriterMessage::Control(OutboundControlMessage::ConnectionRequestCancel {
            request,
        }) => {
            assert_eq!(request.request_id, request_id);
            assert_eq!(request.reason, "caller_cancel");
        }
        other => panic!("unexpected post-terminal router message: {other:?}"),
    }
}
