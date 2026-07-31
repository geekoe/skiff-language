use std::{collections::BTreeMap, fs, process::Command};

#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};

use serde_json::json;
use tempfile::TempDir;

use super::{load_service_config, overlay_config_maps};

#[test]
fn overlay_recurses_replaces_sequences_and_uses_null_tombstones() {
    let mut base = BTreeMap::from([
        (
            "example.com/service".to_string(),
            json!({
                "nested": {"keep": true, "replace": "base", "delete": "old"},
                "sequence": [1, 2],
                "scalar": "base"
            }),
        ),
        ("example.com/removed".to_string(), json!({"value": true})),
    ]);
    overlay_config_maps(
        &mut base,
        BTreeMap::from([
            (
                "example.com/service".to_string(),
                json!({
                    "nested": {"replace": "profile", "delete": null},
                    "sequence": ["new"],
                    "scalar": {"now": "object"}
                }),
            ),
            ("example.com/removed".to_string(), json!(null)),
        ]),
    );
    assert_eq!(
        base,
        BTreeMap::from([(
            "example.com/service".to_string(),
            json!({
                "nested": {"keep": true, "replace": "profile"},
                "sequence": ["new"],
                "scalar": {"now": "object"}
            }),
        )])
    );
}

#[test]
fn loader_applies_base_profile_and_ignored_secret_without_wrapper_keys() {
    let repository = git_repository();
    fs::write(
        repository.path().join(".gitignore"),
        "config.*.secret.yml\n",
    )
    .unwrap();
    fs::write(
        repository.path().join("config.yml"),
        r#"
"example.com/service":
  nested:
    keep: true
    replace: base
  sequence: [base]
  count: 1
"#,
    )
    .unwrap();
    fs::write(
        repository.path().join("config.dev.yml"),
        r#"
"example.com/service":
  nested:
    replace: profile
  sequence:
    - profile
  count: 2
"#,
    )
    .unwrap();
    write_secret(
        &repository.path().join("config.dev.secret.yml"),
        r#"
"example.com/service":
  nested:
    secret: value
  count: null
"#,
    );

    let loaded = load_service_config(repository.path(), "dev").unwrap();
    assert_eq!(
        loaded["example.com/service"],
        json!({
            "nested": {
                "keep": true,
                "replace": "profile",
                "secret": "value"
            },
            "sequence": ["profile"]
        })
        .as_object()
        .unwrap()
        .clone()
        .into_iter()
        .collect()
    );
}

#[test]
fn loader_rejects_unignored_or_tracked_secret_and_non_package_root_keys() {
    let repository = git_repository();
    let secret = repository.path().join("config.dev.secret.yml");
    write_secret(&secret, "\"example.com/service\": { key: value }\n");
    let error = load_service_config(repository.path(), "dev").unwrap_err();
    assert!(error.to_string().contains("ignore rules"));

    fs::write(
        repository.path().join(".gitignore"),
        "config.*.secret.yml\n",
    )
    .unwrap();
    git(repository.path(), &["add", "-f", "config.dev.secret.yml"]);
    let error = load_service_config(repository.path(), "dev").unwrap_err();
    assert!(error.to_string().contains("tracked by git"));

    git(
        repository.path(),
        &["rm", "--cached", "config.dev.secret.yml"],
    );
    fs::remove_file(&secret).unwrap();
    fs::write(
        repository.path().join("config.yml"),
        "config:\n  key: value\n",
    )
    .unwrap();
    let error = load_service_config(repository.path(), "dev").unwrap_err();
    assert!(error.to_string().contains("canonical Package ID"));
}

#[cfg(unix)]
#[test]
fn loader_rejects_insecure_secret_mode_symlinks_and_non_regular_paths() {
    let repository = git_repository();
    fs::write(
        repository.path().join(".gitignore"),
        "config.*.secret.yml\n",
    )
    .unwrap();
    let secret = repository.path().join("config.dev.secret.yml");
    fs::write(
        &secret,
        "\"example.com/service\": { apiKey: must-not-leak }\n",
    )
    .unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
    let error = load_service_config(repository.path(), "dev")
        .unwrap_err()
        .to_string();
    assert!(error.contains("chmod 600"), "{error}");
    assert!(!error.contains("must-not-leak"), "{error}");

    fs::set_permissions(&secret, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(load_service_config(repository.path(), "dev").is_ok());

    fs::remove_file(&secret).unwrap();
    let target = repository.path().join("real-secret.yml");
    write_secret(&target, "\"example.com/service\": { apiKey: linked }\n");
    symlink(&target, &secret).unwrap();
    let error = load_service_config(repository.path(), "dev")
        .unwrap_err()
        .to_string();
    assert!(error.contains("regular file, not a symlink"), "{error}");

    fs::remove_file(&secret).unwrap();
    fs::create_dir(&secret).unwrap();
    let error = load_service_config(repository.path(), "dev")
        .unwrap_err()
        .to_string();
    assert!(error.contains("regular file, not a symlink"), "{error}");
}

#[test]
fn loader_rejects_duplicate_author_keys_in_one_layer() {
    let repository = git_repository();
    fs::write(
        repository.path().join("config.yml"),
        r#"
"example.com/service":
  value: first
"example.com/service":
  value: second
"#,
    )
    .unwrap();
    let error = load_service_config(repository.path(), "dev").unwrap_err();
    assert!(error.to_string().contains("duplicate"));
}

fn git_repository() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]);
    root
}

fn write_secret(path: &std::path::Path, contents: &str) {
    fs::write(path, contents).unwrap();
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn git(root: &std::path::Path, arguments: &[&str]) {
    assert!(Command::new("git")
        .args(arguments)
        .current_dir(root)
        .status()
        .unwrap()
        .success());
}
