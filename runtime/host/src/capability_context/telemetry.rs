//! Runtime-local telemetry capability for native telemetry emission.

use serde_json::{Map, Value};
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_capability_context::{
    RestrictedServiceDiagnostic, RestrictedServiceDiagnosticCauseKind, RuntimeExceptionLog,
    RuntimeExceptionLogReason,
};
use skiff_runtime_model::service_error::ExceptionStackFrame;

use crate::{
    config_view::materialize_json,
    error::{Result, RuntimeError},
    telemetry::{telemetry_event, telemetry_timestamp_now, RequestTelemetryContext},
};
use skiff_runtime_transport::protocol::{
    PlatformEvent, TelemetryEvent, TelemetryLevel, TelemetrySource, TelemetryVisibility,
};

/// The only constructor for business log events (`level` + `message`, no
/// `name`). It is reached by `std.log.*` and by the Runtime-owned exception
/// fallback; every other platform event uses `PlatformEvent` and cannot be
/// mistaken for a business log.
fn business_log_event(
    level: TelemetryLevel,
    message: impl Into<String>,
    attrs: Option<Map<String, Value>>,
) -> TelemetryEvent {
    let mut event = telemetry_event(telemetry_timestamp_now(), TelemetrySource::Runtime);
    event.level = Some(level);
    event.message = Some(message.into());
    event.attrs = attrs;
    event
}

#[derive(Clone)]
pub struct TelemetryCapabilityContext {
    request: Option<RequestTelemetryContext>,
}

impl TelemetryCapabilityContext {
    pub fn new(request: Option<RequestTelemetryContext>) -> Self {
        Self { request }
    }

    pub fn emit_native(&self, target: &str, args: &[Value]) -> Result<Value> {
        if target != "std.telemetry.emit" {
            return Err(RuntimeError::Unsupported(format!(
                "unsupported telemetry target {target}"
            )));
        }
        self.emit_log_args(target, args)
    }

    /// Emits one payload-free Runtime exception record through the same
    /// business-log constructor and request-bound telemetry path used by
    /// `std.log.error`.
    pub(crate) fn emit_runtime_exception_log(&self, log: &RuntimeExceptionLog) -> bool {
        let Some(request) = self.request.as_ref() else {
            return false;
        };
        let mut attrs = Map::from_iter([
            (
                "exceptionIdentity".to_string(),
                Value::String(log.metadata.identity.clone()),
            ),
            (
                "exceptionIdentityHash".to_string(),
                Value::String(log.metadata.identity_hash.clone()),
            ),
            (
                "reason".to_string(),
                Value::String(runtime_exception_log_reason(log.metadata.reason).to_string()),
            ),
        ]);
        if let Some(callable) = log.metadata.callable.as_ref() {
            attrs.insert("callable".to_string(), Value::String(callable.clone()));
        }

        let mut event =
            business_log_event(TelemetryLevel::Error, "Skiff exception raised", Some(attrs));
        self.apply_request_context(&mut event);
        event.trace_id = Some(log.correlation.trace_id.clone());
        event.error_id = Some(log.correlation.error_id.clone());
        request.emit(event)
    }

    /// Projects the eval-owned typed diagnostic into the host's restricted lane.
    ///
    /// The projection has a closed field set. In particular it never accepts a
    /// display string, heap value, source path, function name, or open attrs.
    pub(crate) fn emit_restricted_service_diagnostic(
        &self,
        diagnostic: &RestrictedServiceDiagnostic,
    ) -> bool {
        let Some(request) = self.request.as_ref() else {
            return false;
        };
        let mut event = PlatformEvent::new("service.error.restricted")
            .with_attrs(Some(Map::from_iter([(
                "requestGeneration".to_string(),
                Value::Number(diagnostic.owner.request_generation.into()),
            )])))
            .with_error(Some(Map::from_iter([
                (
                    "kind".to_string(),
                    Value::String("restrictedServiceDiagnostic".to_string()),
                ),
                (
                    "causeKind".to_string(),
                    Value::String(restricted_cause_kind(diagnostic.cause_kind).to_string()),
                ),
                (
                    "source".to_string(),
                    instruction_source_site_value(&diagnostic.source),
                ),
                (
                    "stack".to_string(),
                    Value::Array(
                        diagnostic
                            .stack
                            .iter()
                            .map(exception_stack_frame_value)
                            .collect(),
                    ),
                ),
            ])))
            .into_event(telemetry_timestamp_now(), TelemetrySource::Runtime);
        event.visibility = TelemetryVisibility::Restricted;
        event.service_id = Some(diagnostic.owner.provider_service_id.clone());
        event.activation_identity = Some(diagnostic.owner.provider_activation_id.clone());
        event.runtime_id = request.runtime_id.clone();
        event.request_id = request.request_id.clone();
        event.trace_id = Some(diagnostic.correlation.trace_id.clone());
        event.error_id = Some(diagnostic.correlation.error_id.clone());
        event.span_id = request.span_id.clone();
        event.parent_span_id = request.parent_span_id.clone();
        event.target = Some(diagnostic.owner.operation_id.clone());
        request.emit(event)
    }

    /// Business-log construction for `std.log.*` (reached only through the
    /// `std.telemetry.emit` native bridge). This is the only place that
    /// produces business log events (`level` + `message`, no `name`); platform
    /// code emits through `PlatformEvent` and must not use this surface.
    fn emit_log_args(&self, target: &str, args: &[Value]) -> Result<Value> {
        let level = args
            .first()
            .ok_or_else(|| RuntimeError::Decode(format!("{target} level must be a string")))
            .and_then(decode_telemetry_level)?;
        let message = args
            .get(1)
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError::Decode(format!("{target} message must be a string")))?;
        let attrs = match args.get(2) {
            None | Some(Value::Null) => None,
            Some(Value::Object(_)) => Some(
                materialize_json(args.get(2).cloned().unwrap_or(Value::Null))?
                    .as_object()
                    .cloned()
                    .ok_or_else(|| {
                        RuntimeError::Decode(format!("{target} attrs must be an object"))
                    })?,
            ),
            Some(_) => {
                return Err(RuntimeError::Decode(format!(
                    "{target} attrs must be an object or null"
                )));
            }
        };

        let mut event = business_log_event(level, message, attrs);
        self.apply_request_context(&mut event);

        self.emit(event);
        Ok(Value::Null)
    }

    fn apply_request_context(&self, event: &mut TelemetryEvent) {
        let Some(context) = self.request.as_ref() else {
            return;
        };
        event.service_id = context.service_id.clone();
        event.revision_id = context.revision_id.clone();
        event.build_id = context.build_id.clone();
        event.activation_identity = context.activation_identity.clone();
        event.runtime_id = context.runtime_id.clone();
        event.request_id = context.request_id.clone();
        event.trace_id = context.trace_id.clone();
        event.span_id = context.span_id.clone();
        event.parent_span_id = context.parent_span_id.clone();
        event.target = context.target.clone();
    }

    fn emit(&self, event: TelemetryEvent) {
        if let Some(context) = self.request.as_ref() {
            context.emit(event);
        }
    }
}

fn restricted_cause_kind(kind: RestrictedServiceDiagnosticCauseKind) -> &'static str {
    match kind {
        RestrictedServiceDiagnosticCauseKind::PublicTypedError => "publicTypedError",
        RestrictedServiceDiagnosticCauseKind::InternalError => "internalError",
        RestrictedServiceDiagnosticCauseKind::PlatformError => "platformError",
    }
}

fn runtime_exception_log_reason(reason: RuntimeExceptionLogReason) -> &'static str {
    match reason {
        RuntimeExceptionLogReason::Throw => "throw",
        RuntimeExceptionLogReason::RuntimeProjection => "runtimeProjection",
        RuntimeExceptionLogReason::Timeout => "timeout",
        RuntimeExceptionLogReason::Internal => "internal",
    }
}

fn exception_stack_frame_value(frame: &ExceptionStackFrame) -> Value {
    match frame {
        ExceptionStackFrame::Local { site } => serde_json::json!({
            "kind": "local",
            "site": instruction_source_site_value(site),
        }),
        ExceptionStackFrame::RemoteBoundary {
            service_id,
            operation_id,
            error_id,
        } => serde_json::json!({
            "kind": "remoteBoundary",
            "serviceId": service_id,
            "operationId": operation_id,
            "errorId": error_id,
        }),
    }
}

fn instruction_source_site_value(site: &InstructionSourceSite) -> Value {
    match site {
        InstructionSourceSite::Source { span } => serde_json::json!({
            "kind": "source",
            "span": {
                "sourceId": span.source_id,
                "start": {
                    "line": span.start.line,
                    "column": span.start.column,
                    "offset": span.start.offset,
                },
                "end": {
                    "line": span.end.line,
                    "column": span.end.column,
                    "offset": span.end.offset,
                },
            },
        }),
        InstructionSourceSite::Synthetic { reason } => serde_json::json!({
            "kind": "synthetic",
            "reason": synthetic_site_reason(*reason),
        }),
    }
}

fn synthetic_site_reason(reason: SyntheticInstructionSiteReason) -> &'static str {
    match reason {
        SyntheticInstructionSiteReason::CompilerDesugaring => "compilerDesugaring",
        SyntheticInstructionSiteReason::CompilerGeneratedWrapper => "compilerGeneratedWrapper",
        SyntheticInstructionSiteReason::CompilerGeneratedTestHarness => {
            "compilerGeneratedTestHarness"
        }
        SyntheticInstructionSiteReason::RuntimeBoundaryDispatch => "runtimeBoundaryDispatch",
        SyntheticInstructionSiteReason::RuntimeControlFlow => "runtimeControlFlow",
    }
}

fn decode_telemetry_level(value: &Value) -> Result<TelemetryLevel> {
    match value.as_str() {
        Some("debug") => Ok(TelemetryLevel::Debug),
        Some("info") => Ok(TelemetryLevel::Info),
        Some("warn") => Ok(TelemetryLevel::Warn),
        Some("error") => Ok(TelemetryLevel::Error),
        _ => Err(RuntimeError::Decode(
            "std.telemetry.emit level must be one of debug, info, warn, error".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests;
