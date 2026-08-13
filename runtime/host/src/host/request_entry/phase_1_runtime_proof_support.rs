use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::Value;
use skiff_runtime_model::bytecode_execution_observation::{
    BytecodeExecutionCorrelation, BytecodeExecutionEventSink, BytecodeExecutionObservation,
};
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::protocol::decode_binary_frame;
use tokio::{sync::mpsc, time::timeout};

use super::{
    phase_0_proof_support::{
        receive_correlated_response, CanonicalSkbfRequest, CorrelatedResponse, Correlation,
    },
    RuntimeHost,
};
use crate::host::router_session::ConnectionBootstrap;

mod observations;

pub(super) use observations::phase_1_observation_gaps;

#[derive(Default)]
pub(super) struct Phase1RecordingSink(Mutex<Vec<BytecodeExecutionObservation>>);

impl BytecodeExecutionEventSink for Phase1RecordingSink {
    fn observe(&self, observation: BytecodeExecutionObservation) {
        self.0
            .lock()
            .expect("lock Phase 1 runtime proof recorder")
            .push(observation);
    }
}

impl Phase1RecordingSink {
    pub(super) fn for_correlation(
        &self,
        correlation: &Correlation,
    ) -> Vec<BytecodeExecutionObservation> {
        let expected = BytecodeExecutionCorrelation {
            router_session_id: correlation.router_session_id.clone(),
            request_id: correlation.request_id.clone(),
        };
        self.0
            .lock()
            .expect("lock Phase 1 runtime proof recorder")
            .iter()
            .filter(|observation| observation.correlation == expected)
            .cloned()
            .collect()
    }
}

pub(super) fn phase_1_correlation(scenario_id: &str) -> Correlation {
    Correlation {
        router_session_id: format!("skiff-router-session-v1:opaque:phase-1-{scenario_id}"),
        request_id: format!("phase-1-{scenario_id}-request"),
        scenario_id: format!("phase-1-{scenario_id}"),
    }
}

pub(super) async fn run_phase_1_request(
    host: &RuntimeHost,
    bootstrap: &ConnectionBootstrap,
    correlation: &Correlation,
    request: CanonicalSkbfRequest,
) -> CorrelatedResponse {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let router_session = correlation.router_session_epoch();
    host.spawn_bytecode_request(
        &router_session,
        request.header,
        request.body,
        bootstrap,
        sender,
    )
    .await;

    let response = receive_correlated_response(&mut receiver, &correlation.request_id).await;
    drain_after_terminal(&mut receiver, &correlation.request_id).await;
    response
}

async fn drain_after_terminal(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    request_id: &str,
) {
    timeout(Duration::from_secs(10), async {
        while let Some(message) = receiver.recv().await {
            let RouterWriterMessage::Binary(frame) = message else {
                continue;
            };
            let decoded = decode_binary_frame(&frame)
                .expect("decode frame emitted after the Phase 1 correlated terminal");
            let Some(header) = decoded.header.as_object() else {
                continue;
            };
            let same_request = header.get("requestId").and_then(Value::as_str) == Some(request_id);
            let terminal = matches!(
                header.get("type").and_then(Value::as_str),
                Some("response.end" | "response.error")
            );
            assert!(
                !(same_request && terminal),
                "Phase 1 production request {request_id} emitted a second terminal frame"
            );
        }
    })
    .await
    .expect("Phase 1 router writer channel did not close after its terminal");
}

pub(super) fn shared_sink(sink: &Arc<Phase1RecordingSink>) -> Arc<dyn BytecodeExecutionEventSink> {
    sink.clone()
}
