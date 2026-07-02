use std::fs;

use skiff_compiler::{read_service_config, read_service_config_with_profile};

mod common;
use common::TestDir;

#[test]
fn merges_base_and_profile_service_config_overlays() {
    let temp = TestDir::new("skiff-compiler", "service-config-overlay");
    let root = temp.path();
    fs::write(
        root.join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
timeout:
  default: 1000
  methods:
    ExampleService.base: 2000
    ExampleService.drop: 3000
websocket: internal.base.SocketEntry
"#,
    )
    .unwrap();
    fs::write(
        root.join("service.prod.yml"),
        r#"
timeout:
  default: 4000
  methods:
    ExampleService.profile: 2500
    ExampleService.drop: null
websocket: internal.prod.SocketEntry
"#,
    )
    .unwrap();

    let config = read_service_config_with_profile(root, Some("prod")).unwrap();

    assert_eq!(config.publication.id.as_str(), "example.com/example");
    assert!(config.publication.api.entries().next().is_none());
    assert_eq!(config.runtime.timeout.default, Some(4000));
    assert_eq!(config.runtime.timeout.methods["ExampleService.base"], 2000);
    assert_eq!(
        config.runtime.timeout.methods["ExampleService.profile"],
        2500
    );
    assert!(!config
        .runtime
        .timeout
        .methods
        .contains_key("ExampleService.drop"));
    assert_eq!(
        config.runtime.websocket.unwrap().target.as_deref(),
        Some("internal.prod.SocketEntry")
    );
}

#[test]
fn rejects_removed_service_transports_field() {
    let temp = TestDir::new("skiff-compiler", "service-config-transports-removed");
    let root = temp.path();
    fs::write(
        root.join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
transports:
  std: direct
"#,
    )
    .unwrap();

    let error = read_service_config(root).unwrap_err().to_string();
    assert!(
        error.contains("unknown field `transports`"),
        "unexpected error: {error}"
    );
}

#[test]
fn merges_http_response_limit_overlays() {
    let temp = TestDir::new("skiff-compiler", "service-http-overlay");
    let root = temp.path();
    fs::write(
        root.join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  response:
    maxBytes: 100
"#,
    )
    .unwrap();
    fs::write(
        root.join("service.prod.yml"),
        r#"
http:
  response:
    maxBytes: 200
"#,
    )
    .unwrap();
    let config = read_service_config_with_profile(root, Some("prod")).unwrap();

    let max_bytes = config
        .runtime
        .http
        .and_then(|http| http.response)
        .and_then(|response| response.max_bytes)
        .unwrap();
    assert_eq!(max_bytes, 200);
}

#[test]
fn rejects_invalid_http_response_limit_in_profile_overlay() {
    let temp = TestDir::new("skiff-compiler", "service-http-overlay-invalid");
    let root = temp.path();
    fs::write(
        root.join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
http:
  response:
    maxBytes: 100
"#,
    )
    .unwrap();
    fs::write(
        root.join("service.prod.yml"),
        r#"
http:
  response:
    maxBytes: 0
"#,
    )
    .unwrap();

    let error = read_service_config_with_profile(root, Some("prod")).unwrap_err();

    assert!(
        error.to_string().contains("http.response.maxBytes"),
        "unexpected error: {error}"
    );
}

#[test]
fn rejects_service_local_overlay_without_profile() {
    let temp = TestDir::new("skiff-compiler", "service-config-local-overlay");
    let root = temp.path();
    fs::write(
        root.join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
timeout:
  default: 3000
"#,
    )
    .unwrap();
    fs::write(
        root.join("service.local.yml"),
        r#"
timeout:
  default: 7000
"#,
    )
    .unwrap();

    let error = read_service_config_with_profile(root, None).unwrap_err();
    let message = error.to_string();

    assert!(
        message.contains("service.local.yml is no longer a supported service definition overlay"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("service.yml plus service.<profile>.yml"),
        "unexpected error: {message}"
    );
    assert!(
        message.contains("config.<profile>.secret.yml"),
        "unexpected error: {message}"
    );
}

#[test]
fn rejects_invalid_service_config_profile_names() {
    let temp = TestDir::new("skiff-compiler", "service-config-invalid-profile");
    let root = temp.path();
    fs::write(
        root.join("service.yml"),
        r#"
id: example.com/example
version: 1.0.0
"#,
    )
    .unwrap();

    let error = read_service_config_with_profile(root, Some("prod-us")).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("must match [A-Za-z_][A-Za-z0-9_]*"),
        "unexpected error: {error}"
    );
}
