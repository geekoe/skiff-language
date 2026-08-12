use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use skiff_artifact_model::{IngressProtocol, Opcode};
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionCorrelation, BytecodeExecutionEvent, BytecodeExecutionEventSink,
    BytecodeExecutionObservation, BytecodeGatewayCallableRole, BytecodeRequestTerminal,
    BytecodeRouteEntrySelector,
};
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::protocol::decode_binary_frame;
use tokio::{sync::mpsc, time::timeout};

use super::phase_0_proof_support::{
    receive_correlated_response, runtime_host, CorrelatedResponse, Correlation, PublishedFixture,
};

#[derive(Default)]
struct RecordingSink {
    observations: Mutex<Vec<BytecodeExecutionObservation>>,
}

impl BytecodeExecutionEventSink for RecordingSink {
    fn observe(&self, observation: BytecodeExecutionObservation) {
        self.observations
            .lock()
            .expect("lock Phase 0 observation recorder")
            .push(observation);
    }
}

impl RecordingSink {
    fn snapshot(&self) -> Vec<BytecodeExecutionObservation> {
        self.observations
            .lock()
            .expect("lock Phase 0 observation recorder")
            .clone()
    }
}

async fn drain_until_router_writer_closes(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    request_id: &str,
) {
    timeout(Duration::from_secs(10), async {
        while let Some(message) = receiver.recv().await {
            let RouterWriterMessage::Binary(frame) = message else {
                continue;
            };
            let decoded = decode_binary_frame(&frame)
                .expect("decode frame emitted after the correlated terminal");
            let Some(header) = decoded.header.as_object() else {
                continue;
            };
            let is_same_request =
                header.get("requestId").and_then(serde_json::Value::as_str) == Some(request_id);
            let is_terminal = matches!(
                header.get("type").and_then(serde_json::Value::as_str),
                Some("response.end" | "response.error")
            );
            assert!(
                !(is_same_request && is_terminal),
                "production emitted a second correlated terminal response"
            );
        }
    })
    .await
    .expect("router writer channel did not close after the correlated terminal");
}

#[tokio::test(flavor = "current_thread")]
async fn phase_0_vcp_production_composition() {
    let correlation = Correlation::new("vcp-1-success");
    let fixture = PublishedFixture::build("phase-0-vcp-success");
    let request = fixture.canonical_request(&correlation, "unary");
    assert!(!request.frame.is_empty());
    assert_eq!(request.body, b"2");

    let expected_ingress = fixture
        .deployment_artifact
        .ingress
        .iter()
        .find(|binding| {
            binding.selector.protocol == IngressProtocol::Http
                && binding.selector.method.as_deref() == Some("POST")
                && binding.selector.path == "/phase-0/vcp"
        })
        .expect("published fixture retains the canonical VCP ingress");
    let expected_gateway_key = expected_ingress.gateway_entry_key.clone();
    let expected_correlation = BytecodeExecutionCorrelation {
        router_session_id: correlation.router_session_id.clone(),
        request_id: correlation.request_id.clone(),
    };

    let sink = Arc::new(RecordingSink::default());
    let mut host = runtime_host(&correlation);
    host.bytecode_execution_event_sink = sink.clone();
    let bootstrap = fixture.connection_bootstrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    host.spawn_bytecode_request(
        &correlation.router_session_id,
        request.header,
        request.body,
        &bootstrap,
        sender,
    )
    .await;

    let response = receive_correlated_response(&mut receiver, &correlation.request_id).await;
    drain_until_router_writer_closes(&mut receiver, &correlation.request_id).await;
    let observations = sink.snapshot();
    assert_eq!(
        observations.len(),
        5,
        "the VCP requires exactly five events"
    );
    for (ordinal, observation) in observations.iter().enumerate() {
        assert_eq!(observation.correlation, expected_correlation);
        assert_eq!(observation.ordinal, ordinal as u64);
    }

    let selected = match &observations[0].event {
        BytecodeExecutionEvent::DeploymentImageSelected(selected) => selected,
        other => panic!("ordinal 0 must select the deployment image, got {other:?}"),
    };
    assert_eq!(selected.deployment, fixture.deployment);
    assert_eq!(
        selected.deployment_build_id,
        fixture.deployment.deployment_artifact_identity
    );
    assert_eq!(fixture.release_pointer.deployment, selected.deployment);

    let pinned = match &observations[1].event {
        BytecodeExecutionEvent::RouteEntryPinned(pinned) => pinned,
        other => panic!("ordinal 1 must pin the verified route entry, got {other:?}"),
    };
    assert_eq!(pinned.image_owner, selected.deployment);
    assert_eq!(
        pinned.selector,
        BytecodeRouteEntrySelector::Gateway(expected_ingress.selector.clone())
    );
    assert_eq!(pinned.gateway_key.as_ref(), Some(&expected_gateway_key));
    assert_eq!(
        pinned.gateway_identity.as_ref(),
        Some(&fixture.gateway_identity)
    );
    assert_eq!(
        pinned.callable_role,
        Some(BytecodeGatewayCallableRole::Handler)
    );

    let dispatched = match &observations[2].event {
        BytecodeExecutionEvent::VmFirstInstructionDispatched(dispatched) => dispatched,
        other => panic!("ordinal 2 must be the first successful VM dispatch, got {other:?}"),
    };
    assert_eq!(dispatched.image_owner, pinned.image_owner);
    assert_eq!(
        dispatched.root_entry_function_index,
        pinned.verified_function_index
    );
    assert_eq!(
        dispatched.current_function_index,
        pinned.verified_function_index
    );
    assert_eq!(dispatched.instruction_index, 0);
    assert_eq!(dispatched.opcode, Opcode::LoadSlot);

    let terminal = match &observations[3].event {
        BytecodeExecutionEvent::RequestTerminalClaimed(terminal) => terminal,
        other => panic!("ordinal 3 must claim the request terminal, got {other:?}"),
    };
    assert_eq!(terminal.terminal, BytecodeRequestTerminal::Succeeded);
    assert!(matches!(
        &observations[4].event,
        BytecodeExecutionEvent::RequestCleanupComplete(_)
    ));

    match response {
        CorrelatedResponse::End { header, body, .. } => {
            assert_eq!(header.request_id, correlation.request_id);
            assert!(header.payload_present);
            assert_eq!(body, b"3.0");
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                serde_json::json!(3.0)
            );
        }
        CorrelatedResponse::Error { header, error, .. } => {
            panic!("VCP returned correlated error {header:?}: {error:?}")
        }
    }
}
