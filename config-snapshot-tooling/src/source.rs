use std::{collections::BTreeMap, fs, path::Path, process::Command};

use serde_json::{Map, Number, Value};

use crate::error::{invalid, io_error};
use crate::ConfigSnapshotToolingResult;

pub type ServiceConfigLayers = BTreeMap<String, BTreeMap<String, Value>>;

pub fn load_service_config(
    service_root: &Path,
    profile: &str,
) -> ConfigSnapshotToolingResult<ServiceConfigLayers> {
    validate_profile(profile)?;
    let paths = [
        service_root.join("config.yml"),
        service_root.join(format!("config.{profile}.yml")),
        service_root.join(format!("config.{profile}.secret.yml")),
    ];
    if paths[2].exists() {
        verify_secret_file_is_ignored(&paths[2])?;
    }

    let mut merged = BTreeMap::new();
    for path in paths {
        let Some(layer) = read_optional_layer(&path)? else {
            continue;
        };
        overlay_config_maps(&mut merged, layer);
    }
    package_partitions(merged, service_root)
}

pub fn overlay_config_maps(base: &mut BTreeMap<String, Value>, overlay: BTreeMap<String, Value>) {
    for (key, incoming) in overlay {
        if incoming.is_null() {
            base.remove(&key);
            continue;
        }
        match (base.get_mut(&key), incoming) {
            (Some(Value::Object(existing)), Value::Object(incoming)) => {
                overlay_json_objects(existing, incoming);
            }
            (_, incoming) => {
                base.insert(key, incoming);
            }
        }
    }
}

pub fn verify_secret_file_is_ignored(path: &Path) -> ConfigSnapshotToolingResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|source| io_error("inspect secret config", path, source))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid(
            path,
            "secret config must be a regular file, not a symlink",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid(path, "secret config has no parent directory"))?;
    let repository = git_output(parent, &["rev-parse", "--show-toplevel"])?;
    let repository = fs::canonicalize(repository.trim())
        .map_err(|source| io_error("resolve git repository", repository.trim(), source))?;
    let canonical_path =
        fs::canonicalize(path).map_err(|source| io_error("resolve secret config", path, source))?;
    let relative = canonical_path.strip_prefix(&repository).map_err(|_| {
        invalid(
            path,
            format!(
                "secret config is outside its discovered git repository {}",
                repository.display()
            ),
        )
    })?;
    let relative = relative.to_string_lossy();
    if git_status(
        &repository,
        &["ls-files", "--error-unmatch", "--", relative.as_ref()],
    )? {
        return Err(invalid(
            path,
            "secret config is tracked by git; remove it from version control",
        ));
    }
    if !git_status(
        &repository,
        &[
            "check-ignore",
            "--quiet",
            "--no-index",
            "--",
            relative.as_ref(),
        ],
    )? {
        return Err(invalid(
            path,
            "secret config must be covered by the repository ignore rules",
        ));
    }
    Ok(())
}

fn read_optional_layer(
    path: &Path,
) -> ConfigSnapshotToolingResult<Option<BTreeMap<String, Value>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(io_error("inspect config", path, source)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid(path, "config source must be a regular file"));
    }
    let source =
        fs::read_to_string(path).map_err(|source| io_error("read config", path, source))?;
    if source.trim().is_empty() {
        return Ok(Some(BTreeMap::new()));
    }
    let yaml = serde_yaml::from_str::<serde_yaml::Value>(&source).map_err(|source| {
        crate::ConfigSnapshotToolingError::Yaml {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let json = yaml_to_json(yaml, path)?;
    let Value::Object(object) = json else {
        return Err(invalid(
            path,
            "config file root must be a mapping from canonical Package ID to local config",
        ));
    };
    Ok(Some(object.into_iter().collect()))
}

fn package_partitions(
    merged: BTreeMap<String, Value>,
    path: &Path,
) -> ConfigSnapshotToolingResult<ServiceConfigLayers> {
    merged
        .into_iter()
        .map(|(package_id, value)| {
            if package_id.trim().is_empty()
                || package_id.chars().any(char::is_whitespace)
                || !package_id.contains('/')
            {
                return Err(invalid(
                    path,
                    format!("config root key {package_id:?} is not a canonical Package ID"),
                ));
            }
            let Value::Object(config) = value else {
                return Err(invalid(
                    path,
                    format!("config partition {package_id} must be an object"),
                ));
            };
            Ok((package_id, config.into_iter().collect()))
        })
        .collect()
}

fn overlay_json_objects(base: &mut Map<String, Value>, overlay: Map<String, Value>) {
    for (key, incoming) in overlay {
        if incoming.is_null() {
            base.remove(&key);
            continue;
        }
        match (base.get_mut(&key), incoming) {
            (Some(Value::Object(existing)), Value::Object(incoming)) => {
                overlay_json_objects(existing, incoming);
            }
            (_, incoming) => {
                base.insert(key, incoming);
            }
        }
    }
}

fn yaml_to_json(value: serde_yaml::Value, path: &Path) -> ConfigSnapshotToolingResult<Value> {
    match value {
        serde_yaml::Value::Null => Ok(Value::Null),
        serde_yaml::Value::Bool(value) => Ok(Value::Bool(value)),
        serde_yaml::Value::Number(value) => {
            let number = if let Some(value) = value.as_i64() {
                Number::from(value)
            } else if let Some(value) = value.as_u64() {
                Number::from(value)
            } else if let Some(value) = value.as_f64() {
                Number::from_f64(value)
                    .ok_or_else(|| invalid(path, "config number must be finite"))?
            } else {
                return Err(invalid(path, "unsupported YAML number"));
            };
            Ok(Value::Number(number))
        }
        serde_yaml::Value::String(value) => Ok(Value::String(value)),
        serde_yaml::Value::Sequence(values) => values
            .into_iter()
            .map(|value| yaml_to_json(value, path))
            .collect::<ConfigSnapshotToolingResult<Vec<_>>>()
            .map(Value::Array),
        serde_yaml::Value::Mapping(values) => {
            let mut object = Map::new();
            for (key, value) in values {
                let serde_yaml::Value::String(key) = key else {
                    return Err(invalid(path, "config mapping keys must be strings"));
                };
                if object
                    .insert(key.clone(), yaml_to_json(value, path)?)
                    .is_some()
                {
                    return Err(invalid(path, format!("duplicate config key {key:?}")));
                }
            }
            Ok(Value::Object(object))
        }
        serde_yaml::Value::Tagged(_) => Err(invalid(path, "YAML tags are not supported in config")),
    }
}

fn validate_profile(profile: &str) -> ConfigSnapshotToolingResult<()> {
    if profile.is_empty()
        || profile == "."
        || profile == ".."
        || !profile
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(invalid(
            "<profile>",
            "config profile must be a canonical ASCII profile token",
        ));
    }
    Ok(())
}

fn git_output(cwd: &Path, args: &[&str]) -> ConfigSnapshotToolingResult<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|source| io_error("run git", cwd, source))?;
    if !output.status.success() {
        return Err(invalid(
            cwd,
            format!(
                "could not prove secret config ignore ownership: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| invalid(cwd, "git returned a non-UTF-8 repository path"))
}

fn git_status(cwd: &Path, args: &[&str]) -> ConfigSnapshotToolingResult<bool> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|source| io_error("run git", cwd, source))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(invalid(cwd, "git ignore verification failed")),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, process::Command};

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
        fs::write(
            repository.path().join("config.dev.secret.yml"),
            r#"
"example.com/service":
  nested:
    secret: value
  count: null
"#,
        )
        .unwrap();

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
        fs::write(&secret, "\"example.com/service\": { key: value }\n").unwrap();
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

    fn git(root: &std::path::Path, arguments: &[&str]) {
        assert!(Command::new("git")
            .args(arguments)
            .current_dir(root)
            .status()
            .unwrap()
            .success());
    }
}
