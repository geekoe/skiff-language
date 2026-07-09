mod actor;
mod cancellation;
mod capability_error;
mod config;
mod db;
mod execution_control;
mod file;
mod http;
mod native_projection;
mod outbound_control;
mod outbound_request;
mod outbound_response;
mod request_payload;
mod response;
mod stream;
mod telemetry;
mod time;
mod websocket;

pub use actor::{
    ActorCapabilityApi, ActorCapabilityContext, ActorClient, OwnedActorCapabilityContext,
};
pub use cancellation::{
    CancellationSignals, CancellationToken, CompletionSignal, RequestAbortSignal,
};
pub use capability_error::{CapabilityError, CapabilityFuture, CapabilityResult};
pub use config::{ConfigCapabilityApi, ConfigCapabilityContext, OwnedConfigCapabilityContext};
pub use db::{
    DbCapabilityContext, DbCapabilityContextApi, DbCapabilityError, DbCapabilityFactory,
    DbCapabilityFuture, DbCapabilityLeaseHandle, DbCapabilityLeaseHold,
    DbCapabilityLeaseHoldHandle, DbCapabilityResult, DbCapabilitySource, DbCapabilityStore,
    DbCapabilityStoreApi, DbDocument, DbKey, DbOneSelector, DbOrderDirection, DbOrderEntry,
    DbPageResult, DbProviderBuildInput, DbProviderConfig, DbProviderFactory, DbProviderSource,
    DbQuery, DbRecoverableRuntimeContext, DbRecoverableRuntimeExpectedPlans, DbRuntimeChange,
    DbRuntimeSetOp, DbWriteResult, FieldPath, FileCapabilityRecord, ServiceDbChange,
    ServiceDbChangeOp, ServiceDbFindOptions,
};
pub use execution_control::{
    ExecutionBudgetFailure, ExecutionBudgetReason, ExecutionControl, ExecutionControlApi,
    ExecutionControlError, ExecutionControlResult, OwnedExecutionControl, OwnedExecutionControlApi,
};
pub use file::{
    FileCapabilityApi, FileCapabilityContext, FileCapabilityError, FileCapabilityFuture,
    FileCapabilityResult, FileCapabilitySource, FileCapabilitySourceApi, FileChunkFuture,
    FileChunkSource, FileSourceStreamApi, FileSourceStreamContext,
};
pub use http::{
    HttpCapabilityFuture, HttpClientCapabilityApi, HttpClientCapabilityContext, HttpRuntimeOptions,
    HTTP_REQUEST_ADMIN_OVERRIDE_ENV,
};
pub use native_projection::{
    project_native_capability_context, NativeCapabilityContexts, NativeCapabilityProjectionSource,
    NativeFileCapabilityContext, NativeHttpClientCapabilityContext,
    NativeHttpResponseStreamCapabilityContext, NativeTelemetryCapabilityContext,
};
pub use outbound_control::{
    ActorFindControlRequest, ActorKeyControlMetadata, ActorPutControlRequest,
    ActorRemoveControlRequest, ConnectionSendControl, OutboundControlMessage, RequestCancelControl,
    RequestEffectDoubleControl, RequestStartControl, RouterWriterMessage, RuntimeCallerControl,
    RuntimeClientSessionControl, RuntimeDeadlineControl, RuntimeTraceContextControl,
    SpawnSubmitControlRequest, WebSocketConnectionPolicyControl,
    WebSocketConnectionPolicyOverflowControl,
};
pub use outbound_request::{OutboundServiceRequestStart, OutboundStartedRequest};
pub use outbound_response::{
    OutboundRequestRegistry, OutboundRequestRegistryError, OutboundResponse,
    OutboundResponseReceiver, OutboundResponseSender,
};
pub use request_payload::{
    binary_http_request_parts, http_name_value_context, http_name_value_contexts,
    BinaryHttpRequestContext, HttpNameValueContext, InvocationContext, RequestPayloadContext,
    RequestPayloadContextError, RequestPayloadEncoding,
};
pub use response::{HttpNameValue, HttpResponseMetadata, ResponseError};
pub use stream::{
    HttpResponseStreamCapabilityContext, StreamCancelSignal, StreamCancelSignalApi,
    StreamCapabilityContext, StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi,
    StreamRuntimeError, StreamRuntimeResult, StreamSink, StreamSinkApi, TypedStreamSink,
};
pub use telemetry::{TelemetryCapabilityApi, TelemetryCapabilityContext};
pub use time::TimeCapabilityContext;
pub use websocket::{
    OwnedWebsocketCapabilityContext, WebsocketCapabilityApi, WebsocketCapabilityContext,
};

#[cfg(test)]
mod tests {
    use std::fmt;

    use serde_json::json;
    use skiff_runtime_model::error::{RuntimeErrorPayload, TypeIdentity, WirePayload};

    use super::*;

    #[derive(Debug)]
    struct TestWirePayload;

    impl fmt::Display for TestWirePayload {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test producer error")
        }
    }

    impl std::error::Error for TestWirePayload {}

    impl WirePayload for TestWirePayload {
        fn payload(&self) -> RuntimeErrorPayload {
            RuntimeErrorPayload {
                code: "test.ProducerError".to_string(),
                message: "producer failed".to_string(),
                status: Some(599),
                details: Some(json!({
                    "producer": true,
                })),
            }
        }

        fn catch_projection(&self) -> Option<(TypeIdentity, serde_json::Value)> {
            Some((
                TypeIdentity::builtin("test.ProducerCatchError"),
                json!({
                    "caught": true,
                }),
            ))
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn file_capability_error_payload_and_catch_projection_match_public_contract() {
        let file = FileCapabilityError::file("std.file not found: test");
        let payload = file.payload();
        assert_eq!(payload.code, "std.file.FileError");
        assert_eq!(payload.message, "std.file not found: test");
        assert_eq!(payload.details, None);
        assert_eq!(
            file.catch_projection(),
            Some((
                TypeIdentity::builtin("std.file.FileError"),
                json!({
                    "message": "std.file not found: test",
                }),
            ))
        );

        let provider =
            FileCapabilityError::provider_unavailable("svc.account", "no active runtime");
        let payload = provider.payload();
        assert_eq!(payload.code, "std.service.ProviderUnavailableError");
        assert_eq!(payload.message, "no active runtime");
        assert_eq!(
            payload.details,
            Some(json!({
                "target": "svc.account",
                "reason": "no active runtime",
            }))
        );
        assert_eq!(
            provider.catch_projection(),
            Some((
                TypeIdentity::builtin("std.service.ProviderUnavailableError"),
                json!({
                    "target": "svc.account",
                    "reason": "no active runtime",
                }),
            ))
        );

        let decode = FileCapabilityError::decode("bad file payload");
        assert_eq!(decode.payload().code, "InternalError");
        assert_eq!(decode.catch_projection(), None);

        let resource =
            FileCapabilityError::resource_limit_exceeded("response.body", "too large", 10, 8, 4);
        let payload = resource.payload();
        assert_eq!(payload.code, "ResourceLimitExceeded");
        assert_eq!(
            payload.details,
            Some(json!({
                "resource": "response.body",
                "reason": "too large",
                "limit": 10,
                "current": 8,
                "requestedDelta": 4,
            }))
        );
        assert_eq!(resource.catch_projection(), None);
    }

    #[test]
    fn capability_error_payload_and_catch_projection_match_public_contract() {
        let provider = CapabilityError::provider_unavailable("svc.account", "no active runtime");
        let payload = provider.payload();
        assert_eq!(payload.code, "std.service.ProviderUnavailableError");
        assert_eq!(payload.message, "no active runtime");
        assert_eq!(
            payload.details,
            Some(json!({
                "target": "svc.account",
                "reason": "no active runtime",
            }))
        );
        assert_eq!(
            provider.catch_projection(),
            Some((
                TypeIdentity::builtin("std.service.ProviderUnavailableError"),
                json!({
                    "target": "svc.account",
                    "reason": "no active runtime",
                }),
            ))
        );

        let protocol = CapabilityError::protocol("std.websocket.sendTextToConnection", "closed");
        assert_eq!(protocol.payload().code, "std.service.ProtocolError");
        assert_eq!(
            protocol.catch_projection(),
            Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
                json!({
                    "target": "std.websocket.sendTextToConnection",
                    "message": "closed",
                }),
            ))
        );

        let opaque = CapabilityError::opaque(TestWirePayload);
        assert_eq!(opaque.payload().code, "test.ProducerError");
        assert_eq!(
            opaque.catch_projection(),
            Some((
                TypeIdentity::builtin("test.ProducerCatchError"),
                json!({
                    "caught": true,
                }),
            ))
        );
    }

    #[test]
    fn db_capability_source_unavailable_requires_store_with_provider_unavailable() {
        let source = DbCapabilitySource::unavailable();
        let context = source.context_for_request("svc.account", "req-1");

        let error = match context.require_store(
            "std.db.findOne",
            "serviceDb is not configured for this service activation",
        ) {
            Ok(_) => panic!("unavailable DB source should not create a store"),
            Err(error) => error,
        };

        match error {
            DbCapabilityError::ProviderUnavailable { target, reason } => {
                assert_eq!(target, "std.db.findOne");
                assert_eq!(
                    reason,
                    "serviceDb is not configured for this service activation"
                );
            }
            other => panic!("expected ProviderUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn execution_control_error_payload_and_catch_projection_match_public_contract() {
        let cancelled = ExecutionControlError::Cancelled;
        assert_eq!(cancelled.payload().code, "CancelError");
        assert_eq!(
            cancelled.catch_projection(),
            Some((
                TypeIdentity::builtin("CancelError"),
                json!({
                    "message": "request was cancelled",
                }),
            ))
        );

        let cancelled_budget = ExecutionControlError::BudgetExceeded(ExecutionBudgetFailure {
            reason: ExecutionBudgetReason::Cancelled,
            instruction_count: 9,
            limit: Some(10),
            elapsed_ms: 1.5,
        });
        assert_eq!(cancelled_budget.payload().code, "CancelError");
        assert_eq!(
            cancelled_budget.catch_projection().unwrap().0,
            TypeIdentity::builtin("CancelError")
        );

        let timeout = ExecutionControlError::BudgetExceeded(ExecutionBudgetFailure {
            reason: ExecutionBudgetReason::DeadlineExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        });
        let payload = timeout.payload();
        assert_eq!(payload.code, "TimeoutError");
        assert_eq!(payload.message, "execution deadline exceeded");
        assert_eq!(
            payload.details,
            Some(json!({
                "reason": "deadlineExceeded",
                "instructionCount": 42,
                "limit": 100,
                "elapsedMs": 12.5,
            }))
        );
        assert_eq!(
            timeout.catch_projection(),
            Some((
                TypeIdentity::builtin("TimeoutError"),
                json!({
                    "reason": "deadlineExceeded",
                    "instructionCount": 42,
                    "limit": 100,
                    "elapsedMs": 12.5,
                }),
            ))
        );
    }

    #[test]
    fn stream_runtime_error_payload_and_catch_projection_delegate_producer() {
        let decode = StreamRuntimeError::decode("bad stream frame");
        assert_eq!(decode.payload().code, "InternalError");
        assert_eq!(decode.catch_projection(), None);

        let cancelled = StreamRuntimeError::cancelled();
        assert_eq!(cancelled.payload().code, "CancelError");
        assert_eq!(
            cancelled.catch_projection(),
            Some((
                TypeIdentity::builtin("CancelError"),
                json!({
                    "message": "request was cancelled",
                }),
            ))
        );

        let producer = StreamRuntimeError::producer(TestWirePayload);
        assert_eq!(producer.payload().code, "test.ProducerError");
        assert_eq!(
            producer.catch_projection(),
            Some((
                TypeIdentity::builtin("test.ProducerCatchError"),
                json!({
                    "caught": true,
                }),
            ))
        );
    }

    #[test]
    fn request_payload_context_error_payload_and_catch_projection_are_protocol_error() {
        let error = RequestPayloadContextError::MissingBinaryHttp {
            target: "svc.account".to_string(),
        };
        let payload = error.payload();

        assert_eq!(payload.code, "std.service.ProtocolError");
        assert_eq!(payload.message, "binary HTTP request metadata is missing");
        assert_eq!(
            payload.details,
            Some(json!({
                "target": "svc.account",
                "message": "binary HTTP request metadata is missing",
            }))
        );
        assert_eq!(
            error.catch_projection(),
            Some((
                TypeIdentity::builtin("std.service.ProtocolError"),
                json!({
                    "target": "svc.account",
                    "message": "binary HTTP request metadata is missing",
                }),
            ))
        );
    }

    #[test]
    fn outbound_request_registry_error_payload_is_internal_and_not_catchable() {
        let lock = OutboundRequestRegistryError::LockPoisoned;
        assert_eq!(lock.payload().code, "InternalError");
        assert_eq!(
            lock.payload().message,
            "outbound request registry lock is poisoned"
        );
        assert_eq!(lock.catch_projection(), None);

        let duplicate = OutboundRequestRegistryError::DuplicateRequestId("request-1".to_string());
        assert_eq!(duplicate.payload().code, "InternalError");
        assert_eq!(
            duplicate.payload().message,
            "duplicate outbound request id request-1"
        );
        assert_eq!(duplicate.catch_projection(), None);
    }
}
