//! X6 task request seam.
//!
//! Durable function tasks are fresh requests, not VM child invocations. This
//! module owns the request-side marker and payload materialization gate. The
//! exact F6 tuple/record plan is not yet present in the linked signature, so
//! payload materialization remains fail-closed instead of guessing parameter
//! names or re-deriving a plan from runtime values.

use skiff_runtime_linker::DeploymentExecutionEntry;
use skiff_runtime_model::vm_heap::VmHeap;
use skiff_runtime_model::vm_value::ValueSlot;

use crate::{RequestEnvelope, RequestError, RequestResult};

pub(crate) fn is_task_request(request: &RequestEnvelope) -> bool {
    request
        .extra
        .get("task")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub(crate) fn task_arguments(
    _request: &RequestEnvelope,
    _entry: &DeploymentExecutionEntry,
    _heap: &mut dyn VmHeap,
) -> RequestResult<Vec<ValueSlot>> {
    Err(RequestError::Unsupported(task_required_fact().to_string()))
}

pub(crate) fn task_required_fact() -> &'static str {
    "durable task recoverable payload requires the exact F6 linked parameter \
     tuple/record plan; the current linked signature does not retain parameter names"
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn task_envelope() -> RequestEnvelope {
        RequestEnvelope {
            request_id: "task-request".to_string(),
            mode: "unary".to_string(),
            target: "function:main.work".to_string(),
            operation_abi_id: None,
            selector: None,
            service_id: Some("test.skiff/task".to_string()),
            build_id: "build:task".to_string(),
            service_protocol_identity: "protocol:task".to_string(),
            contract_identity: None,
            activation_identity: None,
            ingress_selector: None,
            binary_http: None,
            http_adapter: None,
            test_effects_enabled: false,
            test_effect_doubles: Default::default(),
            payload_bytes: vec![1, 2, 3],
            extra: serde_json::Map::from_iter([("task".to_string(), json!(true))]),
        }
    }

    #[test]
    fn task_marker_is_exact_and_not_inferred_from_http_payload() {
        assert!(is_task_request(&task_envelope()));
        let mut envelope = task_envelope();
        envelope.extra.remove("task");
        assert!(!is_task_request(&envelope));
    }

    #[test]
    fn task_payload_required_fact_names_f6_owner() {
        assert!(task_required_fact().contains("F6 linked parameter"));
    }
}
