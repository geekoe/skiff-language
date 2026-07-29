use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use serde::Deserialize;
use serde_json::{json, Map, Value};
use skiff_artifact_model::InstructionSourceSite;
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
    telemetry::{RequestTelemetryContext, TelemetryEmitter},
};
use skiff_runtime_model::service_error::{
    ErrorCorrelation, ExceptionStackFrame, OpaqueServiceError, ServiceErrorEnvelope,
};
use skiff_runtime_request::RequestError;
use skiff_runtime_transport::{
    protocol::{
        decode_response_error_frame, ResponseErrorFrameHeader, RuntimeErrorFramePayload,
        TelemetryEvent, TelemetryVisibility, ValidatedResponseErrorFrame,
    },
    response_mapper::{response_event_into_frame, OrdinaryResponseEvent},
};

const SCENARIO_JSON: &str = include_str!(
    "../../../testdata/package-service-contract-deployment/service-error-convergence.json"
);
const CORPUS_JSON: &str = include_str!("../../transport/testdata/service-error-response-v2.json");
const CONTROL_CASE: &str = "generic-control-same-safe-values-as-internal";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioFixture {
    corpus_case: String,
    trace_id: String,
    error_id: String,
    private_sentinel: String,
    hops: Vec<HopExpectation>,
    external_safe_message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HopExpectation {
    name: String,
    service_id: String,
    activation_id: String,
    operation_id: String,
    source: InstructionSourceSite,
    local_stack: Vec<InstructionSourceSite>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireCorpus {
    schema_version: u64,
    valid_cases: Vec<WireCase>,
    invalid_cases: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireCase {
    name: String,
    header: ResponseErrorFrameHeader,
    payload_utf8: String,
    expected: Value,
}

#[derive(Debug, Clone)]
struct CapturingEmitter {
    events: Arc<Mutex<Vec<TelemetryEvent>>>,
}

impl TelemetryEmitter for CapturingEmitter {
    fn emit(&self, event: TelemetryEvent) -> bool {
        self.events.lock().expect("event lock").push(event);
        true
    }
}

fn fixtures() -> (ScenarioFixture, WireCorpus) {
    let scenario = serde_json::from_str::<ScenarioFixture>(SCENARIO_JSON)
        .expect("strict service-error convergence fixture");
    let corpus =
        serde_json::from_str::<WireCorpus>(CORPUS_JSON).expect("strict response.error v2 corpus");
    assert_eq!(corpus.schema_version, 1);
    assert!(!corpus.invalid_cases.is_empty());
    assert_eq!(
        scenario
            .hops
            .iter()
            .map(|hop| hop.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    assert!(!scenario.external_safe_message.trim().is_empty());
    (scenario, corpus)
}

fn corpus_case<'a>(corpus: &'a WireCorpus, name: &str) -> &'a WireCase {
    corpus
        .valid_cases
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("missing corpus case {name}"))
}

fn correlation(scenario: &ScenarioFixture) -> ErrorCorrelation {
    ErrorCorrelation {
        trace_id: scenario.trace_id.clone(),
        error_id: scenario.error_id.clone(),
    }
}

fn request_telemetry(
    scenario: &ScenarioFixture,
) -> (RequestTelemetryContext, Arc<Mutex<Vec<TelemetryEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut request = RequestTelemetryContext::new(CapturingEmitter {
        events: Arc::clone(&events),
    });
    let outer = scenario.hops.last().expect("outer forwarding hop");
    request.service_id = Some(outer.service_id.clone());
    request.activation_identity = Some(outer.activation_id.clone());
    request.runtime_id = Some("runtime-p5-f345".to_string());
    request.request_id = Some("request-internal-1".to_string());
    request.trace_id = Some(scenario.trace_id.clone());
    request.span_id = Some("span-p5-f345".to_string());
    request.parent_span_id = Some("span-ingress-p5-f345".to_string());
    request.target = Some(outer.operation_id.clone());
    (request, events)
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

fn diagnostic(scenario: &ScenarioFixture, index: usize) -> RestrictedServiceDiagnostic {
    let hop = &scenario.hops[index];
    let mut stack = hop
        .local_stack
        .iter()
        .cloned()
        .map(|site| ExceptionStackFrame::Local { site })
        .collect::<Vec<_>>();
    if index > 0 {
        let remote = &scenario.hops[index - 1];
        stack.push(ExceptionStackFrame::RemoteBoundary {
            service_id: remote.service_id.clone(),
            operation_id: remote.operation_id.clone(),
            error_id: scenario.error_id.clone(),
        });
    }
    RestrictedServiceDiagnostic {
        owner: RestrictedServiceDiagnosticOwner {
            provider_service_id: hop.service_id.clone(),
            operation_id: hop.operation_id.clone(),
            provider_activation_id: hop.activation_id.clone(),
            request_generation: u64::try_from(index + 1).expect("three finite hops"),
        },
        correlation: correlation(scenario),
        source: hop.source.clone(),
        stack,
        cause_kind: RestrictedServiceDiagnosticCauseKind::InternalError,
    }
}

fn expected_stack(scenario: &ScenarioFixture, index: usize) -> Vec<Value> {
    let hop = &scenario.hops[index];
    let mut stack = hop
        .local_stack
        .iter()
        .map(|site| json!({ "kind": "local", "site": site }))
        .collect::<Vec<_>>();
    if index > 0 {
        let remote = &scenario.hops[index - 1];
        stack.push(json!({
            "kind": "remoteBoundary",
            "serviceId": remote.service_id,
            "operationId": remote.operation_id,
            "errorId": scenario.error_id,
        }));
    }
    stack
}

#[test]
fn c0_internal_bytes_cross_three_typed_host_wire_hops_and_control_stays_generic() {
    let (scenario, corpus) = fixtures();
    let internal = corpus_case(&corpus, &scenario.corpus_case);
    let original_bytes = internal.payload_utf8.as_bytes().to_vec();
    let original = OpaqueServiceError::decode(original_bytes.clone())
        .expect("C0 Internal fixed case must strict decode");
    let internal_message = match original.envelope() {
        ServiceErrorEnvelope::InternalError { payload } => {
            assert_eq!(payload.trace_id, scenario.trace_id);
            assert_eq!(payload.error_id, scenario.error_id);
            payload.message.clone()
        }
        _ => panic!("scenario corpus reference must remain InternalError"),
    };
    assert_eq!(internal.expected["traceId"], scenario.trace_id);
    assert_eq!(internal.expected["errorId"], scenario.error_id);

    let mut current = original;
    let mut forwarded = Vec::new();
    for hop in &scenario.hops {
        let request_error = RequestError::Eval(EvalRuntimeError::FixedServiceFailure(current));
        let carrier = request_error
            .fixed_service_response_failure()
            .expect("typed request extraction must precede generic payload mapping");
        let frame = response_event_into_frame(
            format!("request-internal-{}", hop.name),
            OrdinaryResponseEvent::FixedServiceFailure(carrier),
        )
        .expect("Rust v2 fixed frame");
        let (_header, validated) =
            decode_response_error_frame(&frame).expect("dedicated strict response.error decode");
        let ValidatedResponseErrorFrame::FixedService(decoded) = validated else {
            panic!("fixed discriminator must remain fixed");
        };
        assert_eq!(decoded.encoded_bytes(), original_bytes);
        forwarded.push(decoded.encoded_bytes().to_vec());
        current = decoded;

        let raw_frame = String::from_utf8_lossy(&frame);
        for forbidden in [
            scenario.private_sentinel.as_str(),
            "sourceId",
            "sourceFrame",
            "sourceFrames",
            "\"frames\"",
            "\"stack\"",
            "\"function\"",
            "\"path\"",
        ] {
            assert!(
                !raw_frame.contains(forbidden),
                "{forbidden} must not enter fixed response bytes"
            );
        }
    }
    assert_eq!(forwarded, vec![original_bytes.clone(); 3]);

    let control = corpus_case(&corpus, CONTROL_CASE);
    let ResponseErrorFrameHeader::Control { error, .. } = &control.header else {
        panic!("matching control corpus case must remain control");
    };
    assert_eq!(error.message, internal_message);
    let request_error = RequestError::external_error_payload(
        error.code.clone(),
        error.message.clone(),
        error.status,
        error.details.clone(),
    );
    assert!(request_error.fixed_service_failure().is_none());
    let control_frame = response_event_into_frame(
        control.header.request_id().to_string(),
        OrdinaryResponseEvent::try_error(&request_error)
            .expect("generic control failure is ordinary"),
    )
    .expect("generic control frame");
    let (_control_header, control_validated) =
        decode_response_error_frame(&control_frame).expect("dedicated control decode");
    assert!(matches!(
        control_validated,
        ValidatedResponseErrorFrame::Control(RuntimeErrorFramePayload {
            ref code,
            ref message,
            ..
        }) if code == &error.code && message == &error.message
    ));
}

#[test]
fn production_eval_context_projects_three_correlated_restricted_hops_beside_one_safe_event() {
    let (scenario, corpus) = fixtures();
    let internal = corpus_case(&corpus, &scenario.corpus_case);
    let fixed = OpaqueServiceError::decode(internal.payload_utf8.as_bytes().to_vec())
        .expect("C0 Internal fixed case");
    assert_eq!(fixed.envelope().trace_id(), scenario.trace_id);
    assert_eq!(fixed.envelope().error_id(), scenario.error_id);

    let (request, events) = request_telemetry(&scenario);
    request.emit_trace_with_error_correlation(
        "request.error",
        Some(4.5),
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
            Value::Number(17_u64.into()),
        )])),
        &correlation(&scenario),
    );
    let telemetry = eval_telemetry_context(request);
    for index in 0..scenario.hops.len() {
        telemetry
            .clone()
            .submit_restricted_service_diagnostic(&diagnostic(&scenario, index))
            .expect("production eval telemetry context must reach Host projection");
    }

    let captured = events.lock().expect("event lock");
    assert_eq!(captured.len(), 4);
    let operational = &captured[0];
    assert_eq!(operational.visibility, TelemetryVisibility::Operational);
    assert_eq!(
        operational.trace_id.as_deref(),
        Some(scenario.trace_id.as_str())
    );
    assert_eq!(
        operational.error_id.as_deref(),
        Some(scenario.error_id.as_str())
    );
    assert_eq!(
        operational.error.as_ref().expect("safe error")["kind"],
        "fixedService"
    );
    assert_eq!(
        operational.error.as_ref().expect("safe error")["causeKind"],
        "internalError"
    );
    let operational_json = serde_json::to_string(operational).expect("operational JSON");
    for forbidden in [
        scenario.private_sentinel.as_str(),
        "sourceId",
        "sourceFrame",
        "sourceFrames",
        "\"frames\"",
        "\"stack\"",
        "\"function\"",
        "\"path\"",
    ] {
        assert!(
            !operational_json.contains(forbidden),
            "{forbidden} must stay outside the operational event"
        );
    }

    let service_ids = scenario
        .hops
        .iter()
        .map(|hop| hop.service_id.as_str())
        .collect::<BTreeSet<_>>();
    let activation_ids = scenario
        .hops
        .iter()
        .map(|hop| hop.activation_id.as_str())
        .collect::<BTreeSet<_>>();
    let sources = scenario
        .hops
        .iter()
        .map(|hop| serde_json::to_string(&hop.source).expect("source JSON"))
        .collect::<BTreeSet<_>>();
    let local_stacks = scenario
        .hops
        .iter()
        .map(|hop| serde_json::to_string(&hop.local_stack).expect("stack JSON"))
        .collect::<BTreeSet<_>>();
    assert_eq!(service_ids.len(), 3);
    assert_eq!(activation_ids.len(), 3);
    assert_eq!(sources.len(), 3);
    assert_eq!(local_stacks.len(), 3);

    for (index, hop) in scenario.hops.iter().enumerate() {
        let event = &captured[index + 1];
        assert_eq!(event.visibility, TelemetryVisibility::Restricted);
        assert_eq!(event.service_id.as_deref(), Some(hop.service_id.as_str()));
        assert_eq!(
            event.activation_identity.as_deref(),
            Some(hop.activation_id.as_str())
        );
        assert_eq!(event.target.as_deref(), Some(hop.operation_id.as_str()));
        assert_eq!(event.trace_id.as_deref(), Some(scenario.trace_id.as_str()));
        assert_eq!(event.error_id.as_deref(), Some(scenario.error_id.as_str()));
        let error = event.error.as_ref().expect("restricted diagnostic shape");
        assert_eq!(error["source"], serde_json::to_value(&hop.source).unwrap());
        assert_eq!(
            error["stack"],
            Value::Array(expected_stack(&scenario, index))
        );
        let serialized = serde_json::to_string(event).expect("restricted JSON");
        assert!(!serialized.contains(&scenario.private_sentinel));
        for forbidden in ["modulePath", "artifactPath", "function", "encodedPayload"] {
            assert!(!serialized.contains(forbidden));
        }
    }
}
