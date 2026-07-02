use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use serde::Deserialize;
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
/// kept only as cheap defense-in-depth — 64 MiB is virtual address space committed
/// lazily, so it costs nothing until used — and is no longer load-bearing.
pub const RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RuntimeFileConfig {
    pub router: String,
    pub runtime_home: PathBuf,
    pub artifact_roots: Vec<PathBuf>,
    pub http_response_max_bytes: usize,
    pub http_egress_proxy: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRuntimeFileConfig {
    router: String,
    #[serde(alias = "runtime-home")]
    runtime_home: PathBuf,
    #[serde(default, alias = "artifact-roots")]
    artifact_roots: Vec<PathBuf>,
    #[serde(default)]
    services: Option<serde_yaml::Value>,
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
                "runtime config no longer supports services; use artifactRoots for local runtime artifact load paths"
            );
        }

        Ok(Self {
            router: raw.router,
            runtime_home,
            artifact_roots: resolve_relative_paths(base_dir, raw.artifact_roots)?,
            http_response_max_bytes: runtime_http_response_max_bytes_from_value(&value)?,
            http_egress_proxy: runtime_http_egress_proxy_from_value(&value)?,
        })
    }
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
            "runtime config no longer supports {key}; use artifactRoots for local runtime artifact load paths"
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

fn resolve_relative_paths(base_dir: &Path, paths: Vec<PathBuf>) -> anyhow::Result<Vec<PathBuf>> {
    let mut resolved = Vec::new();
    for (index, path) in paths.into_iter().enumerate() {
        if path.as_os_str().is_empty() {
            anyhow::bail!("runtime config artifactRoots[{index}] must be a non-empty path");
        }
        resolved.push(resolve_relative_path(base_dir, path));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests;
