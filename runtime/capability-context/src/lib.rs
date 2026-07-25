mod actor;
mod actor_invocation;
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
mod stream_cleanup;
mod telemetry;
mod time;
mod websocket;

pub use actor::{
    ActorCapabilityApi, ActorCapabilityContext, ActorClient, OwnedActorCapabilityContext,
};
pub use actor_invocation::{
    ActorInvocationCancellation, ActorInvocationDeadline, ActorInvocationDeclarationOwner,
    ActorInvocationError, ActorInvocationIdentity, ActorInvocationOutcome,
    ActorInvocationOwnerFile, ActorInvocationOwnerUnit, ActorInvocationRequest,
};
pub use cancellation::{
    flag_backed_cancel_waiters_active, CancellationPollingFallbackAllowlistEntry,
    CancellationSignals, CancellationSource, CancellationToken, CompletionSignal,
    RequestAbortSignal, FLAG_BACKED_CANCELLATION_POLLING_FALLBACK_ALLOWLIST,
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
    ActivationIdentityControl, ActorFindControlRequest, ActorGetOrCreateControlRequest,
    ActorKeyControlMetadata, ActorRemoveControlRequest, ActorReplaceControlRequest,
    ConnectionSendControl, OutboundControlMessage, RequestCancelControl,
    RequestEffectDoubleControl, RequestStartControl, RouterWriterMessage, RuntimeCallerControl,
    RuntimeClientSessionControl, RuntimeDeadlineControl, RuntimeTraceContextControl,
    SpawnClaimControlRequest, SpawnCompleteControlRequest, SpawnFailControlRequest,
    SpawnRenewControlRequest, SpawnSubmitControlRequest, WebSocketConnectionPolicyControl,
    WebSocketConnectionPolicyOverflowControl,
};
pub use outbound_request::{OutboundServiceRequestStart, OutboundStartedRequest};
pub use outbound_response::{
    OutboundRequestCancelSendError, OutboundRequestCancelSender, OutboundRequestLease,
    OutboundRequestRegistry, OutboundRequestRegistryError, OutboundRequestTerminalSignal,
    OutboundResponse, OutboundResponseReceiver, OutboundResponseSender,
};
pub use request_payload::{
    binary_http_request_parts, http_name_value_context, http_name_value_contexts,
    BinaryHttpRequestContext, HttpNameValueContext, InvocationContext, RequestPayloadContext,
    RequestPayloadContextError, RequestPayloadEncoding,
};
pub use response::{HttpNameValue, HttpResponseMetadata, ResponseError};
pub use stream::{
    HttpResponseStreamCapabilityContext, StreamCancelSignal, StreamCancelSignalApi,
    StreamCapabilityContext, StreamInternalItem, StreamLifetimeGuard, StreamLifetimeGuardApi,
    StreamPoll, StreamPullSource, StreamRuntime, StreamRuntimeApi, StreamRuntimeError,
    StreamRuntimeOwner, StreamRuntimeResult, StreamSink, StreamSinkApi, TypedStreamSink,
};
pub use stream_cleanup::{
    StreamConsumerCleanup, StreamConsumerEndMarker, StreamConsumptionStatus,
    StreamConsumptionTerminal, SupervisedStreamConsumptionChild, SupervisedStreamConsumptionLease,
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
    use skiff_artifact_model::{
        InstructionSourceSite, PackageBuildId, SyntheticInstructionSiteReason,
    };
    use skiff_runtime_model::{
        addr::ExecutableAddr,
        error::{RuntimeErrorPayload, WirePayload},
        request_heap::RequestHeap,
        runtime_value::RuntimeValue,
        service_error::{
            CatchIdentity, ExceptionStackFrame, OpaqueServiceError, PlatformBuiltinErrorIdentity,
            ServiceErrorEnvelope,
        },
    };

    use super::*;

    #[derive(Debug)]
    struct TestWirePayload;

    fn test_fixture_catch_projection() -> Option<(CatchIdentity, serde_json::Value)> {
        Some((
            PlatformBuiltinErrorIdentity::Http.catch_identity(),
            json!({
                "caught": true,
            }),
        ))
    }

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

        fn catch_projection(&self) -> Option<(CatchIdentity, serde_json::Value)> {
            test_fixture_catch_projection()
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    fn fixed_service_error(label: &str) -> OpaqueServiceError {
        OpaqueServiceError::decode(
            format!(
                r#"{{"kind":"internalError","payload":{{"message":"Internal service error","traceId":"trace-{label}","errorId":"trace-{label}:error"}}}}"#
            )
            .into_bytes(),
        )
        .expect("fixed service error fixture should decode")
    }

    fn public_service_error(stable_schema_key: &str, trace_id: &str) -> OpaqueServiceError {
        OpaqueServiceError::decode(
            format!(
                r#"{{
  "kind":"publicTypedError",
  "packageId":"example.errors",
  "stableSchemaKey":"{stable_schema_key}",
  "packageSchemaTypeId":"schema:{stable_schema_key}",
  "encodedPayload":[123,125],
  "traceId":"{trace_id}",
  "errorId":"{trace_id}:error"
}}"#
            )
            .into_bytes(),
        )
        .expect("public service error fixture should decode")
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
                PlatformBuiltinErrorIdentity::File.catch_identity(),
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
                PlatformBuiltinErrorIdentity::ServiceProviderUnavailable.catch_identity(),
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
                PlatformBuiltinErrorIdentity::ServiceProviderUnavailable.catch_identity(),
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
                PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
                json!({
                    "target": "std.websocket.sendTextToConnection",
                    "message": "closed",
                }),
            ))
        );

        let opaque = CapabilityError::opaque(TestWirePayload);
        assert_eq!(opaque.payload().code, "test.ProducerError");
        assert_eq!(opaque.catch_projection(), test_fixture_catch_projection());

        assert_eq!(
            CapabilityError::decode("invalid capability payload").catch_projection(),
            None
        );
        assert_eq!(
            CapabilityError::unsupported("unsupported capability").catch_projection(),
            None
        );
    }

    #[test]
    fn db_capability_error_catch_projection_matches_public_contract() {
        let provider =
            DbCapabilityError::provider_unavailable("std.db.findOne", "database unavailable");
        assert_eq!(
            provider.catch_projection(),
            Some((
                PlatformBuiltinErrorIdentity::ServiceProviderUnavailable.catch_identity(),
                json!({
                    "target": "std.db.findOne",
                    "reason": "database unavailable",
                }),
            ))
        );

        assert_eq!(
            DbCapabilityError::opaque(TestWirePayload).catch_projection(),
            test_fixture_catch_projection()
        );
        assert_eq!(
            DbCapabilityError::decode("invalid database payload").catch_projection(),
            None
        );
    }

    #[test]
    fn file_capability_wrappers_forward_inner_catch_projection() {
        assert_eq!(
            FileCapabilityError::opaque(TestWirePayload).catch_projection(),
            test_fixture_catch_projection()
        );

        let stream = StreamRuntimeError::producer(TestWirePayload);
        let stream_projection = stream.catch_projection();
        assert_eq!(
            FileCapabilityError::from(stream).catch_projection(),
            stream_projection
        );

        let execution = ExecutionControlError::Cancelled;
        let execution_projection = execution.catch_projection();
        assert_eq!(
            FileCapabilityError::from(execution).catch_projection(),
            execution_projection
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
                PlatformBuiltinErrorIdentity::Cancel.catch_identity(),
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
            PlatformBuiltinErrorIdentity::Cancel.catch_identity()
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
                PlatformBuiltinErrorIdentity::Timeout.catch_identity(),
                json!({
                    "reason": "deadlineExceeded",
                    "instructionCount": 42,
                    "limit": 100,
                    "elapsedMs": 12.5,
                }),
            ))
        );

        let instruction_limit = ExecutionControlError::BudgetExceeded(ExecutionBudgetFailure {
            reason: ExecutionBudgetReason::InstructionLimitExceeded,
            instruction_count: 101,
            limit: Some(100),
            elapsed_ms: 3.5,
        });
        assert_eq!(instruction_limit.payload().code, "TimeoutError");
        assert_eq!(
            instruction_limit.catch_projection().unwrap().0,
            PlatformBuiltinErrorIdentity::Timeout.catch_identity()
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
                PlatformBuiltinErrorIdentity::Cancel.catch_identity(),
                json!({
                    "message": "request was cancelled",
                }),
            ))
        );

        let producer = StreamRuntimeError::producer(TestWirePayload);
        assert_eq!(producer.payload().code, "test.ProducerError");
        assert_eq!(producer.catch_projection(), test_fixture_catch_projection());
    }

    #[test]
    fn fixed_service_stream_terminal_keeps_exact_bytes_and_typed_import_facts() {
        let fixed = fixed_service_error("stream");
        let exact = fixed.encoded_bytes().to_vec();
        let call_site = InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
        };
        let stack = vec![ExceptionStackFrame::Local {
            site: call_site.clone(),
        }];
        let terminal = StreamRuntimeError::fixed_service_failure_with_import(
            fixed,
            PackageBuildId::new("caller-build"),
            ExecutableAddr::service(2, 3),
            call_site.clone(),
            stack.clone(),
            "svc.provider".to_string(),
            "errors.stream".to_string(),
        );

        let (fixed, import) = terminal
            .fixed_service_failure_parts()
            .expect("typed fixed terminal");
        let (
            caller_build,
            caller_executable,
            terminal_site,
            terminal_stack,
            service_id,
            operation_id,
        ) = import.expect("in-process import provenance");
        assert_eq!(fixed.encoded_bytes(), exact);
        assert_eq!(caller_build.as_str(), "caller-build");
        assert_eq!(caller_executable, &ExecutableAddr::service(2, 3));
        assert_eq!(terminal_site, &call_site);
        assert_eq!(terminal_stack, stack);
        assert_eq!(service_id, "svc.provider");
        assert_eq!(operation_id, "errors.stream");

        assert!(StreamRuntimeError::producer(TestWirePayload)
            .fixed_service_failure_parts()
            .is_none());
        assert!(StreamRuntimeError::cancelled()
            .fixed_service_failure_parts()
            .is_none());
    }

    #[test]
    fn unlinked_fixed_stream_terminal_outlives_provider_heap_without_reencoding() {
        let fixed = public_service_error("OpaqueFault", "trace-unlinked-stream");
        let exact = fixed.encoded_bytes().to_vec();
        let mut provider_heap = RequestHeap::default();
        provider_heap
            .alloc_array(vec![RuntimeValue::String(
                "provider-only payload".to_string(),
            )])
            .expect("provider heap fixture allocation");
        let terminal = StreamRuntimeError::fixed_service_failure(fixed);

        drop(provider_heap);

        let (fixed, import) = terminal
            .fixed_service_failure_parts()
            .expect("fixed terminal should remain typed");
        assert!(import.is_none());
        assert_eq!(fixed.encoded_bytes(), exact);
    }

    #[test]
    fn fixed_stream_carrier_does_not_reclassify_platform_and_resource_errors() {
        let platform = OpaqueServiceError::decode(
            br#"{"kind":"platformError","builtinErrorIdentity":"std.db.ConflictError","encodedPayload":[123,125],"traceId":"trace-platform","errorId":"trace-platform:error"}"#
                .to_vec(),
        )
        .expect("platform service error fixture should decode");
        let resource = public_service_error("std.resource.ResourceError", "trace-resource");

        let platform_terminal = StreamRuntimeError::fixed_service_failure(platform);
        let resource_terminal = StreamRuntimeError::fixed_service_failure(resource);
        let (platform, _) = platform_terminal
            .fixed_service_failure_parts()
            .expect("platform terminal");
        let (resource, _) = resource_terminal
            .fixed_service_failure_parts()
            .expect("resource terminal");

        assert!(matches!(
            platform.envelope(),
            ServiceErrorEnvelope::PlatformError { .. }
        ));
        assert!(matches!(
            resource.envelope(),
            ServiceErrorEnvelope::PublicTypedError {
                stable_schema_key,
                ..
            } if stable_schema_key == "std.resource.ResourceError"
        ));
    }

    #[test]
    fn outbound_fixed_failure_is_distinct_from_generic_response_error() {
        let fixed = fixed_service_error("response");
        let exact = fixed.encoded_bytes().to_vec();
        let response = OutboundResponse::fixed_service_failure(fixed);
        assert_eq!(response.kind(), "response.error");
        match response {
            OutboundResponse::FixedServiceFailure(failure) => {
                assert_eq!(failure.error().encoded_bytes(), exact)
            }
            _ => panic!("fixed response must retain its typed carrier"),
        }

        let generic = OutboundResponse::Error(ResponseError {
            code: "std.service.ProviderUnavailableError".to_string(),
            message: "same text is not classification authority".to_string(),
            status: None,
            details: None,
        });
        assert!(matches!(generic, OutboundResponse::Error(_)));
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
                PlatformBuiltinErrorIdentity::ServiceProtocol.catch_identity(),
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
