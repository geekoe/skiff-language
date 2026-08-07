use std::path::PathBuf;

use serde::{de, Deserialize, Deserializer, Serialize};
use serde_json::Value;
use skiff_artifact_model::ConfigShape;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RouterControlFrameHeader {
    pub schema_version: String,
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub artifact_roots: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_reload: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_config: Vec<RouterControlServiceConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<TelemetryControlConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_backend: Option<FileBackendControlConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub enum TelemetryProtocol {
    #[serde(rename = "skiff-telemetry-v1")]
    SkiffTelemetryV1,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryControlConfig {
    pub endpoint: String,
    pub protocol: TelemetryProtocol,
    pub queue_max_events: u32,
    pub batch_max_events: u32,
    pub batch_max_bytes: u32,
    pub flush_interval_ms: u32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TelemetrySource {
    #[serde(rename = "gateway")]
    Gateway,
    #[serde(rename = "router")]
    Router,
    #[serde(rename = "runtime")]
    Runtime,
    #[serde(rename = "provider")]
    Provider,
    #[serde(rename = "test")]
    Test,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TelemetryLevel {
    #[serde(rename = "debug")]
    Debug,
    #[serde(rename = "info")]
    Info,
    #[serde(rename = "warn")]
    Warn,
    #[serde(rename = "error")]
    Error,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TelemetryVisibility {
    Operational,
    Restricted,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBackendControlConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<FileBackendLocalConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oss: Option<FileBackendOssConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBackendLocalConfig {
    pub root: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileBackendOssConfig {
    pub endpoint: String,
    pub bucket: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_id_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_key_secret_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryRegisterEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub protocol: TelemetryProtocol,
    pub producer_id: String,
    pub source: TelemetrySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryEvent {
    pub ts: String,
    pub source: TelemetrySource,
    pub visibility: TelemetryVisibility,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activation_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_capability: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_span_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<TelemetryLevel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attrs: Option<serde_json::Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<serde_json::Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTelemetryEvent {
    ts: String,
    source: TelemetrySource,
    visibility: TelemetryVisibility,
    service_id: Option<String>,
    revision_id: Option<String>,
    build_id: Option<String>,
    activation_identity: Option<String>,
    runtime_id: Option<String>,
    provider_id: Option<String>,
    provider_revision: Option<String>,
    provider_capability: Option<String>,
    provider_target: Option<String>,
    request_id: Option<String>,
    client_request_id: Option<String>,
    trace_id: Option<String>,
    error_id: Option<String>,
    span_id: Option<String>,
    parent_span_id: Option<String>,
    target: Option<String>,
    level: Option<TelemetryLevel>,
    name: Option<String>,
    message: Option<String>,
    attrs: Option<serde_json::Map<String, Value>>,
    error: Option<serde_json::Map<String, Value>>,
    duration_ms: Option<f64>,
    dropped: Option<serde_json::Map<String, Value>>,
}

impl TryFrom<RawTelemetryEvent> for TelemetryEvent {
    type Error = String;

    fn try_from(raw: RawTelemetryEvent) -> Result<Self, Self::Error> {
        if raw
            .error_id
            .as_deref()
            .is_some_and(|error_id| error_id.trim().is_empty())
        {
            return Err("telemetry event errorId must be non-empty when present".to_string());
        }
        if raw.visibility == TelemetryVisibility::Restricted {
            if raw
                .trace_id
                .as_deref()
                .is_none_or(|trace_id| trace_id.trim().is_empty())
            {
                return Err("restricted telemetry event requires a non-empty traceId".to_string());
            }
            if raw
                .error_id
                .as_deref()
                .is_none_or(|error_id| error_id.trim().is_empty())
            {
                return Err("restricted telemetry event requires a non-empty errorId".to_string());
            }
        }
        Ok(Self {
            ts: raw.ts,
            source: raw.source,
            visibility: raw.visibility,
            service_id: raw.service_id,
            revision_id: raw.revision_id,
            build_id: raw.build_id,
            activation_identity: raw.activation_identity,
            runtime_id: raw.runtime_id,
            provider_id: raw.provider_id,
            provider_revision: raw.provider_revision,
            provider_capability: raw.provider_capability,
            provider_target: raw.provider_target,
            request_id: raw.request_id,
            client_request_id: raw.client_request_id,
            trace_id: raw.trace_id,
            error_id: raw.error_id,
            span_id: raw.span_id,
            parent_span_id: raw.parent_span_id,
            target: raw.target,
            level: raw.level,
            name: raw.name,
            message: raw.message,
            attrs: raw.attrs,
            error: raw.error,
            duration_ms: raw.duration_ms,
            dropped: raw.dropped,
        })
    }
}

impl<'de> Deserialize<'de> for TelemetryEvent {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        RawTelemetryEvent::deserialize(deserializer)?
            .try_into()
            .map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryBatchEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: String,
    pub producer_id: String,
    pub seq: u64,
    pub events: Vec<TelemetryEvent>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
#[serde(rename_all = "camelCase")]
pub struct RouterControlEnvelope {
    pub artifact_roots: Vec<PathBuf>,
    #[serde(default)]
    pub dev_reload: Option<bool>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub generation: Option<String>,
    #[serde(default)]
    pub fingerprint: Option<String>,
    #[serde(default)]
    pub service_config: Vec<RouterControlServiceConfig>,
    #[serde(default)]
    pub telemetry: Option<TelemetryControlConfig>,
    #[serde(default)]
    pub file_backend: Option<FileBackendControlConfig>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl RouterControlEnvelope {
    pub fn ordered_artifact_roots(&self) -> std::result::Result<Vec<PathBuf>, String> {
        ordered_artifact_roots(&self.artifact_roots)
    }

    pub fn reject_legacy_config_fields(&self) -> std::result::Result<(), String> {
        self.ordered_artifact_roots()?;
        if self.extra.contains_key("serviceValues") {
            return Err(
                "router.control serviceValues is no longer supported; use serviceConfig"
                    .to_string(),
            );
        }
        if self.extra.contains_key("serviceEnv") {
            return Err(
                "router.control serviceEnv is no longer supported; use serviceConfig".to_string(),
            );
        }
        if let Some(file_backend) = &self.file_backend {
            file_backend.validate()?;
        }
        for (index, service_config) in self.service_config.iter().enumerate() {
            service_config.reject_legacy_config_fields(index)?;
        }
        Ok(())
    }
}

impl FileBackendControlConfig {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.local.is_none() && self.oss.is_none() {
            return Err("router.control fileBackend must configure local or oss".to_string());
        }
        if let Some(local) = &self.local {
            local.validate()?;
        }
        if let Some(oss) = &self.oss {
            oss.validate()?;
        }
        Ok(())
    }
}

impl FileBackendLocalConfig {
    fn validate(&self) -> std::result::Result<(), String> {
        if self.root.as_os_str().is_empty() {
            return Err("router.control fileBackend.local.root must be non-empty".to_string());
        }
        Ok(())
    }
}

impl FileBackendOssConfig {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.endpoint.trim().is_empty() {
            return Err("router.control fileBackend.oss.endpoint must be non-empty".to_string());
        }
        if self.bucket.trim().is_empty() {
            return Err("router.control fileBackend.oss.bucket must be non-empty".to_string());
        }
        if missing_optional_string(self.access_key_id.as_deref())
            && missing_optional_string(self.access_key_id_env.as_deref())
        {
            return Err(
                "router.control fileBackend.oss requires accessKeyIdEnv or accessKeyId".to_string(),
            );
        }
        if missing_optional_string(self.access_key_secret.as_deref())
            && missing_optional_string(self.access_key_secret_env.as_deref())
        {
            return Err(
                "router.control fileBackend.oss requires accessKeySecretEnv or accessKeySecret"
                    .to_string(),
            );
        }
        Ok(())
    }
}

fn missing_optional_string(value: Option<&str>) -> bool {
    match value {
        Some(value) => value.is_empty(),
        None => true,
    }
}

fn deserialize_optional_config_shape<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<ConfigShape>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptionalConfigShapeVisitor;

    impl<'de> de::Visitor<'de> for OptionalConfigShapeVisitor {
        type Value = Option<ConfigShape>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a config shape object")
        }

        fn visit_none<E>(self) -> std::result::Result<Self::Value, E>
        where
            E: de::Error,
        {
            Err(E::custom("configShape must be an object when present"))
        }

        fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            let shape = ConfigShape::deserialize(deserializer)?;
            shape.validate_schema_version().map_err(de::Error::custom)?;
            Ok(Some(shape))
        }
    }

    deserializer.deserialize_option(OptionalConfigShapeVisitor)
}

fn ordered_artifact_roots(artifact_roots: &[PathBuf]) -> std::result::Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();
    for root in artifact_roots {
        push_artifact_root(&mut roots, root)?;
    }
    if roots.is_empty() {
        return Err("router.control artifactRoots must include at least one root".to_string());
    }
    Ok(roots)
}

fn push_artifact_root(roots: &mut Vec<PathBuf>, root: &PathBuf) -> std::result::Result<(), String> {
    if root.as_os_str().is_empty() {
        return Err("router.control artifact roots must not be empty".to_string());
    }
    if roots.iter().any(|existing| existing == root) {
        return Err(format!(
            "router.control artifactRoots contains duplicate root {}",
            root.display()
        ));
    }
    roots.push(root.clone());
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterControlServiceConfig {
    pub service_id: String,
    pub build_id: String,
    pub activation_identity: String,
    pub resolved_config_identity: String,
    #[serde(default)]
    pub resolved_config: Value,
    #[serde(default)]
    pub redacted_resolved_config: Value,
    #[serde(default)]
    pub redaction_projection_identity: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_config_shape",
        skip_serializing_if = "Option::is_none"
    )]
    pub config_shape: Option<ConfigShape>,
    #[serde(default)]
    pub service_db: Option<RouterControlServiceDb>,
    #[serde(default)]
    pub package_configs: Vec<RouterControlPackageConfig>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterControlServiceDb {
    pub mongo_url: String,
    pub storage_service_id: String,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterControlPackageConfig {
    pub package_id: String,
    #[serde(default)]
    pub package_slot: Option<usize>,
    pub alias: String,
    pub resolved_config_identity: String,
    #[serde(default)]
    pub resolved_config: Value,
    #[serde(default)]
    pub redacted_resolved_config: Value,
    #[serde(default)]
    pub redaction_projection_identity: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_optional_config_shape",
        skip_serializing_if = "Option::is_none"
    )]
    pub config_shape: Option<ConfigShape>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl RouterControlServiceConfig {
    fn reject_legacy_config_fields(&self, index: usize) -> std::result::Result<(), String> {
        for field in [
            "valuesSnapshotIdentity",
            "valuesSnapshot",
            "redactedValuesSnapshot",
            "valuesPolicy",
            "resolvedEnvIdentity",
            "resolvedEnv",
            "redactedResolvedEnv",
            "envShape",
            "dependencyRef",
        ] {
            if self.extra.contains_key(field) {
                return Err(format!(
                    "router.control serviceConfig[{index}].{field} is no longer supported"
                ));
            }
        }
        for (package_index, package_config) in self.package_configs.iter().enumerate() {
            package_config.reject_legacy_config_fields(index, package_index)?;
        }
        if let Some(service_db) = &self.service_db {
            service_db.reject_legacy_config_fields(index)?;
        }
        Ok(())
    }
}

impl RouterControlServiceDb {
    fn reject_legacy_config_fields(&self, index: usize) -> std::result::Result<(), String> {
        if self.extra.contains_key("storageNamespace") {
            return Err(format!(
                "router.control serviceConfig[{index}].serviceDb.storageNamespace is no longer supported"
            ));
        }
        Ok(())
    }
}

impl RouterControlPackageConfig {
    fn reject_legacy_config_fields(
        &self,
        service_index: usize,
        package_index: usize,
    ) -> std::result::Result<(), String> {
        for field in [
            "valuesSnapshotIdentity",
            "valuesSnapshot",
            "redactedValuesSnapshot",
            "valuesPolicy",
            "resolvedEnvIdentity",
            "resolvedEnv",
            "redactedResolvedEnv",
            "envShape",
        ] {
            if self.extra.contains_key(field) {
                return Err(format!(
                    "router.control serviceConfig[{service_index}].packageConfigs[{package_index}].{field} is no longer supported"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSendEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: &'static str,
    pub service_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub websocket_entry_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
    pub payload_kind: String,
}

#[derive(Debug, Deserialize)]
pub struct TypedEnvelope {
    #[serde(rename = "type")]
    pub envelope_type: String,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}
