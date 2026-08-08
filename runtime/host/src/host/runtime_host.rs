use std::{
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, OnceLock},
};

use skiff_runtime_capability_context::{
    ConnectionRequestRegistry, DbProviderSource, HttpRuntimeOptions,
};
use skiff_runtime_eval::actor_instance::{
    ActorInstanceFence, ActorInstanceHandle, ActorInstanceSessionTrackError,
    ActorInstanceSessionTracker, ActorInstanceStore,
};
use skiff_runtime_model::request_heap::RequestHeapLimits;
use tokio::sync::Mutex;

use crate::{
    config::{skiff_file_tmp_dir, RuntimeMemoryBudgets},
    loader::assembly_admission::AssemblyAdmissionController,
};

use super::{
    actor_owner_invocations::ActorOwnerInvocationRegistry,
    actor_route_holds::ActorRouteHoldRegistry,
    blob_store::BlobStore,
    file_runtime::FileRuntime,
    request_supervisor::RequestSupervisor,
    telemetry::{
        RuntimeTelemetryConfig, TelemetryConfig, TelemetryExporterHandle, TelemetryFileSink,
        TelemetryFileSinkHandle, TelemetryProducer,
    },
    OutboundRequestRegistry,
};
use crate::capability_context::actor_method_outbound::ActorMethodOutboundRegistry;
use crate::capability_context::TestHttpEntryRegistry;

#[derive(Clone)]
pub struct RuntimeConfig {
    pub db_provider: DbProviderSource,
    pub router_url: String,
    pub base_runtime_id: String,
    pub runtime_home: PathBuf,
    pub profile: String,
    pub http_response_max_bytes: usize,
    pub http_egress_proxy: Option<String>,
}

/// Production startup input for the canonical committed-assembly lifecycle.
///
/// Unlike the focused host-test configuration, this surface cannot carry legacy service
/// definitions. Router bootstrap supplies the connection-scoped artifact path and DB transport.
#[derive(Clone)]
#[cfg(not(test))]
pub struct RuntimeProductionConfig {
    pub db_provider: DbProviderSource,
    pub router_url: String,
    pub base_runtime_id: String,
    pub runtime_home: PathBuf,
    pub http_response_max_bytes: usize,
    pub http_egress_proxy: Option<String>,
    pub telemetry: Option<RuntimeTelemetryConfig>,
}

#[derive(Clone)]
pub struct RuntimeHost {
    pub(super) router_url: String,
    pub(super) base_runtime_id: String,
    pub(super) runtime_home: PathBuf,
    pub(super) frozen_profile: OnceLock<String>,
    pub(super) default_http_response_max_bytes: usize,
    pub(super) http_runtime_options: HttpRuntimeOptions,
    pub(super) memory_budgets: RuntimeMemoryBudgets,
    pub(crate) assembly_admission: Arc<AssemblyAdmissionController>,
    pub(super) artifact_root: Arc<StdMutex<Option<String>>>,
    pub(super) blob_store: Arc<StdMutex<Option<Arc<dyn BlobStore>>>>,
    pub(super) request_supervisor: Arc<RequestSupervisor>,
    pub(super) telemetry: TelemetryProducer,
    pub(super) telemetry_exporter: Arc<Mutex<Option<TelemetryExporterHandle>>>,
    pub(super) telemetry_file_sink: Arc<Mutex<Option<TelemetryFileSinkHandle>>>,
    pub(crate) outbound_requests: Arc<OutboundRequestRegistry>,
    pub(crate) connection_requests: Arc<ConnectionRequestRegistry>,
    pub(crate) actor_method_outbound: Arc<ActorMethodOutboundRegistry>,
    pub(crate) actor_owner_invocations: Arc<ActorOwnerInvocationRegistry>,
    pub(crate) actor_route_holds: Arc<ActorRouteHoldRegistry>,
    pub(crate) actor_instances: Arc<ActorInstanceSessionTracker>,
    pub(crate) test_http_entries: TestHttpEntryRegistry,
}

impl RuntimeHost {
    #[cfg(not(test))]
    pub async fn new_production(config: RuntimeProductionConfig) -> anyhow::Result<Self> {
        let host = Self::new_inner(
            config.db_provider,
            config.router_url,
            config.base_runtime_id,
            config.runtime_home,
            None,
            config.http_response_max_bytes,
            config.http_egress_proxy,
            config.telemetry.clone(),
        )?;
        match config.telemetry.as_ref() {
            // enabled:false / absent -> no producer sink.
            None => {}
            Some(telemetry) => match telemetry.endpoint.as_deref() {
                Some(endpoint) if !endpoint.trim().is_empty() => {
                    let exporter = super::telemetry::TelemetryExporter::new(
                        endpoint.to_string(),
                        host.telemetry.clone(),
                    )
                    .start();
                    let mut slot = host.telemetry_exporter.lock().await;
                    *slot = Some(exporter);
                }
                // endpoint missing/empty -> default JSONL file sink.
                _ => {
                    let sink = TelemetryFileSink::new(host.telemetry.clone()).start();
                    let mut slot = host.telemetry_file_sink.lock().await;
                    *slot = Some(sink);
                }
            },
        }
        Ok(host)
    }

    pub fn new(config: RuntimeConfig) -> anyhow::Result<Self> {
        Self::new_inner(
            config.db_provider,
            config.router_url,
            config.base_runtime_id,
            config.runtime_home,
            Some(config.profile.as_str()),
            config.http_response_max_bytes,
            config.http_egress_proxy,
            None,
        )
    }

    fn new_inner(
        db_provider: DbProviderSource,
        router_url: String,
        base_runtime_id: String,
        runtime_home: PathBuf,
        trusted_profile: Option<&str>,
        http_response_max_bytes: usize,
        http_egress_proxy: Option<String>,
        telemetry: Option<RuntimeTelemetryConfig>,
    ) -> anyhow::Result<Self> {
        let http_runtime_options = runtime_http_options_from_config(http_egress_proxy)?;
        let frozen_profile = OnceLock::new();
        if let Some(profile) = trusted_profile {
            skiff_artifact_model::validate_activation_profile(profile)
                .map_err(|error| anyhow::anyhow!("runtime profile is invalid: {error}"))?;
            let _ = frozen_profile.set(profile.to_string());
        }
        let producer_id = format!(
            "{}:proc:{}",
            base_runtime_id,
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .chars()
                .take(8)
                .collect::<String>()
        );
        let file_root = runtime_home
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("logs")
            .join("telemetry");
        let (telemetry_file_path, telemetry_file_max_bytes, telemetry_file_max_files) =
            match &telemetry {
                Some(telemetry) => (
                    telemetry.file_path.clone(),
                    telemetry.file_max_bytes,
                    telemetry.file_max_files,
                ),
                None => (None, None, None),
            };
        let telemetry = TelemetryProducer::new(TelemetryConfig::for_runtime(
            producer_id,
            base_runtime_id.clone(),
            file_root,
            telemetry_file_path,
            telemetry_file_max_bytes,
            telemetry_file_max_files,
        ));
        let actor_instance_store = Arc::new(ActorInstanceStore::new());
        Ok(Self {
            router_url,
            base_runtime_id: base_runtime_id.clone(),
            runtime_home,
            frozen_profile,
            default_http_response_max_bytes: http_response_max_bytes,
            http_runtime_options,
            memory_budgets: RuntimeMemoryBudgets::default(),
            assembly_admission: Arc::new(AssemblyAdmissionController::new(
                base_runtime_id.clone(),
                db_provider,
            )),
            artifact_root: Arc::new(StdMutex::new(None)),
            blob_store: Arc::new(StdMutex::new(None)),
            request_supervisor: Arc::new(RequestSupervisor::new()),
            telemetry,
            telemetry_exporter: Arc::new(Mutex::new(None)),
            telemetry_file_sink: Arc::new(Mutex::new(None)),
            outbound_requests: Arc::new(OutboundRequestRegistry::default()),
            connection_requests: Arc::new(ConnectionRequestRegistry::new(1024)),
            actor_method_outbound: Arc::new(ActorMethodOutboundRegistry::default()),
            actor_owner_invocations: Arc::new(ActorOwnerInvocationRegistry::default()),
            actor_route_holds: Arc::new(ActorRouteHoldRegistry::default()),
            actor_instances: Arc::new(ActorInstanceSessionTracker::new(actor_instance_store)),
            test_http_entries: TestHttpEntryRegistry::default(),
        })
    }

    /// Records the artifact root opened by the router bootstrap resolver. It is
    /// advertised in capabilities frames so the router can treat this runtime
    /// as a lazy-load candidate over the same store.
    pub(crate) fn set_bootstrap_artifact_root(&self, root: impl Into<String>) {
        if let Ok(mut slot) = self.artifact_root.lock() {
            if slot.is_none() {
                *slot = Some(root.into());
            }
        }
    }

    pub(crate) fn bootstrap_artifact_root(&self) -> Option<String> {
        self.artifact_root.lock().ok().and_then(|slot| slot.clone())
    }

    /// Freezes the trusted activation profile on first router bootstrap and
    /// fails closed when a later bootstrap disagrees with the frozen value.
    pub(crate) fn freeze_bootstrap_profile(&self, profile: &str) -> anyhow::Result<()> {
        skiff_artifact_model::validate_activation_profile(profile)
            .map_err(|error| anyhow::anyhow!("router bootstrap profile is invalid: {error}"))?;
        match self.frozen_profile.get_or_init(|| profile.to_string()) {
            frozen if frozen == profile => Ok(()),
            frozen => Err(anyhow::anyhow!(
                "router bootstrap profile {profile} does not match Runtime frozen profile {frozen}"
            )),
        }
    }

    pub(crate) fn actor_instance_session_lease(
        &self,
        router_session_id: &str,
    ) -> Result<
        skiff_runtime_eval::actor_instance::ActorInstanceSessionLease,
        ActorInstanceSessionTrackError,
    > {
        self.actor_instances.session_lease(router_session_id)
    }

    pub(crate) fn track_actor_instance_with_lease(
        &self,
        session: &skiff_runtime_eval::actor_instance::ActorInstanceSessionLease,
        handle: ActorInstanceHandle,
    ) -> Result<(), ActorInstanceSessionTrackError> {
        let cleanup_handle = handle.clone();
        let result = self.actor_instances.track_with_lease(session, handle);
        if matches!(
            result,
            Err(ActorInstanceSessionTrackError::SessionNotOpen { .. })
        ) {
            self.actor_instances.discard_if_untracked(&cleanup_handle);
        }
        result
    }

    pub(crate) fn open_actor_instance_session(
        &self,
        router_session_id: &str,
    ) -> Result<(), ActorInstanceSessionTrackError> {
        self.actor_instances.open_session(router_session_id)
    }

    pub(crate) fn discard_actor_instances_for_session(&self, router_session_id: &str) -> usize {
        self.actor_instances.discard_session(router_session_id)
    }

    pub(crate) fn begin_actor_upgrade_exact(
        &self,
        router_session_id: &str,
        fence: &ActorInstanceFence,
    ) -> bool {
        self.actor_instances
            .begin_upgrade_exact(router_session_id, fence)
    }

    pub(crate) fn discard_upgrading_actor_exact(
        &self,
        router_session_id: &str,
        fence: &ActorInstanceFence,
    ) -> bool {
        self.actor_instances
            .discard_upgrading_exact(router_session_id, fence)
    }

    pub(crate) fn discard_actor_exact(
        &self,
        router_session_id: &str,
        fence: &ActorInstanceFence,
    ) -> bool {
        self.actor_instances.discard_exact(router_session_id, fence)
    }

    pub fn shutdown_actor_instances(&self) -> usize {
        self.actor_instances.discard_all()
    }

    pub async fn shutdown_telemetry(&self) {
        self.stop_telemetry_exporter().await;
        if let Some(sink) = self.telemetry_file_sink.lock().await.take() {
            sink.shutdown(super::telemetry::EXPORTER_SHUTDOWN_FLUSH_TIMEOUT)
                .await;
        }
    }

    /// Clones the host telemetry producer for process-level emitters (e.g. the
    /// `rust.profile` sampler) that enqueue PlatformEvents outside any
    /// request-scoped context.
    pub fn telemetry_producer(&self) -> TelemetryProducer {
        self.telemetry.clone()
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
