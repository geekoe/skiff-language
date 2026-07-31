use serde_json::Value;
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_eval::{
    error::{BudgetReason, RuntimeError},
    exceptions::{request_exception_for_catch, request_exception_for_rethrow},
};
use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    error::{RuntimeErrorPayload, WirePayload},
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeValue, RuntimeValueCarrier},
    service_error::{
        CatchIdentity, ErrorCorrelation, ExceptionStackFrame, LocalExecutionTypeIdentity,
        NominalTypeIdentity, PlatformBuiltinErrorIdentity, RequestException,
    },
};

fn test_site() -> InstructionSourceSite {
    InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
    }
}

fn local_identity() -> CatchIdentity {
    CatchIdentity::Nominal(NominalTypeIdentity::LocalExecution(
        LocalExecutionTypeIdentity {
            addr: TypeAddr {
                unit: UnitAddr::Service,
                file: FileAddr::loaded_file(0),
                type_index: 7,
            },
            type_arguments: Vec::new(),
        },
    ))
}

#[derive(Debug, thiserror::Error)]
#[error("opaque projection fixture")]
struct OpaqueProjectionFixture {
    payload: RuntimeErrorPayload,
    identity: CatchIdentity,
    value: Value,
}

impl WirePayload for OpaqueProjectionFixture {
    fn payload(&self) -> RuntimeErrorPayload {
        self.payload.clone()
    }

    fn catch_projection(&self) -> Option<(CatchIdentity, Value)> {
        Some((self.identity.clone(), self.value.clone()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_platform_projection_is_exact_and_resource_error_fails_closed() {
        assert_eq!(
            PlatformBuiltinErrorIdentity::from_symbol("std.resource.ResourceError"),
            None,
        );
        let db_error = RuntimeError::db_decode("std.db", "missing id")
            .with_diagnostic_frame(serde_json::json!({ "operation": "fixture.db" }));
        assert_eq!(
            db_error.ordinary_catch_projection(),
            Some((
                PlatformBuiltinErrorIdentity::DbDecode.catch_identity(),
                serde_json::json!({
                    "target": "std.db",
                    "message": "missing id",
                }),
            )),
        );

        let resource_error = RuntimeError::resource_error("prompts/system.md", "missing")
            .with_diagnostic_frame(serde_json::json!({ "operation": "fixture.resource" }));
        let payload = resource_error
            .ordinary_payload()
            .expect("resource error remains ordinary");
        assert_eq!(payload.code, "std.resource.ResourceError");
        assert_eq!(
            payload.details,
            Some(serde_json::json!({
                "path": "prompts/system.md",
                "message": "missing",
                "frames": [{ "operation": "fixture.resource" }],
            })),
        );
        assert_eq!(resource_error.ordinary_catch_projection(), None);
    }

    #[test]
    fn cancellation_terminal_is_not_materialized_for_source_catch_but_timeout_is() {
        let timeout_identity = PlatformBuiltinErrorIdentity::Timeout.catch_identity();
        let source = test_site();
        let stack = vec![ExceptionStackFrame::Local {
            site: source.clone(),
        }];
        let mut heap = RequestHeap::default();

        let cancellation = RuntimeError::Cancelled
            .with_diagnostic_frame(serde_json::json!({ "operation": "fixture.cancel" }));
        let caught = request_exception_for_catch(
            &cancellation,
            std::slice::from_ref(&timeout_identity),
            source.clone(),
            stack.clone(),
            ErrorCorrelation {
                trace_id: "trace-cancel".to_string(),
                error_id: "trace-cancel:local-error:1".to_string(),
            },
            &mut heap,
        )
        .expect("cancellation classification must not fail");
        assert_eq!(caught, None);

        let timeout = RuntimeError::ExecutionBudgetExceeded {
            reason: BudgetReason::DeadlineExceeded,
            instruction_count: 42,
            limit: Some(100),
            elapsed_ms: 12.5,
        };
        let caught = request_exception_for_catch(
            &timeout,
            std::slice::from_ref(&timeout_identity),
            source,
            stack,
            ErrorCorrelation {
                trace_id: "trace-timeout".to_string(),
                error_id: "trace-timeout:local-error:1".to_string(),
            },
            &mut heap,
        )
        .expect("timeout catch projection")
        .expect("timeout remains source-catchable");
        assert_eq!(caught.local_catch_identity(), Some(&timeout_identity));
    }

    #[test]
    fn opaque_projection_forwards_identity_and_payload_without_reconstruction() {
        let identity = PlatformBuiltinErrorIdentity::Http.catch_identity();
        let value = serde_json::json!({
            "message": "upstream failed",
            "detail": { "status": 503 },
        });
        let payload = RuntimeErrorPayload {
            code: "std.http.HttpError".to_string(),
            message: "upstream failed".to_string(),
            status: Some(503),
            details: Some(value.clone()),
        };
        let error = RuntimeError::from_wire_payload(Box::new(OpaqueProjectionFixture {
            payload: payload.clone(),
            identity: identity.clone(),
            value: value.clone(),
        }));

        assert_eq!(error.ordinary_payload(), Some(payload));
        assert_eq!(error.ordinary_catch_projection(), Some((identity, value)),);
    }

    #[test]
    fn request_local_rethrow_reuses_exception_state_without_a_wire_round_trip() {
        let identity = local_identity();
        let source = test_site();
        let stack = vec![ExceptionStackFrame::Local {
            site: source.clone(),
        }];
        let correlation = ErrorCorrelation {
            trace_id: "trace-eval-fixture".to_string(),
            error_id: "trace-eval-fixture:local-error:1".to_string(),
        };
        let exception = RequestException::local(
            RuntimeValueCarrier::identified(RuntimeValue::from("denied"), identity.clone()),
            source.clone(),
            stack.clone(),
            correlation.clone(),
        )
        .expect("request-local exception");
        let mut heap = RequestHeap::default();
        let handle = heap
            .alloc_exception(exception.clone())
            .expect("request-local exception node");
        let carrier = RuntimeValueCarrier::unidentified(RuntimeValue::Heap(handle));

        let rethrown = request_exception_for_rethrow(&carrier, &heap)
            .expect("request-local rethrow must read the existing exception node");

        assert_eq!(rethrown.local_catch_identity(), Some(&identity));
        assert_eq!(rethrown.source(), &source);
        assert_eq!(rethrown.stack(), stack);
        assert_eq!(rethrown.correlation(), &correlation);
        assert_eq!(rethrown, exception);
        assert!(matches!(
            heap.get(handle).expect("same exception node"),
            HeapNode::Exception(stored) if stored == &rethrown
        ));
    }
}
