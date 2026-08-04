//! Golden corpus consumer for the frozen Router process config contract.
//!
//! Reads the same `corpus.json` + YAML fixtures as the TypeScript parser
//! (`router/tests/config-corpus.test.ts`) and asserts identical semantics:
//! valid cases resolve with the frozen normalization, invalid cases reject
//! with the frozen error regexes, and redaction follows the secret paths.

use std::fs;
use std::path::{Component, Path, PathBuf};

use regex::Regex;
use serde_json::Value;

use skiff_router::config::{
    load_router_config, redact_router_config, RouterConfig, ROUTER_CONFIG_REDACTED_VALUE,
    TELEMETRY_TOPICS,
};

const FIXTURES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/router-config");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corpus_schema_and_unique_case_names() {
        let corpus = corpus();
        assert_eq!(corpus["schemaVersion"], "skiff-router-config-corpus-v1");
        assert_eq!(
            corpus["systems"],
            Value::Array(vec![Value::String("router".into())])
        );
        let valid_names = corpus["valid"]
            .as_array()
            .expect("valid entries")
            .iter()
            .map(|entry| entry["name"].as_str().expect("valid name"))
            .collect::<Vec<_>>();
        let invalid_names = corpus["invalid"]
            .as_array()
            .expect("invalid entries")
            .iter()
            .map(|entry| entry["name"].as_str().expect("invalid name"))
            .collect::<Vec<_>>();
        assert_unique(&valid_names, "valid names");
        assert_unique(&invalid_names, "invalid names");
        assert!(
            valid_names.iter().all(|name| !invalid_names.contains(name)),
            "valid and invalid names must not overlap"
        );
        for entry in corpus["invalid"].as_array().expect("invalid entries") {
            assert!(
                entry["error"]
                    .as_str()
                    .is_some_and(|error| !error.is_empty()),
                "{} must declare an error regex",
                entry["name"]
            );
        }
    }

    #[test]
    fn accepts_all_valid_corpus_cases() {
        for entry in corpus()["valid"].as_array().expect("valid entries") {
            let path = fixture_path(entry["path"].as_str().expect("valid path"));
            let config = load_router_config(path.to_str().expect("utf8 path"))
                .unwrap_or_else(|error| panic!("{} must parse: {error}", entry["name"]));
            assert_normalized(entry["name"].as_str().expect("valid name"), &config);
        }
    }

    #[test]
    fn rejects_all_invalid_corpus_cases_with_frozen_errors() {
        for entry in corpus()["invalid"].as_array().expect("invalid entries") {
            let path = fixture_path(entry["path"].as_str().expect("invalid path"));
            let pattern = entry["error"].as_str().expect("invalid error regex");
            let regex = Regex::new(pattern).expect("corpus error must be a valid regex");
            let error = load_router_config(path.to_str().expect("utf8 path"))
                .err()
                .unwrap_or_else(|| {
                    panic!("{} must be rejected", entry["name"].as_str().expect("name"))
                });
            assert!(
                regex.is_match(&error.to_string()),
                "{} error {error:?} must match /{pattern}/",
                entry["name"].as_str().expect("name")
            );
        }
    }

    #[test]
    fn redacts_secret_leaves_without_mutating_the_parsed_config() {
        let fixtures = Path::new(FIXTURES_DIR);
        let direct_path = fixtures.join("valid/direct-secrets.yml");
        let direct = load_router_config(direct_path.to_str().expect("utf8 path"))
            .expect("direct-secrets must parse");
        let redacted = redact_router_config(&direct);
        assert_eq!(redacted.service_db.mongo_url, ROUTER_CONFIG_REDACTED_VALUE);
        let oss = redacted
            .file_backend
            .as_ref()
            .and_then(|backend| backend.oss.as_ref())
            .expect("direct-secrets oss");
        assert_eq!(
            oss.access_key_id.as_deref(),
            Some(ROUTER_CONFIG_REDACTED_VALUE)
        );
        assert_eq!(
            oss.access_key_secret.as_deref(),
            Some(ROUTER_CONFIG_REDACTED_VALUE)
        );
        assert_eq!(
            direct.service_db.mongo_url,
            "mongodb://user:pass@127.0.0.1:27017/skiff"
        );
        assert_eq!(
            redacted.artifacts_path, direct.artifacts_path,
            "redaction must not change non-secret fields"
        );
        assert_eq!(
            oss.endpoint, "https://oss-cn-hangzhou.aliyuncs.com",
            "redaction must keep non-secret OSS fields"
        );

        let env_path = fixtures.join("valid/file-backend.yml");
        let env = load_router_config(env_path.to_str().expect("utf8 path"))
            .expect("file-backend must parse");
        let redacted_env = redact_router_config(&env);
        assert_eq!(
            redacted_env.service_db.mongo_url,
            ROUTER_CONFIG_REDACTED_VALUE
        );
        let env_oss = redacted_env
            .file_backend
            .as_ref()
            .and_then(|backend| backend.oss.as_ref())
            .expect("file-backend oss");
        assert_eq!(
            env_oss.access_key_id_env.as_deref(),
            Some("SKIFF_OSS_ACCESS_KEY_ID"),
            "profile reference names must not be redacted"
        );
        assert_eq!(
            env_oss.access_key_secret_env.as_deref(),
            Some("SKIFF_OSS_ACCESS_KEY_SECRET"),
            "profile reference names must not be redacted"
        );
    }

    fn corpus() -> Value {
        let text = fs::read_to_string(Path::new(FIXTURES_DIR).join("corpus.json"))
            .expect("corpus.json must exist");
        serde_json::from_str(&text).expect("corpus.json must be valid JSON")
    }

    fn fixture_path(relative: &str) -> PathBuf {
        Path::new(FIXTURES_DIR).join(relative)
    }

    fn assert_unique(values: &[&str], label: &str) {
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), values.len(), "{label} must be unique");
    }

    /// Mirrors the TypeScript normalization table in
    /// `router/tests/config-corpus.test.ts`.
    fn assert_normalized(name: &str, config: &RouterConfig) {
        let valid_dir = Path::new(FIXTURES_DIR).join("valid");
        match name {
            "canonical" => {
                assert_eq!(config.profile, "dev");
                assert_eq!(config.host, "127.0.0.1");
                assert_eq!(
                    config.artifacts_path,
                    resolve(&valid_dir, "../var/skiff-artifacts")
                );
                assert_eq!(config.dev_reload, Some(true));
                assert_eq!(config.release_mode, Some(false));
                assert_eq!(config.request_timeout_ms, 20_000);
                assert_eq!(config.activation_prepare_timeout_ms, 120_000);
                assert_eq!(config.http_port, 4000);
                assert_eq!(config.http_max_request_bytes, 67_108_864);
                assert_eq!(config.http_max_response_bytes, 8_388_608);
                assert_eq!(config.runtime_port, 4001);
                assert_eq!(config.runtime_path, "/runtime");
                assert_eq!(config.runtime_max_concurrency, 256);
                assert_eq!(config.websocket_path, "/ws");
                assert_eq!(
                    config.service_db.mongo_url,
                    "mongodb://127.0.0.1:27017/?replicaSet=rs0"
                );
                let telemetry = config.telemetry.as_ref().expect("canonical telemetry");
                assert!(telemetry.enabled);
                assert_eq!(telemetry.endpoint, "ws://127.0.0.1:4002/telemetry");
                assert_eq!(telemetry.protocol, "skiff-telemetry-v1");
                assert_eq!(telemetry.topics, TELEMETRY_TOPICS.to_vec());
                assert_eq!(telemetry.queue_max_events, 10_000);
                assert_eq!(telemetry.batch_max_events, 200);
                assert_eq!(telemetry.batch_max_bytes, 262_144);
                assert_eq!(telemetry.flush_interval_ms, 1000);
                assert_eq!(config.rewrite.len(), 1);
                assert_eq!(config.rewrite[0].host, "account.localhost");
                assert_eq!(config.rewrite[0].path, None);
                assert_eq!(config.rewrite[0].service, "skiff.run/account");
                assert_eq!(config.rewrite[0].version.as_deref(), Some("0.1.0"));
            }
            "minimal" => {
                assert_eq!(config.profile, "dev");
                assert_eq!(config.host, "127.0.0.1");
                assert_eq!(config.artifacts_path, valid_dir.join("artifacts"));
                assert_eq!(config.http_port, 4000);
                assert_eq!(config.runtime_port, 4001);
                assert_eq!(config.runtime_path, "/runtime");
                assert_eq!(config.websocket_path, "/ws");
                assert_eq!(config.request_timeout_ms, 20_000);
                assert_eq!(config.activation_prepare_timeout_ms, 120_000);
                assert_eq!(
                    config.manifests,
                    vec![valid_dir.join("fixtures/hello/manifest.json")]
                );
                assert_eq!(config.runtime_max_concurrency, 1);
                assert_eq!(config.dev_reload, None);
                assert_eq!(config.release_mode, None);
                assert_eq!(config.telemetry, None);
                assert_eq!(config.file_backend, None);
                assert!(config.rewrite.is_empty());
            }
            "renderer-canonical" => {
                assert_eq!(config.profile, "dev");
                assert_eq!(config.host, "127.0.0.1");
                assert_eq!(config.artifacts_path, PathBuf::from("/tmp/skiff/artifacts"));
                assert_eq!(config.dev_reload, Some(true));
                assert_eq!(config.request_timeout_ms, 20_000);
                assert_eq!(config.activation_prepare_timeout_ms, 120_000);
                assert_eq!(config.http_port, 4000);
                assert_eq!(config.http_max_request_bytes, 67_108_864);
                assert_eq!(config.http_max_response_bytes, 8_388_608);
                assert_eq!(config.runtime_port, 4001);
                assert_eq!(config.runtime_path, "/runtime");
                assert_eq!(config.runtime_max_concurrency, 128);
            }
            "aliases" => {
                assert_eq!(config.profile, "staging");
                assert_eq!(config.http_port, 5010);
                assert_eq!(config.runtime_port, 5011);
                assert_eq!(config.runtime_path, "/runtime-dev");
                assert_eq!(config.websocket_path, "/socket");
                assert_eq!(config.manifests, vec![valid_dir.join("manifests/one.json")]);
                assert_eq!(config.runtime_max_concurrency, 2);
            }
            "manifests" => {
                assert_eq!(
                    config.manifests,
                    vec![
                        valid_dir.join("manifests/a.json"),
                        valid_dir.join("manifests/b.json"),
                    ]
                );
            }
            "telemetry" => {
                let telemetry = config.telemetry.as_ref().expect("telemetry");
                assert!(telemetry.enabled);
                assert_eq!(telemetry.endpoint, "ws://127.0.0.1:4002/telemetry");
                assert_eq!(telemetry.protocol, "skiff-telemetry-v1");
                assert_eq!(telemetry.topics, TELEMETRY_TOPICS.to_vec());
                assert_eq!(telemetry.queue_max_events, 5);
                assert_eq!(telemetry.batch_max_events, 3);
                assert_eq!(telemetry.batch_max_bytes, 1024);
                assert_eq!(telemetry.flush_interval_ms, 500);
            }
            "file-backend" => {
                let backend = config.file_backend.as_ref().expect("file backend");
                let local = backend.local.as_ref().expect("local backend");
                assert_eq!(local.root, resolve(&valid_dir, "../var/blobs"));
                let oss = backend.oss.as_ref().expect("oss backend");
                assert_eq!(oss.endpoint, "https://oss-cn-hangzhou.aliyuncs.com");
                assert_eq!(oss.bucket, "skiff-files");
                assert_eq!(oss.region.as_deref(), Some("cn-hangzhou"));
                assert_eq!(
                    oss.access_key_id_env.as_deref(),
                    Some("SKIFF_OSS_ACCESS_KEY_ID")
                );
                assert_eq!(
                    oss.access_key_secret_env.as_deref(),
                    Some("SKIFF_OSS_ACCESS_KEY_SECRET")
                );
                assert_eq!(oss.access_key_id, None);
                assert_eq!(oss.access_key_secret, None);
            }
            "direct-secrets" => {
                assert_eq!(
                    config.service_db.mongo_url,
                    "mongodb://user:pass@127.0.0.1:27017/skiff"
                );
                let oss = config
                    .file_backend
                    .as_ref()
                    .and_then(|backend| backend.oss.as_ref())
                    .expect("direct oss");
                assert_eq!(oss.access_key_id.as_deref(), Some("local-only-id"));
                assert_eq!(oss.access_key_secret.as_deref(), Some("local-only-secret"));
            }
            "rewrite" => {
                assert_eq!(config.rewrite.len(), 2);
                assert_eq!(config.rewrite[0].host, "account.localhost");
                assert_eq!(config.rewrite[0].path.as_deref(), Some("/api"));
                assert_eq!(config.rewrite[0].service, "skiff.run/account");
                assert_eq!(config.rewrite[0].version.as_deref(), Some("0.1.0"));
                assert_eq!(config.rewrite[1].host, "registry.localhost");
                assert_eq!(config.rewrite[1].path, None);
                assert_eq!(config.rewrite[1].service, "skiff.run/registry");
                assert_eq!(config.rewrite[1].version, None);
            }
            "numeric-strings" => {
                assert_eq!(config.http_port, 4000);
                assert_eq!(config.runtime_port, 4001);
                assert_eq!(config.request_timeout_ms, 7000);
                assert_eq!(config.http_max_request_bytes, 16_777_216);
                assert_eq!(config.http_max_response_bytes, 8_388_608);
            }
            _ => panic!("unexpected corpus valid case {name}"),
        }
    }

    /// Lexical `resolve(base, relative)` matching Node's `path.resolve` for
    /// absolute bases (no symlink resolution, `..` collapsed).
    fn resolve(base: &Path, relative: &str) -> PathBuf {
        let joined = if Path::new(relative).is_absolute() {
            PathBuf::from(relative)
        } else {
            base.join(relative)
        };
        let mut normalized = PathBuf::new();
        for component in joined.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if !normalized.pop() {
                        normalized.push("..");
                    }
                }
                other => normalized.push(other.as_os_str()),
            }
        }
        normalized
    }
}
