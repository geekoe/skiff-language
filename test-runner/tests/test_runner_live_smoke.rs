use std::{
    fs,
    path::Path,
    sync::{Mutex, MutexGuard},
};

mod common;
use common::{test_runner::assert_counts, TestDir};
use skiff_test_runner::{run_skiff_tests, run_skiff_tests_with_options, SkiffTestOptions};

static LIVE_SMOKE_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn package_live_smoke_missing_openai_key_is_skipped_when_explicit() {
    let temp = TestDir::new("skiff-packages", "live-smoke-skip");
    let live_test = write_openai_like_package(temp.path());
    let config_path = temp.path().join("config.json");
    fs::write(&config_path, "{}").unwrap();
    let summary = run_live_test(&live_test, config_path);

    assert_counts(&summary, 0, 2, 0);
    assert_message_contains(
        &summary,
        "live generate one image",
        "missing live config openai.apiKey",
    );
    assert_message_contains(
        &summary,
        "live edit one image",
        "missing live config openai.apiKey",
    );
}

#[test]
fn package_live_smoke_ignores_doubles_and_skips_generic_missing_key() {
    let temp = TestDir::new("skiff-packages", "package-live-smoke-generic-skip");
    let package_dir = temp.path().join("pkg");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/live
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(package_dir.join("api.yml"), "api: { marker: api.marker }\n").unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            function marker() -> bool {
                return true
            }
        "#,
    )
    .unwrap();
    let live_test = package_dir.join("api.live.test.skiff");
    fs::write(
        &live_test,
        r#"
            test "custom live key skips after runtime lookup" {
                const apiKey = config.require<string>("dashscope.apiKey")
                assert root.api.marker()
                assert apiKey != ""
            }
        "#,
    )
    .unwrap();
    fs::write(
        package_dir.join("skiff.test-doubles.json"),
        r#"
{
  "tests": {
    "custom live key skips after runtime lookup": {
      "app.fake.effect": {
        "response": null
      }
    }
  }
}
"#,
    )
    .unwrap();
    let config_path = temp.path().join("config.json");
    fs::write(&config_path, "{}").unwrap();
    let summary = run_live_test(&live_test, config_path);

    assert_counts(&summary, 0, 1, 0);
    assert_message_contains(
        &summary,
        "custom live key skips after runtime lookup",
        "missing live config dashscope.apiKey",
    );
    assert!(
        !format_summary(&summary).contains("invalid test double"),
        "{}",
        format_summary(&summary)
    );
}

#[test]
fn package_live_smoke_ignores_service_proxy_config() {
    let temp = TestDir::new("skiff-packages", "live-smoke-unsafe-proxy");
    let live_test = write_openai_like_package(temp.path());
    let config_path = temp.path().join("config.json");
    fs::write(
        &config_path,
        r#"
{
  "openai": {
    "apiKey": "sk-test"
  },
  "http": {
    "proxy": "http://127.0.0.1:9"
  }
}
"#,
    )
    .unwrap();
    let summary = run_live_test(&live_test, config_path);

    assert_eq!(summary.failed, 2, "{}", format_summary(&summary));
    assert_message_contains_any(
        &summary,
        "live generate one image",
        &["ProviderUnavailableError: connection failed"],
    );
    assert_message_absent(&summary, "http://127.0.0.1:9");
    assert_message_absent(&summary, "127.0.0.1");
    assert_message_absent(&summary, "sk-test");
}

fn assert_message_contains_any(
    summary: &skiff_test_runner::SkiffTestSummary,
    name: &str,
    messages: &[&str],
) {
    assert!(
        summary.results.iter().any(|result| {
            result.name == name
                && result
                    .message
                    .as_deref()
                    .is_some_and(|actual| messages.iter().any(|expected| actual.contains(expected)))
        }),
        "expected {name:?} message containing one of {messages:?}\n{}",
        format_summary(summary)
    );
}

#[test]
fn package_live_filename_without_live_flag_runs_normal_mode() {
    let temp = TestDir::new("skiff-packages", "live-filename-normal-mode");
    let live_test = write_openai_like_package(temp.path());
    let _guard = live_smoke_lock();
    let summary = run_skiff_tests(&live_test, None)
        .expect("live-named file should run in normal mode without --live");

    assert_eq!(summary.failed, 2, "{}", format_summary(&summary));
    assert_message_contains(
        &summary,
        "live generate one image",
        "path openai.apiKey required value is missing or null",
    );
    assert!(
        !format_summary(&summary).contains("live smoke test requires --live"),
        "{}",
        format_summary(&summary)
    );
}

#[test]
fn package_live_smoke_missing_dashscope_api_key_is_skipped() {
    let temp = TestDir::new("skiff-packages", "live-smoke-nested-dashscope-config");
    let live_test = write_llm_like_package(temp.path());
    let config_path = temp.path().join("config.json");
    fs::write(
        &config_path,
        r#"
{
  "dashscope": {
    "baseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1"
  }
}
"#,
    )
    .unwrap();
    let summary = run_live_test(&live_test, config_path);

    assert_counts(&summary, 0, 2, 0);
    assert_message_contains(
        &summary,
        "live chat returns assistant message",
        "missing live config dashscope.apiKey",
    );
    assert_message_contains(
        &summary,
        "live stream returns text",
        "missing live config dashscope.apiKey",
    );
}

#[test]
fn service_live_smoke_has_websocket_writer_for_explicit_file() {
    let temp = TestDir::new("skiff-test-runner", "service-live-websocket-writer");
    let service_dir = temp.path().join("service");
    fs::create_dir_all(service_dir.join("api")).unwrap();
    fs::write(
        service_dir.join("service.yml"),
        r#"
id: example.com/wstest
version: 1.0.0
"#,
    )
    .unwrap();
    let test_file = service_dir.join("api").join("socket.live.test.skiff");
    fs::write(
        &test_file,
        r#"
            import std

            test defaultRun false

            test "live websocket send has writer" {
                std.websocket.sendTextToBusinessIdentity("live-user-1", "hello")
                assert true
            }
        "#,
    )
    .unwrap();
    let config_path = temp.path().join("config.json");
    fs::write(&config_path, "{}").unwrap();

    let summary = run_live_test(&test_file, config_path);
    assert_counts(&summary, 1, 0, 0);
    assert_message_absent(&summary, "router writer is not available");
}

#[test]
fn service_http_stream_live_fixture_skips_without_bailian_key() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/http-stream-live/internal/http_stream_live.live.test.skiff");
    let temp = TestDir::new("skiff-test-runner", "http-stream-live-missing-config");
    let config_path = temp.path().join("config.json");
    fs::write(
        &config_path,
        r#"
{
  "bailian": {
    "baseUrl": "https://dashscope.aliyuncs.com/compatible-mode/v1"
  }
}
"#,
    )
    .unwrap();

    let summary = run_live_test(&fixture, config_path);

    assert_counts(&summary, 0, 1, 0);
    assert_message_contains(
        &summary,
        "live raw http stream route forwards OpenAI-compatible SSE",
        "service.bailian.apiKey",
    );
}

#[test]
fn package_live_smoke_requires_config_file_when_explicit() {
    let temp = TestDir::new("skiff-packages", "live-smoke-requires-config");
    let live_test = write_openai_like_package(temp.path());
    let error = run_skiff_tests_with_options(
        &live_test,
        None,
        &SkiffTestOptions {
            live: true,
            allow_network: true,
            config_path: None,
            ..SkiffTestOptions::default()
        },
    )
    .expect_err("explicit live tests require a config file")
    .to_string();

    assert!(error.contains("--live tests require --config <path>"));
}

fn write_openai_like_package(root: &Path) -> std::path::PathBuf {
    let package_dir = root.join("openailike");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/openailike
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.yml"),
        "api: { imageGenerate: api.imageGenerate, imageEdit: api.imageEdit }\n",
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            import std

            function apiKey() -> string {
                return config.require<string>("openai.apiKey")
            }

            function request() -> std.http.HttpClientRequest {
                return std.http.HttpClientRequest {
                    method: "GET",
                    url: "https://example.test/live",
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(apiKey()),
                    timeoutMs: null,
                }
            }

            function imageGenerate() -> number {
                const response = std.http.request(request())
                return response.status
            }

            function imageEdit() -> number {
                const response = std.http.request(request())
                return response.status
            }
        "#,
    )
    .unwrap();
    let live_test = package_dir.join("api.live.test.skiff");
    fs::write(
        &live_test,
        r#"
            test defaultRun false

            test "live generate one image" {
                assert root.api.imageGenerate() == 200
            }

            test "live edit one image" {
                assert root.api.imageEdit() == 200
            }
        "#,
    )
    .unwrap();
    live_test
}

fn write_llm_like_package(root: &Path) -> std::path::PathBuf {
    let package_dir = root.join("llmlike");
    fs::create_dir_all(&package_dir).unwrap();
    fs::write(
        package_dir.join("package.yml"),
        r#"
id: example.com/llmlike
version: 1.0.0
"#,
    )
    .unwrap();
    fs::write(
        package_dir.join("api.yml"),
        "api: { chat: api.chat, stream: api.stream }\n",
    )
    .unwrap();
    fs::write(
        package_dir.join("api.skiff"),
        r#"
            import std

            function apiKey() -> string {
                return config.require<string>("dashscope.apiKey")
            }

            function request() -> std.http.HttpClientRequest {
                return std.http.HttpClientRequest {
                    method: "GET",
                    url: "https://example.test/live",
                    headers: Array.empty<std.http.HttpHeader>(),
                    body: bytes.fromUtf8(apiKey()),
                    timeoutMs: null,
                }
            }

            function chat() -> number {
                const response = std.http.request(request())
                return response.status
            }

            function stream() -> number {
                const response = std.http.request(request())
                return response.status
            }
        "#,
    )
    .unwrap();
    let live_test = package_dir.join("api.live.test.skiff");
    fs::write(
        &live_test,
        r#"
            test defaultRun false

            test "live chat returns assistant message" {
                assert root.api.chat() == 200
            }

            test "live stream returns text" {
                assert root.api.stream() == 200
            }
        "#,
    )
    .unwrap();
    live_test
}

fn run_live_test(
    input: &Path,
    config_path: std::path::PathBuf,
) -> skiff_test_runner::SkiffTestSummary {
    let _guard = live_smoke_lock();
    run_skiff_tests_with_options(
        input,
        None,
        &SkiffTestOptions {
            live: true,
            allow_network: true,
            config_path: Some(config_path),
            ..SkiffTestOptions::default()
        },
    )
    .expect("live test should produce a runtime summary")
}

fn live_smoke_lock() -> MutexGuard<'static, ()> {
    LIVE_SMOKE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn assert_message_contains(
    summary: &skiff_test_runner::SkiffTestSummary,
    name: &str,
    message: &str,
) {
    assert!(
        summary.results.iter().any(|result| {
            result.name == name
                && result
                    .message
                    .as_deref()
                    .is_some_and(|actual| actual.contains(message))
        }),
        "expected {name:?} message containing {message:?}\n{}",
        format_summary(summary)
    );
}

fn assert_message_absent(summary: &skiff_test_runner::SkiffTestSummary, message: &str) {
    assert!(
        !format_summary(summary).contains(message),
        "expected no result message containing {message:?}\n{}",
        format_summary(summary)
    );
}

fn format_summary(summary: &skiff_test_runner::SkiffTestSummary) -> String {
    summary
        .results
        .iter()
        .map(|result| {
            let status = if result.skipped {
                "SKIP"
            } else if result.passed {
                "PASS"
            } else {
                "FAIL"
            };
            let message = result.message.as_deref().unwrap_or("");
            format!("{status} {}::{} {message}", result.module_path, result.name)
        })
        .collect::<Vec<_>>()
        .join("\n")
}
