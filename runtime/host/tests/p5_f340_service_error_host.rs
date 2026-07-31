use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};
use skiff_artifact_model::{
    InstructionSourceSite, SourcePosition, SourceSpanRef, SyntheticInstructionSiteReason,
};
use skiff_runtime_capability_context::{
    CancellationToken, RestrictedServiceDiagnostic, RestrictedServiceDiagnosticCauseKind,
    RestrictedServiceDiagnosticOwner,
};
use skiff_runtime_eval::error::RuntimeError as EvalRuntimeError;
use skiff_runtime_host::{
    capability_context::{
        EffectDispatchContext, HttpEffectContext, HttpRuntimeOptions, TelemetryCapabilityContext,
    },
    eval_capability_adapter,
    host::telemetry::{redact_event, DEFAULT_EVENT_MAX_BYTES, DEFAULT_STRING_MAX_CHARS},
    telemetry::{telemetry_event, RequestTelemetryContext, TelemetryEmitter},
};
use skiff_runtime_model::service_error::{
    ErrorCorrelation, ExceptionStackFrame, OpaqueServiceError,
};
use skiff_runtime_request::RequestError;
use skiff_runtime_transport::{
    protocol::{
        decode_response_error_frame, encode_binary_frame, ResponseErrorFrameHeader,
        RuntimeErrorFramePayload, TelemetryEvent, TelemetrySource, TelemetryTopic,
        TelemetryVisibility, ValidatedResponseErrorFrame,
    },
    response_mapper::{response_event_into_frame, OrdinaryResponseEvent},
};

#[derive(Debug, Clone)]
struct CapturingEmitter {
    events: Arc<Mutex<Vec<TelemetryEvent>>>,
    accept: bool,
}

impl TelemetryEmitter for CapturingEmitter {
    fn emit(&self, event: TelemetryEvent) -> bool {
        if !self.accept {
            return false;
        }
        self.events.lock().expect("event lock").push(event);
        true
    }
}

fn request_telemetry(accept: bool) -> (RequestTelemetryContext, Arc<Mutex<Vec<TelemetryEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut request = RequestTelemetryContext::new(CapturingEmitter {
        events: Arc::clone(&events),
        accept,
    });
    request.service_id = Some("caller/service".to_string());
    request.build_id = Some("caller-build".to_string());
    request.activation_identity = Some("caller-activation".to_string());
    request.runtime_id = Some("runtime-1".to_string());
    request.request_id = Some("request-1".to_string());
    request.trace_id = Some("ingress-trace".to_string());
    request.span_id = Some("ingress-span".to_string());
    request.parent_span_id = Some("ingress-parent".to_string());
    request.target = Some("caller.operation".to_string());
    (request, events)
}

fn diagnostic() -> RestrictedServiceDiagnostic {
    let source = InstructionSourceSite::Source {
        span: SourceSpanRef {
            source_id: 41,
            start: SourcePosition {
                line: 7,
                column: 3,
                offset: Some(22),
            },
            end: SourcePosition {
                line: 7,
                column: 11,
                offset: Some(30),
            },
        },
    };
    RestrictedServiceDiagnostic {
        owner: RestrictedServiceDiagnosticOwner {
            provider_service_id: "provider/service".to_string(),
            operation_id: "provider.run".to_string(),
            provider_activation_id: "provider-activation".to_string(),
            request_generation: 9,
        },
        correlation: ErrorCorrelation {
            trace_id: "fixed-trace".to_string(),
            error_id: "fixed-error".to_string(),
        },
        source: source.clone(),
        stack: vec![
            ExceptionStackFrame::Local { site: source },
            ExceptionStackFrame::Local {
                site: InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::RuntimeBoundaryDispatch,
                },
            },
            ExceptionStackFrame::RemoteBoundary {
                service_id: "callee/service".to_string(),
                operation_id: "callee.run".to_string(),
                error_id: "callee-error".to_string(),
            },
        ],
        cause_kind: RestrictedServiceDiagnosticCauseKind::InternalError,
    }
}

fn eval_telemetry_context(
    request: RequestTelemetryContext,
) -> skiff_runtime_capability_context::TelemetryCapabilityContext {
    let concrete = EffectDispatchContext::new(
        HttpEffectContext::new(None, 1024, CancellationToken::new()),
        TelemetryCapabilityContext::new(Some(request)),
        HttpRuntimeOptions::explicit(false),
    );
    eval_capability_adapter::effects(concrete).telemetry_context()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operational_fixed_event_uses_top_level_correlation_and_safe_error_shape() {
        let mut ordinary = telemetry_event(
            TelemetryTopic::Trace,
            "2026-07-26T00:00:00.000Z",
            TelemetrySource::Runtime,
        );
        assert_eq!(ordinary.visibility, TelemetryVisibility::Operational);
        assert_eq!(ordinary.error_id, None);

        let (request, events) = request_telemetry(true);
        let correlation = ErrorCorrelation {
            trace_id: "fixed-trace".to_string(),
            error_id: "fixed-error".to_string(),
        };
        request.emit_trace_with_error_correlation(
            "request.error",
            Some(3.5),
            Some(Map::from_iter([
                (
                    "kind".to_string(),
                    Value::String("fixedService".to_string()),
                ),
                (
                    "causeKind".to_string(),
                    Value::String("internalError".to_string()),
                ),
            ])),
            Some(Map::from_iter([(
                "instructionCount".to_string(),
                Value::Number(7_u64.into()),
            )])),
            &correlation,
        );

        let event = events.lock().expect("event lock").pop().expect("event");
        assert_eq!(event.visibility, TelemetryVisibility::Operational);
        assert_eq!(event.trace_id.as_deref(), Some("fixed-trace"));
        assert_eq!(event.error_id.as_deref(), Some("fixed-error"));
        assert_eq!(event.span_id.as_deref(), Some("ingress-span"));
        assert_eq!(event.parent_span_id.as_deref(), Some("ingress-parent"));
        assert_eq!(event.error.as_ref().unwrap()["kind"], "fixedService");
        let serialized = serde_json::to_string(&event).expect("serialize event");
        for private in [
            "provider-private-secret",
            "stack",
            "sourcePath",
            "function",
            "encodedPayload",
        ] {
            assert!(!serialized.contains(private), "{private} must stay absent");
        }

        ordinary.name = Some("request.cancel".to_string());
        assert_eq!(ordinary.visibility, TelemetryVisibility::Operational);
        assert_eq!(ordinary.error_id, None);
    }

    #[test]
    fn production_eval_context_projects_one_typed_restricted_event_to_the_same_emitter() {
        let (request, events) = request_telemetry(true);
        let telemetry = eval_telemetry_context(request);

        telemetry
            .clone()
            .submit_restricted_service_diagnostic(&diagnostic())
            .expect("production sink must use the request emitter");

        let events = events.lock().expect("event lock");
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.visibility, TelemetryVisibility::Restricted);
        assert_eq!(event.service_id.as_deref(), Some("provider/service"));
        assert_eq!(
            event.activation_identity.as_deref(),
            Some("provider-activation")
        );
        assert_eq!(event.target.as_deref(), Some("provider.run"));
        assert_eq!(event.request_id.as_deref(), Some("request-1"));
        assert_eq!(event.runtime_id.as_deref(), Some("runtime-1"));
        assert_eq!(event.trace_id.as_deref(), Some("fixed-trace"));
        assert_eq!(event.error_id.as_deref(), Some("fixed-error"));
        assert_eq!(event.span_id.as_deref(), Some("ingress-span"));
        assert_eq!(
            event.build_id, None,
            "caller build must not own provider data"
        );
        assert_eq!(event.attrs.as_ref().unwrap()["requestGeneration"], 9);
        let error = event.error.as_ref().expect("restricted error");
        assert_eq!(error["kind"], "restrictedServiceDiagnostic");
        assert_eq!(error["causeKind"], "internalError");
        assert_eq!(error["source"]["span"]["sourceId"], 41);
        assert_eq!(error["stack"].as_array().unwrap().len(), 3);
        assert_eq!(error["stack"][0]["kind"], "local");
        assert_eq!(
            error["stack"][1]["site"]["reason"],
            "runtimeBoundaryDispatch"
        );
        assert_eq!(error["stack"][2]["kind"], "remoteBoundary");
        assert_eq!(error["stack"][2]["serviceId"], "callee/service");

        let serialized = serde_json::to_string(event).expect("serialize event");
        for forbidden in [
            "provider-private-secret",
            "modulePath",
            "artifactPath",
            "function",
            "display",
            "encodedPayload",
            "runtimeValue",
            "typeAddr",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "{forbidden} must not enter the typed projection"
            );
        }
    }

    #[test]
    fn restricted_sink_failure_does_not_mutate_fixed_bytes() {
        let encoded = br#"{
      "kind":"internalError",
      "payload":{
        "message":"The service could not complete the request.",
        "traceId":"fixed-trace",
        "errorId":"fixed-error"
      }
    }"#
        .to_vec();
        let fixed = OpaqueServiceError::decode(encoded.clone()).expect("fixed fixture");
        let (request, events) = request_telemetry(false);
        let telemetry = eval_telemetry_context(request);

        assert!(telemetry
            .submit_restricted_service_diagnostic(&diagnostic())
            .is_err());
        assert!(events.lock().expect("event lock").is_empty());
        assert_eq!(fixed.encoded_bytes(), encoded);
    }

    #[test]
    fn restricted_projection_is_covered_by_secret_redaction_and_event_budget() {
        let (request, events) = request_telemetry(true);
        eval_telemetry_context(request)
            .submit_restricted_service_diagnostic(&diagnostic())
            .expect("restricted event");
        let event = events.lock().expect("event lock")[0].clone();

        let mut secret_event = event.clone();
        secret_event
            .error
            .as_mut()
            .unwrap()
            .insert("secret".to_string(), json!("provider-private-secret"));
        let secret_event = redact_event(
            secret_event,
            DEFAULT_STRING_MAX_CHARS,
            DEFAULT_EVENT_MAX_BYTES,
        );
        assert_eq!(secret_event.error.as_ref().unwrap()["secret"], "[redacted]");

        let mut oversized = event;
        oversized.error.as_mut().unwrap().insert(
            "stack".to_string(),
            Value::Array(
                (0..256)
                    .map(|_| Value::String("oversized-private-sentinel".repeat(8)))
                    .collect(),
            ),
        );
        let oversized = redact_event(oversized, 64, 1024);
        assert_eq!(oversized.error.as_ref().unwrap()["truncated"], true);
        assert_eq!(oversized.attrs.as_ref().unwrap()["truncated"], true);
        let serialized = serde_json::to_vec(&oversized).unwrap();
        assert!(serialized.len() <= 1024);
        assert!(!String::from_utf8(serialized)
            .unwrap()
            .contains("oversized-private-sentinel"));
    }

    #[test]
    fn request_to_wire_preserves_three_fixed_payloads() {
        let fixtures = [
        br#"{"kind":"publicTypedError","packageId":"example.com/errors","stableSchemaKey":"not-found","packageSchemaTypeId":"type:not-found","encodedPayload":[123,125],"traceId":"trace-public","errorId":"error-public"}"#
            .as_slice(),
        br#"{
          "kind":"internalError",
          "payload":{
            "message":"The service could not complete the request.",
            "traceId":"trace-internal",
            "errorId":"error-internal"
          }
        }"#
        .as_slice(),
        br#"{"kind":"platformError","builtinErrorIdentity":"std.db.ConflictError","encodedPayload":[123,125],"traceId":"trace-platform","errorId":"error-platform"}"#
            .as_slice(),
    ];

        for (index, encoded) in fixtures.into_iter().enumerate() {
            let encoded = encoded.to_vec();
            let fixed = OpaqueServiceError::decode(encoded.clone()).expect("fixed fixture");
            let request_error = RequestError::Eval(EvalRuntimeError::FixedServiceFailure(fixed));
            let event = OrdinaryResponseEvent::FixedServiceFailure(
                request_error
                    .fixed_service_response_failure()
                    .expect("request typed extraction"),
            );
            let frame = response_event_into_frame(format!("request-{index}"), event)
                .expect("fixed response frame");

            let (_header, validated) =
                decode_response_error_frame(&frame).expect("dedicated response.error decode");
            let ValidatedResponseErrorFrame::FixedService(decoded) = validated else {
                panic!("fixed frame must remain fixed")
            };
            assert_eq!(decoded.encoded_bytes(), encoded);
        }
    }

    #[test]
    fn matching_generic_control_stays_generic_and_payload_rules_fail_closed() {
        let request_error = RequestError::external_error_payload(
            "InternalError".to_string(),
            "canonical service failure".to_string(),
            Some(500),
            Some(json!({"traceId": "control-only"})),
        );
        assert!(request_error.fixed_service_failure().is_none());
        let frame = response_event_into_frame(
            "request-control".to_string(),
            OrdinaryResponseEvent::try_error(&request_error)
                .expect("generic control failure is ordinary"),
        )
        .expect("generic control frame");
        let (_header, validated) = decode_response_error_frame(&frame).expect("control decode");
        assert!(matches!(
            validated,
            ValidatedResponseErrorFrame::Control(ref error)
                if error.code == "InternalError"
                    && error.message == "canonical service failure"
        ));
        let fixed_empty = encode_binary_frame(
            &ResponseErrorFrameHeader::fixed_service("request-fixed-empty".to_string()),
            &[],
        )
        .expect("frame container");
        assert!(decode_response_error_frame(&fixed_empty).is_err());

        let control_nonempty = encode_binary_frame(
            &ResponseErrorFrameHeader::control(
                "request-control-nonempty".to_string(),
                RuntimeErrorFramePayload {
                    code: "InternalError".to_string(),
                    message: "control".to_string(),
                    status: Some(500),
                    details: None,
                },
            ),
            b"not-allowed",
        )
        .expect("frame container");
        assert!(decode_response_error_frame(&control_nonempty).is_err());
    }
}
