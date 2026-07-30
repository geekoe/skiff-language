use super::*;

pub fn runtime_factory() -> eval_capabilities::EvalRuntimeFactory {
    eval_capabilities::EvalRuntimeFactory::new(RuntimeEvalFactory)
}

pub fn execution_control<'a>(
    execution: skiff_runtime_request::ExecutionControl<'a>,
) -> eval_capabilities::ExecutionControl<'a> {
    capability_contract::ExecutionControl::new(RuntimeExecutionControl(execution.owned()))
}

pub fn config_context<'a>(
    context: concrete::ConfigCapabilityContext<'a>,
) -> eval_capabilities::ConfigCapabilityContext<'a> {
    eval_capabilities::ConfigCapabilityContext::new(RuntimeConfigCapabilityContext(context))
}

pub fn db_context(
    context: concrete::DbCapabilityContext,
) -> eval_capabilities::DbCapabilityContext {
    context
}

pub fn file_source(
    source: concrete::FileCapabilitySource,
) -> eval_capabilities::FileCapabilitySource {
    capability_contract::FileCapabilitySource::new(RuntimeFileCapabilitySource(source))
}

pub fn effects(
    context: concrete::EffectDispatchContext,
) -> eval_capabilities::EffectDispatchContext {
    eval_capabilities::EffectDispatchContext::new(RuntimeEffectDispatchContext(context))
}

pub fn websocket<'a>(
    context: concrete::WebsocketCapabilityContext<'a>,
    owned: RuntimeOwnedWebsocketParts,
) -> eval_capabilities::WebsocketCapabilityContext<'a> {
    let shared = RuntimeWebsocketCapabilityContext {
        context,
        owned: owned.clone(),
    };
    if owned.request_transport.is_some() {
        eval_capabilities::WebsocketCapabilityContext::with_request_api(
            shared,
            RuntimeWebsocketRequestCapabilityContext(owned),
        )
    } else {
        eval_capabilities::WebsocketCapabilityContext::new(shared)
    }
}

pub fn websocket_from_request<'a>(
    service_id: &'a str,
    websocket_entry_id: Option<&'a str>,
    router_sender: Option<&'a mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
) -> eval_capabilities::WebsocketCapabilityContext<'a> {
    websocket(
        concrete::WebsocketCapabilityContext::with_entry_id(
            service_id,
            websocket_entry_id,
            router_sender,
        ),
        RuntimeOwnedWebsocketParts {
            service_id: service_id.to_string(),
            websocket_entry_id: websocket_entry_id.map(str::to_string),
            router_sender: router_sender.cloned(),
            request_transport: None,
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn websocket_from_runtime_request<'a>(
    service_id: &'a str,
    websocket_entry_id: Option<&'a str>,
    router_sender: Option<&'a mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    connection_requests: Arc<ConnectionRequestRegistry>,
    router_session: ConnectionRequestSession,
) -> eval_capabilities::WebsocketCapabilityContext<'a> {
    let request_transport = RuntimeConnectionRequestParts {
        registry: connection_requests,
        session: router_session,
    };
    websocket(
        concrete::WebsocketCapabilityContext::with_entry_id(
            service_id,
            websocket_entry_id,
            router_sender,
        ),
        RuntimeOwnedWebsocketParts {
            service_id: service_id.to_string(),
            websocket_entry_id: websocket_entry_id.map(str::to_string),
            router_sender: router_sender.cloned(),
            request_transport: Some(request_transport),
        },
    )
}

pub(crate) fn actor_from_request<'a>(
    runtime_id: &'a str,
    service_id: &'a str,
    service_version: &'a str,
    request: &'a RequestEnvelope,
    operation: &'a RuntimeOperation,
    activation_identity: Option<&'a ActivationIdentityControl>,
    router_sender: Option<&'a mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
    outbound_requests: &'a Arc<OutboundRequestRegistry>,
    actor_method_outbound: &'a Arc<ActorMethodOutboundRegistry>,
    cancellation: CancellationToken,
) -> eval_capabilities::ActorCapabilityContext<'a> {
    let invocation = invocation_context_from_request(
        runtime_id,
        service_id,
        service_version,
        request,
        operation,
    );
    let context = concrete::ActorClientContext::new(
        invocation,
        activation_identity,
        router_sender,
        outbound_requests.as_ref(),
        cancellation.clone(),
    );
    let owned = RuntimeOwnedActorParts {
        runtime_id: context.runtime_id().to_string(),
        service_id: context.service_id().to_string(),
        service_version: context.service_version().to_string(),
        request_id: context.request_id().to_string(),
        request_target: context.request_target().to_string(),
        request_build_id: context.request_build_id().to_string(),
        request_service_protocol_identity: context.request_service_protocol_identity().to_string(),
        operation_service_protocol_identity: context
            .operation_service_protocol_identity()
            .map(str::to_string),
        activation_identity: context.activation_identity().cloned(),
        trace_id: context.trace_id().map(str::to_string),
        router_sender: router_sender.cloned(),
        outbound_requests: outbound_requests.clone(),
        actor_method_outbound: actor_method_outbound.clone(),
        cancellation,
    };
    actor(context, owned)
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct TestActorCapabilityFactory {
    actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
}

#[cfg(any(test, feature = "test-support"))]
impl TestActorCapabilityFactory {
    pub fn actor_from_request<'a>(
        &'a self,
        runtime_id: &'a str,
        service_id: &'a str,
        service_version: &'a str,
        request: &'a RequestEnvelope,
        operation: &'a RuntimeOperation,
        router_sender: Option<&'a mpsc::UnboundedSender<concrete::RouterWriterMessage>>,
        outbound_requests: &'a Arc<OutboundRequestRegistry>,
        cancellation: CancellationToken,
    ) -> eval_capabilities::ActorCapabilityContext<'a> {
        actor_from_request(
            runtime_id,
            service_id,
            service_version,
            request,
            operation,
            None,
            router_sender,
            outbound_requests,
            &self.actor_method_outbound,
            cancellation,
        )
    }
}

#[derive(Clone)]
struct RuntimeEvalFactory;

impl eval_capabilities::EvalRuntimeFactoryApi for RuntimeEvalFactory {
    fn stream_runtime(&self) -> eval_capabilities::StreamRuntime {
        capability_contract::StreamRuntime::new(RuntimeStreamRuntime(
            concrete::StreamRuntime::default(),
        ))
    }

    fn reusable_test_effect_doubles(
        &self,
        doubles: HashMap<String, eval_capabilities::TestEffectDouble>,
        stream_runtime: &eval_capabilities::StreamRuntime,
        test_effects_enabled: bool,
    ) -> eval_capabilities::TestEffectDoubleContext {
        eval_capabilities::TestEffectDoubleContext::new(RuntimeTestEffectDoubleContext(
            concrete::TestEffectDoubleContext::reusable(
                doubles
                    .into_iter()
                    .map(|(target, double)| (target, concrete_test_double(double)))
                    .collect(),
                concrete_stream_runtime(stream_runtime).clone(),
                test_effects_enabled,
            ),
        ))
    }

    fn one_shot_test_effect_double_sequences(
        &self,
        doubles: HashMap<String, Vec<eval_capabilities::TestEffectDouble>>,
        stream_runtime: &eval_capabilities::StreamRuntime,
        test_effects_enabled: bool,
    ) -> eval_capabilities::TestEffectDoubleContext {
        eval_capabilities::TestEffectDoubleContext::new(RuntimeTestEffectDoubleContext(
            concrete::TestEffectDoubleContext::one_shot_sequences(
                doubles
                    .into_iter()
                    .map(|(target, doubles)| {
                        (
                            target,
                            doubles.into_iter().map(concrete_test_double).collect(),
                        )
                    })
                    .collect(),
                concrete_stream_runtime(stream_runtime).clone(),
                test_effects_enabled,
            ),
        ))
    }
}

#[cfg(test)]
mod websocket_context_tests {
    use std::time::Duration;

    use super::*;
    use capability_contract::{
        CancellationSource, ConnectionRequestRegistry, ConnectionRequestSession,
        ConnectionRequestTerminal, OutboundControlMessage, RouterWriterMessage,
    };

    fn connection_request(
        message: RouterWriterMessage,
    ) -> (capability_contract::ConnectionRequestControl, Vec<u8>) {
        match message {
            RouterWriterMessage::Control(OutboundControlMessage::ConnectionRequest {
                request,
                payload,
            }) => (request, payload),
            other => panic!("unexpected router message: {other:?}"),
        }
    }

    fn connection_request_cancel(
        message: RouterWriterMessage,
    ) -> capability_contract::ConnectionRequestCancelControl {
        match message {
            RouterWriterMessage::Control(OutboundControlMessage::ConnectionRequestCancel {
                request,
            }) => request,
            other => panic!("unexpected router message: {other:?}"),
        }
    }

    fn assert_connection_request_registry_empty(registry: &ConnectionRequestRegistry) {
        assert_eq!(registry.pending_count(), 0);
        assert_eq!(registry.active_lease_count(), 0);
        assert_eq!(registry.active_timer_count(), 0);
    }

    #[tokio::test]
    async fn f445h_i6_websocket_scope_uses_current_scope_registry() {
        let registry = Arc::new(ConnectionRequestRegistry::new(4));
        let session =
            ConnectionRequestSession::new("router-session-provider").expect("canonical session");
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "a".repeat(64));
        let provider = websocket_from_runtime_request(
            "service:provider",
            Some(&websocket_entry_id),
            Some(&sender),
            Arc::clone(&registry),
            session.clone(),
        );

        let request = provider.request_json_to_connection(
            "connection-1".to_string(),
            "status.get".to_string(),
            br#"{"include":"summary"}"#.to_vec(),
            test_execution_control(),
        );
        tokio::pin!(request);
        let queued = tokio::select! {
            message = receiver.recv() => message.expect("connection request frame"),
            result = &mut request => panic!("request settled before queue: {result:?}"),
        };
        let request_id = match queued {
            RouterWriterMessage::Control(OutboundControlMessage::ConnectionRequest {
                request,
                payload,
            }) => {
                assert_eq!(request.service_id, "service:provider");
                assert_eq!(request.websocket_entry_id, websocket_entry_id);
                assert_eq!(request.connection_id, "connection-1");
                assert_eq!(request.method, "status.get");
                assert_eq!(payload, br#"{"include":"summary"}"#);
                request.request_id
            }
            other => panic!("unexpected router message: {other:?}"),
        };
        assert_eq!(registry.pending_count(), 1);
        assert!(registry.complete(
            &session,
            &request_id,
            ConnectionRequestTerminal::Success(br#"{"ok":true}"#.to_vec()),
        ));
        assert_eq!(
            request.await.expect("attached Host future"),
            ConnectionRequestTerminal::Success(br#"{"ok":true}"#.to_vec())
        );
        assert_connection_request_registry_empty(&registry);
    }

    #[tokio::test]
    async fn f445h_i6_websocket_scope_remote_terminal_releases_all_state() {
        let registry = Arc::new(ConnectionRequestRegistry::new(4));
        let session = ConnectionRequestSession::new("router-session-remote").expect("session");
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "b".repeat(64));
        let context = websocket_from_runtime_request(
            "service:caller",
            Some(&websocket_entry_id),
            Some(&sender),
            Arc::clone(&registry),
            session.clone(),
        );

        let request = context.request_json_to_connection(
            "connection-remote".to_string(),
            "status.get".to_string(),
            br#"{}"#.to_vec(),
            test_execution_control(),
        );
        tokio::pin!(request);
        let queued = tokio::select! {
            message = receiver.recv() => message.expect("connection request frame"),
            result = &mut request => panic!("request settled before queue: {result:?}"),
        };
        let (control, _) = connection_request(queued);
        let remote = ConnectionRequestTerminal::Remote {
            code: -32_001,
            message: "provider unavailable".to_string(),
            data: Some(br#"{"retry":true}"#.to_vec()),
        };
        assert!(registry.complete(&session, &control.request_id, remote.clone()));
        assert_eq!(request.await.expect("remote terminal"), remote);
        assert_connection_request_registry_empty(&registry);
    }

    #[tokio::test]
    async fn f445h_i6_websocket_scope_ancestor_stop_emits_hint_after_local_release() {
        let registry = Arc::new(ConnectionRequestRegistry::new(4));
        let session = ConnectionRequestSession::new("router-session-cancel").expect("session");
        let cancellation = CancellationSource::new();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "c".repeat(64));
        let context = websocket_from_runtime_request(
            "service:caller",
            Some(&websocket_entry_id),
            Some(&sender),
            Arc::clone(&registry),
            session,
        );

        let execution_control = test_execution_control_from(cancellation.token(), None);
        let request = context.request_json_to_connection(
            "connection-cancel".to_string(),
            "status.get".to_string(),
            br#"{}"#.to_vec(),
            execution_control,
        );
        tokio::pin!(request);
        let queued = tokio::select! {
            message = receiver.recv() => message.expect("connection request frame"),
            result = &mut request => panic!("request settled before queue: {result:?}"),
        };
        let (control, _) = connection_request(queued);
        cancellation.cancel();
        assert_eq!(
            request.await.expect("cancel terminal"),
            ConnectionRequestTerminal::AncestorCancelled
        );
        let cancel =
            connection_request_cancel(receiver.recv().await.expect("dedicated connection cancel"));
        assert_eq!(cancel.request_id, control.request_id);
        assert_eq!(cancel.reason, "caller_cancel");
        assert_connection_request_registry_empty(&registry);
    }

    #[tokio::test]
    async fn f445h_i6_websocket_scope_derived_deadline_emits_hint_and_releases_state() {
        let registry = Arc::new(ConnectionRequestRegistry::new(4));
        let session = ConnectionRequestSession::new("router-session-deadline").expect("session");
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "d".repeat(64));
        let context = websocket_from_runtime_request(
            "service:caller",
            Some(&websocket_entry_id),
            Some(&sender),
            Arc::clone(&registry),
            session,
        );

        let deadline = Instant::now() + Duration::from_millis(100);
        let execution_control = test_execution_control()
            .derive_scope(
                deadline,
                skiff_artifact_model::InstructionSourceSite::Synthetic {
                    reason:
                        skiff_artifact_model::SyntheticInstructionSiteReason::RuntimeControlFlow,
                },
            )
            .expect("derived current execution scope");
        let request = context.request_json_to_connection(
            "connection-deadline".to_string(),
            "status.get".to_string(),
            br#"{}"#.to_vec(),
            execution_control,
        );
        tokio::pin!(request);
        let queued = tokio::select! {
            message = receiver.recv() => message.expect("connection request frame"),
            result = &mut request => panic!("request settled before queue: {result:?}"),
        };
        let (control, _) = connection_request(queued);
        assert!(control.deadline.is_some());
        assert_eq!(
            request.await.expect("deadline terminal"),
            ConnectionRequestTerminal::DeadlineExceeded
        );
        let cancel =
            connection_request_cancel(receiver.recv().await.expect("dedicated connection cancel"));
        assert_eq!(cancel.request_id, control.request_id);
        assert_eq!(cancel.reason, "deadline_exceeded");
        assert_connection_request_registry_empty(&registry);
    }

    #[tokio::test]
    async fn f445h_i6_websocket_scope_disconnect_session_fence_releases_state() {
        let registry = Arc::new(ConnectionRequestRegistry::new(4));
        let session = ConnectionRequestSession::new("router-session-disconnect").expect("session");
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let websocket_entry_id = format!("skiff-websocket-entry-v1:sha256:{}", "e".repeat(64));
        let context = websocket_from_runtime_request(
            "service:caller",
            Some(&websocket_entry_id),
            Some(&sender),
            Arc::clone(&registry),
            session.clone(),
        );

        let request = context.request_json_to_connection(
            "connection-disconnect".to_string(),
            "status.get".to_string(),
            br#"{}"#.to_vec(),
            test_execution_control(),
        );
        tokio::pin!(request);
        let queued = tokio::select! {
            message = receiver.recv() => message.expect("connection request frame"),
            result = &mut request => panic!("request settled before queue: {result:?}"),
        };
        let (control, _) = connection_request(queued);
        assert_eq!(registry.disconnect_session(&session), 1);
        assert_eq!(
            request.await.expect("disconnect terminal"),
            ConnectionRequestTerminal::TransportUnavailable
        );
        assert!(!registry.complete(
            &session,
            &control.request_id,
            ConnectionRequestTerminal::Success(b"late".to_vec())
        ));
        assert!(receiver.try_recv().is_err());
        assert_connection_request_registry_empty(&registry);
    }

    #[tokio::test]
    async fn default_connection_request_capability_remains_unsupported() {
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let context = websocket_from_request(
            "service:caller",
            Some("websocket-entry:caller"),
            Some(&sender),
        );

        let error = context
            .request_json_to_connection(
                "connection-1".to_string(),
                "status.get".to_string(),
                br#"{}"#.to_vec(),
                test_execution_control(),
            )
            .await
            .expect_err("unattached request capability must fail closed");
        assert!(error.to_string().contains("execution is not attached"));
        assert!(receiver.try_recv().is_err());
    }

    fn test_execution_control() -> capability_contract::OwnedExecutionControl {
        test_execution_control_from(CancellationToken::new(), None)
    }

    fn test_execution_control_from(
        cancellation: CancellationToken,
        deadline: Option<Instant>,
    ) -> capability_contract::OwnedExecutionControl {
        use skiff_runtime_request::execution_budget::{ExecutionBudget, ExecutionBudgetConfig};

        let budget = Arc::new(ExecutionBudget::new(
            ExecutionBudgetConfig::disabled(),
            deadline,
        ));
        let execution = skiff_runtime_request::ExecutionControl::new(cancellation, &budget);
        super::execution_control(execution).owned()
    }
}
