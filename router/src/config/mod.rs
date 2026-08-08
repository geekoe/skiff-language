//! Frozen Router process config parser (C-config).
//!
//! Consumes the same golden corpus as the TypeScript parser
//! (`router/src/router/config.ts`) with identical schema/defaults, relative
//! path resolution, secret redaction, unknown-key rejection and YAML safety
//! rules. The TypeScript parser and renderers are not modified.

mod strict_yaml;

use std::fmt;
use std::path::{Component, Path, PathBuf};

use skiff_artifact_model::validate_activation_profile;

use strict_yaml::JsonValue;

pub const ROUTER_CONFIG_REDACTED_VALUE: &str = "[REDACTED]";
pub const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 20_000;
pub const DEFAULT_HTTP_PORT: u16 = 4000;
pub const DEFAULT_RUNTIME_PORT: u16 = 4001;
pub const DEFAULT_RUNTIME_PATH: &str = "/runtime";
pub const DEFAULT_WEBSOCKET_PATH: &str = "/ws";
pub const DEFAULT_MANIFEST: &str = "fixtures/hello/manifest.json";
pub const TELEMETRY_PROTOCOL: &str = "skiff-telemetry-v1";
pub const DEFAULT_TELEMETRY_QUEUE_MAX_EVENTS: u64 = 10_000;
pub const DEFAULT_TELEMETRY_BATCH_MAX_EVENTS: u64 = 200;
pub const DEFAULT_TELEMETRY_BATCH_MAX_BYTES: u64 = 262_144;
pub const DEFAULT_TELEMETRY_FLUSH_INTERVAL_MS: u64 = 1000;
pub const DEFAULT_PROFILE_SAMPLING_HZ: u64 = 1000;
pub const DEFAULT_PROFILE_EXPORT_INTERVAL_MS: u64 = 60_000;

/// JavaScript safe-integer upper bound (2^53). `f64` cannot represent 2^53 - 1,
/// so values at or above 2^53 compare exactly like `Number.isSafeInteger`.
const JS_SAFE_INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0;

const TOP_LEVEL_KEYS: &[&str] = &[
    "artifactsPath",
    "devReload",
    "fileBackend",
    "host",
    "http",
    "httpPort",
    "manifest",
    "manifests",
    "profile",
    "profileSampling",
    "releaseMode",
    "requestTimeoutMs",
    "rewrite",
    "runDir",
    "runtime",
    "runtimePath",
    "runtimePort",
    "serviceDb",
    "telemetry",
    "websocket",
];
const HTTP_KEYS: &[&str] = &["port", "maxRequestBytes", "maxResponseBytes"];
const RUNTIME_KEYS: &[&str] = &["port", "path", "maxConcurrency"];
const WEBSOCKET_KEYS: &[&str] = &["path"];
const SERVICE_DB_KEYS: &[&str] = &["mongoUrl"];
const TELEMETRY_KEYS: &[&str] = &[
    "enabled",
    "endpoint",
    "protocol",
    "queueMaxEvents",
    "batchMaxEvents",
    "batchMaxBytes",
    "flushIntervalMs",
    "filePath",
    "fileMaxBytes",
    "fileMaxFiles",
];
/// `profileSampling` block keys (rust.profile contract §2). Note: the
/// sampling block cannot be named `profile` in the router config because the
/// top-level `profile` key is the frozen required activation profile string;
/// the runtime config (no such key) uses `profile` per the contract.
const PROFILE_SAMPLING_KEYS: &[&str] = &["enabled", "samplingHz", "exportIntervalMs"];
const FILE_BACKEND_KEYS: &[&str] = &["local", "oss"];
const FILE_BACKEND_LOCAL_KEYS: &[&str] = &["root"];
const FILE_BACKEND_OSS_KEYS: &[&str] = &[
    "endpoint",
    "bucket",
    "region",
    "accessKeyId",
    "accessKeySecret",
    "accessKeyIdEnv",
    "accessKeySecretEnv",
];
const REWRITE_FIELDS: &[&str] = &["host", "path", "service", "version"];

/// The single frozen Router process config contract (see
/// `doc/implementation/router-rust-migration-c-config-leaf.md`).
#[derive(Debug, Clone, PartialEq)]
pub struct RouterConfig {
    pub artifacts_path: PathBuf,
    pub dev_reload: Option<bool>,
    pub host: String,
    pub http_max_request_bytes: u64,
    pub http_max_response_bytes: u64,
    pub http_port: u16,
    pub manifests: Vec<PathBuf>,
    pub profile: String,
    /// Optional rust.profile sampling block (`profileSampling`; `None` when
    /// absent or `enabled: false`). Mirrors the `telemetry` block.
    pub profile_sampling: Option<ProfileConfig>,
    pub release_mode: Option<bool>,
    pub request_timeout_ms: u64,
    pub rewrite: Vec<RouterRewriteRule>,
    pub runtime_path: String,
    pub runtime_port: u16,
    pub runtime_max_concurrency: u64,
    /// Optional process run directory: when set, the binary acquires an
    /// exclusive `<run_dir>/router.pid` file before starting listeners
    /// (process self-defense against double start).
    pub run_dir: Option<String>,
    pub file_backend: Option<FileBackendConfig>,
    pub service_db: ServiceDbConfig,
    pub telemetry: Option<TelemetryConfig>,
    pub websocket_path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouterRewriteRule {
    pub host: String,
    pub path: Option<String>,
    pub service: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServiceDbConfig {
    pub mongo_url: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileBackendConfig {
    pub local: Option<FileBackendLocalConfig>,
    pub oss: Option<FileBackendOssConfig>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileBackendLocalConfig {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FileBackendOssConfig {
    pub endpoint: String,
    pub bucket: String,
    pub region: Option<String>,
    pub access_key_id: Option<String>,
    pub access_key_secret: Option<String>,
    pub access_key_id_env: Option<String>,
    pub access_key_secret_env: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TelemetryConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub protocol: String,
    pub queue_max_events: u64,
    pub batch_max_events: u64,
    pub batch_max_bytes: u64,
    pub flush_interval_ms: u64,
    /// Optional JSONL sink file override: absolute paths are used directly,
    /// relative paths resolve against `<artifacts_path.parent()>/logs/telemetry`.
    pub file_path: Option<PathBuf>,
    pub file_max_bytes: Option<u64>,
    pub file_max_files: Option<u64>,
}

/// Rust profile sampling block (rust.profile contract §2). `max_stacks` is
/// not configurable in the yml (contract example exposes three keys); the
/// producer uses the `skiff-profiling` default of 2048.
#[derive(Debug, Clone, PartialEq)]
pub struct ProfileConfig {
    pub enabled: bool,
    pub sampling_hz: u64,
    pub export_interval_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterConfigError(pub String);

impl fmt::Display for RouterConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for RouterConfigError {}

/// Loads and validates the Router process config at `config_path`.
pub fn load_router_config(config_path: &str) -> Result<RouterConfig, RouterConfigError> {
    let absolute = absolute_path(config_path);
    let text = std::fs::read_to_string(&absolute).map_err(|_| {
        RouterConfigError(format!(
            "failed to read router config {}; copy router.example.yml to router.yml first",
            absolute.display()
        ))
    })?;
    let label = format!("router config {}", absolute.display());
    let root = strict_yaml::parse_strict_yaml_object(&text, &label).map_err(RouterConfigError)?;
    let config_dir = absolute
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    parse_router_config(root, &config_dir)
}

/// Diagnostic projection: secret leaves are replaced by
/// `ROUTER_CONFIG_REDACTED_VALUE`. The original config is not mutated and the
/// runtime bootstrap continues to use the unredacted values.
pub fn redact_router_config(config: &RouterConfig) -> RouterConfig {
    let mut redacted = config.clone();
    redacted.service_db.mongo_url = ROUTER_CONFIG_REDACTED_VALUE.to_string();
    if let Some(backend) = &mut redacted.file_backend {
        if let Some(oss) = &mut backend.oss {
            if oss.access_key_id.is_some() {
                oss.access_key_id = Some(ROUTER_CONFIG_REDACTED_VALUE.to_string());
            }
            if oss.access_key_secret.is_some() {
                oss.access_key_secret = Some(ROUTER_CONFIG_REDACTED_VALUE.to_string());
            }
        }
    }
    redacted
}

fn parse_router_config(
    root: JsonValue,
    config_dir: &Path,
) -> Result<RouterConfig, RouterConfigError> {
    if root
        .get("http")
        .and_then(|http| http.get("bodyLimitBytes"))
        .is_some()
    {
        return Err(error(
            "router config http.bodyLimitBytes is no longer supported",
        ));
    }

    let manifests =
        read_manifests(absent_or(get(&root, "manifests")).or_else(|| get(&root, "manifest")))?;
    reject_removed_artifact_root_config(&root)?;
    let artifacts_path = resolve_config_path(
        config_dir,
        &read_required_string(get(&root, "artifactsPath"), "artifactsPath")?,
    );
    let dev_reload = read_optional_boolean(get(&root, "devReload"), "devReload")?;
    let release_mode = read_optional_boolean(get(&root, "releaseMode"), "releaseMode")?;
    reject_removed_values_config(get(&root, "values"))?;
    let profile = read_required_profile(get(&root, "profile"), "profile")?;
    reject_removed_hosts(get(&root, "hosts"))?;

    reject_unknown_keys(Some(&root), TOP_LEVEL_KEYS, "")?;
    reject_unknown_keys(get(&root, "http"), HTTP_KEYS, "http")?;
    reject_unknown_keys(get(&root, "runtime"), RUNTIME_KEYS, "runtime")?;
    reject_unknown_keys(get(&root, "websocket"), WEBSOCKET_KEYS, "websocket")?;

    let http = get(&root, "http");
    let runtime = get(&root, "runtime");
    let websocket = get(&root, "websocket");

    let host = read_string(get(&root, "host"), "host", "127.0.0.1")?;
    let http_max_request_bytes = read_required_positive_integer(
        http.and_then(|http| http.get("maxRequestBytes")),
        "http.maxRequestBytes",
    )?;
    let http_max_response_bytes = read_required_positive_integer(
        http.and_then(|http| http.get("maxResponseBytes")),
        "http.maxResponseBytes",
    )?;
    let http_port = read_port(
        absent_or(get(&root, "httpPort")).or_else(|| http.and_then(|http| http.get("port"))),
        "http.port",
        DEFAULT_HTTP_PORT,
    )?;
    let manifests = manifests
        .into_iter()
        .map(|manifest| resolve_config_path(config_dir, &manifest))
        .collect();
    let request_timeout_ms = read_positive_integer(
        get(&root, "requestTimeoutMs"),
        "requestTimeoutMs",
        DEFAULT_REQUEST_TIMEOUT_MS,
    )?;
    let rewrite = read_rewrite_rules(get(&root, "rewrite"))?;
    let runtime_path = read_path(
        absent_or(get(&root, "runtimePath"))
            .or_else(|| runtime.and_then(|runtime| runtime.get("path"))),
        "runtime.path",
        DEFAULT_RUNTIME_PATH,
    )?;
    let runtime_max_concurrency = read_required_positive_config_integer(
        runtime.and_then(|runtime| runtime.get("maxConcurrency")),
        "runtime.maxConcurrency",
    )?;
    let runtime_port = read_port(
        absent_or(get(&root, "runtimePort"))
            .or_else(|| runtime.and_then(|runtime| runtime.get("port"))),
        "runtime.port",
        DEFAULT_RUNTIME_PORT,
    )?;
    let service_db = read_service_db_config(get(&root, "serviceDb"))?;
    let websocket_path = read_path(
        websocket.and_then(|websocket| websocket.get("path")),
        "websocket.path",
        DEFAULT_WEBSOCKET_PATH,
    )?;
    let run_dir = read_optional_non_empty_string(get(&root, "runDir"), "runDir")?;

    let file_backend = read_file_backend_config(get(&root, "fileBackend"), config_dir)?;
    let telemetry = read_telemetry_config(get(&root, "telemetry"))?;
    let profile_sampling = read_profile_config(get(&root, "profileSampling"))?;

    Ok(RouterConfig {
        artifacts_path,
        dev_reload,
        host,
        http_max_request_bytes,
        http_max_response_bytes,
        http_port,
        manifests,
        profile,
        profile_sampling,
        release_mode,
        request_timeout_ms,
        rewrite,
        runtime_path,
        runtime_port,
        runtime_max_concurrency,
        run_dir,
        file_backend,
        service_db,
        telemetry,
        websocket_path,
    })
}

fn read_service_db_config(value: Option<&JsonValue>) -> Result<ServiceDbConfig, RouterConfigError> {
    let Some(value) = require_object(value, "serviceDb")? else {
        return Err(error("router config serviceDb must be an object"));
    };
    if value.get("storageNamespace").is_some() {
        return Err(error(
            "router config serviceDb.storageNamespace is no longer supported",
        ));
    }
    reject_unknown_keys(Some(value), SERVICE_DB_KEYS, "serviceDb")?;
    Ok(ServiceDbConfig {
        mongo_url: read_required_string(value.get("mongoUrl"), "serviceDb.mongoUrl")?,
    })
}

fn read_file_backend_config(
    value: Option<&JsonValue>,
    config_dir: &Path,
) -> Result<Option<FileBackendConfig>, RouterConfigError> {
    let Some(value) = require_object(value, "fileBackend")? else {
        return Ok(None);
    };
    reject_unknown_keys(Some(value), FILE_BACKEND_KEYS, "fileBackend")?;
    let local = read_file_backend_local_config(value.get("local"), config_dir)?;
    let oss = read_file_backend_oss_config(value.get("oss"))?;
    if local.is_none() && oss.is_none() {
        return Err(error(
            "router config fileBackend must configure local or oss",
        ));
    }
    Ok(Some(FileBackendConfig { local, oss }))
}

fn read_file_backend_local_config(
    value: Option<&JsonValue>,
    config_dir: &Path,
) -> Result<Option<FileBackendLocalConfig>, RouterConfigError> {
    let Some(value) = require_object(value, "fileBackend.local")? else {
        return Ok(None);
    };
    reject_unknown_keys(Some(value), FILE_BACKEND_LOCAL_KEYS, "fileBackend.local")?;
    let root = resolve_config_path(
        config_dir,
        &read_required_string(value.get("root"), "fileBackend.local.root")?,
    );
    Ok(Some(FileBackendLocalConfig { root }))
}

fn read_file_backend_oss_config(
    value: Option<&JsonValue>,
) -> Result<Option<FileBackendOssConfig>, RouterConfigError> {
    let Some(value) = require_object(value, "fileBackend.oss")? else {
        return Ok(None);
    };
    reject_unknown_keys(Some(value), FILE_BACKEND_OSS_KEYS, "fileBackend.oss")?;
    let endpoint = read_required_string(value.get("endpoint"), "fileBackend.oss.endpoint")?;
    let bucket = read_required_string(value.get("bucket"), "fileBackend.oss.bucket")?;
    let region = read_optional_non_empty_string(value.get("region"), "fileBackend.oss.region")?;
    let access_key_id =
        read_optional_non_empty_string(value.get("accessKeyId"), "fileBackend.oss.accessKeyId")?;
    let access_key_secret = read_optional_non_empty_string(
        value.get("accessKeySecret"),
        "fileBackend.oss.accessKeySecret",
    )?;
    let access_key_id_env = read_optional_non_empty_string(
        value.get("accessKeyIdEnv"),
        "fileBackend.oss.accessKeyIdEnv",
    )?;
    let access_key_secret_env = read_optional_non_empty_string(
        value.get("accessKeySecretEnv"),
        "fileBackend.oss.accessKeySecretEnv",
    )?;
    if access_key_id.is_none() && access_key_id_env.is_none() {
        return Err(error(
            "router config fileBackend.oss requires accessKeyIdEnv or accessKeyId",
        ));
    }
    if access_key_secret.is_none() && access_key_secret_env.is_none() {
        return Err(error(
            "router config fileBackend.oss requires accessKeySecretEnv or accessKeySecret",
        ));
    }
    Ok(Some(FileBackendOssConfig {
        endpoint,
        bucket,
        region,
        access_key_id,
        access_key_secret,
        access_key_id_env,
        access_key_secret_env,
    }))
}

fn read_telemetry_config(
    value: Option<&JsonValue>,
) -> Result<Option<TelemetryConfig>, RouterConfigError> {
    let Some(value) = require_object(value, "telemetry")? else {
        return Ok(None);
    };
    reject_unknown_keys(Some(value), TELEMETRY_KEYS, "telemetry")?;
    let enabled = read_optional_boolean(value.get("enabled"), "telemetry.enabled")?.unwrap_or(true);
    if !enabled {
        return Ok(None);
    }
    let Some(endpoint_value) = absent_or(value.get("endpoint")) else {
        return Ok(None);
    };
    let endpoint = read_string(Some(endpoint_value), "telemetry.endpoint", "")?;
    let protocol = read_telemetry_protocol(value.get("protocol"))?;
    Ok(Some(TelemetryConfig {
        enabled: true,
        endpoint,
        protocol,
        queue_max_events: read_positive_integer(
            value.get("queueMaxEvents"),
            "telemetry.queueMaxEvents",
            DEFAULT_TELEMETRY_QUEUE_MAX_EVENTS,
        )?,
        batch_max_events: read_positive_integer(
            value.get("batchMaxEvents"),
            "telemetry.batchMaxEvents",
            DEFAULT_TELEMETRY_BATCH_MAX_EVENTS,
        )?,
        batch_max_bytes: read_positive_integer(
            value.get("batchMaxBytes"),
            "telemetry.batchMaxBytes",
            DEFAULT_TELEMETRY_BATCH_MAX_BYTES,
        )?,
        flush_interval_ms: read_positive_integer(
            value.get("flushIntervalMs"),
            "telemetry.flushIntervalMs",
            DEFAULT_TELEMETRY_FLUSH_INTERVAL_MS,
        )?,
        file_path: read_optional_non_empty_string(value.get("filePath"), "telemetry.filePath")?
            .map(PathBuf::from),
        file_max_bytes: read_optional_positive_integer(
            value.get("fileMaxBytes"),
            "telemetry.fileMaxBytes",
        )?,
        file_max_files: read_optional_positive_integer(
            value.get("fileMaxFiles"),
            "telemetry.fileMaxFiles",
        )?,
    }))
}
fn read_telemetry_protocol(value: Option<&JsonValue>) -> Result<String, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(TELEMETRY_PROTOCOL.to_string()),
        Some(JsonValue::String(text)) if text == TELEMETRY_PROTOCOL => {
            Ok(TELEMETRY_PROTOCOL.to_string())
        }
        _ => Err(error(format!(
            "router config telemetry.protocol must be {TELEMETRY_PROTOCOL}"
        ))),
    }
}

fn read_profile_config(
    value: Option<&JsonValue>,
) -> Result<Option<ProfileConfig>, RouterConfigError> {
    let Some(value) = require_object(value, "profileSampling")? else {
        return Ok(None);
    };
    reject_unknown_keys(Some(value), PROFILE_SAMPLING_KEYS, "profileSampling")?;
    let enabled = read_optional_boolean(value.get("enabled"), "profileSampling.enabled")?
        .unwrap_or(false);
    if !enabled {
        return Ok(None);
    }
    Ok(Some(ProfileConfig {
        enabled: true,
        sampling_hz: read_positive_integer(
            value.get("samplingHz"),
            "profileSampling.samplingHz",
            DEFAULT_PROFILE_SAMPLING_HZ,
        )?,
        export_interval_ms: read_positive_integer(
            value.get("exportIntervalMs"),
            "profileSampling.exportIntervalMs",
            DEFAULT_PROFILE_EXPORT_INTERVAL_MS,
        )?,
    }))
}

fn read_rewrite_rules(
    value: Option<&JsonValue>,
) -> Result<Vec<RouterRewriteRule>, RouterConfigError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let Some(rules) = value.as_array() else {
        return Err(error("router config rewrite must be an array"));
    };
    let mut parsed = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (index, raw_rule) in rules.iter().enumerate() {
        let prefix = format!("rewrite[{index}]");
        if raw_rule.as_object().is_none() {
            return Err(error(format!("router config {prefix} must be an object")));
        }
        reject_unknown_keys(Some(raw_rule), REWRITE_FIELDS, &prefix)?;
        let host = normalize_host(&read_rewrite_required_string(
            raw_rule.get("host"),
            &format!("{prefix}.host"),
        )?);
        if host.is_empty() {
            return Err(error(format!(
                "router config {prefix}.host must be a non-empty host"
            )));
        }
        let path = read_rewrite_optional_path(raw_rule.get("path"), &format!("{prefix}.path"))?;
        let service =
            read_rewrite_required_string(raw_rule.get("service"), &format!("{prefix}.service"))?;
        if !is_publication_id(&service) {
            return Err(error(format!(
                "router config {prefix}.service must be a valid publication id"
            )));
        }
        let version =
            read_rewrite_optional_string(raw_rule.get("version"), &format!("{prefix}.version"))?;
        if let Some(version) = &version {
            if !is_valid_version(version) {
                return Err(error(format!(
                    "router config {prefix}.version must be a valid version"
                )));
            }
        }
        let key = format!("{host}\u{0}{}", path.as_deref().unwrap_or(""));
        if !seen.insert(key) {
            return Err(error(match &path {
                Some(path) => format!("duplicate router rewrite rule for host {host} path {path}"),
                None => format!("duplicate router rewrite rule for host {host}"),
            }));
        }
        parsed.push(RouterRewriteRule {
            host,
            path,
            service,
            version,
        });
    }
    Ok(parsed)
}

fn read_required_profile(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<String, RouterConfigError> {
    let Some(value) = value else {
        return Err(error(format!("router config {name} is required")));
    };
    if value.is_null() {
        return Err(error(format!("router config {name} is required")));
    }
    let profile = read_string(Some(value), name, String::new())?;
    if !is_valid_profile(&profile) {
        return Err(error(format!(
            "router config {name} must be 1-200 ASCII letters, digits, dot, dash, or underscore and must not be . or .."
        )));
    }
    Ok(profile)
}

fn read_manifests(value: Option<&JsonValue>) -> Result<Vec<String>, RouterConfigError> {
    let Some(value) = value else {
        return Ok(vec![DEFAULT_MANIFEST.to_string()]);
    };
    if value.is_null() {
        return Ok(vec![DEFAULT_MANIFEST.to_string()]);
    }
    if let Some(_text) = value.as_str() {
        return Ok(vec![read_string(
            Some(value),
            "manifest",
            DEFAULT_MANIFEST,
        )?]);
    }
    let Some(items) = value.as_array() else {
        return Err(error(
            "router config manifests must be a non-empty string array",
        ));
    };
    if items.is_empty() {
        return Err(error(
            "router config manifests must be a non-empty string array",
        ));
    }
    let mut manifests = Vec::new();
    for (index, item) in items.iter().enumerate() {
        match item.as_str() {
            Some(text) if !text.trim().is_empty() => manifests.push(text.trim().to_string()),
            _ => {
                return Err(error(format!(
                    "router config manifests[{index}] must be a non-empty string"
                )));
            }
        }
    }
    Ok(manifests)
}

fn read_required_string(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<String, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Err(error(format!(
            "router config {name} must be a non-empty string"
        ))),
        Some(JsonValue::String(text)) if !text.trim().is_empty() => Ok(text.trim().to_string()),
        _ => Err(error(format!(
            "router config {name} must be a non-empty string"
        ))),
    }
}

fn read_optional_non_empty_string(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<Option<String>, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        _ => read_required_string(value, name).map(Some),
    }
}

fn read_string(
    value: Option<&JsonValue>,
    name: &str,
    fallback: impl Into<String>,
) -> Result<String, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(fallback.into()),
        _ => read_required_string(value, name),
    }
}

fn read_optional_boolean(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<Option<bool>, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(boolean)) => Ok(Some(*boolean)),
        Some(JsonValue::String(text)) => match text.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(Some(true)),
            "false" => Ok(Some(false)),
            _ => Err(error(format!("router config {name} must be a boolean"))),
        },
        _ => Err(error(format!("router config {name} must be a boolean"))),
    }
}

fn read_path(
    value: Option<&JsonValue>,
    name: &str,
    fallback: &str,
) -> Result<String, RouterConfigError> {
    let path = read_string(value, name, fallback)?;
    if !path.starts_with('/') {
        return Err(error(format!("router config {name} must start with /")));
    }
    Ok(path)
}

fn read_port(
    value: Option<&JsonValue>,
    name: &str,
    fallback: u16,
) -> Result<u16, RouterConfigError> {
    let port = read_positive_integer(value, name, fallback as u64)?;
    if port > u16::MAX as u64 {
        return Err(error(format!("router config {name} must be <= 65535")));
    }
    Ok(port as u16)
}

fn read_positive_integer(
    value: Option<&JsonValue>,
    name: &str,
    fallback: u64,
) -> Result<u64, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(fallback),
        _ => read_required_positive_integer(value, name),
    }
}

fn read_optional_positive_integer(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<Option<u64>, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        _ => read_required_positive_integer(value, name).map(Some),
    }
}

fn read_required_positive_integer(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<u64, RouterConfigError> {
    let number = match value {
        None
        | Some(JsonValue::Null)
        | Some(JsonValue::Bool(_))
        | Some(JsonValue::Array(_))
        | Some(JsonValue::Object(_)) => {
            return Err(error(format!(
                "router config {name} must be a positive integer"
            )));
        }
        Some(JsonValue::Number(number)) => *number,
        Some(JsonValue::String(text)) => parse_js_number(text),
    };
    if !is_js_safe_positive_integer(number) {
        return Err(error(format!(
            "router config {name} must be a positive integer"
        )));
    }
    Ok(number as u64)
}

/// `runtime.maxConcurrency` follows the TypeScript
/// `readRequiredPositiveConfigInteger`: the value must be a JSON number, not a
/// numeric string.
fn read_required_positive_config_integer(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<u64, RouterConfigError> {
    let Some(JsonValue::Number(number)) = value else {
        return Err(error(format!(
            "router config {name} must be a positive integer"
        )));
    };
    if !is_js_safe_positive_integer(*number) {
        return Err(error(format!(
            "router config {name} must be a positive integer"
        )));
    }
    Ok(*number as u64)
}

fn reject_unknown_keys(
    value: Option<&JsonValue>,
    allowed: &[&str],
    path: &str,
) -> Result<(), RouterConfigError> {
    let Some(value) = value else {
        return Ok(());
    };
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    for (key, _) in object {
        if !allowed.contains(&key.as_str()) {
            return Err(error(if path.is_empty() {
                format!("router config {key} is not supported")
            } else {
                format!("router config {path}.{key} is not supported")
            }));
        }
    }
    Ok(())
}

fn reject_removed_artifact_root_config(root: &JsonValue) -> Result<(), RouterConfigError> {
    if root.get("artifactRoot").is_some() {
        return Err(error(
            "router config artifactRoot is no longer supported; use artifactsPath",
        ));
    }
    if root.get("artifactRoots").is_some() {
        return Err(error(
            "router config artifactRoots is no longer supported; use artifactsPath",
        ));
    }
    if root.get("artifacts").is_some() {
        return Err(error(
            "router config artifacts is no longer supported; use artifactsPath",
        ));
    }
    Ok(())
}

fn reject_removed_hosts(value: Option<&JsonValue>) -> Result<(), RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(()),
        _ => Err(error(
            "router config hosts is no longer supported; declare RuntimeAssembly globalIngress Hosts",
        )),
    }
}

fn reject_removed_values_config(value: Option<&JsonValue>) -> Result<(), RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(()),
        Some(JsonValue::Object(entries)) if entries.iter().any(|(key, _)| key == "profile") => {
            Err(error(
                "router config values.profile is no longer supported; set top-level profile instead",
            ))
        }
        _ => Err(error(
            "router config values is no longer supported; set top-level profile and keep runtime config in config*.yml",
        )),
    }
}

fn read_rewrite_required_string(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<String, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Err(error(format!("router config {name} is required"))),
        Some(JsonValue::String(text)) if !text.trim().is_empty() => Ok(text.trim().to_string()),
        _ => Err(error(format!(
            "router config {name} must be a non-empty string"
        ))),
    }
}

fn read_rewrite_optional_string(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<Option<String>, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        _ => read_rewrite_required_string(value, name).map(Some),
    }
}

fn read_rewrite_optional_path(
    value: Option<&JsonValue>,
    name: &str,
) -> Result<Option<String>, RouterConfigError> {
    let path = read_rewrite_optional_string(value, name)?;
    if let Some(path) = &path {
        if !path.starts_with('/') {
            return Err(error(format!("router config {name} must start with /")));
        }
    }
    Ok(path)
}

fn normalize_host(value: &str) -> String {
    let without_port = value.trim().to_lowercase();
    let without_port = without_port.strip_suffix('.').unwrap_or(&without_port);
    if let Some(rest) = without_port.strip_prefix('[') {
        return match rest.find(']') {
            Some(closing) => format!("[{}]", &rest[..closing]),
            None => without_port.to_string(),
        };
    }
    without_port.split(':').next().unwrap_or("").to_string()
}

fn is_publication_id(value: &str) -> bool {
    if value.len() > 63 || value == "std" || value.contains('~') || value.trim() != value {
        return false;
    }
    let Some((authority, local)) = value.split_once('/') else {
        return false;
    };
    let labels: Vec<&str> = authority.split('.').collect();
    if labels.len() < 2 || labels.iter().any(|label| !is_authority_label(label)) {
        return false;
    }
    !local.is_empty() && local.split('/').all(is_local_path_segment)
}

fn is_authority_label(label: &str) -> bool {
    if label.is_empty() {
        return false;
    }
    let bytes = label.as_bytes();
    if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) {
        return false;
    }
    let last = bytes[bytes.len() - 1];
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return false;
    }
    bytes
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn is_local_path_segment(segment: &str) -> bool {
    if segment.is_empty() {
        return false;
    }
    let bytes = segment.as_bytes();
    if !bytes[0].is_ascii_lowercase() {
        return false;
    }
    let last = bytes[bytes.len() - 1];
    if !(last.is_ascii_lowercase() || last.is_ascii_digit() || last == b'_') {
        return false;
    }
    bytes.iter().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-' || *byte == b'_'
    })
}

fn is_valid_version(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

fn is_valid_profile(value: &str) -> bool {
    validate_activation_profile(value).is_ok()
}

fn is_js_safe_positive_integer(number: f64) -> bool {
    number > 0.0 && number.fract() == 0.0 && number < JS_SAFE_INTEGER_LIMIT
}

/// JavaScript `Number()` conversion for numeric strings accepted by the
/// positive-integer readers (trimmed, decimal/hex/octal/binary, Infinity).
fn parse_js_number(text: &str) -> f64 {
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    if matches!(text, "Infinity" | "+Infinity") {
        return f64::INFINITY;
    }
    if text == "-Infinity" {
        return f64::NEG_INFINITY;
    }
    let (radix, digits, negative) =
        if let Some(rest) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            (16, rest, false)
        } else if let Some(rest) = text.strip_prefix("0o").or_else(|| text.strip_prefix("0O")) {
            (8, rest, false)
        } else if let Some(rest) = text.strip_prefix("0b").or_else(|| text.strip_prefix("0B")) {
            (2, rest, false)
        } else if let Some(rest) = text.strip_prefix('-') {
            (10, rest, true)
        } else {
            (10, text.strip_prefix('+').unwrap_or(text), false)
        };
    if radix != 10
        && !digits.is_empty()
        && digits.chars().all(|character| character.is_digit(radix))
    {
        let mut value = 0.0;
        for character in digits.chars() {
            let digit = character.to_digit(radix).unwrap_or(0) as f64;
            value = value * radix as f64 + digit;
        }
        return if negative { -value } else { value };
    }
    match digits.parse::<f64>() {
        Ok(value) => {
            if negative {
                -value
            } else {
                value
            }
        }
        Err(_) => f64::NAN,
    }
}

fn resolve_config_path(base: &Path, value: &str) -> PathBuf {
    let path = Path::new(value);
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    normalize_path(&joined)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
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

fn absolute_path(path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        normalize_path(path)
    } else {
        match std::env::current_dir() {
            Ok(cwd) => normalize_path(&cwd.join(path)),
            Err(_) => path.to_path_buf(),
        }
    }
}

fn require_object<'a>(
    value: Option<&'a JsonValue>,
    name: &str,
) -> Result<Option<&'a JsonValue>, RouterConfigError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value @ JsonValue::Object(_)) => Ok(Some(value)),
        Some(_) => Err(error(format!("router config {name} must be an object"))),
    }
}

fn get<'a>(value: &'a JsonValue, key: &str) -> Option<&'a JsonValue> {
    value.get(key)
}

/// Treats `null` as absent, matching TypeScript's `??` semantics.
fn absent_or(value: Option<&JsonValue>) -> Option<&JsonValue> {
    match value {
        None | Some(JsonValue::Null) => None,
        Some(value) => Some(value),
    }
}

fn error(message: impl Into<String>) -> RouterConfigError {
    RouterConfigError(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_id_contract_matches_typescript() {
        assert!(is_publication_id("skiff.run/account"));
        assert!(is_publication_id("skiff.run/registry"));
        assert!(is_publication_id("a.b/c"));
        assert!(!is_publication_id("NotAService"));
        assert!(!is_publication_id("skiff.run"));
        assert!(!is_publication_id("skiff.run/"));
        assert!(!is_publication_id("std/account"));
        assert!(!is_publication_id("skiff.run/Account"));
        assert!(!is_publication_id("skiff.run/acc~ount"));
    }

    #[test]
    fn version_contract_matches_typescript() {
        assert!(is_valid_version("0.1.0"));
        assert!(is_valid_version("release-2026:8_1"));
        assert!(!is_valid_version("bad version"));
        assert!(!is_valid_version(""));
    }

    #[test]
    fn normalize_host_matches_typescript() {
        assert_eq!(
            normalize_host("Account.Localhost:4000."),
            "account.localhost"
        );
        assert_eq!(normalize_host("registry.localhost"), "registry.localhost");
        assert_eq!(normalize_host("[::1]:4000"), "[::1]");
        assert_eq!(normalize_host("127.0.0.1"), "127.0.0.1");
    }

    #[test]
    fn run_dir_parses_and_defaults_to_none() {
        let minimal = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/router-config/valid/minimal.yml"
        ))
        .expect("minimal fixture");

        let with_run_dir = temp_config_file(&format!("{minimal}\nrunDir: {}\n", run_dir_path()));
        let config = load_router_config(with_run_dir.to_str().expect("utf8 path"))
            .expect("runDir must parse");
        assert_eq!(config.run_dir, Some(run_dir_path()));

        let without_run_dir = temp_config_file(&minimal);
        let config = load_router_config(without_run_dir.to_str().expect("utf8 path"))
            .expect("config without runDir must parse");
        assert_eq!(config.run_dir, None);
    }

    #[test]
    fn empty_run_dir_is_rejected() {
        let minimal = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/router-config/valid/minimal.yml"
        ))
        .expect("minimal fixture");
        let config_path = temp_config_file(&format!("{minimal}\nrunDir: \"\"\n"));
        let error = load_router_config(config_path.to_str().expect("utf8 path"))
            .expect_err("empty runDir must be rejected");
        assert!(
            error.to_string().contains("runDir"),
            "error {error:?} must mention runDir"
        );
    }

    fn run_dir_path() -> String {
        format!("{}/run", std::env::temp_dir().display())
    }

    fn temp_config_file(content: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "skiff-router-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("create temp dir");
        let path = directory.join("router.yml");
        std::fs::write(&path, content).expect("write temp config");
        path
    }
}

/// Canonicalize an artifacts path for identity comparison (the runtime
/// advertises its canonicalized artifact root; the lazy-load candidate rule
/// compares byte-for-byte). Falls back to the literal path when the directory
/// does not exist yet.
pub fn canonicalize_artifact_root(path: &std::path::Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}
