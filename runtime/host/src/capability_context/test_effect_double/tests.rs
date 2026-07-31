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
    let runtime = runtime_from_wire_required_plan(&wire, Some(&plan), "nested request", &mut heap)
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
