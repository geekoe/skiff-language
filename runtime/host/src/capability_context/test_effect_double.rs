//! Test double target matching and argument/result boundary conversion.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use serde_json::Value;
use skiff_runtime_boundary::{contract::RuntimeBoundaryContract, plan::BoundaryUse};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{HeapNode, RuntimeMap, RuntimeValue},
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};

use super::{StreamRuntime, TARGET_STD_HTTP_REQUEST, TARGET_STD_HTTP_SSE, TARGET_STD_HTTP_STREAM};
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
                        let reusable = doubles.len() == 1;
                        (
                            target,
                            doubles
                                .into_iter()
                                .map(|double| RegisteredTestEffectDouble { double, reusable })
                                .collect(),
                        )
                    })
                    .collect(),
            )),
        }
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

    pub fn dispatch_test_effect_double(
        &self,
        target: &str,
        input: Option<&Value>,
    ) -> Option<Result<Value>> {
        if !is_test_double_target(target) {
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
        if !is_test_double_target(target) {
            return None;
        }
        let Some(double) = self.next_test_effect_double(target) else {
            return self.missing_double_error(target).map(Err);
        };
        if let Some(expected) = &double.expect_request {
            let actual = input.unwrap_or(&RuntimeValue::Null);
            let mut expected_heap = RequestHeap::default();
            let expected_plan = arg_plan.map(|plan| fixture_subset_plan(plan, expected));
            let expected_runtime = match runtime_from_wire_required_plan(
                expected,
                expected_plan.as_ref(),
                &format!("test double request {target}"),
                &mut expected_heap,
            ) {
                Ok(value) => value,
                Err(error) => return Some(Err(error)),
            };
            match runtime_value_contains(actual, heap, &expected_runtime, &expected_heap) {
                Ok(true) => {}
                Ok(false) => {
                    return Some(Err(RuntimeError::Decode(format!(
                        "test double expectation failed for {target}: expected request subset {expected}, got runtime input {actual:?}"
                    ))));
                }
                Err(error) => return Some(Err(error)),
            }
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

fn is_test_double_target(target: &str) -> bool {
    matches!(
        target,
        TARGET_STD_HTTP_REQUEST | TARGET_STD_HTTP_STREAM | TARGET_STD_HTTP_SSE
    ) || is_std_log_stable_target(target)
}

fn is_std_log_stable_target(target: &str) -> bool {
    matches!(
        target,
        "std.log.debug" | "std.log.info" | "std.log.warn" | "std.log.error"
    )
}

fn runtime_value_contains(
    actual: &RuntimeValue,
    actual_heap: &RequestHeap,
    expected: &RuntimeValue,
    expected_heap: &RequestHeap,
) -> Result<bool> {
    match (actual, expected) {
        (RuntimeValue::Null, RuntimeValue::Null) => Ok(true),
        (RuntimeValue::Bool(actual), RuntimeValue::Bool(expected)) => Ok(actual == expected),
        (RuntimeValue::Number(actual), RuntimeValue::Number(expected)) => Ok(actual == expected),
        (RuntimeValue::String(actual), RuntimeValue::String(expected)) => Ok(actual == expected),
        (RuntimeValue::ActorRef(actual), RuntimeValue::ActorRef(expected)) => {
            Ok(actual == expected)
        }
        (RuntimeValue::Heap(actual), RuntimeValue::Heap(expected)) => runtime_heap_node_contains(
            actual_heap.get(*actual)?,
            actual_heap,
            expected_heap.get(*expected)?,
            expected_heap,
        ),
        _ => Ok(false),
    }
}

fn runtime_heap_node_contains(
    actual: &HeapNode,
    actual_heap: &RequestHeap,
    expected: &HeapNode,
    expected_heap: &RequestHeap,
) -> Result<bool> {
    match (actual, expected) {
        (HeapNode::Bytes(actual), HeapNode::Bytes(expected)) => Ok(actual == expected),
        (HeapNode::Array(actual), HeapNode::Array(expected)) => {
            if actual.len() != expected.len() {
                return Ok(false);
            }
            actual
                .iter()
                .zip(expected.iter())
                .try_fold(true, |matches, (actual, expected)| {
                    if !matches {
                        return Ok(false);
                    }
                    runtime_value_contains(actual, actual_heap, expected, expected_heap)
                })
        }
        (HeapNode::Object(actual), HeapNode::Object(expected)) => expected
            .fields()
            .iter()
            .try_fold(true, |matches, (key, expected_value)| {
                if !matches {
                    return Ok(false);
                }
                let Some(actual_value) = actual.fields().get(key) else {
                    return Ok(false);
                };
                runtime_value_contains(actual_value, actual_heap, expected_value, expected_heap)
            }),
        (HeapNode::Map(actual), HeapNode::Map(expected)) => {
            runtime_map_contains(actual, actual_heap, expected, expected_heap)
        }
        _ => Ok(false),
    }
}

fn runtime_map_contains(
    actual: &RuntimeMap,
    actual_heap: &RequestHeap,
    expected: &RuntimeMap,
    expected_heap: &RequestHeap,
) -> Result<bool> {
    expected
        .iter()
        .try_fold(true, |matches, (key, expected_value)| {
            if !matches {
                return Ok(false);
            }
            let Some(actual_value) = actual.get(key) else {
                return Ok(false);
            };
            runtime_value_contains(actual_value, actual_heap, expected_value, expected_heap)
        })
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
        let fixture_plan = fixture_subset_plan(&plan, fixture);
        let mut fixture_heap = RequestHeap::default();
        let fixture = runtime_from_wire_required_plan(
            fixture,
            Some(&fixture_plan),
            "fixture HTTP request",
            &mut fixture_heap,
        )?;
        runtime_value_contains(&actual, &actual_heap, &fixture, &fixture_heap)
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
}
