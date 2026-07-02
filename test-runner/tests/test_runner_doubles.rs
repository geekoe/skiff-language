use std::{fs, path::Path};

mod common;
use common::{
    test_runner::{assert_counts, assert_failed, assert_passed, run_tests, run_tests_error},
    TestDir,
};
use skiff_test_runner::{run_skiff_tests_with_options, SkiffTestOptions};

fn write_service_config(service_dir: &Path) {
    fs::create_dir_all(service_dir).unwrap();
    fs::write(
        service_dir.join("service.yml"),
        r#"
id: example.com/test-doubles
version: 1.0.0
"#,
    )
    .unwrap();
}

fn write_marker_service(service_dir: &Path) {
    write_service_config(service_dir);
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::write(
        service_dir.join("api").join("calc.skiff"),
        r#"
            function marker() -> bool {
                return true
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("calc.test.skiff"),
        r#"
            test "uses doubles" {
                assert true
            }
        "#,
    )
    .unwrap();
}

#[test]
fn cli_test_fails_std_http_request_without_test_double() {
    let temp = TestDir::new("skiff-compiler", "test-http-double-missing");
    let service_dir = temp.path().join("service");
    fs::create_dir_all(&service_dir).unwrap();
    fs::write(
        service_dir.join("service.yml"),
        r#"
id: example.com/httpclient
version: 1.0.0
"#,
    )
    .unwrap();
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::write(
        service_dir.join("api").join("client.skiff"),
        r#"
            import std

            function postText(url: string, value: string) -> number {
                const response = std.http.request(std.http.HttpClientRequest {
                    method: "POST",
                    url: url,
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(value),
                    timeoutMs: null,
                })
                return response.status
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("client.test.skiff"),
        r#"
            import std

            function postText(url: string, value: string) -> number {
                const response = std.http.request(std.http.HttpClientRequest {
                    method: "POST",
                    url: url,
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(value),
                    timeoutMs: null,
                })
                return response.status
            }

            test "std.http.client.request needs an explicit test double" {
                assert postText("https://example.test/echo", "ping") == 202
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&service_dir);
    assert_counts(&summary, 0, 0, 1);
    assert_failed(
        &summary,
        "std.http.client.request needs an explicit test double",
        "no test double registered for std.http.client.request",
    );
}

#[test]
fn cli_test_live_flag_disables_std_http_test_double_policy_for_explicit_file() {
    let temp = TestDir::new("skiff-compiler", "test-live-explicit-policy");
    let service_dir = temp.path().join("service");
    write_service_config(&service_dir);
    fs::create_dir_all(service_dir.join("api")).unwrap();
    let config_path = temp.path().join("config.json");
    fs::write(&config_path, "{}").unwrap();
    let test_file = service_dir.join("api").join("client.live.test.skiff");
    fs::write(
        service_dir.join("api").join("client.skiff"),
        r#"
            function marker() -> bool {
                return true
            }
        "#,
    )
    .unwrap();
    fs::write(
        &test_file,
        r#"
            import std

            test defaultRun false

            test "live flag uses runtime network policy" {
                const response = std.http.request(std.http.HttpClientRequest {
                    method: "GET",
                    url: "http://127.0.0.1:1/blocked",
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(""),
                    timeoutMs: null,
                })
                assert response.status == 200
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("skiff.test-doubles.json"),
        r#"
{
  "tests": {
    "live flag uses runtime network policy": {
      "app.fake.effect": {
        "response": null
      }
    }
  }
}
"#,
    )
    .unwrap();

    let summary = run_skiff_tests_with_options(
        &test_file,
        None,
        &SkiffTestOptions {
            live: true,
            allow_network: true,
            config_path: Some(config_path),
            ..SkiffTestOptions::default()
        },
    )
    .expect("live test should run through the runtime");

    assert_counts(&summary, 0, 0, 1);
    assert_failed(
        &summary,
        "live flag uses runtime network policy",
        "std.http.request.url points to a blocked network target",
    );
    assert!(!common::test_runner::format_summary(&summary)
        .contains("no test double registered for std.http.client.request"));
}

#[test]
fn cli_test_uses_std_http_request_test_double_by_target_id() {
    let temp = TestDir::new("skiff-compiler", "test-http-double");
    let service_dir = temp.path().join("service");
    fs::create_dir_all(&service_dir).unwrap();
    fs::write(
        service_dir.join("service.yml"),
        r#"
id: example.com/httpclient
version: 1.0.0
"#,
    )
    .unwrap();
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::write(
        service_dir.join("api").join("client.skiff"),
        r#"
            import std

            function postText(url: string, value: string) -> number {
                const response = std.http.request(std.http.HttpClientRequest {
                    method: "POST",
                    url: url,
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(value),
                    timeoutMs: null,
                })
                return response.status
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("client.test.skiff"),
        r#"
            import std

            function postText(url: string, value: string) -> number {
                const response = std.http.request(std.http.HttpClientRequest {
                    method: "POST",
                    url: url,
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(value),
                    timeoutMs: null,
                })
                return response.status
            }

            test "std.http.client.request can be replaced by target id" {
                assert postText("https://example.test/echo", "ping") == 202
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("skiff.test-doubles.json"),
        r#"
{
  "tests": {
    "api.client::std.http.client.request can be replaced by target id": {
      "std.http.client.request": {
        "expectRequest": {
          "url": "https://example.test/echo",
          "body": { "__skiffBytesBase64": "cGluZw==" }
        },
        "response": {
          "status": 202,
          "headers": [],
          "body": "cG9uZw=="
        }
      }
    }
  }
}
"#,
    )
    .unwrap();

    let summary = run_tests(&service_dir);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(
        &summary,
        "std.http.client.request can be replaced by target id",
    );
}

#[test]
fn cli_test_rejects_legacy_std_http_response_body_double() {
    let temp = TestDir::new("skiff-compiler", "test-http-double-legacy-body");
    let service_dir = temp.path().join("service");
    write_marker_service(&service_dir);
    fs::write(
        service_dir.join("skiff.test-doubles.json"),
        r#"
{
  "tests": {
    "api.calc::uses doubles": {
      "std.http.client.request": {
        "response": {
          "status": 202,
          "headers": [],
          "body": { "tag": "text", "value": "pong" }
        }
      }
    }
  }
}
"#,
    )
    .unwrap();

    let error = run_tests_error(&service_dir);
    assert!(
        error.contains("std.http.client.request response.body.__skiffBytesBase64 is required"),
        "{error}"
    );
}

#[test]
fn cli_test_rejects_invalid_base64_std_http_response_body_double() {
    let temp = TestDir::new("skiff-compiler", "test-http-double-invalid-base64-body");
    let service_dir = temp.path().join("service");
    write_marker_service(&service_dir);
    fs::write(
        service_dir.join("skiff.test-doubles.json"),
        r#"
{
  "tests": {
    "api.calc::uses doubles": {
      "std.http.client.request": {
        "response": {
          "status": 202,
          "headers": [],
          "body": "not base64"
        }
      }
    }
  }
}
"#,
    )
    .unwrap();

    let error = run_tests_error(&service_dir);
    assert!(
        error.contains("std.http.client.request response.body must be valid base64"),
        "{error}"
    );
}

#[test]
fn cli_test_rejects_arbitrary_function_test_double_target() {
    let temp = TestDir::new("skiff-compiler", "test-double-arbitrary-target");
    let service_dir = temp.path().join("service");
    write_service_config(&service_dir);
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::write(
        service_dir.join("api").join("calc.skiff"),
        r#"
            function marker() -> bool {
                return true
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("calc.test.skiff"),
        r#"
            test "arbitrary function double is rejected" {
                assert true
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("skiff.test-doubles.json"),
        r#"
{
  "tests": {
    "api.calc::arbitrary function double is rejected": {
      "secretOffset": {
        "response": 40
      }
    }
  }
}
"#,
    )
    .unwrap();

    let error = run_tests_error(&service_dir);
    assert!(
        error.contains("unsupported test double target secretOffset"),
        "{error}"
    );
}

#[test]
fn cli_test_rejects_legacy_values_test_double_field() {
    let temp = TestDir::new("skiff-compiler", "test-double-legacy-values");
    let service_dir = temp.path().join("service");
    write_service_config(&service_dir);
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::write(
        service_dir.join("api").join("calc.skiff"),
        r#"
            function marker() -> bool {
                return true
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("calc.test.skiff"),
        r#"
            test "legacy values field is rejected" {
                assert true
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("skiff.test-doubles.json"),
        r#"
{
  "values": {}
}
"#,
    )
    .unwrap();

    let error = run_tests_error(&service_dir);
    assert!(error.contains("unknown field `values`"), "{error}");
}

#[test]
fn cli_test_clears_std_http_request_doubles_between_tests() {
    let temp = TestDir::new("skiff-compiler", "test-http-double-isolation");
    let service_dir = temp.path().join("service");
    fs::create_dir_all(&service_dir).unwrap();
    fs::write(
        service_dir.join("service.yml"),
        r#"
id: example.com/httpclient
version: 1.0.0
"#,
    )
    .unwrap();
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::write(
        service_dir.join("api").join("client.skiff"),
        r#"
            import std

            function postText(url: string, value: string) -> number {
                const response = std.http.request(std.http.HttpClientRequest {
                    method: "POST",
                    url: url,
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(value),
                    timeoutMs: null,
                })
                return response.status
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("client.test.skiff"),
        r#"
            import std

            function postText(url: string, value: string) -> number {
                const response = std.http.request(std.http.HttpClientRequest {
                    method: "POST",
                    url: url,
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(value),
                    timeoutMs: null,
                })
                return response.status
            }

            test "first test has std.http.client.request double" {
                assert postText("https://example.test/echo", "ping") == 202
            }

            test "second test starts without std.http.client.request double" {
                assert postText("https://example.test/echo", "ping") == 202
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("skiff.test-doubles.json"),
        r#"
{
  "tests": {
    "api.client::first test has std.http.client.request double": {
      "std.http.client.request": {
        "expectRequest": {
          "url": "https://example.test/echo",
          "body": { "__skiffBytesBase64": "cGluZw==" }
        },
        "response": {
          "status": 202,
          "headers": [],
          "body": "cG9uZw=="
        }
      }
    }
  }
}
"#,
    )
    .unwrap();

    let summary = run_tests(&service_dir);
    assert_counts(&summary, 1, 0, 1);
    assert_passed(&summary, "first test has std.http.client.request double");
    assert_failed(
        &summary,
        "second test starts without std.http.client.request double",
        "no test double registered for std.http.client.request",
    );
}

#[test]
fn cli_test_applies_test_specific_config_without_leaking() {
    let temp = TestDir::new("skiff-compiler", "test-specific-config");
    let service_dir = temp.path().join("service");
    fs::create_dir_all(&service_dir).unwrap();
    fs::write(
        service_dir.join("service.yml"),
        r#"
id: example.com/config
version: 1.0.0
"#,
    )
    .unwrap();
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::write(
        service_dir.join("api").join("calc.skiff"),
        r#"
            function configuredValue() -> string? {
                return config.optional<string>("app.value")
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("calc.test.skiff"),
        r#"
            test "first test starts without test config" {
                assert root.api.calc.configuredValue() == null
            }

            test "second test has test config" {
                assert root.api.calc.configuredValue() == "enabled"
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("skiff.test-doubles.json"),
        r#"
{
  "configs": {
    "api.calc::second test has test config": {
      "app": {
        "value": "enabled"
      }
    }
  }
}
"#,
    )
    .unwrap();

    let summary = run_tests(&service_dir);
    assert_counts(&summary, 2, 0, 0);
    assert_passed(&summary, "first test starts without test config");
    assert_passed(&summary, "second test has test config");
}

#[test]
fn cli_test_uses_ordered_std_http_request_sequence_double() {
    let temp = TestDir::new("skiff-compiler", "test-http-double-sequence");
    let service_dir = temp.path().join("service");
    fs::create_dir_all(&service_dir).unwrap();
    fs::write(
        service_dir.join("service.yml"),
        r#"
id: example.com/httpclient
version: 1.0.0
"#,
    )
    .unwrap();
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::write(
        service_dir.join("api").join("client.skiff"),
        r#"
            import std

            function postText(url: string, value: string) -> string {
                const response = std.http.request(std.http.HttpClientRequest {
                    method: "POST",
                    url: url,
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(value),
                    timeoutMs: null,
                })
                return response.body.toUtf8String()
            }

            function postPair() -> string {
                return postText("https://example.test/one", "first")
                    .concat(postText("https://example.test/two", "second"))
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("api").join("client.test.skiff"),
        r#"
            test "std.http.client.request can use ordered response sequence" {
                assert root.api.client.postPair() == "onetwo"
            }
        "#,
    )
    .unwrap();
    fs::write(
        service_dir.join("skiff.test-doubles.json"),
        r#"
{
  "tests": {
    "api.client::std.http.client.request can use ordered response sequence": {
      "std.http.client.request": {
        "sequence": [
          {
            "expectRequest": {
              "url": "https://example.test/one"
            },
            "response": {
              "status": 200,
              "headers": [],
              "body": "b25l"
            }
          },
          {
            "expectRequest": {
              "url": "https://example.test/two"
            },
            "response": {
              "status": 200,
              "headers": [],
              "body": "dHdv"
            }
          }
        ]
      }
    }
  }
}
"#,
    )
    .unwrap();

    let summary = run_tests(&service_dir);
    assert_counts(&summary, 1, 0, 0);
    assert_passed(
        &summary,
        "std.http.client.request can use ordered response sequence",
    );
}

#[test]
fn cli_test_package_fails_std_http_stream_without_test_double() {
    let temp = TestDir::new("skiff-compiler", "package-http-stream-double-missing");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/stream
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.yml"),
        "api: { sawStreamResponse: api.sawStreamResponse }\n",
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            import std

            function sawStreamResponse(url: string) -> bool {
                const response = std.http.stream(std.http.HttpClientRequest {
                    method: "GET",
                    url: url,
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(""),
                    timeoutMs: null,
                })
                return response.status >= 100
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "std.http.client.stream needs an explicit test double" {
                assert root.api.sawStreamResponse("https://example.test/stream")
            }
        "#,
    )
    .unwrap();

    let summary = run_tests(&package_dir.join("api.test.skiff"));
    assert_counts(&summary, 0, 0, 1);
    assert_failed(
        &summary,
        "std.http.client.stream needs an explicit test double",
        "no test double registered for std.http.client.stream",
    );
}

#[test]
fn cli_test_package_directory_rejects_unqualified_test_double_key() {
    let temp = TestDir::new("skiff-compiler", "package-test-double-key");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/math
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.yml"),
        "api: { publicAnswer: api.publicAnswer }\n",
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function publicAnswer() -> number {
                return 42
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.test.skiff"),
        r#"
            test "uses doubles" {
                assert root.api.publicAnswer() == 42
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join("skiff.test-doubles.json"),
        r#"
{
  "tests": {
    "uses doubles": {}
  }
}
"#,
    )
    .unwrap();

    let error = run_tests_error(&package_dir);
    assert!(error.contains("directory test doubles must use fully qualified test keys"));
}
