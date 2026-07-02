use std::{
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, RwLock},
};

use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_capability_context::{DbProviderConfig, DbProviderSource, HttpRuntimeOptions};
use skiff_runtime_linked_program::{LinkedProgramImage, RuntimeProgramIdentity};
use skiff_runtime_model::request_heap::RequestHeapLimits;
use tokio::sync::Mutex;

use crate::{
    artifact_cache::RuntimeArtifactCaches, config::skiff_file_tmp_dir,
    config_view::RuntimeConfigView, error::Result, loader::ArtifactLoadOptions,
};

use super::{
    blob_store::BlobStore,
    file_runtime::FileRuntime,
    request_supervisor::RequestSupervisor,
    route_registry,
    service_context::ServiceRuntimeContext,
    spawn_worker,
    state::ArtifactLoadState,
    telemetry::{TelemetryConfig, TelemetryExporterHandle, TelemetryProducer},
    LoadedBuildRegistry, OutboundRequestRegistry, ServiceRouteState,
};

#[derive(Clone)]
pub struct RuntimeConfig {
    pub db_provider: DbProviderSource,
    pub services: Vec<RuntimeServiceConfig>,
    pub router_url: String,
    pub base_runtime_id: String,
    pub runtime_home: PathBuf,
    pub artifact_roots: Vec<PathBuf>,
    pub http_response_max_bytes: usize,
    pub http_egress_proxy: Option<String>,
}

#[derive(Clone)]
pub struct RuntimeServiceConfig {
    pub runtime_program_identity: RuntimeProgramIdentity,
    pub linked_image: Arc<LinkedProgramImage>,
    pub runtime_activation: Arc<RuntimeActivation>,
    pub http_response_max_bytes: usize,
    pub use_runtime_default_http_response_max_bytes: bool,
    pub runtime_id: String,
    pub revision_id: String,
    pub contract_identity: String,
    pub implementation_identity: String,
    pub artifact_identity: String,
    pub activation_identity: Option<String>,
    pub resolved_config_identity: Option<String>,
    pub config: RuntimeConfigView,
    pub package_configs: Vec<RuntimeConfigView>,
    pub service_db: Option<DbProviderConfig>,
}

#[derive(Clone)]
pub struct RuntimeHost {
    pub(super) router_url: String,
    pub(super) base_runtime_id: String,
    pub(super) runtime_home: PathBuf,
    pub(super) default_http_response_max_bytes: usize,
    pub(super) http_runtime_options: HttpRuntimeOptions,
    pub(super) db_provider: DbProviderSource,
    pub(super) configured_artifact_roots: Arc<Vec<PathBuf>>,
    pub(super) artifact_load_state: Arc<Mutex<ArtifactLoadState>>,
    pub(super) artifact_caches: Arc<RuntimeArtifactCaches>,
    pub(super) package_test_start_executor:
        Arc<super::package_test_entry::PackageTestStartExecutor>,
    pub(super) package_test_template_builds:
        Arc<super::package_test_entry::PackageTestTemplateBuildLocks>,
    pub(super) blob_store: Arc<StdMutex<Option<Arc<dyn BlobStore>>>>,
    pub(crate) state: Arc<RwLock<ServiceRouteState>>,
    pub(super) loaded_builds: Arc<LoadedBuildRegistry>,
    pub(super) spawn_workers: Arc<spawn_worker::SpawnWorkerRegistry>,
    pub(super) request_supervisor: Arc<RequestSupervisor>,
    pub(super) telemetry: TelemetryProducer,
    pub(super) telemetry_exporter: Arc<Mutex<Option<TelemetryExporterHandle>>>,
    pub(crate) outbound_requests: Arc<OutboundRequestRegistry>,
}

impl RuntimeHost {
    pub fn new(config: RuntimeConfig) -> anyhow::Result<Self> {
        let db_provider = config.db_provider.clone();
        let http_runtime_options = runtime_http_options_from_config(config.http_egress_proxy)?;
        let services = route_registry::apply_default_http_response_limits(
            config.services,
            config.http_response_max_bytes,
        );
        let state = route_registry::build_service_route_state(
            services,
            config.http_response_max_bytes,
            &db_provider,
        )?;
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
        let loaded_builds = Arc::new(LoadedBuildRegistry::from_build_ids(state.build_ids()));
        Ok(Self {
            router_url: config.router_url,
            base_runtime_id: config.base_runtime_id,
            runtime_home: config.runtime_home,
            default_http_response_max_bytes: config.http_response_max_bytes,
            http_runtime_options,
            db_provider,
            configured_artifact_roots: Arc::new(config.artifact_roots.clone()),
            artifact_load_state: Arc::new(Mutex::new(ArtifactLoadState {
                artifact_roots: config.artifact_roots,
                load_options: ArtifactLoadOptions::release(),
                service_config: Vec::new(),
                epoch: 0,
            })),
            artifact_caches: Arc::new(RuntimeArtifactCaches::new()),
            package_test_start_executor: Arc::new(
                super::package_test_entry::PackageTestStartExecutor::default(),
            ),
            package_test_template_builds: Arc::new(
                super::package_test_entry::PackageTestTemplateBuildLocks::default(),
            ),
            blob_store: Arc::new(StdMutex::new(None)),
            state: Arc::new(RwLock::new(state)),
            loaded_builds,
            spawn_workers: Arc::new(spawn_worker::SpawnWorkerRegistry::default()),
            request_supervisor: Arc::new(RequestSupervisor::new()),
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

    pub(crate) fn begin_build_execution(
        &self,
        build_id: &str,
    ) -> Result<super::BuildExecutionGuard> {
        self.loaded_builds.begin_execution(build_id)
    }

    pub(crate) fn request_heap_limits(&self) -> RequestHeapLimits {
        let mut limits = RequestHeapLimits::default();
        limits.max_estimated_bytes = self.artifact_caches.memory_budgets().request_heap_bytes;
        limits
    }

    pub(crate) fn service_snapshot(&self) -> Vec<Arc<ServiceRuntimeContext>> {
        self.state
            .read()
            .map(|state| state.services.iter().cloned().collect())
            .unwrap_or_default()
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
