use std::{
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
};

use skiff_runtime_capability_context::{DbProviderSource, HttpRuntimeOptions};
use skiff_runtime_model::request_heap::RequestHeapLimits;
use tokio::sync::Mutex;

use crate::{
    config::{skiff_file_tmp_dir, RuntimeMemoryBudgets},
    error::Result,
    loader::assembly_admission::AssemblyAdmissionController,
};

use super::{
    blob_store::BlobStore,
    file_runtime::FileRuntime,
    request_supervisor::RequestSupervisor,
    spawn_worker,
    telemetry::{TelemetryConfig, TelemetryExporterHandle, TelemetryProducer},
    websocket_generation::WebSocketGenerationRegistry,
    OutboundRequestRegistry,
};

#[derive(Clone)]
pub struct RuntimeConfig {
    pub db_provider: DbProviderSource,
    pub router_url: String,
    pub base_runtime_id: String,
    pub runtime_home: PathBuf,
    pub environment: String,
    pub artifact_root: PathBuf,
    pub http_response_max_bytes: usize,
    pub http_egress_proxy: Option<String>,
}

/// Production startup input for the canonical committed-assembly lifecycle.
///
/// Unlike the focused host-test configuration, this surface cannot carry legacy service
/// definitions. Startup recovers the exact environment tuple from the canonical artifact root.
#[derive(Clone)]
#[cfg(not(test))]
pub struct RuntimeProductionConfig {
    pub db_provider: DbProviderSource,
    pub router_url: String,
    pub base_runtime_id: String,
    pub runtime_home: PathBuf,
    pub environment: String,
    pub artifact_root: PathBuf,
    pub http_response_max_bytes: usize,
    pub http_egress_proxy: Option<String>,
}

#[derive(Clone)]
pub struct RuntimeHost {
    pub(super) router_url: String,
    pub(super) base_runtime_id: String,
    pub(super) runtime_home: PathBuf,
    pub(super) environment: String,
    pub(super) artifact_root: PathBuf,
    pub(super) default_http_response_max_bytes: usize,
    pub(super) http_runtime_options: HttpRuntimeOptions,
    pub(super) db_provider: DbProviderSource,
    pub(super) memory_budgets: RuntimeMemoryBudgets,
    pub(crate) assembly_admission: Arc<AssemblyAdmissionController>,
    pub(super) blob_store: Arc<StdMutex<Option<Arc<dyn BlobStore>>>>,
    pub(super) spawn_workers: Arc<spawn_worker::SpawnWorkerRegistry>,
    pub(super) request_supervisor: Arc<RequestSupervisor>,
    pub(super) websocket_generations: Arc<WebSocketGenerationRegistry>,
    pub(super) telemetry: TelemetryProducer,
    pub(super) telemetry_exporter: Arc<Mutex<Option<TelemetryExporterHandle>>>,
    pub(crate) outbound_requests: Arc<OutboundRequestRegistry>,
}

impl RuntimeHost {
    #[cfg(not(test))]
    pub fn new_production(config: RuntimeProductionConfig) -> anyhow::Result<Self> {
        Self::new(RuntimeConfig {
            db_provider: config.db_provider,
            router_url: config.router_url,
            base_runtime_id: config.base_runtime_id,
            runtime_home: config.runtime_home,
            environment: config.environment,
            artifact_root: config.artifact_root,
            http_response_max_bytes: config.http_response_max_bytes,
            http_egress_proxy: config.http_egress_proxy,
        })
    }

    pub fn new(config: RuntimeConfig) -> anyhow::Result<Self> {
        let db_provider = config.db_provider.clone();
        let http_runtime_options = runtime_http_options_from_config(config.http_egress_proxy)?;
        let (environment, artifact_root) = {
            skiff_artifact_model::validate_activation_environment(&config.environment)
                .map_err(|error| anyhow::anyhow!("runtime environment is invalid: {error}"))?;
            if config.artifact_root.as_os_str().is_empty() {
                anyhow::bail!("runtime artifact root must be a non-empty path");
            }
            (config.environment.clone(), config.artifact_root.clone())
        };
        let producer_id = format!(
            "{}:proc:{}",
            config.base_runtime_id,
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .chars()
                .take(8)
                .collect::<String>()
        );
        let telemetry = TelemetryProducer::new(TelemetryConfig::for_runtime(
            producer_id,
            config.base_runtime_id.clone(),
        ));
        Ok(Self {
            router_url: config.router_url,
            base_runtime_id: config.base_runtime_id.clone(),
            runtime_home: config.runtime_home,
            environment,
            artifact_root,
            default_http_response_max_bytes: config.http_response_max_bytes,
            http_runtime_options,
            db_provider,
            memory_budgets: RuntimeMemoryBudgets::default(),
            assembly_admission: Arc::new(AssemblyAdmissionController::new(
                config.base_runtime_id.clone(),
            )),
            blob_store: Arc::new(StdMutex::new(None)),
            spawn_workers: Arc::new(spawn_worker::SpawnWorkerRegistry::default()),
            request_supervisor: Arc::new(RequestSupervisor::new()),
            websocket_generations: Arc::new(WebSocketGenerationRegistry::default()),
            telemetry,
            telemetry_exporter: Arc::new(Mutex::new(None)),
            outbound_requests: Arc::new(OutboundRequestRegistry::default()),
        })
    }

    pub async fn shutdown_telemetry(&self) {
        self.stop_telemetry_exporter().await;
    }

    pub fn blob_store(&self) -> Option<Arc<dyn BlobStore>> {
        self.blob_store
            .lock()
            .ok()
            .and_then(|store| store.as_ref().cloned())
    }

    pub(super) fn file_runtime(&self) -> Arc<FileRuntime> {
        Arc::new(FileRuntime::new(
            self.blob_store(),
            skiff_file_tmp_dir(&self.runtime_home),
        ))
    }

    pub(crate) fn request_heap_limits(&self) -> RequestHeapLimits {
        let mut limits = RequestHeapLimits::default();
        limits.max_estimated_bytes = self.memory_budgets.request_heap_bytes;
        limits
    }

    pub(super) fn production_assembly_resolver(
        &self,
    ) -> Result<skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver> {
        if self.artifact_root.as_os_str().is_empty() {
            return Err(crate::error::RuntimeError::invalid_artifact(
                "whole-assembly activation requires exactly one configured canonical artifact root"
                    .to_string(),
            ));
        }
        skiff_runtime_loader::FilesystemRuntimeAssemblyContentResolver::open(&self.artifact_root)
            .map_err(|error| crate::error::RuntimeError::invalid_artifact(error.to_string()))
    }
}

fn runtime_http_options_from_config(
    http_egress_proxy: Option<String>,
) -> anyhow::Result<HttpRuntimeOptions> {
    let http_egress_proxy = http_egress_proxy
        .map(|proxy| validate_runtime_http_egress_proxy(&proxy))
        .transpose()?;
    Ok(HttpRuntimeOptions::from_env().with_egress_proxy(http_egress_proxy))
}

fn validate_runtime_http_egress_proxy(raw: &str) -> anyhow::Result<String> {
    if raw.trim().is_empty() {
        anyhow::bail!("runtime config http.egress.proxy must be a non-empty string");
    }
    let url = reqwest::Url::parse(raw)
        .map_err(|_| anyhow::anyhow!("runtime config http.egress.proxy is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        anyhow::bail!("runtime config http.egress.proxy must use http or https scheme");
    }
    if url.host().is_none() {
        anyhow::bail!("runtime config http.egress.proxy must be an absolute URL with host");
    }
    Ok(url.to_string())
}
