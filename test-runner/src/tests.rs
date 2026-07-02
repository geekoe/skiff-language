use serde_json::json;

use std::fs;

use super::doubles::{
    live_missing_config_skip_message, read_runtime_test_inputs, service_db_mongo_url_from_config,
    service_db_mongo_url_from_router_config_path,
};
use super::runtime_process::{config_wrapped_for_router, RuntimeLiveMetadata};
use super::service::runtime_live_expected_error;
use super::SkiffTestOptions;

#[test]
fn live_missing_config_skip_message_is_not_package_specific() {
    assert_eq!(
        live_missing_config_skip_message(
            "provider unavailable for app.live: missing app.provider.apiKey"
        )
        .as_deref(),
        Some("missing live config app.provider.apiKey; skipping live smoke test")
    );
    assert_eq!(
        live_missing_config_skip_message(
            "decode error for config.require: path app.provider.apiKey required value is missing or null"
        )
        .as_deref(),
        Some("missing live config app.provider.apiKey; skipping live smoke test")
    );
    assert_eq!(
        live_missing_config_skip_message(
            "decode error for config.get: path legacy.provider.apiKey required value is missing or null"
        )
        .as_deref(),
        Some("missing live config legacy.provider.apiKey; skipping live smoke test")
    );
    assert_eq!(
        live_missing_config_skip_message(
            "configShape entry path app.provider.apiKey required value is missing or null"
        )
        .as_deref(),
        Some("missing live config app.provider.apiKey; skipping live smoke test")
    );
    assert_eq!(
        live_missing_config_skip_message(
            "empty service config shape decode failed: configShape entry path openai.apiKey required value is missing or null"
        )
        .as_deref(),
        Some("missing live config openai.apiKey; skipping live smoke test")
    );
    assert_eq!(
        live_missing_config_skip_message(
            "service config shape decode failed: configShape entry path dashscope.apiKey required value is missing or null"
        )
        .as_deref(),
        Some("missing live config dashscope.apiKey; skipping live smoke test")
    );
    assert_eq!(
        live_missing_config_skip_message(
            "router test dispatch failed: HTTP 400 DecodeError: path service.bailian.apiKey required value is missing or null"
        )
        .as_deref(),
        Some("missing live config service.bailian.apiKey; skipping live smoke test")
    );
    assert_eq!(
        live_missing_config_skip_message(
            "decode error for config.require: path app.provider.apiKey must be a string"
        ),
        None
    );
    assert_eq!(
        live_missing_config_skip_message(
            "configShape entry path app.provider.apiKey must be a string"
        ),
        None
    );
    assert_eq!(
        live_missing_config_skip_message("provider unavailable for app.live: provider HTTP 429"),
        None
    );
    assert_eq!(live_missing_config_skip_message("assertion failed"), None);
}

#[test]
fn live_config_can_carry_service_db_activation_config() {
    assert_eq!(
        service_db_mongo_url_from_config(&json!({
            "serviceDb": {
                "mongoUrl": "mongodb://127.0.0.1:27017/skiff-test"
            }
        }))
        .unwrap()
        .as_deref(),
        Some("mongodb://127.0.0.1:27017/skiff-test")
    );
    assert_eq!(
        service_db_mongo_url_from_config(&json!({ "serviceDb": null }))
            .unwrap()
            .as_deref(),
        None
    );
    assert!(
        service_db_mongo_url_from_config(&json!({ "serviceDb": {} }))
            .unwrap_err()
            .contains("serviceDb.mongoUrl is required")
    );
}

#[test]
fn router_config_service_db_extraction_reads_only_service_db() {
    let temp =
        std::env::temp_dir().join(format!("skiff-test-runner-router-{}", std::process::id()));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();
    let router_path = temp.join("router.yml");
    fs::write(
        &router_path,
        r#"
profile: local
host: 127.0.0.1
http:
  port: 4000
serviceDb:
  mongoUrl: "mongodb://127.0.0.1:27017/?directConnection=true"
"#,
    )
    .unwrap();

    assert_eq!(
        service_db_mongo_url_from_router_config_path(&router_path)
            .unwrap()
            .as_deref(),
        Some("mongodb://127.0.0.1:27017/?directConnection=true")
    );

    fs::remove_dir_all(&temp).unwrap();
}

#[test]
fn package_file_tests_load_shared_package_doubles() {
    let temp = std::env::temp_dir().join(format!(
        "skiff-test-runner-package-file-doubles-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    let package = temp.join("pkg");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        temp.join("skiff.test-doubles.json"),
        r#"{
  "config": {
    "cookieName": "shared_session",
    "serviceDb": {
      "mongoUrl": "mongodb://127.0.0.1:27017/skiff-test"
    }
  }
}"#,
    )
    .unwrap();
    fs::write(
        package.join("package.yml"),
        "id: example.com/pkg\nversion: 0.1.0\n",
    )
    .unwrap();
    let test_file = package.join("session.test.skiff");
    fs::write(&test_file, "test \"noop\" { assert true }\n").unwrap();

    let inputs = read_runtime_test_inputs(&test_file, true, &SkiffTestOptions::default()).unwrap();
    assert_eq!(inputs.config["cookieName"], "shared_session");
    assert_eq!(
        inputs.service_db_mongo_url.as_deref(),
        Some("mongodb://127.0.0.1:27017/skiff-test")
    );

    fs::remove_dir_all(&temp).unwrap();
}

#[test]
fn runtime_live_metadata_is_injected_into_service_config() {
    let config = config_wrapped_for_router(
        &json!({
            "runtimeLive": {
                "operation": "enabled"
            }
        }),
        None,
        std::iter::empty::<&str>(),
        Some(RuntimeLiveMetadata {
            service_id: "example.com/live-runtime",
            version: "test",
        }),
    );

    assert_eq!(
        config.pointer("/service/runtimeLive/operation"),
        Some(&json!("enabled"))
    );
    assert_eq!(
        config.pointer("/service/runtimeLive/serviceId"),
        Some(&json!("example.com/live-runtime"))
    );
    assert_eq!(
        config.pointer("/service/runtimeLive/version"),
        Some(&json!("test"))
    );
}

#[test]
fn runtime_live_expected_error_config_accepts_code_and_message_matcher() {
    let expected = runtime_live_expected_error(&json!({
        "runtimeLive": {
            "expectedError": {
                "code": "ResourceLimitExceeded",
                "messageContains": "64 MiB guard limit"
            }
        }
    }))
    .unwrap()
    .expect("expected runtime error should parse");

    assert_eq!(expected.code(), "ResourceLimitExceeded");

    let shorthand = runtime_live_expected_error(&json!({
        "runtimeLive": {
            "expectedError": "ProviderUnavailableError"
        }
    }))
    .unwrap()
    .expect("expected runtime error shorthand should parse");

    assert_eq!(shorthand.code(), "ProviderUnavailableError");
}

#[test]
fn runtime_live_expected_error_config_rejects_ambiguous_shapes() {
    assert!(runtime_live_expected_error(&json!({
        "runtimeLive": {
            "expectedError": {
                "code": ""
            }
        }
    }))
    .unwrap_err()
    .contains("code must be a non-empty string"));

    assert!(runtime_live_expected_error(&json!({
        "runtimeLive": {
            "expectedError": {
                "code": "ResourceLimitExceeded",
                "message": "wrong field"
            }
        }
    }))
    .unwrap_err()
    .contains("unknown field message"));
}
