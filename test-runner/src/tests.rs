use serde_json::json;

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

use super::doubles::{
    live_missing_config_skip_message, local_instance_config_path_for_default_service_db,
    read_runtime_test_inputs, service_db_mongo_url_from_config,
    service_db_mongo_url_from_router_config_path,
};
use super::runtime_process::{config_wrapped_for_router, RuntimeLiveMetadata};
use super::service::runtime_live_expected_error;
use super::SkiffTestOptions;

static NEXT_TEMP_ID: AtomicUsize = AtomicUsize::new(0);
static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvVarGuard {
    name: &'static str,
    value: Option<std::ffi::OsString>,
}

struct CurrentDirGuard {
    value: PathBuf,
}

impl EnvVarGuard {
    fn set(name: &'static str, value: &Path) -> Self {
        let guard = Self {
            name,
            value: env::var_os(name),
        };
        env::set_var(name, value);
        guard
    }

    fn remove(name: &'static str) -> Self {
        let guard = Self {
            name,
            value: env::var_os(name),
        };
        env::remove_var(name);
        guard
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        if let Some(value) = &self.value {
            env::set_var(self.name, value);
        } else {
            env::remove_var(self.name);
        }
    }
}

impl CurrentDirGuard {
    fn set(path: &Path) -> Self {
        let guard = Self {
            value: env::current_dir().unwrap(),
        };
        fs::create_dir_all(path).unwrap();
        env::set_current_dir(path).unwrap();
        guard
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        env::set_current_dir(&self.value).unwrap();
    }
}

fn temp_test_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "skiff-test-runner-{label}-{}-{}",
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::SeqCst)
    ))
}

fn write_test_file(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "test \"noop\" { assert true }\n").unwrap();
}

fn write_local_instance_config(project_root: &Path, text: &str) -> PathBuf {
    let instance_root = project_root.join(".skiff-instance");
    fs::create_dir_all(&instance_root).unwrap();
    let config_path = instance_root.join("config.yml");
    fs::write(&config_path, text).unwrap();
    config_path
}

fn write_router_service_db(dev_home: &Path, mongo_url: &str) {
    fs::create_dir_all(dev_home).unwrap();
    fs::write(
        dev_home.join("router.yml"),
        format!(
            r#"
profile: local
serviceDb:
  mongoUrl: "{mongo_url}"
"#
        ),
    )
    .unwrap();
}

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
fn default_service_db_uses_nearest_local_instance_from_input_file() {
    let temp = temp_test_dir("nearest-local-instance");
    let root = temp.join("repo");
    let service = root.join("services/account");
    let input = service.join("internal/session.test.skiff");
    write_test_file(&input);
    write_local_instance_config(&root, "devHome: root-home\n");
    write_router_service_db(
        &root.join(".skiff-instance/root-home"),
        "mongodb://127.0.0.1:27017/root",
    );
    write_local_instance_config(&service, "devHome: service-home\n");
    write_router_service_db(
        &service.join(".skiff-instance/service-home"),
        "mongodb://127.0.0.1:27017/service",
    );

    let inputs = read_runtime_test_inputs(&input, true, &SkiffTestOptions::default()).unwrap();

    assert_eq!(
        inputs.service_db_mongo_url.as_deref(),
        Some("mongodb://127.0.0.1:27017/service")
    );
    fs::remove_dir_all(&temp).unwrap();
}

#[test]
fn default_service_db_resolves_default_dev_home_under_instance_root() {
    let temp = temp_test_dir("default-dev-home");
    let project = temp.join("repo");
    let input = project.join("tests/storage.test.skiff");
    write_test_file(&input);
    write_local_instance_config(&project, "components:\n  mongo: disabled\n");
    write_router_service_db(
        &project.join(".skiff-instance/dev-home"),
        "mongodb://127.0.0.1:27017/default-dev-home",
    );

    let inputs = read_runtime_test_inputs(&input, true, &SkiffTestOptions::default()).unwrap();

    assert_eq!(
        inputs.service_db_mongo_url.as_deref(),
        Some("mongodb://127.0.0.1:27017/default-dev-home")
    );
    fs::remove_dir_all(&temp).unwrap();
}

#[test]
fn default_service_db_resolves_relative_dev_home_under_instance_root() {
    let temp = temp_test_dir("relative-dev-home");
    let project = temp.join("repo");
    let input = project.join("tests/storage.test.skiff");
    write_test_file(&input);
    write_local_instance_config(&project, "devHome: state/dev\n");
    write_router_service_db(
        &project.join(".skiff-instance/state/dev"),
        "mongodb://127.0.0.1:27017/relative-dev-home",
    );

    let inputs = read_runtime_test_inputs(&input, true, &SkiffTestOptions::default()).unwrap();

    assert_eq!(
        inputs.service_db_mongo_url.as_deref(),
        Some("mongodb://127.0.0.1:27017/relative-dev-home")
    );
    fs::remove_dir_all(&temp).unwrap();
}

#[test]
fn default_service_db_falls_back_to_cwd_local_instance_walk() {
    let temp = temp_test_dir("cwd-local-instance");
    let workspace = temp.join("workspace");
    let input_root = temp.join("external-package");
    let input = input_root.join("tests/storage.test.skiff");
    let cwd = workspace.join("packages/account");
    write_test_file(&input);
    fs::create_dir_all(&cwd).unwrap();
    let config_path = write_local_instance_config(&workspace, "devHome: dev-home\n");

    assert_eq!(
        local_instance_config_path_for_default_service_db(&input, true, Some(&cwd)).as_deref(),
        Some(config_path.as_path())
    );
    fs::remove_dir_all(&temp).unwrap();
}

#[test]
fn default_service_db_without_local_instance_does_not_use_home_fallback() {
    let _env_lock = ENV_LOCK.lock().unwrap();
    let temp = temp_test_dir("no-home-fallback");
    let project = temp.join("repo");
    let input = project.join("tests/storage.test.skiff");
    let cwd = temp.join("cwd-without-local-instance");
    let home = temp.join("home");
    write_test_file(&input);
    write_router_service_db(
        &home.join(".skiff").join("dev"),
        "mongodb://127.0.0.1:27017/legacy-home",
    );
    let _home = EnvVarGuard::set("HOME", &home);
    let _user_profile = EnvVarGuard::remove("USERPROFILE");

    let inputs = {
        let _cwd = CurrentDirGuard::set(&cwd);
        read_runtime_test_inputs(&input, true, &SkiffTestOptions::default()).unwrap()
    };

    assert_eq!(inputs.service_db_mongo_url, None);
    fs::remove_dir_all(&temp).unwrap();
}

#[test]
fn explicit_config_service_db_overrides_default_local_instance() {
    let temp = temp_test_dir("explicit-config-wins");
    let project = temp.join("repo");
    let input = project.join("tests/storage.test.skiff");
    let config_path = temp.join("test-config.json");
    write_test_file(&input);
    write_local_instance_config(&project, "devHome: dev-home\n");
    write_router_service_db(
        &project.join(".skiff-instance/dev-home"),
        "mongodb://127.0.0.1:27017/default-local-instance",
    );
    fs::write(
        &config_path,
        r#"{
  "serviceDb": {
    "mongoUrl": "mongodb://127.0.0.1:27017/explicit-config"
  }
}"#,
    )
    .unwrap();

    let inputs = read_runtime_test_inputs(
        &input,
        true,
        &SkiffTestOptions {
            config_path: Some(config_path),
            ..SkiffTestOptions::default()
        },
    )
    .unwrap();

    assert_eq!(
        inputs.service_db_mongo_url.as_deref(),
        Some("mongodb://127.0.0.1:27017/explicit-config")
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
