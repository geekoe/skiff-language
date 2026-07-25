//! Test double target matching and argument/result boundary conversion.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use serde_json::Value;
use skiff_runtime_boundary::{contract::RuntimeBoundaryContract, plan::BoundaryUse};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::RuntimeValue,
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};

#[cfg(test)]
use super::TARGET_STD_HTTP_REQUEST;
use super::{StreamRuntime, TARGET_STD_HTTP_SSE, TARGET_STD_HTTP_STREAM};
use crate::{
    config_view::materialize_internal_json,
    error::{Result, RuntimeError},
};
fn runtime_from_wire_required_plan(
    value: &Value,
    expected_type: Option<&RuntimeTypePlan>,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let expected_type = expected_type.ok_or_else(|| {
        RuntimeError::invalid_artifact(format!(
            "{boundary} boundary is missing expected type descriptor"
        ))
    })?;
    Ok(RuntimeBoundaryContract::default()
        .codec_for_expected(expected_type, BoundaryUse::TypedJson, boundary)
        .from_wire_json(value, heap)?)
}

fn runtime_from_wire_internal_handle_required_plan(
    value: &Value,
    expected_type: Option<&RuntimeTypePlan>,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<RuntimeValue> {
    let expected_type = expected_type.ok_or_else(|| {
        RuntimeError::invalid_artifact(format!(
            "{boundary} boundary is missing expected type descriptor"
        ))
    })?;
    Ok(RuntimeBoundaryContract::default()
        .codec_for_expected(expected_type, BoundaryUse::NativeReturn, boundary)
        .from_wire_json_internal_handle(value, heap)?)
}

fn runtime_to_wire_required_plan(
    value: &RuntimeValue,
    expected_type: Option<&RuntimeTypePlan>,
    boundary: &str,
    heap: &mut RequestHeap,
) -> Result<Value> {
    let expected_type = expected_type.ok_or_else(|| {
        RuntimeError::invalid_artifact(format!(
            "{boundary} boundary is missing expected type descriptor"
        ))
    })?;
    RuntimeBoundaryContract::default()
        .codec_for_expected(expected_type, BoundaryUse::NativeArg, boundary)
        .to_wire_json(value, heap)
        .map_err(Into::into)
}

#[derive(Clone, Debug)]
pub struct TestEffectDouble {
    pub expect_request: Option<Value>,
    pub response: Value,
}

#[derive(Clone, Debug, Default)]
pub struct TestEffectDoubleRegistry {
    entries: Arc<Mutex<HashMap<String, VecDeque<RegisteredTestEffectDouble>>>>,
}

impl TestEffectDoubleRegistry {
    pub fn reusable(test_effect_doubles: HashMap<String, TestEffectDouble>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(
                test_effect_doubles
                    .into_iter()
                    .map(|(target, double)| {
                        (
                            target,
                            VecDeque::from([RegisteredTestEffectDouble {
                                double,
                                reusable: true,
                            }]),
                        )
                    })
                    .collect(),
            )),
        }
    }

    pub fn one_shot_sequences(test_effect_doubles: HashMap<String, Vec<TestEffectDouble>>) -> Self {
        Self {
            entries: Arc::new(Mutex::new(
                test_effect_doubles
                    .into_iter()
                    .map(|(target, doubles)| {
                        (
                            target,
                            doubles
                                .into_iter()
                                .map(|double| RegisteredTestEffectDouble {
                                    double,
                                    reusable: false,
                                })
                                .collect(),
                        )
                    })
                    .collect(),
            )),
        }
    }

    pub fn contains(&self, target: &str) -> bool {
        self.entries
            .lock()
            .expect("test effect double registry lock poisoned")
            .contains_key(target)
    }

    pub fn remaining(&self) -> Vec<(String, usize)> {
        let registry = self
            .entries
            .lock()
            .expect("test effect double registry lock poisoned");
        let mut remaining = registry
            .iter()
            .map(|(target, entries)| (target.clone(), entries.len()))
            .collect::<Vec<_>>();
        remaining.sort_by(|left, right| left.0.cmp(&right.0));
        remaining
    }

    pub fn next(&self, target: &str) -> Option<TestEffectDouble> {
        let mut registry = self
            .entries
            .lock()
            .expect("test effect double registry lock poisoned");
        let queue = registry.get_mut(target)?;
        let registered = if queue.front().is_some_and(|entry| entry.reusable) {
            queue.front().cloned()
        } else {
            queue.pop_front()
        }?;
        if queue.is_empty() {
            registry.remove(target);
        }
        Some(registered.double)
    }
}

#[derive(Clone, Debug)]
struct RegisteredTestEffectDouble {
    double: TestEffectDouble,
    reusable: bool,
}

#[derive(Clone, Debug)]
pub struct TestEffectDoubleContext {
    registry: TestEffectDoubleRegistry,
    stream_runtime: StreamRuntime,
    test_effects_enabled: bool,
}

impl TestEffectDoubleContext {
    pub fn reusable(
        test_effect_doubles: HashMap<String, TestEffectDouble>,
        stream_runtime: StreamRuntime,
        test_effects_enabled: bool,
    ) -> Self {
        Self::new(
            TestEffectDoubleRegistry::reusable(test_effect_doubles),
            stream_runtime,
            test_effects_enabled,
        )
    }

    pub fn one_shot_sequences(
        test_effect_doubles: HashMap<String, Vec<TestEffectDouble>>,
        stream_runtime: StreamRuntime,
        test_effects_enabled: bool,
    ) -> Self {
        Self::new(
            TestEffectDoubleRegistry::one_shot_sequences(test_effect_doubles),
            stream_runtime,
            test_effects_enabled,
        )
    }

    pub fn new(
        registry: TestEffectDoubleRegistry,
        stream_runtime: StreamRuntime,
        test_effects_enabled: bool,
    ) -> Self {
        Self {
            registry,
            stream_runtime,
            test_effects_enabled,
        }
    }

    pub fn missing_double_error(&self, target: &str) -> Option<RuntimeError> {
        self.test_effects_enabled
            .then(|| RuntimeError::Unsupported(format!("no test double registered for {target}")))
    }

    pub fn require_non_test_mode(&self, target: &str) -> Result<()> {
        match self.missing_double_error(target) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn next_test_effect_double(&self, target: &str) -> Option<TestEffectDouble> {
        self.registry.next(target)
    }

    pub fn unused_effects_error(&self) -> Option<RuntimeError> {
        let remaining = self.registry.remaining();
        (!remaining.is_empty()).then(|| {
            RuntimeError::Decode(format!(
                "unused test effects: {}",
                remaining
                    .iter()
                    .map(|(target, count)| format!("{target} ({count} outcome(s))"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })
    }

    pub fn dispatch_test_effect_double(
        &self,
        target: &str,
        input: Option<&Value>,
    ) -> Option<Result<Value>> {
        if !self.registry.contains(target) {
            return None;
        }
        let double = self.next_test_effect_double(target)?;
        if let Some(expected) = &double.expect_request {
            let actual = input.unwrap_or(&Value::Null);
            if !json_contains(actual, expected) {
                return Some(Err(RuntimeError::Decode(format!(
                    "test double expectation failed for {target}: expected request subset {expected}, got {actual}"
                ))));
            }
        }
        if is_stream_source_double_target(target) {
            let events = match &double.response {
                Value::Array(items) => items.clone(),
                value => vec![value.clone()],
            };
            let events = events
                .into_iter()
                .map(materialize_internal_json)
                .collect::<Result<Vec<_>>>();
            return Some(events.map(|events| self.stream_runtime.buffered_stream(events)));
        }
        Some(materialize_internal_json(double.response.clone()))
    }

    pub fn dispatch_test_stable_target_double(
        &self,
        target: &str,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        if !is_std_log_stable_target(target) {
            return None;
        }
        self.dispatch_test_effect_double(target, None)
            .map(|result| {
                result.and_then(|value| {
                    runtime_from_wire_required_plan(
                        &value,
                        return_plan,
                        &format!("test double response {target}"),
                        heap,
                    )
                })
            })
    }

    pub fn dispatch_test_host_operation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        self.dispatch_test_http_effect_invocation_double(target, input, arg_plan, return_plan, heap)
    }

    pub fn dispatch_test_http_effect_invocation_double(
        &self,
        target: &str,
        input: Option<&RuntimeValue>,
        arg_plan: Option<&RuntimeTypePlan>,
        return_plan: Option<&RuntimeTypePlan>,
        heap: &mut RequestHeap,
    ) -> Option<Result<RuntimeValue>> {
        if !self.registry.contains(target) {
            return None;
        }
        let Some(double) = self.next_test_effect_double(target) else {
            return self.missing_double_error(target).map(Err);
        };
        if let Some(expected) = &double.expect_request {
            let Some(arg_plan) = arg_plan else {
                return Some(Err(RuntimeError::invalid_artifact(format!(
                    "test double request {target} boundary is missing expected type descriptor"
                ))));
            };
            let actual = input.unwrap_or(&RuntimeValue::Null);
            let actual = match runtime_to_wire_required_plan(
                actual,
                Some(arg_plan),
                &format!("test double request {target}"),
                heap,
            ) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            let expected_plan = fixture_subset_plan(arg_plan, expected);
            let mut expected_heap = RequestHeap::default();
            let expected_runtime = match runtime_from_wire_required_plan(
                expected,
                Some(&expected_plan),
                &format!("test double request {target} expectation"),
                &mut expected_heap,
            ) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            let expected = match runtime_to_wire_required_plan(
                &expected_runtime,
                Some(&expected_plan),
                &format!("test double request {target} expectation"),
                &mut expected_heap,
            ) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            if !json_contains(&actual, &expected) {
                return Some(Err(RuntimeError::Decode(format!(
                    "test double expectation failed for {target}: expected request subset {expected}, got a nonmatching materialized request"
                ))));
            }
        }
        if let Some(payload) = typed_throw_payload(&double.response) {
            return Some(Err(RuntimeError::Opaque(Box::new(TestTypedEffectError {
                target: target.to_string(),
                payload: payload.clone(),
            }))));
        }
        if let Some(events) = stream_outcome_events(&double.response) {
            let events = match events
                .iter()
                .cloned()
                .map(materialize_internal_json)
                .collect::<Result<Vec<_>>>()
            {
                Ok(events) => events,
                Err(error) => return Some(Err(error)),
            };
            let response = self.stream_runtime.buffered_stream(events);
            return Some(runtime_from_wire_internal_handle_required_plan(
                &response,
                return_plan,
                &format!("{target} stream response"),
                heap,
            ));
        }
        let response = if is_stream_source_double_target(target) {
            let events = match &double.response {
                Value::Array(items) => items.clone(),
                value => vec![value.clone()],
            };
            let events = match events
                .into_iter()
                .map(materialize_internal_json)
                .collect::<Result<Vec<_>>>()
            {
                Ok(events) => events,
                Err(error) => return Some(Err(error)),
            };
            self.stream_runtime.buffered_stream(events)
        } else {
            double.response.clone()
        };

        let boundary = format!("{target} response");
        Some(if target == TARGET_STD_HTTP_STREAM {
            runtime_from_wire_internal_handle_required_plan(&response, return_plan, &boundary, heap)
        } else {
            runtime_from_wire_required_plan(&response, return_plan, &boundary, heap)
        })
    }
}

fn fixture_subset_plan(plan: &RuntimeTypePlan, fixture: &Value) -> RuntimeTypePlan {
    let mut subset = plan.clone();
    subset.node = match plan.node() {
        RuntimeTypeNode::Alias(inner) => {
            RuntimeTypeNode::Alias(Box::new(fixture_subset_plan(inner, fixture)))
        }
        RuntimeTypeNode::Nullable(inner) if !fixture.is_null() => {
            RuntimeTypeNode::Nullable(Box::new(fixture_subset_plan(inner, fixture)))
        }
        RuntimeTypeNode::Representation { type_name, payload } => RuntimeTypeNode::Representation {
            type_name: type_name.clone(),
            payload: Box::new(fixture_subset_plan(payload, fixture)),
        },
        RuntimeTypeNode::Record {
            fields,
            boundary_record_kind,
        } => {
            let fixture = fixture.as_object();
            RuntimeTypeNode::Record {
                fields: fields
                    .iter()
                    .filter_map(|field| {
                        fixture
                            .and_then(|object| object.get(&field.name))
                            .map(|value| {
                                let mut field = field.clone();
                                field.ty = fixture_subset_plan(&field.ty, value);
                                field
                            })
                    })
                    .collect(),
                boundary_record_kind: boundary_record_kind.clone(),
            }
        }
        RuntimeTypeNode::Array(item) => RuntimeTypeNode::Array(Box::new(
            fixture
                .as_array()
                .and_then(|items| items.first())
                .map_or_else(
                    || (**item).clone(),
                    |value| fixture_subset_plan(item, value),
                ),
        )),
        _ => plan.node().clone(),
    };
    subset
}

fn json_contains(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => expected.iter().all(|(key, value)| {
            actual
                .get(key)
                .is_some_and(|actual_value| json_contains(actual_value, value))
        }),
        (Value::Array(actual), Value::Array(expected)) => {
            actual.len() == expected.len()
                && actual
                    .iter()
                    .zip(expected.iter())
                    .all(|(actual_value, expected_value)| {
                        json_contains(actual_value, expected_value)
                    })
        }
        _ => actual == expected,
    }
}

fn is_stream_source_double_target(target: &str) -> bool {
    target == TARGET_STD_HTTP_SSE
}

fn typed_throw_payload(value: &Value) -> Option<&Value> {
    let object = value.as_object()?;
    (object.get("__skiffTestEffectOutcome")?.as_str()? == "throw")
        .then(|| object.get("payload"))
        .flatten()
}

fn stream_outcome_events(value: &Value) -> Option<&[Value]> {
    let object = value.as_object()?;
    if object.get("__skiffTestEffectOutcome")?.as_str()? != "stream" {
        return None;
    }
    object.get("events")?.as_array().map(Vec::as_slice)
}

#[derive(Debug, thiserror::Error)]
#[error("test effect `{target}` threw its declared typed payload")]
struct TestTypedEffectError {
    target: String,
    payload: Value,
}

impl skiff_runtime_model::error::WirePayload for TestTypedEffectError {
    fn payload(&self) -> skiff_runtime_model::error::RuntimeErrorPayload {
        skiff_runtime_model::error::RuntimeErrorPayload {
            code: "TestTypedEffect".to_string(),
            message: self.to_string(),
            status: None,
            details: Some(self.payload.clone()),
        }
    }

    fn catch_projection(&self) -> Option<(skiff_runtime_model::error::TypeIdentity, Value)> {
        Some((
            skiff_runtime_model::error::TypeIdentity::builtin("TestTypedEffect"),
            self.payload.clone(),
        ))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn is_std_log_stable_target(target: &str) -> bool {
    matches!(
        target,
        "std.log.debug" | "std.log.info" | "std.log.warn" | "std.log.error"
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use skiff_runtime_model::type_plan::RuntimeRecordFieldPlan;

    use super::*;

    fn leaf(label: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
        RuntimeTypePlan::new(label, None, node)
    }

    fn http_request_plan() -> RuntimeTypePlan {
        let header = RuntimeTypePlan::synthetic_request_record(vec![
            RuntimeRecordFieldPlan::new("name", leaf("String", RuntimeTypeNode::String), true),
            RuntimeRecordFieldPlan::new("value", leaf("String", RuntimeTypeNode::String), true),
        ]);
        RuntimeTypePlan::synthetic_request_record(vec![
            RuntimeRecordFieldPlan::new("method", leaf("String", RuntimeTypeNode::String), true),
            RuntimeRecordFieldPlan::new("url", leaf("String", RuntimeTypeNode::String), true),
            RuntimeRecordFieldPlan::new("headers", RuntimeTypePlan::synthetic_array(header), true),
            RuntimeRecordFieldPlan::new("body", leaf("Bytes", RuntimeTypeNode::Bytes), true),
            RuntimeRecordFieldPlan::new(
                "timeoutMs",
                RuntimeTypePlan::synthetic_nullable(leaf("Integer", RuntimeTypeNode::Integer)),
                false,
            ),
        ])
    }

    fn typed_fixture_matches(actual: &Value, fixture: &Value) -> Result<bool> {
        let plan = http_request_plan();
        let mut actual_heap = RequestHeap::default();
        let actual = runtime_from_wire_required_plan(
            actual,
            Some(&plan),
            "actual HTTP request",
            &mut actual_heap,
        )?;
        let actual = runtime_to_wire_required_plan(
            &actual,
            Some(&plan),
            "test double HTTP request",
            &mut actual_heap,
        )?;
        Ok(json_contains(&actual, fixture))
    }

    fn actual_request() -> Value {
        json!({
            "method": "PUT",
            "url": "https://demo-bucket.oss-cn-hangzhou.aliyuncs.com/photos/a.txt",
            "headers": [
                { "name": "Content-Type", "value": "text/plain" },
                { "name": "Authorization", "value": "OSS test-id:signature" }
            ],
            "body": { "__skiffBytesBase64": "aGVsbG8=" },
            "timeoutMs": 5000
        })
    }

    #[test]
    fn typed_http_fixture_matches_record_headers_bytes_and_allows_omitted_fields() {
        let actual = actual_request();
        let fixture = json!({
            "method": "PUT",
            "url": "https://demo-bucket.oss-cn-hangzhou.aliyuncs.com/photos/a.txt",
            "headers": [
                { "name": "Content-Type", "value": "text/plain" },
                { "name": "Authorization", "value": "OSS test-id:signature" }
            ],
            "body": { "__skiffBytesBase64": "aGVsbG8=" }
        });

        assert!(typed_fixture_matches(&actual, &fixture).expect("typed fixture should decode"));
    }

    #[test]
    fn typed_http_fixture_rejects_method_url_header_body_and_signature_mismatches() {
        let actual = actual_request();
        for fixture in [
            json!({ "method": "POST" }),
            json!({ "url": "https://example.test/wrong" }),
            json!({ "headers": [
                { "name": "Content-Type", "value": "application/json" },
                { "name": "Authorization", "value": "OSS test-id:signature" }
            ] }),
            json!({ "body": { "__skiffBytesBase64": "d3Jvbmc=" } }),
            json!({ "headers": [
                { "name": "Content-Type", "value": "text/plain" },
                { "name": "Authorization", "value": "OSS test-id:wrong" }
            ] }),
        ] {
            assert!(
                !typed_fixture_matches(&actual, &fixture).expect("typed fixture should decode"),
                "fixture unexpectedly matched: {fixture}"
            );
        }
    }

    #[test]
    fn request_materialization_preserves_nested_maps_and_nullable_values() {
        let metadata = RuntimeTypePlan::synthetic_map(
            leaf("String", RuntimeTypeNode::String),
            RuntimeTypePlan::synthetic_nullable(leaf("String", RuntimeTypeNode::String)),
        );
        let plan = RuntimeTypePlan::synthetic_request_record(vec![
            RuntimeRecordFieldPlan::new("metadata", metadata, true),
            RuntimeRecordFieldPlan::new(
                "note",
                RuntimeTypePlan::synthetic_nullable(leaf("String", RuntimeTypeNode::String)),
                false,
            ),
        ]);
        let wire = json!({
            "metadata": {
                "present": "value",
                "absent": null
            },
            "note": null
        });
        let mut heap = RequestHeap::default();
        let runtime =
            runtime_from_wire_required_plan(&wire, Some(&plan), "nested request", &mut heap)
                .expect("nested request should decode");

        let materialized =
            runtime_to_wire_required_plan(&runtime, Some(&plan), "nested request", &mut heap)
                .expect("nested request should materialize");

        assert_eq!(materialized, wire);
    }

    #[test]
    fn request_materialization_rejects_runtime_type_mismatch() {
        let plan = http_request_plan();
        let mut heap = RequestHeap::default();
        let error = runtime_to_wire_required_plan(
            &RuntimeValue::String("not a request".to_string()),
            Some(&plan),
            "test double request",
            &mut heap,
        )
        .expect_err("request type mismatch must fail closed");

        assert!(
            error.to_string().contains("expected heap object"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn one_shot_response_sequence_order_is_unchanged() {
        let first = TestEffectDouble {
            expect_request: None,
            response: json!({ "status": 200 }),
        };
        let second = TestEffectDouble {
            expect_request: None,
            response: json!({ "status": 201 }),
        };
        let registry = TestEffectDoubleRegistry::one_shot_sequences(HashMap::from([(
            TARGET_STD_HTTP_REQUEST.to_string(),
            vec![first.clone(), second.clone()],
        )]));

        assert_eq!(
            registry
                .next(TARGET_STD_HTTP_REQUEST)
                .expect("first response")
                .response,
            first.response
        );
        assert_eq!(
            registry
                .next(TARGET_STD_HTTP_REQUEST)
                .expect("second response")
                .response,
            second.response
        );
        assert!(registry.next(TARGET_STD_HTTP_REQUEST).is_none());
    }

    #[test]
    fn single_outcome_is_consumed_and_exhaustion_is_observable() {
        let registry = TestEffectDoubleRegistry::one_shot_sequences(HashMap::from([(
            "dependency.call".to_string(),
            vec![TestEffectDouble {
                expect_request: None,
                response: json!("only"),
            }],
        )]));
        assert_eq!(
            registry.next("dependency.call").unwrap().response,
            json!("only")
        );
        assert!(registry.next("dependency.call").is_none());
    }

    #[test]
    fn remaining_reports_unused_outcomes_precisely() {
        let registry = TestEffectDoubleRegistry::one_shot_sequences(HashMap::from([(
            "dependency.call".to_string(),
            vec![
                TestEffectDouble {
                    expect_request: None,
                    response: json!(1),
                },
                TestEffectDouble {
                    expect_request: None,
                    response: json!(2),
                },
            ],
        )]));
        let _ = registry.next("dependency.call");
        assert_eq!(
            registry.remaining(),
            vec![("dependency.call".to_string(), 1)]
        );
    }

    #[test]
    fn registries_are_isolated_between_parallel_cases() {
        let case = || {
            TestEffectDoubleRegistry::one_shot_sequences(HashMap::from([(
                "dependency.call".to_string(),
                vec![TestEffectDouble {
                    expect_request: None,
                    response: json!("case"),
                }],
            )]))
        };
        let left = case();
        let right = case();
        let left_thread = std::thread::spawn(move || left.next("dependency.call"));
        let right_thread = std::thread::spawn(move || right.next("dependency.call"));
        assert_eq!(left_thread.join().unwrap().unwrap().response, json!("case"));
        assert_eq!(
            right_thread.join().unwrap().unwrap().response,
            json!("case")
        );
    }
}
