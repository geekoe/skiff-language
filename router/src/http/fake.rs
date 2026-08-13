//! Fake HTTP dispatcher (C-dispatch §7.7 fake seam) for the W-http real
//! boundary probe. Test-only: no production composition consumes it.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::Value;
use skiff_runtime_request_contract::OpaqueServiceError;
use skiff_runtime_transport::cancel_reason::RequestCancelReason;
use skiff_runtime_transport::protocol::BytecodeRequestStartFrameHeader;
use skiff_runtime_transport::protocol::{
    ResponseErrorFrameHeader, RuntimeErrorFramePayload, RuntimeHttpNameValueFrameHeader,
};
use tokio::sync::watch;

use super::dispatch::{
    cancel_reason_for_terminal, DispatchRequest, HttpDispatchError, HttpDispatchPort,
    PendingTerminalSource, TestDispatchOutcome, UnaryHttpResponse,
};
use super::stream::HttpStreamSink;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedDispatchRequest {
    pub header: BytecodeRequestStartFrameHeader,
    pub payload_bytes: Bytes,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedCancel {
    pub request_id: String,
    pub reason: RequestCancelReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeDispatchPlan {
    UnaryOk {
        status: u16,
        headers: Vec<(String, String)>,
        payload: Bytes,
    },
    UnaryControlError {
        code: String,
        message: String,
        status: Option<u16>,
        details: Option<Value>,
    },
    UnaryFixedServiceError {
        trace_id: String,
        error_id: String,
    },
    UnaryRuntimeCancel,
    UnaryHang,
    Stream {
        events: Vec<FakeStreamEvent>,
    },
    StreamHang,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeStreamEvent {
    Start {
        status: u16,
        headers: Vec<(String, String)>,
    },
    Chunk {
        seq: u64,
        payload: Bytes,
    },
    End,
    Delay {
        duration: Duration,
    },
    RuntimeCancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StreamPhase {
    WaitingStart,
    Streaming { next_seq: u64 },
    Terminal,
}

struct FakeState {
    plans: VecDeque<FakeDispatchPlan>,
    requests: Vec<RecordedDispatchRequest>,
    cancels: Vec<RecordedCancel>,
    requests_tx: watch::Sender<usize>,
    cancels_tx: watch::Sender<usize>,
}

/// Scripted dispatcher used by `router/tests/http_*` real-socket probes.
#[derive(Clone)]
pub struct FakeHttpDispatcher {
    state: Arc<Mutex<FakeState>>,
    requests_rx: watch::Receiver<usize>,
    cancels_rx: watch::Receiver<usize>,
}

impl FakeHttpDispatcher {
    pub fn new(plans: Vec<FakeDispatchPlan>) -> Self {
        let (requests_tx, requests_rx) = watch::channel(0);
        let (cancels_tx, cancels_rx) = watch::channel(0);
        Self {
            state: Arc::new(Mutex::new(FakeState {
                plans: plans.into(),
                requests: Vec::new(),
                cancels: Vec::new(),
                requests_tx,
                cancels_tx,
            })),
            requests_rx,
            cancels_rx,
        }
    }

    pub fn recorded_requests(&self) -> Vec<RecordedDispatchRequest> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .requests
            .clone()
    }

    pub fn recorded_cancels(&self) -> Vec<RecordedCancel> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .cancels
            .clone()
    }

    pub async fn wait_for_requests(&mut self, expected: usize) {
        wait_for_count(&mut self.requests_rx, expected).await;
    }

    pub async fn wait_for_cancels(&mut self, expected: usize) {
        wait_for_count(&mut self.cancels_rx, expected).await;
    }

    fn record_request(&self, request: &DispatchRequest) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.requests.push(RecordedDispatchRequest {
            header: request.header.clone(),
            payload_bytes: request.payload_bytes.clone(),
            timeout: request.timeout,
        });
        let count = state.requests.len();
        let _ = state.requests_tx.send(count);
    }

    fn record_cancel(&self, request_id: &str, reason: RequestCancelReason) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.cancels.push(RecordedCancel {
            request_id: request_id.to_string(),
            reason,
        });
        let count = state.cancels.len();
        let _ = state.cancels_tx.send(count);
    }

    fn take_plan(&self) -> Option<FakeDispatchPlan> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .plans
            .pop_front()
    }

    async fn run_hang(&self, request: &DispatchRequest) -> HttpDispatchError {
        tokio::select! {
            biased;
            reason = request.client_disconnect.clone().wait() => {
                let reason = reason.unwrap_or(RequestCancelReason::CallerCancel);
                self.record_cancel(&request.header.request_id, reason);
                HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::ClientDisconnect,
                    message: "client disconnect observed while dispatch was pending".to_string(),
                }
            }
            _ = tokio::time::sleep(request.timeout) => {
                self.record_cancel(&request.header.request_id, RequestCancelReason::Timeout);
                HttpDispatchError::Timeout {
                    timeout_ms: request.timeout.as_millis() as u64,
                }
            }
        }
    }
}

async fn wait_for_count(receiver: &mut watch::Receiver<usize>, expected: usize) {
    loop {
        if *receiver.borrow() >= expected {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}

fn protocol_error(
    request_id: &str,
    dispatcher: &FakeHttpDispatcher,
    message: &str,
) -> HttpDispatchError {
    dispatcher.record_cancel(request_id, RequestCancelReason::ProtocolError);
    HttpDispatchError::Cancelled {
        source: PendingTerminalSource::ProtocolError,
        message: message.to_string(),
    }
}

fn fixed_service_error(trace_id: &str, error_id: &str) -> OpaqueServiceError {
    OpaqueServiceError::internal_error("boom", trace_id, error_id)
        .expect("fixed service envelope must encode")
}

fn http_headers(headers: &[(String, String)]) -> Vec<RuntimeHttpNameValueFrameHeader> {
    headers
        .iter()
        .map(|(name, value)| RuntimeHttpNameValueFrameHeader {
            name: name.clone(),
            value: value.clone(),
        })
        .collect()
}

#[async_trait]
impl HttpDispatchPort for FakeHttpDispatcher {
    async fn dispatch_unary(
        &self,
        request: DispatchRequest,
    ) -> Result<UnaryHttpResponse, HttpDispatchError> {
        self.record_request(&request);
        let Some(plan) = self.take_plan() else {
            return Err(HttpDispatchError::Cancelled {
                source: PendingTerminalSource::RuntimeDisconnect,
                message: "fake dispatcher has no plan".to_string(),
            });
        };
        match plan {
            FakeDispatchPlan::UnaryOk {
                status,
                headers,
                payload,
            } => Ok(UnaryHttpResponse {
                status,
                headers: http_headers(&headers),
                payload,
            }),
            FakeDispatchPlan::UnaryControlError {
                code,
                message,
                status,
                details,
            } => Err(HttpDispatchError::Control {
                code,
                message,
                status,
                details,
            }),
            FakeDispatchPlan::UnaryFixedServiceError { trace_id, error_id } => Err(
                HttpDispatchError::FixedService(fixed_service_error(&trace_id, &error_id)),
            ),
            FakeDispatchPlan::UnaryRuntimeCancel => Err(HttpDispatchError::Cancelled {
                source: PendingTerminalSource::RuntimeRequestCancel,
                message: "runtime cancelled the request".to_string(),
            }),
            FakeDispatchPlan::UnaryHang => Err(self.run_hang(&request).await),
            FakeDispatchPlan::Stream { .. } | FakeDispatchPlan::StreamHang => {
                Err(HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::ProtocolError,
                    message: "stream plan used for unary dispatch".to_string(),
                })
            }
        }
    }

    async fn dispatch_stream(
        &self,
        request: DispatchRequest,
        sink: Arc<dyn HttpStreamSink>,
    ) -> Result<(), HttpDispatchError> {
        self.record_request(&request);
        let Some(plan) = self.take_plan() else {
            return Err(HttpDispatchError::Cancelled {
                source: PendingTerminalSource::RuntimeDisconnect,
                message: "fake dispatcher has no plan".to_string(),
            });
        };
        let FakeDispatchPlan::Stream { events } = plan else {
            if matches!(plan, FakeDispatchPlan::StreamHang) {
                return Err(self.run_hang(&request).await);
            }
            return Err(HttpDispatchError::Cancelled {
                source: PendingTerminalSource::ProtocolError,
                message: "unary plan used for stream dispatch".to_string(),
            });
        };
        let mut phase = StreamPhase::WaitingStart;
        for event in events {
            match event {
                FakeStreamEvent::Start { status, headers } => {
                    if !matches!(phase, StreamPhase::WaitingStart) {
                        return Err(protocol_error(
                            &request.header.request_id,
                            self,
                            "duplicate response.start",
                        ));
                    }
                    phase = StreamPhase::Streaming { next_seq: 0 };
                    if let Err(error) = sink
                        .enqueue_start(
                            skiff_runtime_transport::protocol::RuntimeHttpResponseFrameHeader {
                                status,
                                headers: http_headers(&headers),
                            },
                        )
                        .await
                    {
                        return Err(self.sink_failure(&request.header.request_id, error));
                    }
                }
                FakeStreamEvent::Chunk { seq, payload } => {
                    let StreamPhase::Streaming { next_seq } = &phase else {
                        return Err(protocol_error(
                            &request.header.request_id,
                            self,
                            "response.chunk before response.start",
                        ));
                    };
                    if seq != *next_seq {
                        return Err(protocol_error(
                            &request.header.request_id,
                            self,
                            "response.chunk seq mismatch",
                        ));
                    }
                    phase = StreamPhase::Streaming {
                        next_seq: next_seq + 1,
                    };
                    if let Err(error) = sink.enqueue_chunk(payload).await {
                        return Err(self.sink_failure(&request.header.request_id, error));
                    }
                }
                FakeStreamEvent::End => {
                    if !matches!(phase, StreamPhase::Streaming { .. }) {
                        return Err(protocol_error(
                            &request.header.request_id,
                            self,
                            "response.end before response.start",
                        ));
                    }
                    phase = StreamPhase::Terminal;
                    if let Err(error) = sink.enqueue_end().await {
                        return Err(self.sink_failure(&request.header.request_id, error));
                    }
                }
                FakeStreamEvent::Delay { duration } => {
                    tokio::select! {
                        biased;
                        _ = tokio::time::sleep(duration) => {}
                        reason = request.client_disconnect.clone().wait() => {
                            let reason = reason.unwrap_or(RequestCancelReason::CallerCancel);
                            self.record_cancel(&request.header.request_id, reason);
                            return Err(HttpDispatchError::Cancelled {
                                source: PendingTerminalSource::ClientDisconnect,
                                message: "client disconnect observed during stream delay".to_string(),
                            });
                        }
                    }
                }
                FakeStreamEvent::RuntimeCancel => {
                    // Runtime-initiated cancel sends no Router→Runtime frame
                    // (C-dispatch §4.3).
                    return Err(HttpDispatchError::Cancelled {
                        source: PendingTerminalSource::RuntimeRequestCancel,
                        message: "runtime cancelled the stream".to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    async fn dispatch_test(
        &self,
        request: DispatchRequest,
    ) -> Result<TestDispatchOutcome, HttpDispatchError> {
        self.record_request(&request);
        let Some(plan) = self.take_plan() else {
            return Err(HttpDispatchError::Cancelled {
                source: PendingTerminalSource::RuntimeDisconnect,
                message: "fake dispatcher has no plan".to_string(),
            });
        };
        match plan {
            FakeDispatchPlan::UnaryOk {
                status,
                headers,
                payload,
            } => Ok(TestDispatchOutcome::End(UnaryHttpResponse {
                status,
                headers: http_headers(&headers),
                payload,
            })),
            FakeDispatchPlan::UnaryControlError {
                code,
                message,
                status,
                details,
            } => Ok(TestDispatchOutcome::Error(
                ResponseErrorFrameHeader::control(
                    request.header.request_id.clone(),
                    RuntimeErrorFramePayload {
                        code,
                        message,
                        status,
                        details,
                    },
                ),
                Bytes::new(),
            )),
            FakeDispatchPlan::UnaryFixedServiceError { trace_id, error_id } => {
                let error = fixed_service_error(&trace_id, &error_id);
                Ok(TestDispatchOutcome::Error(
                    ResponseErrorFrameHeader::fixed_service(request.header.request_id.clone()),
                    Bytes::from(error.into_encoded_bytes()),
                ))
            }
            FakeDispatchPlan::UnaryRuntimeCancel => Err(HttpDispatchError::Cancelled {
                source: PendingTerminalSource::RuntimeRequestCancel,
                message: "runtime cancelled the request".to_string(),
            }),
            FakeDispatchPlan::UnaryHang => Err(self.run_hang(&request).await),
            FakeDispatchPlan::Stream { .. } | FakeDispatchPlan::StreamHang => {
                Err(HttpDispatchError::Cancelled {
                    source: PendingTerminalSource::ProtocolError,
                    message: "stream plan used for test dispatch".to_string(),
                })
            }
        }
    }
}

impl FakeHttpDispatcher {
    fn sink_failure(
        &self,
        request_id: &str,
        error: super::stream::HttpStreamError,
    ) -> HttpDispatchError {
        let source = error.terminal_source();
        if let Some(reason) = cancel_reason_for_terminal(source) {
            self.record_cancel(request_id, reason);
        }
        HttpDispatchError::Cancelled {
            source,
            message: error.message,
        }
    }
}

impl std::fmt::Debug for FakeHttpDispatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("FakeHttpDispatcher")
    }
}
