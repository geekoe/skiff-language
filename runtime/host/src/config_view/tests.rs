use serde_json::{json, Value};
use skiff_artifact_model::ConfigShape;
use skiff_runtime_boundary::type_descriptor::{RuntimeTypePlan, RuntimeTypePlanDescriptorExt};

use super::{materialize_internal_json, materialize_json, sanitize_wire_json, RuntimeConfigView};

#[test]
fn config_require_decodes_supported_types() {
    let config = RuntimeConfigView::from_resolved_config(
        json!({
            "app": {
                "name": "demo",
                "rate": 1.5,
                "enabled": true,
                "config": { "temperature": 0.2 },
                "list": [1, 2, 3]
            }
        }),
        config_shape(),
    )
    .expect("config should build");

    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("app.name")],
                Some(&type_plan("string"))
            )
            .unwrap(),
        json!("demo")
    );
    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("app.rate")],
                Some(&type_plan("number"))
            )
            .unwrap(),
        json!(1.5)
    );
    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("app.enabled")],
                Some(&type_plan("bool"))
            )
            .unwrap(),
        json!(true)
    );
    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("app.config")],
                Some(&type_plan("JsonObject")),
            )
            .unwrap(),
        json!({ "temperature": 0.2 })
    );
    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("app.list")],
                Some(&type_plan("Json"))
            )
            .unwrap(),
        json!([1, 2, 3])
    );
}

#[test]
fn config_optional_missing_or_null_returns_null() {
    let config =
        RuntimeConfigView::from_resolved_config(json!({"app": {"optional": null}}), config_shape())
            .expect("config should build");

    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.optional",
                &[json!("app.optional")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        Value::Null
    );
    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.optional",
                &[json!("app.missing")],
                Some(&type_plan("number")),
            )
            .unwrap(),
        Value::Null
    );
}

#[test]
fn config_require_missing_or_null_is_decode_target_error() {
    let config =
        RuntimeConfigView::from_resolved_config(json!({"app": {"apiKey": null}}), config_shape())
            .expect("config should build");

    let missing = config
        .dispatch_typed_config_target(
            "config.require",
            &[json!("app.missing")],
            Some(&type_plan("string")),
        )
        .expect_err("missing non-null config must fail");
    assert_config_error_without_value(
        missing,
        "config.require",
        "app.missing",
        "missing or null",
        "secret",
    );

    let null = config
        .dispatch_typed_config_target(
            "config.require",
            &[json!("app.apiKey")],
            Some(&type_plan("string")),
        )
        .expect_err("null non-null config must fail");
    assert_config_error_without_value(
        null,
        "config.require",
        "app.apiKey",
        "missing or null",
        "\"apiKey\"",
    );
}

#[test]
fn config_require_type_mismatch_reports_target_without_actual_value() {
    let config = RuntimeConfigView::from_resolved_config(
        json!({"app": {"apiKey": "secret-value", "config": []}}),
        config_shape(),
    )
    .expect("config should build");

    let string_error = config
        .dispatch_typed_config_target(
            "config.require",
            &[json!("app.apiKey")],
            Some(&type_plan("number")),
        )
        .expect_err("string must not decode as number");
    assert_config_error_without_value(
        string_error,
        "config.require",
        "app.apiKey",
        "number",
        "secret-value",
    );

    let object_error = config
        .dispatch_typed_config_target(
            "config.require",
            &[json!("app.config")],
            Some(&type_plan("JsonObject")),
        )
        .expect_err("array must not decode as JsonObject");
    assert_config_error_without_value(
        object_error,
        "config.require",
        "app.config",
        "JsonObject",
        "[]",
    );
}

#[test]
fn runtime_config_view_validates_shape_at_activation_time() {
    let missing = RuntimeConfigView::from_resolved_config(
        json!({}),
        config_shape_with_entries(json!([
            { "path": "app.apiKey", "type": "string", "required": true }
        ])),
    )
    .expect_err("missing required config should fail before dispatch");
    assert!(
        missing
            .to_string()
            .contains("app.apiKey required value is missing or null"),
        "unexpected error: {missing}"
    );

    let required_null = RuntimeConfigView::from_resolved_config(
        json!({"app": {"apiKey": null}}),
        config_shape_with_entries(json!([
            { "path": "app.apiKey", "type": "string", "required": true }
        ])),
    )
    .expect_err("required null config should fail before dispatch");
    assert!(
        required_null
            .to_string()
            .contains("app.apiKey required value is missing or null"),
        "unexpected error: {required_null}"
    );

    RuntimeConfigView::from_resolved_config(
        json!({"app": {"optional": null}}),
        config_shape_with_entries(json!([
            { "path": "app.optional", "type": "number", "required": false },
            { "path": "app.missing", "type": "string", "required": false }
        ])),
    )
    .expect("optional null and missing config should pass activation validation");

    let mismatch = RuntimeConfigView::from_resolved_config(
        json!({"app": {"rate": "fast"}}),
        config_shape_with_entries(json!([
            { "path": "app.rate", "type": "number", "required": true }
        ])),
    )
    .expect_err("type mismatch should fail before dispatch");
    assert!(
        mismatch.to_string().contains("app.rate must be a number"),
        "unexpected error: {mismatch}"
    );
}

#[test]
fn config_require_and_optional_reject_nullable_type_args() {
    let config = RuntimeConfigView::from_resolved_config(json!({}), config_shape())
        .expect("config should build");

    let require_error = config
        .dispatch_typed_config_target(
            "config.require",
            &[json!("app.missing")],
            Some(&nullable_type_plan(type_plan("string"))),
        )
        .expect_err("require must reject nullable type args");
    assert!(
        require_error.to_string().contains("non-nullable"),
        "unexpected error: {require_error}"
    );

    let optional_error = config
        .dispatch_typed_config_target(
            "config.optional",
            &[json!("app.missing")],
            Some(&nullable_type_plan(type_plan("number"))),
        )
        .expect_err("optional must reject nullable type args");
    assert!(
        optional_error.to_string().contains("non-nullable"),
        "unexpected error: {optional_error}"
    );
}

#[test]
fn config_require_optional_and_has_apply_new_target_semantics() {
    let config = RuntimeConfigView::from_resolved_config(
        json!({
            "app": {
                "name": "demo",
                "empty": null,
                "config": { "temperature": 0.2 }
            }
        }),
        config_shape(),
    )
    .expect("config should build");

    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.require",
                &[json!("app.name")],
                Some(&type_plan("string"))
            )
            .unwrap(),
        json!("demo")
    );
    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.optional",
                &[json!("app.missing")],
                Some(&type_plan("string")),
            )
            .unwrap(),
        Value::Null
    );
    assert_eq!(
        config
            .dispatch_typed_config_target(
                "config.optional",
                &[json!("app.empty")],
                Some(&type_plan("Json"))
            )
            .unwrap(),
        Value::Null
    );
    assert_eq!(
        config
            .dispatch_typed_config_target("config.has", &[json!("app.config")], None)
            .unwrap(),
        json!(true)
    );
    assert_eq!(
        config
            .dispatch_typed_config_target("config.has", &[json!("app.empty")], None)
            .unwrap(),
        json!(false)
    );
    assert_eq!(
        config
            .dispatch_typed_config_target("config.has", &[json!("app.missing")], None)
            .unwrap(),
        json!(false)
    );
}

#[test]
fn config_require_missing_or_null_is_clear_decode_target_error() {
    let config =
        RuntimeConfigView::from_resolved_config(json!({"app": {"apiKey": null}}), config_shape())
            .expect("config should build");

    let missing = config
        .dispatch_typed_config_target(
            "config.require",
            &[json!("app.missing")],
            Some(&type_plan("string")),
        )
        .expect_err("missing required config must fail");
    assert_config_error_without_value(
        missing,
        "config.require",
        "app.missing",
        "missing or null",
        "secret",
    );

    let null = config
        .dispatch_typed_config_target(
            "config.require",
            &[json!("app.apiKey")],
            Some(&type_plan("string")),
        )
        .expect_err("null required config must fail");
    assert_config_error_without_value(
        null,
        "config.require",
        "app.apiKey",
        "missing or null",
        "\"apiKey\"",
    );
}

#[test]
fn config_get_dispatch_target_is_unsupported() {
    let config =
        RuntimeConfigView::from_resolved_config(json!({"app": {"name": "demo"}}), config_shape())
            .expect("config should build");

    let error = config
        .dispatch_typed_config_target("config.get", &[], None)
        .expect_err("config.get should be rejected through runtime dispatch");
    let message = error.to_string();
    assert!(
        message.contains("unsupported config native target config.get"),
        "unexpected error: {message}"
    );
}

#[test]
fn runtime_config_rejects_non_object_activation_payloads() {
    let error = RuntimeConfigView::from_resolved_config(json!("not-object"), config_shape())
        .expect_err("resolvedConfig must be object");
    assert!(error
        .to_string()
        .contains("resolvedConfig must be a JSON object"));

    let error = RuntimeConfigView::from_resolved_config_with_redaction(
        json!({}),
        config_shape(),
        json!("not-object"),
    )
    .expect_err("redactedResolvedConfig must be object");
    assert!(error
        .to_string()
        .contains("redactedResolvedConfig must be a JSON object"));
}

#[test]
fn boundary_adapter_rejects_heap_handles_from_wire_json() {
    let error = sanitize_wire_json(json!({
        "ok": true,
        "nested": {
            "__skiffHeapHandle": 7
        }
    }))
    .expect_err("wire JSON must not carry runtime heap handles");

    assert!(error.to_string().contains("internal runtime handle"));
}

#[test]
fn boundary_adapter_rejects_handles_inside_protocol_envelopes() {
    let error = materialize_json(json!({
        "__skiffBytesBase64": "YQ==",
        "__skiffHeapHandle": 7
    }))
    .expect_err("protocol envelopes must not hide runtime handles");

    assert!(error.to_string().contains("internal runtime handle"));
}

#[test]
fn boundary_adapter_strips_stream_handles_from_protocol_json() {
    let value = materialize_json(json!({
        "__skiffStreamId": "stream-1",
        "kind": "event"
    }))
    .expect("protocol materialization should strip internal stream handles");

    assert_eq!(value, json!({"kind": "event"}));
}

#[test]
fn boundary_internal_adapter_preserves_stream_handles() {
    let value = materialize_internal_json(json!({
        "__skiffStreamId": "stream-1"
    }))
    .expect("internal bridge materialization should preserve stream handles");

    assert_eq!(value, json!({"__skiffStreamId": "stream-1"}));
}

#[test]
fn boundary_internal_adapter_strips_non_canonical_stream_handle_fields() {
    let value = materialize_internal_json(json!({
        "__skiffStreamId": "stream-1",
        "kind": "event"
    }))
    .expect("non-canonical stream-looking objects should be sanitized");

    assert_eq!(value, json!({"kind": "event"}));
}

#[test]
fn boundary_adapter_preserves_skiff_prefixed_business_fields() {
    let value = materialize_json(json!({
        "__skiffRepresentationType": "Example",
        "__skiffRepresentationValue": {
            "visible": true
        },
        "regular": "kept"
    }))
    .expect("skiff-prefixed fields should be ordinary JSON");

    assert_eq!(
        value,
        json!({
            "__skiffRepresentationType": "Example",
            "__skiffRepresentationValue": {
                "visible": true
            },
            "regular": "kept"
        })
    );
}

fn config_shape() -> ConfigShape {
    config_shape_from_json(json!({
        "schemaVersion": "skiff-config-shape-v1",
        "entries": []
    }))
}

fn config_shape_with_entries(entries: Value) -> ConfigShape {
    config_shape_from_json(json!({
        "schemaVersion": "skiff-config-shape-v1",
        "entries": entries
    }))
}

fn config_shape_from_json(value: Value) -> ConfigShape {
    serde_json::from_value(value).expect("test config shape should parse")
}

fn type_plan(name: &str) -> RuntimeTypePlan {
    RuntimeTypePlan::from_descriptor(&json!({ "kind": "builtin", "name": name, "args": [] }))
        .expect("config test type plan should build")
}

fn nullable_type_plan(inner: RuntimeTypePlan) -> RuntimeTypePlan {
    RuntimeTypePlan::synthetic_nullable(inner)
}

fn assert_config_error_without_value(
    error: crate::error::RuntimeError,
    target: &str,
    path: &str,
    expected: &str,
    forbidden: &str,
) {
    let message = error.to_string();
    assert!(message.contains(target), "unexpected error: {message}");
    assert!(message.contains(path), "unexpected error: {message}");
    assert!(message.contains(expected), "unexpected error: {message}");
    assert!(
        !message.contains(forbidden),
        "error leaked value content: {message}"
    );
}
