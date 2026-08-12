use std::time::Duration;

use serde_json::Value;
use skiff_runtime_request::RouterWriterMessage;
use skiff_runtime_transport::protocol::{
    decode_binary_frame, decode_bytecode_request_start_frame, decode_response_end_frame,
    decode_response_error_frame, encode_binary_frame, BytecodeHttpRequestFrameHeader,
    BytecodeRequestCallerFrameHeader, BytecodeRequestIngressFrameHeader,
    BytecodeRequestIngressProtocol, BytecodeRequestRoutingFrameHeader,
    BytecodeRequestStartFrameHeader, BytecodeRequestStartFrameWireHeader,
    BytecodeRequestTraceFrameHeader, ResponseEndFrameHeader, ResponseErrorFrameHeader,
    ValidatedResponseErrorFrame, RUNTIME_FRAME_SCHEMA_VERSION,
};
use tokio::{sync::mpsc, time::timeout};

use super::{Correlation, PublishedFixture};

const REQUEST_BODY: &[u8] = b"2";

pub(super) struct CanonicalSkbfRequest {
    pub(super) frame: Vec<u8>,
    pub(super) header: BytecodeRequestStartFrameWireHeader,
    pub(super) body: Vec<u8>,
}

impl PublishedFixture {
    pub(super) fn http_header(
        &self,
        correlation: &Correlation,
        mode: &str,
    ) -> BytecodeRequestStartFrameHeader {
        BytecodeRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id: correlation.request_id.clone(),
            mode: mode.to_string(),
            caller: BytecodeRequestCallerFrameHeader {
                kind: "gateway".to_string(),
            },
            routing: BytecodeRequestRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: None,
                assembly_generation: None,
                deployment: self.deployment.clone(),
                build_id: Some(
                    self.deployment
                        .deployment_artifact_identity
                        .as_str()
                        .to_string(),
                ),
                gateway_entry_identity: self.gateway_identity.clone(),
                ingress: BytecodeRequestIngressFrameHeader {
                    protocol: BytecodeRequestIngressProtocol::Http,
                    method: "POST".to_string(),
                    path: "/phase-0/vcp".to_string(),
                },
            },
            client_session: None,
            deadline: None,
            trace: BytecodeRequestTraceFrameHeader {
                trace_id: format!("trace-{}", correlation.request_id),
                span_id: format!("span-{}", correlation.request_id),
                parent_span_id: None,
                sampled: None,
            },
            http_request: BytecodeHttpRequestFrameHeader {
                method: "POST".to_string(),
                url: "http://phase-0.example.test/phase-0/vcp".to_string(),
                path: "/phase-0/vcp".to_string(),
                query: Vec::new(),
                headers: Vec::new(),
            },
            test_effects_enabled: false,
            test_case_capability: None,
            test_case_parent_request_id: None,
        }
    }

    pub(super) fn canonical_request(
        &self,
        correlation: &Correlation,
        mode: &str,
    ) -> CanonicalSkbfRequest {
        let frame = encode_binary_frame(&self.http_header(correlation, mode), REQUEST_BODY)
            .expect("encode canonical Phase 0 SKBF request");
        let (header, body) = decode_bytecode_request_start_frame(&frame)
            .expect("production decoder accepts canonical Phase 0 request");
        CanonicalSkbfRequest {
            frame,
            header,
            body,
        }
    }
}

pub(super) enum CorrelatedResponse {
    End {
        frame: Vec<u8>,
        header: ResponseEndFrameHeader,
        body: Vec<u8>,
    },
    Error {
        frame: Vec<u8>,
        header: ResponseErrorFrameHeader,
        error: ValidatedResponseErrorFrame,
    },
}

pub(super) fn decode_typed_response(frame: Vec<u8>) -> CorrelatedResponse {
    let decoded = decode_binary_frame(&frame).expect("decode terminal response SKBF");
    match decoded.header.get("type").and_then(Value::as_str) {
        Some("response.end") => {
            let (header, body) =
                decode_response_end_frame(&frame).expect("production response.end decoder");
            CorrelatedResponse::End {
                frame,
                header,
                body,
            }
        }
        Some("response.error") => {
            let (header, error) =
                decode_response_error_frame(&frame).expect("production response.error decoder");
            CorrelatedResponse::Error {
                frame,
                header,
                error,
            }
        }
        other => panic!("expected terminal response.end or response.error, got {other:?}"),
    }
}

pub(super) async fn receive_correlated_response(
    receiver: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    request_id: &str,
) -> CorrelatedResponse {
    timeout(Duration::from_secs(10), async {
        loop {
            let message = receiver.recv().await.expect("router writer channel closed");
            let RouterWriterMessage::Binary(frame) = message else {
                continue;
            };
            let decoded = decode_binary_frame(&frame).expect("decode router writer binary frame");
            let Some(header) = decoded.header.as_object() else {
                continue;
            };
            if header.get("requestId").and_then(Value::as_str) != Some(request_id) {
                continue;
            }
            if matches!(
                header.get("type").and_then(Value::as_str),
                Some("response.end" | "response.error")
            ) {
                return decode_typed_response(frame);
            }
        }
    })
    .await
    .expect("correlated terminal response timeout")
}
