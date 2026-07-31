use serde_json::{json, Value};

use skiff_artifact_model::ConfigShapeValueType;

use super::ConfigTargetType;
use crate::type_descriptor::{RuntimeTypePlan, RuntimeTypePlanDescriptorExt};

#[test]
fn config_target_type_decodes_supported_type_args() {
    for (name, value) in [
        ("string", json!("text")),
        ("number", json!(1.5)),
        ("bool", json!(true)),
        ("Json", json!([1, 2, 3])),
        ("JsonObject", json!({"ok": true})),
    ] {
        let target_type =
            ConfigTargetType::from_type_plan("config.require", Some(&type_plan(name)))
                .expect("target type should decode");

        assert_eq!(
            target_type
                .decode_value("config.require", "app.value", &value)
                .expect("config value should decode"),
            value
        );
    }
}

#[test]
fn config_target_type_rejects_nullable_and_unsupported_type_args() {
    let nullable = ConfigTargetType::from_type_plan(
        "config.optional",
        Some(&RuntimeTypePlan::synthetic_nullable(type_plan("string"))),
    )
    .expect_err("nullable config type args should be rejected");
    assert!(nullable.to_string().contains("non-nullable"));

    let unsupported = ConfigTargetType::from_type_plan("config.require", Some(&type_plan("Date")))
        .expect_err("unsupported config type should be rejected");
    assert!(unsupported.to_string().contains("unsupported"));
}

#[test]
fn config_target_type_matches_shape_values() {
    let string_type = ConfigTargetType::from_shape_type(ConfigShapeValueType::String);
    assert!(string_type.matches_value(&json!("text")));
    assert!(!string_type.matches_value(&json!(7)));

    let json_type = ConfigTargetType::from_shape_type(ConfigShapeValueType::Json);
    assert!(json_type.matches_value(&Value::Null));

    let object_type = ConfigTargetType::from_shape_type(ConfigShapeValueType::JsonObject);
    assert!(object_type.matches_value(&json!({"ok": true})));
    assert!(!object_type.matches_value(&json!([1, 2, 3])));
}

#[test]
fn config_value_decode_reports_target_and_path_without_value() {
    let target_type =
        ConfigTargetType::from_type_plan("config.require", Some(&type_plan("number")))
            .expect("target type should decode");

    let error = target_type
        .decode_value("config.require", "app.apiKey", &json!("secret-value"))
        .expect_err("mismatched config value should fail");
    let message = error.to_string();

    assert!(message.contains("config.require"));
    assert!(message.contains("app.apiKey"));
    assert!(message.contains("number"));
    assert!(!message.contains("secret-value"));
}

fn type_plan(name: &str) -> RuntimeTypePlan {
    RuntimeTypePlan::from_descriptor(&json!({ "kind": "builtin", "name": name, "args": [] }))
        .expect("config test type plan should build")
}
