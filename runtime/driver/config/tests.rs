use std::{
    fs::{self, write},
    path::PathBuf,
};

use super::{
    prepare_runtime_home, skiff_file_tmp_dir, RuntimeFileConfig, DEFAULT_HTTP_RESPONSE_MAX_BYTES,
};

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "runtime-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&path).expect("temp dir should be created");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn config_rejects_top_level_artifact_key() {
    let temp = TempDir::new("config-artifact");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "artifact: service-assembly.json",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let error = RuntimeFileConfig::load(&config_path).expect_err("artifact key should be rejected");

    assert_eq!(
        error.to_string(),
        "runtime config no longer supports artifact; use artifactRoots for local runtime artifact load paths"
    );
}

#[test]
fn config_does_not_require_artifacts() {
    let temp = TempDir::new("config-root");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let config = RuntimeFileConfig::load(&config_path).expect("config should load");

    assert_eq!(config.runtime_home, temp.path.join(".runtime-home"));
    assert!(config.artifact_roots.is_empty());
}

#[test]
fn config_reads_runtime_artifact_roots() {
    let temp = TempDir::new("config-artifact-roots");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "artifactRoots:",
            "  - artifacts",
            "  - /var/lib/skiff/artifacts",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let config = RuntimeFileConfig::load(&config_path).expect("config should load");

    assert_eq!(
        config.artifact_roots,
        vec![
            temp.path.join("artifacts"),
            PathBuf::from("/var/lib/skiff/artifacts")
        ]
    );
}

#[test]
fn config_ignores_legacy_mongo_url() {
    let temp = TempDir::new("config-mongo-url");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "mongo-url: mongodb://global",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let config = RuntimeFileConfig::load(&config_path).expect("mongo-url should be ignored");

    assert_eq!(config.router, "ws://127.0.0.1:4001/runtime");
}

#[test]
fn config_reading_http_response_max_bytes_from_runtime_config() {
    let temp = TempDir::new("config-http-max");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "http:",
            "  response:",
            "    maxBytes: 12345",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let config = RuntimeFileConfig::load(&config_path).expect("config should load");

    assert_eq!(config.http_response_max_bytes, 12345);
}

#[test]
fn config_rejects_http_response_max_bytes_zero() {
    let temp = TempDir::new("config-http-max-zero");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "http:",
            "  response:",
            "    maxBytes: 0",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let error = RuntimeFileConfig::load(&config_path).expect_err("zero maxBytes should fail");

    assert_eq!(
        error.to_string(),
        "runtime config http.response.maxBytes must be greater than zero"
    );
}

#[test]
fn config_rejects_http_response_max_bytes_too_large() {
    let temp = TempDir::new("config-http-max-large");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "http:",
            "  response:",
            "    maxBytes: 18446744073709551616.0",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let error = RuntimeFileConfig::load(&config_path).expect_err("too large maxBytes should fail");

    assert_eq!(
        error.to_string(),
        "runtime config http.response.maxBytes must fit within system integer size"
    );
}

#[test]
fn config_defaults_http_response_max_bytes_when_missing() {
    let temp = TempDir::new("config-http-max-default");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let config = RuntimeFileConfig::load(&config_path).expect("config should load");
    assert_eq!(
        config.http_response_max_bytes,
        DEFAULT_HTTP_RESPONSE_MAX_BYTES
    );
}

#[test]
fn config_reads_runtime_http_egress_proxy() {
    let temp = TempDir::new("config-http-egress-proxy");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "http:",
            "  egress:",
            "    proxy: http://127.0.0.1:7897",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let config = RuntimeFileConfig::load(&config_path).expect("config should load");

    assert_eq!(
        config.http_egress_proxy.as_deref(),
        Some("http://127.0.0.1:7897/")
    );
}

#[test]
fn config_defaults_runtime_http_egress_proxy_when_missing() {
    let temp = TempDir::new("config-http-egress-proxy-default");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let config = RuntimeFileConfig::load(&config_path).expect("config should load");

    assert_eq!(config.http_egress_proxy, None);
}

#[test]
fn config_rejects_runtime_http_egress_proxy_without_http_scheme() {
    let temp = TempDir::new("config-http-egress-proxy-scheme");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "http:",
            "  egress:",
            "    proxy: socks5://127.0.0.1:7897",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let error = RuntimeFileConfig::load(&config_path).expect_err("proxy scheme should fail");

    assert_eq!(
        error.to_string(),
        "runtime config http.egress.proxy must use http or https scheme"
    );
}

#[test]
fn config_rejects_runtime_http_egress_proxy_invalid_url() {
    let temp = TempDir::new("config-http-egress-proxy-host");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "http:",
            "  egress:",
            "    proxy: http://",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let error = RuntimeFileConfig::load(&config_path).expect_err("proxy url should fail");

    assert_eq!(
        error.to_string(),
        "runtime config http.egress.proxy is invalid"
    );
}

#[test]
fn config_rejects_runtime_http_egress_proxy_with_non_string_value() {
    let temp = TempDir::new("config-http-egress-proxy-type");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "http:",
            "  egress:",
            "    proxy: 7897",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let error = RuntimeFileConfig::load(&config_path).expect_err("proxy type should fail");

    assert_eq!(
        error.to_string(),
        "runtime config http.egress.proxy must be a string"
    );
}

#[test]
fn config_rejects_top_level_artifacts_key() {
    let temp = TempDir::new("config-artifacts");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "artifacts: artifact-root",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let error =
        RuntimeFileConfig::load(&config_path).expect_err("artifacts list should be rejected");

    assert!(error.to_string().contains("no longer supports artifacts"));
}

#[test]
fn config_rejects_services_list() {
    let temp = TempDir::new("config-services");
    let config_path = temp.path.join("runtime.yml");
    write(
        &config_path,
        [
            "router: ws://127.0.0.1:4001/runtime",
            "runtime-home: .runtime-home",
            "services:",
            "  - artifact: service-assembly.json",
            "",
        ]
        .join("\n"),
    )
    .expect("config should be written");

    let error =
        RuntimeFileConfig::load(&config_path).expect_err("services list should be rejected");

    assert!(error.to_string().contains("no longer supports services"));
}

#[test]
fn runtime_home_persists_runtime_id() {
    let temp = TempDir::new("home");
    let runtime_id = prepare_runtime_home(&temp.path).expect("runtime home should prepare");
    let second_runtime_id =
        prepare_runtime_home(&temp.path).expect("runtime home should prepare again");

    assert_eq!(runtime_id, second_runtime_id);
    assert!(runtime_id.starts_with("runtime-"));
    assert!(temp.path.join("cache").join("artifacts").is_dir());
    assert!(temp.path.join("tmp").is_dir());
    assert!(skiff_file_tmp_dir(&temp.path).is_dir());
}

#[test]
fn runtime_home_cleans_skiff_file_tmp_on_start() {
    let temp = TempDir::new("home-skiff-file-tmp");
    let stale = skiff_file_tmp_dir(&temp.path).join("stale-upload");
    fs::create_dir_all(stale.parent().expect("stale file should have parent"))
        .expect("skiff-file tmp should be created");
    write(&stale, b"stale").expect("stale temp file should be written");

    prepare_runtime_home(&temp.path).expect("runtime home should prepare");

    assert!(skiff_file_tmp_dir(&temp.path).is_dir());
    assert!(!stale.exists());
}
