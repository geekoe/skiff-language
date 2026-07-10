use std::{collections::HashMap, sync::Arc, time::Instant};

use serde_json::Value;
use skiff_runtime_capability_context::{
    CancellationToken, ExecutionBudgetFailure, ExecutionBudgetReason, ExecutionControlError,
    OutboundControlMessage, OutboundRequestCancelSendError, OutboundRequestCancelSender,
    OutboundRequestLease, OutboundRequestRegistry, OutboundResponse, OutboundResponseReceiver,
    RequestCancelControl, RequestEffectDoubleControl, RequestStartControl, RouterWriterMessage,
    RuntimeCallerControl, RuntimeClientSessionControl, RuntimeDeadlineControl,
    RuntimeTraceContextControl,
};
pub use skiff_runtime_capability_context::{OutboundServiceRequestStart, OutboundStartedRequest};
use skiff_runtime_linked_program::{ServiceDependencyConstraint, ServiceTimeoutConfig};
use skiff_runtime_model::request_heap::{RequestHeap, RequestHeapLimits};
use skiff_runtime_request::execution_budget::ExecutionBudget;
use skiff_runtime_transport::cancel_reason::request_cancel_wire_reason_for_internal;
use time::{format_description::well_known::Rfc3339, Duration as TimeDuration, OffsetDateTime};
use tokio::sync::mpsc;

use crate::error::{Result, RuntimeError};

#[derive(Clone)]
pub struct OutboundServiceContext {
    caller_request_id: String,
    caller_target: String,
    client_session: Option<RuntimeClientSessionControl>,
    caller_deadline: OutboundCallerDeadline,
    service_timeout: ServiceTimeoutConfig,
    trace: OutboundTraceMetadata,
    service_dependencies: Vec<ServiceDependencyConstraint>,
    test_effects_enabled: bool,
    test_effect_doubles: HashMap<String, Vec<RequestEffectDoubleControl>>,
    execution_budget: Arc<ExecutionBudget>,
    cancel_signal: CancellationToken,
    request_heap_limits: RequestHeapLimits,
    router_sender: Option<mpsc::UnboundedSender<RouterWriterMessage>>,
    outbound_requests: Arc<OutboundRequestRegistry>,
}

pub type ServiceDispatchContext = OutboundServiceContext;

pub struct OutboundServiceContextInput {
    pub caller_request_id: String,
    pub caller_target: String,
    pub client_session: Option<RuntimeClientSessionControl>,
    pub caller_deadline: OutboundCallerDeadline,
    pub service_timeout: ServiceTimeoutConfig,
    pub trace: OutboundTraceMetadata,
    pub service_dependencies: Vec<ServiceDependencyConstraint>,
    pub test_effects_enabled: bool,
    pub test_effect_doubles: HashMap<String, Vec<RequestEffectDoubleControl>>,
    pub execution_budget: Arc<ExecutionBudget>,
    pub cancellation: CancellationToken,
    pub request_heap_limits: RequestHeapLimits,
    pub router_sender: Option<mpsc::UnboundedSender<RouterWriterMessage>>,
    pub outbound_requests: Arc<OutboundRequestRegistry>,
}

impl OutboundServiceContext {
    pub fn new(input: OutboundServiceContextInput) -> Self {
        Self {
            caller_request_id: input.caller_request_id,
            caller_target: input.caller_target,
            client_session: input.client_session,
            caller_deadline: input.caller_deadline,
            service_timeout: input.service_timeout,
            trace: input.trace,
            service_dependencies: input.service_dependencies,
            test_effects_enabled: input.test_effects_enabled,
            test_effect_doubles: input.test_effect_doubles,
            execution_budget: input.execution_budget,
            cancel_signal: input.cancellation,
            request_heap_limits: input.request_heap_limits,
            router_sender: input.router_sender,
            outbound_requests: input.outbound_requests,
        }
    }

    pub fn service_dependencies(&self) -> &[ServiceDependencyConstraint] {
        self.service_dependencies.as_slice()
    }

    pub fn outbound_requests(&self) -> &OutboundRequestRegistry {
        self.outbound_requests.as_ref()
    }

    pub fn test_effects_enabled(&self) -> bool {
        self.test_effects_enabled
    }

    pub fn test_effect_doubles(&self) -> HashMap<String, Vec<RequestEffectDoubleControl>> {
        self.test_effect_doubles.clone()
    }

    pub fn request_heap(&self) -> RequestHeap {
        RequestHeap::new(self.request_heap_limits.clone())
    }

    pub fn cancel_signal(&self) -> CancellationToken {
        self.cancel_signal.clone()
    }

    pub fn effective_timeout_ms(&self, operation_timeout_ms: Option<u64>) -> Option<u64> {
        let configured_timeout_ms = operation_timeout_ms.or(self.service_timeout.default_ms);
        configured_timeout_ms.map_or_else(
            || self.request_deadline_ms(),
            |configured_timeout_ms| {
                Some(
                    self.request_deadline_ms()
                        .map_or(configured_timeout_ms, |deadline_ms| {
                            deadline_ms.min(configured_timeout_ms)
                        }),
                )
            },
        )
    }

    pub fn outbound_deadline_error(&self) -> RuntimeError {
        match self.poll_execution_budget() {
            Err(error) => error,
            Ok(()) => {
                let stats = self.execution_budget.stats_snapshot();
                RuntimeError::execution_budget_exceeded(ExecutionBudgetFailure {
                    reason: ExecutionBudgetReason::DeadlineExceeded,
                    instruction_count: stats.instruction_count,
                    limit: stats.budget_limit,
                    elapsed_ms: stats.elapsed_ms,
                })
            }
        }
    }

    pub fn start_request(
        &self,
        start: OutboundServiceRequestStart,
        payload: Vec<u8>,
    ) -> Result<OutboundStartedRequest> {
        let request_id = self.next_request_id();
        let drop_cancel_reason = if start.mode == "serverStream" {
            "stream_cancelled"
        } else {
            "caller_cancel"
        };
        let request = self.request_start_control(start, request_id.clone());
        let command = OutboundControlMessage::RequestStart { request, payload };
        let (response_rx, lease) =
            self.open_outbound_response_lease(&request_id, drop_cancel_reason)?;
        if let Err(error) = self.send_outbound_request(&request_id, command) {
            lease.cancel("runtime_disconnect");
            return Err(error);
        }
        Ok(OutboundStartedRequest {
            request_id,
            response_rx,
            lease,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn request_start_control_for_test(
        &self,
        start: OutboundServiceRequestStart,
        request_id: String,
    ) -> RequestStartControl {
        self.request_start_control(start, request_id)
    }

    pub async fn receive_response(
        &self,
        lease: &OutboundRequestLease,
        target: &str,
        receiver: &mut OutboundResponseReceiver,
        timeout_ms: Option<u64>,
    ) -> Result<OutboundResponse> {
        tokio::select! {
            result = receiver.recv() => {
                result.ok_or_else(|| RuntimeError::ProviderUnavailable {
                    target: target.to_string(),
                    reason: "outbound response channel closed".to_string(),
                })
            }
            _ = self.cancel_signal.wait_cancelled() => {
                lease.cancel("caller_cancel");
                Err(RuntimeError::cancelled())
            }
            _ = wait_outbound_deadline(timeout_ms), if timeout_ms.is_some() => {
                lease.cancel("deadline_exceeded");
                Err(self.outbound_deadline_error())
            }
        }
    }

    fn next_request_id(&self) -> String {
        format!(
            "{}:service:{}",
            self.caller_request_id,
            uuid::Uuid::new_v4()
        )
    }

    fn open_outbound_response_lease(
        &self,
        request_id: &str,
        drop_cancel_reason: &'static str,
    ) -> Result<(OutboundResponseReceiver, OutboundRequestLease)> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let lease = self.outbound_requests.insert_with_lease(
            request_id.to_string(),
            sender,
            self.outbound_cancel_sender(),
            drop_cancel_reason,
        )?;
        Ok((receiver, lease))
    }

    fn send_outbound_request(
        &self,
        request_id: &str,
        command: OutboundControlMessage,
    ) -> Result<()> {
        let sender =
            self.router_sender
                .as_ref()
                .ok_or_else(|| RuntimeError::ProviderUnavailable {
                    target: request_id.to_string(),
                    reason: "router writer is not available".to_string(),
                })?;
        sender
            .send(RouterWriterMessage::Control(command))
            .map_err(|_| RuntimeError::ProviderUnavailable {
                target: request_id.to_string(),
                reason: "router writer channel closed".to_string(),
            })
    }

    fn outbound_cancel_sender(&self) -> Option<OutboundRequestCancelSender> {
        let sender = self.router_sender.clone()?;
        Some(Arc::new(move |request_id, reason| {
            sender
                .send(cancel_message(request_id, reason))
                .map_err(|_| OutboundRequestCancelSendError::Closed)
        }))
    }

    fn request_deadline_ms(&self) -> Option<u64> {
        self.caller_deadline.remaining_timeout_ms()
    }

    fn deadline_control(
        &self,
        operation_timeout_ms: Option<u64>,
    ) -> Option<RuntimeDeadlineControl> {
        let timeout_ms = self.effective_timeout_ms(operation_timeout_ms)?;
        Some(RuntimeDeadlineControl {
            timeout_ms,
            expires_at: deadline_expires_at(timeout_ms),
        })
    }

    fn trace_control(&self) -> RuntimeTraceContextControl {
        self.trace.to_control()
    }

    fn poll_execution_budget(&self) -> Result<()> {
        match self
            .execution_budget
            .poll(self.cancel_signal.is_cancelled(), Instant::now())
        {
            Ok(()) => Ok(()),
            Err(ExecutionBudgetReason::Cancelled) => {
                Err(RuntimeError::from(ExecutionControlError::Cancelled))
            }
            Err(reason) => {
                let stats = self.execution_budget.stats_snapshot();
                Err(RuntimeError::execution_budget_exceeded(
                    ExecutionBudgetFailure {
                        reason,
                        instruction_count: stats.instruction_count,
                        limit: stats.budget_limit,
                        elapsed_ms: stats.elapsed_ms,
                    },
                ))
            }
        }
    }

    fn request_start_control(
        &self,
        start: OutboundServiceRequestStart,
        request_id: String,
    ) -> RequestStartControl {
        RequestStartControl {
            request_id,
            mode: start.mode,
            caller: RuntimeCallerControl {
                kind: "service".to_string(),
                target: self.caller_target.clone(),
            },
            target: start.target,
            operation_abi_id: Some(start.operation_abi_id),
            selector: Some(start.selector),
            service_id: Some(start.service_id),
            version: Some(start.version),
            build_id: start.build_id,
            service_protocol_identity: start.service_protocol_identity,
            activation_identity: start.activation_identity,
            gateway_entry_identity: None,
            business_identity: None,
            websocket_entry_id: None,
            client_session: self.client_session.clone(),
            deadline: self.deadline_control(start.timeout_ms),
            trace: self.trace_control(),
            test_effects_enabled: self.test_effects_enabled,
            test_effect_doubles: start.test_effect_doubles,
        }
    }
}

#[derive(Clone)]
pub struct OutboundCallerDeadline {
    pub timeout_ms: Option<u64>,
    pub expires_at: Option<String>,
}

impl OutboundCallerDeadline {
    pub fn from_extra(extra: &serde_json::Map<String, Value>) -> Self {
        let deadline = extra.get("deadline").and_then(Value::as_object);
        Self {
            timeout_ms: deadline
                .and_then(|deadline| deadline.get("timeoutMs"))
                .and_then(Value::as_u64),
            expires_at: deadline
                .and_then(|deadline| deadline.get("expiresAt"))
                .and_then(Value::as_str)
                .map(str::to_string),
        }
    }

    fn remaining_timeout_ms(&self) -> Option<u64> {
        let timeout_ms = self.timeout_ms;
        let Some(expires_at) = self.expires_at.as_deref() else {
            return timeout_ms;
        };
        let Ok(expires_at) = OffsetDateTime::parse(expires_at, &Rfc3339) else {
            return timeout_ms;
        };
        let now = OffsetDateTime::now_utc();
        if expires_at <= now {
            return Some(0);
        }
        let remaining_ms = (expires_at - now).whole_milliseconds();
        let remaining_ms = remaining_ms.try_into().unwrap_or(u64::MAX);
        Some(timeout_ms.map_or(remaining_ms, |timeout_ms| timeout_ms.min(remaining_ms)))
    }
}

#[derive(Clone)]
pub struct OutboundTraceMetadata {
    pub trace_id: String,
    pub parent_span_id: Option<String>,
    pub sampled: Option<bool>,
}

impl OutboundTraceMetadata {
    pub fn from_extra(caller_request_id: &str, trace: Option<&Value>) -> Self {
        let trace = trace.and_then(Value::as_object);
        Self {
            trace_id: trace
                .and_then(|trace| trace.get("traceId"))
                .and_then(Value::as_str)
                .unwrap_or(caller_request_id)
                .to_string(),
            parent_span_id: trace
                .and_then(|trace| trace.get("spanId"))
                .and_then(Value::as_str)
                .map(str::to_string),
            sampled: trace
                .and_then(|trace| trace.get("sampled"))
                .and_then(Value::as_bool),
        }
    }

    fn to_control(&self) -> RuntimeTraceContextControl {
        RuntimeTraceContextControl {
            trace_id: self.trace_id.clone(),
            span_id: format!("service-{}", uuid::Uuid::new_v4().simple()),
            parent_span_id: self.parent_span_id.clone(),
            sampled: self.sampled,
        }
    }
}

async fn wait_outbound_deadline(timeout_ms: Option<u64>) {
    let Some(timeout_ms) = timeout_ms else {
        std::future::pending::<()>().await;
        return;
    };
    tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)).await;
}

fn deadline_expires_at(timeout_ms: u64) -> String {
    let millis = timeout_ms.min(i64::MAX as u64) as i64;
    (OffsetDateTime::now_utc() + TimeDuration::milliseconds(millis))
        .format(&Rfc3339)
        .unwrap_or_default()
}

fn cancel_message(request_id: &str, reason: &str) -> RouterWriterMessage {
    let request = RequestCancelControl {
        request_id: request_id.to_string(),
        reason: request_cancel_wire_reason_for_internal(reason).to_string(),
    };
    RouterWriterMessage::Control(OutboundControlMessage::RequestCancel { request })
}

#[cfg(test)]
mod tests {
    use super::*;
    use skiff_runtime_request::execution_budget::ExecutionBudget;

    fn cancel_reason(reason: &str) -> String {
        match cancel_message("request-test", reason) {
            RouterWriterMessage::Control(OutboundControlMessage::RequestCancel { request }) => {
                request.reason
            }
            other => panic!("expected request.cancel control command, got {other:?}"),
        }
    }

    fn test_context(
        router_sender: Option<mpsc::UnboundedSender<RouterWriterMessage>>,
    ) -> OutboundServiceContext {
        test_context_with_cancellation(router_sender, CancellationToken::new())
    }

    fn test_context_with_cancellation(
        router_sender: Option<mpsc::UnboundedSender<RouterWriterMessage>>,
        cancellation: CancellationToken,
    ) -> OutboundServiceContext {
        OutboundServiceContext {
            caller_request_id: "request-parent".to_string(),
            caller_target: "caller.target".to_string(),
            client_session: None,
            caller_deadline: OutboundCallerDeadline {
                timeout_ms: None,
                expires_at: None,
            },
            service_timeout: ServiceTimeoutConfig::default(),
            trace: OutboundTraceMetadata {
                trace_id: "trace-parent".to_string(),
                parent_span_id: None,
                sampled: None,
            },
            service_dependencies: Vec::new(),
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            execution_budget: Arc::new(ExecutionBudget::disabled()),
            cancel_signal: cancellation,
            request_heap_limits: RequestHeapLimits::default(),
            router_sender,
            outbound_requests: Arc::new(OutboundRequestRegistry::default()),
        }
    }

    fn unary_request_start(timeout_ms: Option<u64>) -> OutboundServiceRequestStart {
        OutboundServiceRequestStart {
            service_id: "skiff.run/account".to_string(),
            version: "0.1.0".to_string(),
            build_id: "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            service_protocol_identity: "skiff-protocol-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            operation_abi_id: "operation:account:lookup".to_string(),
            selector: "operation:operation:account:lookup".to_string(),
            target: "account.lookup".to_string(),
            mode: "unary".to_string(),
            timeout_ms,
            activation_identity: None,
            test_effect_doubles: HashMap::new(),
        }
    }

    async fn expect_request_start(
        router_rx: &mut mpsc::UnboundedReceiver<RouterWriterMessage>,
    ) -> String {
        match tokio::time::timeout(std::time::Duration::from_secs(1), router_rx.recv())
            .await
            .expect("router should receive request.start")
            .expect("router channel should stay open")
        {
            RouterWriterMessage::Control(OutboundControlMessage::RequestStart {
                request,
                payload,
            }) => {
                assert_eq!(payload, b"payload".to_vec());
                request.request_id
            }
            other => panic!("expected request.start control command, got {other:?}"),
        }
    }

    fn assert_cancel_message(
        message: RouterWriterMessage,
        expected_request_id: &str,
        expected_reason: &str,
    ) {
        match message {
            RouterWriterMessage::Control(OutboundControlMessage::RequestCancel { request }) => {
                assert_eq!(request.request_id, expected_request_id);
                assert_eq!(request.reason, expected_reason);
            }
            other => panic!("expected request.cancel control command, got {other:?}"),
        }
    }

    #[test]
    fn cancel_frame_preserves_protocol_reasons() {
        assert_eq!(cancel_reason("caller_cancel"), "caller_cancel");
        assert_eq!(cancel_reason("timeout"), "timeout");
    }

    #[test]
    fn cancel_control_maps_internal_reasons_to_router_reasons() {
        assert_eq!(cancel_reason("deadline_exceeded"), "deadline_exceeded");
        assert_eq!(
            cancel_reason("unexpected_stream_response"),
            "protocol_error"
        );
        assert_eq!(cancel_reason("stream_cancelled"), "stream_dropped");
    }

    #[tokio::test]
    async fn outbound_request_lease_complete_drop_does_not_cancel() {
        let (router_sender, mut router_rx) = mpsc::unbounded_channel();
        let context = test_context(Some(router_sender));
        let (_response_rx, lease) = context
            .open_outbound_response_lease("request-stream", "stream_cancelled")
            .expect("lease should open");

        assert_eq!(context.outbound_requests.pending_count(), 1);
        assert_eq!(context.outbound_requests.active_lease_count(), 1);

        lease.complete();
        drop(lease);

        assert_eq!(context.outbound_requests.pending_count(), 0);
        assert_eq!(context.outbound_requests.active_lease_count(), 0);
        assert!(router_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn outbound_request_lease_drop_sends_stream_dropped_cancel() {
        let (router_sender, mut router_rx) = mpsc::unbounded_channel();
        let context = test_context(Some(router_sender));
        let request_id = "request-stream".to_string();
        let (_response_rx, lease) = context
            .open_outbound_response_lease(&request_id, "stream_cancelled")
            .expect("lease should open");

        drop(lease);

        let message = tokio::time::timeout(std::time::Duration::from_secs(1), router_rx.recv())
            .await
            .expect("stream drop should emit request cancel")
            .expect("router writer channel should stay open");
        assert_cancel_message(message, &request_id, "stream_dropped");

        assert_eq!(context.outbound_requests.pending_count(), 0);
        assert_eq!(context.outbound_requests.active_lease_count(), 0);
    }

    #[test]
    fn outbound_request_lease_sender_closed_records_counter() {
        let (router_sender, router_rx) = mpsc::unbounded_channel();
        let context = test_context(Some(router_sender));
        let (_response_rx, lease) = context
            .open_outbound_response_lease("request-stream", "stream_cancelled")
            .expect("lease should open");

        drop(router_rx);
        drop(lease);

        assert_eq!(context.outbound_requests.pending_count(), 0);
        assert_eq!(context.outbound_requests.active_lease_count(), 0);
        assert_eq!(
            context.outbound_requests.cancel_send_failed_closed_count(),
            1
        );
    }

    #[tokio::test]
    async fn outbound_request_lease_repeated_terminal_sends_cancel_once() {
        let (router_sender, mut router_rx) = mpsc::unbounded_channel();
        let context = test_context(Some(router_sender));
        let (_response_rx, lease) = context
            .open_outbound_response_lease("request-stream", "stream_cancelled")
            .expect("lease should open");

        lease.cancel("stream_cancelled");
        lease.complete();
        drop(lease);

        let message = tokio::time::timeout(std::time::Duration::from_secs(1), router_rx.recv())
            .await
            .expect("first terminal should emit request cancel")
            .expect("router writer channel should stay open");
        assert_cancel_message(message, "request-stream", "stream_dropped");
        assert!(router_rx.try_recv().is_err());
        assert_eq!(context.outbound_requests.pending_count(), 0);
        assert_eq!(context.outbound_requests.active_lease_count(), 0);
    }

    #[tokio::test]
    async fn unary_outbound_normal_response_cleans_registry_and_lease() {
        let (router_sender, mut router_rx) = mpsc::unbounded_channel();
        let context = test_context(Some(router_sender));
        let started = context
            .start_request(unary_request_start(None), b"payload".to_vec())
            .expect("unary outbound request should start");
        let request_id = expect_request_start(&mut router_rx).await;
        assert_eq!(request_id, started.request_id);
        assert_eq!(context.outbound_requests.pending_count(), 1);
        assert_eq!(context.outbound_requests.active_lease_count(), 1);

        context
            .outbound_requests
            .sender(&request_id)
            .expect("pending sender should exist")
            .send(OutboundResponse::End {
                payload: b"encoded-result".to_vec(),
            })
            .expect("response should deliver to runtime receiver");
        let OutboundStartedRequest {
            mut response_rx,
            lease,
            ..
        } = started;
        let response = context
            .receive_response(&lease, "account.lookup", &mut response_rx, None)
            .await
            .expect("response should be received");
        assert!(matches!(
            response,
            OutboundResponse::End { payload } if payload == b"encoded-result"
        ));

        lease.complete();
        drop(lease);
        assert_eq!(context.outbound_requests.pending_count(), 0);
        assert_eq!(context.outbound_requests.active_lease_count(), 0);
        assert!(router_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn unary_outbound_provider_error_cleans_registry_and_lease() {
        let (router_sender, mut router_rx) = mpsc::unbounded_channel();
        let context = test_context(Some(router_sender));
        let started = context
            .start_request(unary_request_start(None), b"payload".to_vec())
            .expect("unary outbound request should start");
        let request_id = expect_request_start(&mut router_rx).await;

        context
            .outbound_requests
            .sender(&request_id)
            .expect("pending sender should exist")
            .send(OutboundResponse::Error(
                skiff_runtime_capability_context::ResponseError {
                    code: "ProviderError".to_string(),
                    message: "callee failed".to_string(),
                    status: Some(503),
                    details: None,
                },
            ))
            .expect("response should deliver to runtime receiver");
        let OutboundStartedRequest {
            mut response_rx,
            lease,
            ..
        } = started;
        let response = context
            .receive_response(&lease, "account.lookup", &mut response_rx, None)
            .await
            .expect("provider error frame should be received");
        assert!(matches!(
            response,
            OutboundResponse::Error(error)
                if error.message == "callee failed" && error.status == Some(503)
        ));

        lease.complete();
        drop(lease);
        assert_eq!(context.outbound_requests.pending_count(), 0);
        assert_eq!(context.outbound_requests.active_lease_count(), 0);
        assert!(router_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn unary_outbound_caller_cancel_cleans_registry_and_lease() {
        let (router_sender, mut router_rx) = mpsc::unbounded_channel();
        let cancellation = CancellationToken::source();
        let context = test_context_with_cancellation(Some(router_sender), cancellation.token());
        let started = context
            .start_request(unary_request_start(None), b"payload".to_vec())
            .expect("unary outbound request should start");
        let request_id = expect_request_start(&mut router_rx).await;
        let OutboundStartedRequest {
            mut response_rx,
            lease,
            ..
        } = started;

        cancellation.cancel();
        let result = context
            .receive_response(&lease, "account.lookup", &mut response_rx, None)
            .await;
        assert!(result.is_err());

        let message = tokio::time::timeout(std::time::Duration::from_secs(1), router_rx.recv())
            .await
            .expect("caller cancel should emit request cancel")
            .expect("router writer channel should stay open");
        assert_cancel_message(message, &request_id, "caller_cancel");
        drop(lease);
        assert_eq!(context.outbound_requests.pending_count(), 0);
        assert_eq!(context.outbound_requests.active_lease_count(), 0);
    }

    #[tokio::test]
    async fn unary_outbound_timeout_cleans_registry_and_lease() {
        let (router_sender, mut router_rx) = mpsc::unbounded_channel();
        let context = test_context(Some(router_sender));
        let started = context
            .start_request(unary_request_start(Some(1)), b"payload".to_vec())
            .expect("unary outbound request should start");
        let request_id = expect_request_start(&mut router_rx).await;
        let OutboundStartedRequest {
            mut response_rx,
            lease,
            ..
        } = started;

        let result = context
            .receive_response(&lease, "account.lookup", &mut response_rx, Some(1))
            .await;
        assert!(result.is_err());

        let message = tokio::time::timeout(std::time::Duration::from_secs(1), router_rx.recv())
            .await
            .expect("deadline should emit request cancel")
            .expect("router writer channel should stay open");
        assert_cancel_message(message, &request_id, "deadline_exceeded");
        drop(lease);
        assert_eq!(context.outbound_requests.pending_count(), 0);
        assert_eq!(context.outbound_requests.active_lease_count(), 0);
    }

    #[test]
    fn unary_outbound_router_disconnect_cleans_registry_and_lease() {
        let (router_sender, router_rx) = mpsc::unbounded_channel();
        drop(router_rx);
        let context = test_context(Some(router_sender));

        let result = context.start_request(unary_request_start(None), b"payload".to_vec());
        assert!(result.is_err());
        assert_eq!(context.outbound_requests.pending_count(), 0);
        assert_eq!(context.outbound_requests.active_lease_count(), 0);
        assert_eq!(
            context.outbound_requests.cancel_send_failed_closed_count(),
            1
        );
    }

    #[test]
    fn outbound_request_start_control_includes_operation_abi_id_and_selector() {
        let context = OutboundServiceContext {
            caller_request_id: "request-parent".to_string(),
            caller_target: "caller.target".to_string(),
            client_session: None,
            caller_deadline: OutboundCallerDeadline {
                timeout_ms: None,
                expires_at: None,
            },
            service_timeout: ServiceTimeoutConfig::default(),
            trace: OutboundTraceMetadata {
                trace_id: "trace-parent".to_string(),
                parent_span_id: Some("span-parent".to_string()),
                sampled: Some(true),
            },
            service_dependencies: Vec::new(),
            test_effects_enabled: false,
            test_effect_doubles: HashMap::new(),
            execution_budget: Arc::new(ExecutionBudget::disabled()),
            cancel_signal: CancellationToken::new(),
            request_heap_limits: RequestHeapLimits::default(),
            router_sender: None,
            outbound_requests: Arc::new(OutboundRequestRegistry::default()),
        };

        let request = context.request_start_control(
            OutboundServiceRequestStart {
                service_id: "skiff.run/account".to_string(),
                version: "0.1.0".to_string(),
                build_id: "skiff-service-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
                service_protocol_identity: "skiff-protocol-v1:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
                operation_abi_id: "operation:account:lookup".to_string(),
                selector: "operation:operation:account:lookup".to_string(),
                target: "legacy.display.target".to_string(),
                mode: "unary".to_string(),
                timeout_ms: None,
                activation_identity: None,
                test_effect_doubles: HashMap::new(),
            },
            "request-child".to_string(),
        );

        assert_eq!(
            request.operation_abi_id.as_deref(),
            Some("operation:account:lookup")
        );
        assert_eq!(
            request.selector.as_deref(),
            Some("operation:operation:account:lookup")
        );
        // Version is the addressing coordinate the router resolves; it must be
        // emitted on the outbound request.
        assert_eq!(request.version.as_deref(), Some("0.1.0"));
    }
}
