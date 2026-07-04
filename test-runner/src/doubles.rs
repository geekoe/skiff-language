use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::{
    sources::{find_package_root, should_skip_package_dir},
    types::TestEffectDouble,
    SkiffTestError, SkiffTestOptions,
};

const LOCAL_INSTANCE_CONFIG_DIR: &str = ".skiff-instance";
const LOCAL_INSTANCE_CONFIG_FILE: &str = "config.yml";
const DEFAULT_LOCAL_INSTANCE_DEV_HOME: &str = "dev-home";
const TEST_DOUBLES_FILE: &str = "skiff.test-doubles.json";
const STD_HTTP_REQUEST_TARGET: &str = "std.http.client.request";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TestDoublesManifest {
    #[serde(default = "empty_json_object")]
    pub(crate) config: JsonValue,
    #[serde(default)]
    pub(crate) configs: HashMap<String, JsonValue>,
    #[serde(default)]
    pub(crate) tests: HashMap<String, HashMap<String, TestDoubleDefinition>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalInstanceConfig {
    dev_home: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum TestDoubleDefinition {
    Single(SingleTestDoubleDefinition),
    Sequence(SequenceTestDoubleDefinition),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SingleTestDoubleDefinition {
    expect_request: Option<JsonValue>,
    response: JsonValue,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SequenceTestDoubleDefinition {
    sequence: Vec<SingleTestDoubleDefinition>,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeTestDoubles {
    pub(crate) config: JsonValue,
    pub(crate) configs: HashMap<String, JsonValue>,
    pub(crate) service_db_mongo_url: Option<String>,
    pub(crate) tests: HashMap<String, HashMap<String, Vec<TestEffectDouble>>>,
}

pub(crate) fn live_missing_config_skip_message(message: &str) -> Option<String> {
    let missing = if message.starts_with("provider unavailable for ") {
        let (_, missing) = message.split_once(": missing ")?;
        missing.trim()
    } else if let Some(missing) = missing_config_path_from_diagnostic(message) {
        missing
    } else {
        return None;
    };
    let missing = missing.trim();
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "missing live config {missing}; skipping live smoke test"
    ))
}

fn missing_config_path_from_diagnostic(message: &str) -> Option<&str> {
    for line in message.lines() {
        let candidate = runtime_error_message_segment(line.trim());
        if let Some(missing) = missing_config_path_from_decode_error(candidate, "config.require") {
            return Some(missing);
        }
        if let Some(missing) = missing_config_path_from_decode_error(candidate, "config.get") {
            return Some(missing);
        }
        if let Some(missing) = missing_config_path_from_required_error(candidate) {
            return Some(missing);
        }
    }
    None
}

fn runtime_error_message_segment(line: &str) -> &str {
    let line = line.strip_prefix("DecodeError: ").unwrap_or(line);
    line.split_once("DecodeError: ")
        .map(|(_, message)| message)
        .unwrap_or(line)
}

fn missing_config_path_from_decode_error<'a>(message: &'a str, target: &str) -> Option<&'a str> {
    let prefix = format!("decode error for {target}: path ");
    let rest = message.strip_prefix(&prefix)?;
    missing_config_path_from_required_suffix(rest)
}

fn missing_config_path_from_required_error(message: &str) -> Option<&str> {
    if let Some(rest) = message
        .find("final resolvedConfig ")
        .and_then(|index| message[index..].strip_prefix("final resolvedConfig "))
        .and_then(|rest| rest.strip_suffix(" is required"))
    {
        return Some(rest.trim());
    }
    if let Some(rest) = message
        .find("configShape entry path ")
        .and_then(|index| message[index..].strip_prefix("configShape entry path "))
    {
        return missing_config_path_from_required_suffix(rest);
    }
    let rest = message.strip_prefix("path ")?;
    missing_config_path_from_required_suffix(rest)
}

fn missing_config_path_from_required_suffix(message: &str) -> Option<&str> {
    message
        .strip_suffix(" required value is missing or null")
        .map(str::trim)
}

fn read_runtime_test_doubles(
    input: &Path,
    input_is_file: bool,
) -> Result<RuntimeTestDoubles, SkiffTestError> {
    if !input_is_file {
        return read_runtime_test_doubles_from_dir(input);
    }

    let mut paths = Vec::new();
    if let Some(package_root) = find_package_root(input, true) {
        if let Some(shared_path) = shared_package_test_doubles_path(&package_root) {
            paths.push(shared_path);
        }
    }
    if let Some(path) = test_doubles_manifest_path(input, input_is_file) {
        if !paths.iter().any(|existing| existing == &path) {
            paths.push(path);
        }
    }

    let mut config = empty_json_object();
    let mut configs = HashMap::new();
    let mut tests = HashMap::new();
    for path in paths {
        let Some(manifest) = read_test_doubles_manifest_path(&path)? else {
            continue;
        };
        merge_json_object(&mut config, manifest.config);
        for (test_name, test_config) in
            runtime_test_configs_from_manifest(&path, manifest.configs, false)?
        {
            configs.insert(test_name, test_config);
        }
        for (test_name, doubles) in runtime_test_doubles_from_manifest(&path, manifest.tests)? {
            tests.insert(test_name, doubles);
        }
    }
    let service_db_mongo_url = service_db_mongo_url_from_config(&config).map_err(|message| {
        SkiffTestError::InvalidTestDouble {
            path: input.display().to_string(),
            message,
        }
    })?;
    Ok(RuntimeTestDoubles {
        config,
        configs,
        service_db_mongo_url,
        tests,
    })
}

fn read_runtime_test_doubles_from_dir(input: &Path) -> Result<RuntimeTestDoubles, SkiffTestError> {
    let mut paths = Vec::new();
    if let Some(shared_path) = shared_package_test_doubles_path(input) {
        paths.push(shared_path);
    }
    collect_test_doubles_paths(input, &mut paths)?;
    paths.sort();
    let mut config = empty_json_object();
    let mut configs = HashMap::new();
    let mut tests = HashMap::new();
    for path in paths {
        let text = fs::read_to_string(&path).map_err(|source| SkiffTestError::ReadTestDoubles {
            path: path.display().to_string(),
            source,
        })?;
        let manifest: TestDoublesManifest =
            serde_json::from_str(&text).map_err(|source| SkiffTestError::ParseTestDoubles {
                path: path.display().to_string(),
                source,
            })?;
        merge_json_object(&mut config, manifest.config);
        for (test_name, test_config) in
            runtime_test_configs_from_manifest(&path, manifest.configs, true)?
        {
            if configs.contains_key(&test_name) {
                return Err(SkiffTestError::InvalidTestDouble {
                    path: path.display().to_string(),
                    message: format!("duplicate test config key {test_name}"),
                });
            }
            configs.insert(test_name, test_config);
        }
        for (test_name, doubles) in manifest.tests {
            if !test_name.contains("::") {
                return Err(SkiffTestError::InvalidTestDouble {
                    path: path.display().to_string(),
                    message: format!(
                        "directory test doubles must use fully qualified test keys, got {test_name}"
                    ),
                });
            }
            if tests.contains_key(&test_name) {
                return Err(SkiffTestError::InvalidTestDouble {
                    path: path.display().to_string(),
                    message: format!("duplicate test double key {test_name}"),
                });
            }
            let doubles = runtime_test_double_targets_from_manifest(&path, doubles)?;
            tests.insert(test_name, doubles);
        }
    }
    let service_db_mongo_url = service_db_mongo_url_from_config(&config).map_err(|message| {
        SkiffTestError::InvalidTestDouble {
            path: input.display().to_string(),
            message,
        }
    })?;
    Ok(RuntimeTestDoubles {
        config,
        configs,
        service_db_mongo_url,
        tests,
    })
}

fn shared_package_test_doubles_path(input: &Path) -> Option<PathBuf> {
    let package_root = find_package_root(input, false)?;
    if package_root != input {
        return None;
    }
    let path = package_root.parent()?.join(TEST_DOUBLES_FILE);
    path.is_file().then_some(path)
}

pub(crate) fn read_runtime_test_inputs(
    input: &Path,
    input_is_file: bool,
    options: &SkiffTestOptions,
) -> Result<RuntimeTestDoubles, SkiffTestError> {
    let mut inputs = if options.live {
        RuntimeTestDoubles {
            config: empty_json_object(),
            configs: HashMap::new(),
            service_db_mongo_url: None,
            tests: HashMap::new(),
        }
    } else {
        read_runtime_test_doubles(input, input_is_file)?
    };
    if let Some(config_path) = &options.config_path {
        let (config, configs) = read_config_snapshot(config_path)?;
        inputs.config = config;
        inputs.configs = configs;
        inputs.service_db_mongo_url =
            service_db_mongo_url_from_config(&inputs.config).map_err(|message| {
                SkiffTestError::InvalidTestDouble {
                    path: config_path.display().to_string(),
                    message,
                }
            })?;
    } else if options.live {
        return Err(SkiffTestError::RuntimeSetup {
            message: "--live tests require --config <path>".to_string(),
        });
    } else if inputs.service_db_mongo_url.is_none() {
        inputs.service_db_mongo_url =
            read_default_dev_router_service_db_mongo_url(input, input_is_file)?;
    }
    Ok(inputs)
}

fn runtime_test_configs_from_manifest(
    path: &Path,
    configs: HashMap<String, JsonValue>,
    require_qualified_keys: bool,
) -> Result<HashMap<String, JsonValue>, SkiffTestError> {
    configs
        .into_iter()
        .map(|(test_name, config)| {
            if require_qualified_keys && !test_name.contains("::") {
                return Err(SkiffTestError::InvalidTestDouble {
                    path: path.display().to_string(),
                    message: format!(
                        "directory test configs must use fully qualified test keys, got {test_name}"
                    ),
                });
            }
            if !config.is_object() {
                return Err(SkiffTestError::InvalidTestDouble {
                    path: path.display().to_string(),
                    message: format!("test config for {test_name} must be a JSON object"),
                });
            }
            Ok((test_name, config))
        })
        .collect()
}

pub(crate) fn service_db_mongo_url_from_config(
    config: &JsonValue,
) -> Result<Option<String>, String> {
    let Some(service_db) = config.get("serviceDb") else {
        return Ok(None);
    };
    if service_db.is_null() {
        return Ok(None);
    }
    let Some(object) = service_db.as_object() else {
        return Err("config serviceDb must be a JSON object".to_string());
    };
    match object.get("mongoUrl") {
        Some(JsonValue::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        Some(JsonValue::String(_)) => {
            Err("config serviceDb.mongoUrl must be a non-empty string".to_string())
        }
        Some(_) => Err("config serviceDb.mongoUrl must be a string".to_string()),
        None => Err("config serviceDb.mongoUrl is required when serviceDb is present".to_string()),
    }
}

pub(crate) fn service_db_mongo_url_for_runtime_test(
    default_mongo_url: Option<&str>,
    effective_config: &JsonValue,
) -> Result<Option<String>, String> {
    if effective_config.get("serviceDb").is_some() {
        return service_db_mongo_url_from_config(effective_config);
    }
    Ok(default_mongo_url.map(str::to_string))
}

fn runtime_test_doubles_from_manifest(
    path: &Path,
    tests: HashMap<String, HashMap<String, TestDoubleDefinition>>,
) -> Result<HashMap<String, HashMap<String, Vec<TestEffectDouble>>>, SkiffTestError> {
    tests
        .into_iter()
        .map(|(test_name, doubles)| {
            Ok((
                test_name,
                runtime_test_double_targets_from_manifest(path, doubles)?,
            ))
        })
        .collect()
}

fn runtime_test_double_targets_from_manifest(
    path: &Path,
    doubles: HashMap<String, TestDoubleDefinition>,
) -> Result<HashMap<String, Vec<TestEffectDouble>>, SkiffTestError> {
    doubles
        .into_iter()
        .map(|(target, double)| {
            let sequence = runtime_test_double_sequence_from_manifest(path, &target, double)?;
            Ok((target, sequence))
        })
        .collect()
}

fn runtime_test_double_sequence_from_manifest(
    path: &Path,
    target: &str,
    double: TestDoubleDefinition,
) -> Result<Vec<TestEffectDouble>, SkiffTestError> {
    let sequence = match double {
        TestDoubleDefinition::Single(single) => vec![single],
        TestDoubleDefinition::Sequence(sequence) => {
            if sequence.sequence.is_empty() {
                return Err(SkiffTestError::InvalidTestDouble {
                    path: path.display().to_string(),
                    message: format!("test double sequence for {target} must not be empty"),
                });
            }
            sequence.sequence
        }
    };
    sequence
        .into_iter()
        .map(|step| {
            validate_runtime_test_double(target, &step.response).map_err(|message| {
                SkiffTestError::InvalidTestDouble {
                    path: path.display().to_string(),
                    message,
                }
            })?;
            Ok(TestEffectDouble {
                expect_request: step.expect_request,
                response: step.response,
            })
        })
        .collect()
}

fn collect_test_doubles_paths(root: &Path, paths: &mut Vec<PathBuf>) -> Result<(), SkiffTestError> {
    for entry in fs::read_dir(root).map_err(|source| SkiffTestError::ReadTestDoubles {
        path: root.display().to_string(),
        source,
    })? {
        let entry = entry.map_err(|source| SkiffTestError::ReadTestDoubles {
            path: root.display().to_string(),
            source,
        })?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|source| SkiffTestError::ReadTestDoubles {
                path: path.display().to_string(),
                source,
            })?;
        if file_type.is_dir() {
            if !should_skip_package_dir(&path) {
                collect_test_doubles_paths(&path, paths)?;
            }
        } else if file_type.is_file()
            && path.file_name().and_then(|name| name.to_str()) == Some(TEST_DOUBLES_FILE)
        {
            paths.push(path);
        }
    }
    Ok(())
}

fn merge_json_object(target: &mut JsonValue, next: JsonValue) {
    let (Some(target), Some(next)) = (target.as_object_mut(), next.as_object()) else {
        return;
    };
    for (key, value) in next {
        if let Some(existing) = target.get_mut(key) {
            if existing.is_object() && value.is_object() {
                merge_json_object(existing, value.clone());
                continue;
            }
        }
        target.insert(key.clone(), value.clone());
    }
}

fn read_config_snapshot(
    path: &Path,
) -> Result<(JsonValue, HashMap<String, JsonValue>), SkiffTestError> {
    let text = fs::read_to_string(path).map_err(|source| SkiffTestError::ReadTestDoubles {
        path: path.display().to_string(),
        source,
    })?;
    let mut config = parse_config_snapshot(path, &text)?;
    let Some(config_object) = config.as_object_mut() else {
        return Err(SkiffTestError::InvalidTestDouble {
            path: path.display().to_string(),
            message: "config snapshot must be a JSON object".to_string(),
        });
    };
    let configs = match config_object.remove("configs") {
        Some(value) => {
            let configs: HashMap<String, JsonValue> =
                serde_json::from_value(value).map_err(|source| {
                    SkiffTestError::ParseTestDoubles {
                        path: path.display().to_string(),
                        source,
                    }
                })?;
            runtime_test_configs_from_manifest(path, configs, false)?
        }
        None => HashMap::new(),
    };
    Ok((config, configs))
}

fn parse_config_snapshot(path: &Path, text: &str) -> Result<JsonValue, SkiffTestError> {
    if has_yaml_extension(path) {
        return parse_yaml_config_snapshot(path, text);
    }
    match serde_json::from_str::<JsonValue>(text) {
        Ok(value) => Ok(value),
        Err(json_source) => {
            if path.extension().is_some() {
                return Err(SkiffTestError::ParseTestDoubles {
                    path: path.display().to_string(),
                    source: json_source,
                });
            }
            parse_yaml_config_snapshot(path, text)
        }
    }
}

fn parse_yaml_config_snapshot(path: &Path, text: &str) -> Result<JsonValue, SkiffTestError> {
    serde_yaml::from_str::<JsonValue>(text).map_err(|source| SkiffTestError::InvalidTestDouble {
        path: path.display().to_string(),
        message: format!("failed to parse config snapshot as YAML: {source}"),
    })
}

fn has_yaml_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yaml" | "yml")
    )
}

fn read_default_dev_router_service_db_mongo_url(
    input: &Path,
    input_is_file: bool,
) -> Result<Option<String>, SkiffTestError> {
    let cwd = env::current_dir().ok();
    let Some(config_path) =
        local_instance_config_path_for_default_service_db(input, input_is_file, cwd.as_deref())
    else {
        return Ok(None);
    };
    let dev_home = dev_home_from_local_instance_config_path(&config_path)?;
    let router_config_path = dev_home.join("router.yml");
    if !router_config_path.is_file() {
        return Ok(None);
    }
    service_db_mongo_url_from_router_config_path(&router_config_path)
}

pub(crate) fn local_instance_config_path_for_default_service_db(
    input: &Path,
    input_is_file: bool,
    cwd: Option<&Path>,
) -> Option<PathBuf> {
    let input_start = default_service_db_input_start_dir(input, input_is_file);
    find_local_instance_config_upward(&input_start)
        .or_else(|| cwd.and_then(find_local_instance_config_upward))
}

fn default_service_db_input_start_dir(input: &Path, input_is_file: bool) -> PathBuf {
    if !input_is_file {
        return input.to_path_buf();
    }
    input
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

fn find_local_instance_config_upward(start: &Path) -> Option<PathBuf> {
    let mut current = if start.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        start.to_path_buf()
    };
    loop {
        let candidate = current
            .join(LOCAL_INSTANCE_CONFIG_DIR)
            .join(LOCAL_INSTANCE_CONFIG_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
        if !current.pop() {
            return None;
        }
    }
}

fn dev_home_from_local_instance_config_path(path: &Path) -> Result<PathBuf, SkiffTestError> {
    let text = fs::read_to_string(path).map_err(|source| SkiffTestError::ReadTestDoubles {
        path: path.display().to_string(),
        source,
    })?;
    let config: LocalInstanceConfig =
        serde_yaml::from_str(&text).map_err(|source| SkiffTestError::InvalidTestDouble {
            path: path.display().to_string(),
            message: format!("failed to parse local instance config as YAML: {source}"),
        })?;
    let raw_dev_home = config
        .dev_home
        .unwrap_or_else(|| DEFAULT_LOCAL_INSTANCE_DEV_HOME.to_string());
    if raw_dev_home.is_empty() {
        return Err(SkiffTestError::InvalidTestDouble {
            path: path.display().to_string(),
            message: "local instance devHome must be a non-empty string".to_string(),
        });
    }
    let instance_root = path
        .parent()
        .ok_or_else(|| SkiffTestError::InvalidTestDouble {
            path: path.display().to_string(),
            message: "local instance config path must have a parent directory".to_string(),
        })?;
    Ok(resolve_local_instance_path(instance_root, &raw_dev_home))
}

fn resolve_local_instance_path(base_dir: &Path, value: &str) -> PathBuf {
    let expanded = expand_home_path(value);
    if expanded.is_absolute() {
        expanded
    } else {
        base_dir.join(expanded)
    }
}

fn expand_home_path(value: &str) -> PathBuf {
    if value == "~" {
        return home_dir()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn home_dir() -> Option<std::ffi::OsString> {
    env::var_os("HOME").or_else(|| env::var_os("USERPROFILE"))
}

pub(crate) fn service_db_mongo_url_from_router_config_path(
    path: &Path,
) -> Result<Option<String>, SkiffTestError> {
    let text = fs::read_to_string(path).map_err(|source| SkiffTestError::ReadTestDoubles {
        path: path.display().to_string(),
        source,
    })?;
    let router_config = parse_yaml_config_snapshot(path, &text)?;
    service_db_mongo_url_from_config(&router_config).map_err(|message| {
        SkiffTestError::InvalidTestDouble {
            path: path.display().to_string(),
            message,
        }
    })
}

fn test_doubles_manifest_path(input: &Path, input_is_file: bool) -> Option<PathBuf> {
    let base = if input_is_file {
        input.parent()?
    } else {
        input
    };
    let path = base.join(TEST_DOUBLES_FILE);
    path.is_file().then_some(path)
}

fn read_test_doubles_manifest_path(
    path: &Path,
) -> Result<Option<TestDoublesManifest>, SkiffTestError> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|source| SkiffTestError::ReadTestDoubles {
        path: path.display().to_string(),
        source,
    })?;
    let manifest: TestDoublesManifest =
        serde_json::from_str(&text).map_err(|source| SkiffTestError::ParseTestDoubles {
            path: path.display().to_string(),
            source,
        })?;
    Ok(Some(manifest))
}

fn empty_json_object() -> JsonValue {
    JsonValue::Object(JsonMap::new())
}

pub(crate) fn doubles_for_runtime_test(
    test_doubles: &HashMap<String, HashMap<String, Vec<TestEffectDouble>>>,
    module_path: &str,
    test_name: &str,
) -> HashMap<String, Vec<TestEffectDouble>> {
    doubles_for_runtime_test_modules(test_doubles, &[module_path], test_name)
}

pub(crate) fn doubles_for_runtime_test_modules(
    test_doubles: &HashMap<String, HashMap<String, Vec<TestEffectDouble>>>,
    module_paths: &[&str],
    test_name: &str,
) -> HashMap<String, Vec<TestEffectDouble>> {
    for module_path in module_paths {
        let qualified = format!("{module_path}::{test_name}");
        if let Some(doubles) = test_doubles.get(&qualified) {
            return doubles.clone();
        }
    }
    test_doubles.get(test_name).cloned().unwrap_or_default()
}

pub(crate) fn config_for_runtime_test(
    global_config: &JsonValue,
    test_configs: &HashMap<String, JsonValue>,
    module_path: &str,
    test_name: &str,
) -> JsonValue {
    config_for_runtime_test_modules(global_config, test_configs, &[module_path], test_name)
}

pub(crate) fn config_for_runtime_test_modules(
    global_config: &JsonValue,
    test_configs: &HashMap<String, JsonValue>,
    module_paths: &[&str],
    test_name: &str,
) -> JsonValue {
    let mut config = global_config.clone();
    let test_config = module_paths
        .iter()
        .find_map(|module_path| test_configs.get(&format!("{module_path}::{test_name}")))
        .or_else(|| test_configs.get(test_name));
    if let Some(test_config) = test_config {
        merge_json_object(&mut config, test_config.clone());
    }
    config
}

fn validate_runtime_test_double(target: &str, response: &JsonValue) -> Result<(), String> {
    if !is_supported_test_double_target(target) {
        return Err(format!("unsupported test double target {target}"));
    }
    if target == STD_HTTP_REQUEST_TARGET {
        validate_http_client_response(response)?;
    }
    Ok(())
}

fn is_supported_test_double_target(target: &str) -> bool {
    matches!(
        target,
        STD_HTTP_REQUEST_TARGET | "std.http.client.sse" | "std.http.client.stream"
    )
}

fn validate_http_client_response(value: &JsonValue) -> Result<(), String> {
    let object = expect_object(value, "std.http.client.request response")?;
    expect_exact_fields(
        object,
        &["status", "headers", "body"],
        "std.http.client.request response",
    )?;
    if !matches!(object.get("status"), Some(JsonValue::Number(_))) {
        return Err("std.http.client.request response.status must be a number".to_string());
    }
    validate_headers(
        object
            .get("headers")
            .ok_or_else(|| "std.http.client.request response.headers is required".to_string())?,
    )?;
    validate_http_body(
        object
            .get("body")
            .ok_or_else(|| "std.http.client.request response.body is required".to_string())?,
    )
}

fn validate_headers(value: &JsonValue) -> Result<(), String> {
    let JsonValue::Array(headers) = value else {
        return Err("std.http.client.request response.headers must be an array".to_string());
    };
    for header in headers {
        let object = expect_object(header, "std.http.client.request response header")?;
        expect_exact_fields(
            object,
            &["name", "value"],
            "std.http.client.request response header",
        )?;
        if !matches!(object.get("name"), Some(JsonValue::String(_))) {
            return Err(
                "std.http.client.request response header.name must be a string".to_string(),
            );
        }
        if !matches!(object.get("value"), Some(JsonValue::String(_))) {
            return Err(
                "std.http.client.request response header.value must be a string".to_string(),
            );
        }
    }
    Ok(())
}

fn validate_http_body(value: &JsonValue) -> Result<(), String> {
    match value {
        JsonValue::String(encoded) => validate_base64(encoded, "std.http.client.request response.body"),
        JsonValue::Object(object) => {
            expect_exact_fields(
                object,
                &["__skiffBytesBase64"],
                "std.http.client.request response.body",
            )?;
            let Some(JsonValue::String(encoded)) = object.get("__skiffBytesBase64") else {
                return Err(
                    "std.http.client.request response.body.__skiffBytesBase64 must be a string"
                        .to_string(),
                );
            };
            validate_base64(encoded, "std.http.client.request response.body.__skiffBytesBase64")
        }
        _ => Err(
            "std.http.client.request response.body must be a base64 string or bytes metadata object"
                .to_string(),
        ),
    }
}

fn validate_base64(value: &str, label: &str) -> Result<(), String> {
    let clean = value.trim();
    if clean.len() % 4 != 0 {
        return Err(format!("{label} must be valid base64"));
    }
    let mut padding = 0usize;
    for byte in clean.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'+' | b'/' if padding == 0 => {}
            b'=' => padding += 1,
            _ => return Err(format!("{label} must be valid base64")),
        }
        if padding > 2 {
            return Err(format!("{label} must be valid base64"));
        }
    }
    Ok(())
}

fn expect_object<'a>(
    value: &'a JsonValue,
    label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    let JsonValue::Object(object) = value else {
        return Err(format!("{label} must be an object"));
    };
    Ok(object)
}

fn expect_exact_fields(
    object: &JsonMap<String, JsonValue>,
    fields: &[&str],
    label: &str,
) -> Result<(), String> {
    for field in fields {
        if !object.contains_key(*field) {
            return Err(format!("{label}.{field} is required"));
        }
    }
    for key in object.keys() {
        if !fields.contains(&key.as_str()) {
            return Err(format!("{label} has unknown field {key}"));
        }
    }
    Ok(())
}
