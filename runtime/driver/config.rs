use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use skiff_runtime_host::host::telemetry::RuntimeTelemetryConfig;
use url::Url;

pub const DEFAULT_HTTP_RESPONSE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Stack size for the runtime's tokio worker threads.
///
/// History: skiff stream consumption used to co-drive each producer inline with
/// its consumer in a single tokio task, so a `for event in <stream-producer>`
/// whose producer body consumed another stream-producer nested
/// `run_stream_producer` / `exec_program_stream_for_in` / `call_program_executable`
/// (`#[async_recursion]`) state machines all polled on one native stack. The LLM
/// streaming path (`agentLlm.streamChat` -> `sampleAgentLlmEventsFromSseBody` ->
/// remoteLlm `streamManagedChat` -> `llm-api decode` -> `sendChatBody` ->
/// `emitProviderSse` -> `std.http.sse`) chains ~8 producers, and the previous
/// 8 MiB stack overflowed ("thread 'tokio-rt-worker' has overflowed its stack")
/// and crash-looped the runtime. Raising the stack to 64 MiB was the original
/// mitigation.
///
/// Root fix (current): every `Stream` producer now runs in its own
/// `tokio::spawn`ed task (see `spawn_stream_producer` in
/// `eval/program_stream.rs`, backed by `OwnedProgramExecutionContext`), so the
/// consumer only polls the bounded stream channel and native stack depth is
/// constant regardless of producer nesting. The depth dependence is gone (proven
/// by `runtime_program_deeply_nested_stream_producers_are_stack_depth_independent`,
/// which runs a 40+ deep chain on a 1 MiB stack). This generous size is therefore
/// kept only as cheap defense-in-depth — both sizes below are virtual address
/// space committed lazily, so they cost nothing until used — and is no longer
/// load-bearing.
///
/// `MAX_PROGRAM_CALL_DEPTH` (128) is the other half of the recursion defense:
/// ordinary non-tail calls still nest `#[async_recursion]` evaluator futures on
/// the worker's native stack, so the guard must fire before the stack is
/// exhausted. Measured per-layer native consumption on this codebase
/// (non-tail countdown, one evaluator frame per logical call): release
/// ~272 KiB/layer, debug ~1.04 MiB/layer (unoptimized frames are much larger).
/// The worker stack therefore differs by build profile:
///
/// - release (production deploy profile): 128 MiB, comfortably above the
///   measured ~34 MiB (pre-unification) / ~60-80 MiB (post-unification) for a
///   full 128-layer chain; see
///   `runtime_program_non_tail_recursion_deep_chain_hits_raised_guard` tests
///   (ignored by default; run with `--ignored` and a larger
///   `SKIFF_NON_TAIL_DEPTH_STACK_KIB` for stack characterization).
/// - debug (`debug_assertions`, used by `cargo test`, the dev instance and CI):
///   384 MiB. After the HeapAccess unification the measured per-layer native
///   stack is ~1.5 MiB, so a 128-layer chain alone reaches ~192 MiB and real
///   paths (websocket connect, stream producers) add frames on top — the old
///   192 MiB worker stack overflowed (`tokio-rt-worker has overflowed its
///   stack`) during websocket connect. The guard at 128 must never be the
///   *first* native-stack boundary for an unoptimized build.
#[cfg(debug_assertions)]
pub const RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES: usize = 384 * 1024 * 1024;
#[cfg(not(debug_assertions))]
pub const RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RuntimeFileConfig {
    pub router: String,
    pub runtime_home: PathBuf,
    pub service_db_encryption_keyring_file: Option<PathBuf>,
    pub http_response_max_bytes: usize,
    pub http_egress_proxy: Option<String>,
    pub telemetry: Option<RuntimeTelemetryConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeFileConfig {
    router: String,
    #[serde(alias = "runtime-home")]
    runtime_home: PathBuf,
    #[serde(default)]
    service_db: Option<RawRuntimeServiceDbConfig>,
    #[serde(default)]
    services: Option<serde_yaml::Value>,
    #[serde(default)]
    telemetry: Option<RawRuntimeTelemetryConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeTelemetryConfig {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    file_path: Option<String>,
    #[serde(default)]
    file_max_bytes: Option<u64>,
    #[serde(default)]
    file_max_files: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeServiceDbConfig {
    #[serde(default)]
    encryption: Option<RawRuntimeServiceDbEncryptionConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawRuntimeServiceDbEncryptionConfig {
    #[serde(default)]
    keyring_file: Option<String>,
}

impl RuntimeFileConfig {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("failed to read {}: {error}", path.display()))?;
        let value: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|error| {
            anyhow::anyhow!("failed to parse {} as YAML: {error}", path.display())
        })?;
        reject_unsupported_top_level_key(&value, "artifact")?;
        reject_unsupported_top_level_key(&value, "artifacts")?;
        reject_unsupported_top_level_key(&value, "artifactRoots")?;
        reject_unsupported_top_level_key(&value, "artifact-roots")?;
        reject_unsupported_top_level_key(&value, "artifactRoot")?;
        let mut raw_value = value.clone();
        remove_top_level_key(&mut raw_value, "http");
        let raw: RawRuntimeFileConfig = serde_yaml::from_value(raw_value).map_err(|error| {
            anyhow::anyhow!(
                "failed to parse {} as runtime config: {error}",
                path.display()
            )
        })?;
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let runtime_home = resolve_relative_path(base_dir, raw.runtime_home);
        if raw.services.is_some() {
            anyhow::bail!(
                "runtime config no longer supports services; Router bootstrap owns artifact loading"
            );
        }

        Ok(Self {
            router: raw.router,
            runtime_home,
            service_db_encryption_keyring_file: runtime_service_db_encryption_keyring_file(
                base_dir,
                raw.service_db,
            )?,
            http_response_max_bytes: runtime_http_response_max_bytes_from_value(&value)?,
            http_egress_proxy: runtime_http_egress_proxy_from_value(&value)?,
            telemetry: runtime_telemetry(raw.telemetry)?,
        })
    }
}

/// Parses the telemetry block into the host producer config.
///
/// `enabled: false` disables telemetry entirely (`None`); an absent telemetry
/// block defaults to the JSONL file sink (three-state: endpoint / file /
/// disabled). `endpoint: Some(non-empty)` selects the WS exporter, otherwise
/// the default JSONL file sink is used. The `filePath` override is passed
/// through unresolved: absolute paths are used directly, relative paths
/// resolve against the default `<runtime_home.parent()>/logs/telemetry` root
/// in the host.
fn runtime_telemetry(
    telemetry: Option<RawRuntimeTelemetryConfig>,
) -> anyhow::Result<Option<RuntimeTelemetryConfig>> {
    if telemetry.as_ref().is_some_and(|t| t.enabled == Some(false)) {
        return Ok(None);
    }
    let telemetry = telemetry.unwrap_or(RawRuntimeTelemetryConfig {
        enabled: None,
        endpoint: None,
        file_path: None,
        file_max_bytes: None,
        file_max_files: None,
    });
    Ok(Some(RuntimeTelemetryConfig {
        endpoint: telemetry.endpoint.filter(|endpoint| !endpoint.trim().is_empty()),
        file_path: telemetry.file_path.map(PathBuf::from),
        file_max_bytes: telemetry.file_max_bytes,
        file_max_files: telemetry.file_max_files,
    }))
}

fn runtime_service_db_encryption_keyring_file(
    base_dir: &Path,
    service_db: Option<RawRuntimeServiceDbConfig>,
) -> anyhow::Result<Option<PathBuf>> {
    let Some(keyring_file) = service_db
        .and_then(|service_db| service_db.encryption)
        .and_then(|encryption| encryption.keyring_file)
    else {
        return Ok(None);
    };
    if keyring_file.trim().is_empty() {
        anyhow::bail!("runtime config serviceDb.encryption.keyringFile must be a non-empty string");
    }
    Ok(Some(resolve_relative_path(
        base_dir,
        PathBuf::from(keyring_file),
    )))
}

fn runtime_http_response_max_bytes_from_value(value: &serde_yaml::Value) -> anyhow::Result<usize> {
    let Some(max_bytes) = yaml_path(value, &["http", "response", "maxBytes"]) else {
        return Ok(DEFAULT_HTTP_RESPONSE_MAX_BYTES);
    };
    parse_runtime_http_response_max_bytes(max_bytes)
}

fn runtime_http_egress_proxy_from_value(
    value: &serde_yaml::Value,
) -> anyhow::Result<Option<String>> {
    let Some(proxy) = yaml_path(value, &["http", "egress", "proxy"]) else {
        return Ok(None);
    };
    if proxy.is_null() {
        return Ok(None);
    }
    let raw = proxy
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("runtime config http.egress.proxy must be a string"))?;
    Ok(Some(parse_runtime_http_egress_proxy(raw)?))
}

fn yaml_path<'a>(value: &'a serde_yaml::Value, path: &[&str]) -> Option<&'a serde_yaml::Value> {
    path.iter().try_fold(value, |current, segment| {
        current
            .as_mapping()?
            .get(serde_yaml::Value::String((*segment).to_string()))
    })
}

fn parse_runtime_http_response_max_bytes(value: &serde_yaml::Value) -> anyhow::Result<usize> {
    let number = if let Some(number) = value.as_u64() {
        number
    } else if let Some(number) = value.as_f64() {
        if !number.is_finite() || number.fract() != 0.0 || number < 0.0 {
            anyhow::bail!("runtime config http.response.maxBytes must be a positive integer");
        }
        if number >= u64::MAX as f64 {
            anyhow::bail!(
                "runtime config http.response.maxBytes must fit within system integer size"
            );
        }
        number as u64
    } else {
        anyhow::bail!("runtime config http.response.maxBytes must be a positive integer");
    };

    if number == 0 {
        anyhow::bail!("runtime config http.response.maxBytes must be greater than zero");
    }

    usize::try_from(number).map_err(|_| {
        anyhow::anyhow!("runtime config http.response.maxBytes must fit within system integer size")
    })
}

fn parse_runtime_http_egress_proxy(raw: &str) -> anyhow::Result<String> {
    if raw.trim().is_empty() {
        anyhow::bail!("runtime config http.egress.proxy must be a non-empty string");
    }
    let url = Url::parse(raw)
        .map_err(|_| anyhow::anyhow!("runtime config http.egress.proxy is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("runtime config http.egress.proxy must use http or https scheme");
    }
    if url.host().is_none() {
        anyhow::bail!("runtime config http.egress.proxy must be an absolute URL with host");
    }
    Ok(url.to_string())
}

fn reject_unsupported_top_level_key(value: &serde_yaml::Value, key: &str) -> anyhow::Result<()> {
    let Some(mapping) = value.as_mapping() else {
        return Ok(());
    };
    if mapping.contains_key(serde_yaml::Value::String(key.to_string())) {
        anyhow::bail!(
            "runtime config no longer supports {key}; Router bootstrap owns artifactsPath"
        );
    }
    Ok(())
}

fn remove_top_level_key(value: &mut serde_yaml::Value, key: &str) {
    let Some(mapping) = value.as_mapping_mut() else {
        return;
    };
    mapping.remove(serde_yaml::Value::String(key.to_string()));
}

pub fn prepare_runtime_home(runtime_home: &Path) -> anyhow::Result<String> {
    fs::create_dir_all(runtime_home)
        .map_err(|error| anyhow::anyhow!("failed to create {}: {error}", runtime_home.display()))?;
    fs::create_dir_all(runtime_home.join("cache").join("artifacts")).map_err(|error| {
        anyhow::anyhow!(
            "failed to create {}: {error}",
            runtime_home.join("cache").join("artifacts").display()
        )
    })?;
    let tmp_dir = runtime_home.join("tmp");
    fs::create_dir_all(&tmp_dir)
        .map_err(|error| anyhow::anyhow!("failed to create {}: {error}", tmp_dir.display()))?;
    prepare_skiff_file_tmp_dir(&tmp_dir)?;

    let runtime_id_path = runtime_home.join("runtime-id");
    match fs::read_to_string(&runtime_id_path) {
        Ok(value) => {
            let runtime_id = value.trim().to_string();
            if runtime_id.is_empty() {
                anyhow::bail!("{} is empty", runtime_id_path.display());
            }
            return Ok(runtime_id);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(anyhow::anyhow!(
                "failed to read {}: {error}",
                runtime_id_path.display()
            ));
        }
    }

    let runtime_id = format!("runtime-{}", uuid::Uuid::new_v4());
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&runtime_id_path)
    {
        Ok(mut file) => {
            writeln!(file, "{runtime_id}").map_err(|error| {
                anyhow::anyhow!("failed to write {}: {error}", runtime_id_path.display())
            })?;
            file.sync_all().map_err(|error| {
                anyhow::anyhow!("failed to sync {}: {error}", runtime_id_path.display())
            })?;
            Ok(runtime_id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let value = fs::read_to_string(&runtime_id_path).map_err(|read_error| {
                anyhow::anyhow!("failed to read {}: {read_error}", runtime_id_path.display())
            })?;
            let runtime_id = value.trim().to_string();
            if runtime_id.is_empty() {
                anyhow::bail!("{} is empty", runtime_id_path.display());
            }
            Ok(runtime_id)
        }
        Err(error) => Err(anyhow::anyhow!(
            "failed to create {}: {error}",
            runtime_id_path.display()
        )),
    }
}

pub fn skiff_file_tmp_dir(runtime_home: &Path) -> PathBuf {
    runtime_home.join("tmp").join("skiff-file")
}

fn prepare_skiff_file_tmp_dir(tmp_dir: &Path) -> anyhow::Result<()> {
    let skiff_file = tmp_dir.join("skiff-file");
    match fs::remove_dir_all(&skiff_file) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(anyhow::anyhow!(
                "failed to clean {}: {error}",
                skiff_file.display()
            ));
        }
    }
    fs::create_dir_all(&skiff_file)
        .map_err(|error| anyhow::anyhow!("failed to create {}: {error}", skiff_file.display()))
}

fn resolve_relative_path(base_dir: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        base_dir.join(path)
    }
}

#[cfg(test)]
mod tests;
