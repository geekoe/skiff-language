use std::{
    collections::HashMap,
    env,
    sync::{Arc, Mutex as StdMutex},
};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;
use sha2::{Digest, Sha256};
use skiff_runtime_linked_program::{
    config_and_effect_metadata_shape, LinkedProgramImageResolverExt, RuntimeProgramIdentity,
};
use skiff_runtime_loader::ArtifactGraphCache;
use skiff_runtime_package_test::{
    LoadedPackageTestRuntimeProgram, PackageTestDispatchSelection, PackageTestRuntimeBuilder,
};
use tokio::sync::{mpsc, OwnedMutexGuard, Semaphore};
use tracing::warn;

use crate::{
    artifact_cache::PackageTestRuntimeTemplateCache,
    error::{Result, RuntimeError},
    loader::load_package_test_local_config,
};
use skiff_runtime_request::{
    RequestEffectDouble, RequestEnvelope, RuntimeOperation, RuntimeOperationParameter,
};
use skiff_runtime_transport::protocol::PackageTestStartFrameHeader;

use super::{
    route_registry::{build_service_db_source, package_test_revision_id},
    spawn_worker, RouterWriterMessage, RuntimeHost, ServiceOperationContext, ServiceRuntimeContext,
};

const PACKAGE_TEST_ACTIVATION_ID_PREFIX: &str = "skiff-package-test-run-v1:";
const PACKAGE_TEST_SERVICE_DB_PREFIX: &str = "skiff.run/package-test-db-";
const PACKAGE_TEST_START_CONCURRENCY_ENV: &str = "SKIFF_PACKAGE_TEST_START_CONCURRENCY";
const PACKAGE_TEST_START_MAX_SAFE_CONCURRENCY: usize = 1024;
const PACKAGE_TEST_START_MIN_ADMISSION: usize = 64;
const PACKAGE_TEST_START_ADMISSION_MULTIPLIER: usize = 8;

#[derive(Debug)]
pub(super) struct PackageTestStartExecutor {
    admission_permits: Arc<Semaphore>,
    start_permits: Arc<Semaphore>,
    #[cfg(test)]
    max_concurrency: usize,
    max_admission: usize,
    pending: Arc<StdMutex<HashMap<String, PendingPackageTestStart>>>,
}

#[derive(Debug, Default)]
struct PendingPackageTestStart {
    cancelled: bool,
}

#[derive(Debug)]
pub(crate) struct PackageTestPendingStart {
    request_id: String,
    pending: Arc<StdMutex<HashMap<String, PendingPackageTestStart>>>,
    finished: bool,
}

impl Default for PackageTestStartExecutor {
    fn default() -> Self {
        Self::new(default_package_test_start_concurrency())
    }
}

impl PackageTestStartExecutor {
    fn new(max_concurrency: usize) -> Self {
        let max_concurrency = sanitize_package_test_start_concurrency(max_concurrency);
        let max_admission = default_package_test_start_admission(max_concurrency);
        Self {
            admission_permits: Arc::new(Semaphore::new(max_admission)),
            start_permits: Arc::new(Semaphore::new(max_concurrency)),
            #[cfg(test)]
            max_concurrency,
            max_admission,
            pending: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    pub(super) fn submit(
        self: &Arc<Self>,
        host: RuntimeHost,
        header: PackageTestStartFrameHeader,
        payload: Vec<u8>,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let admission_permit = match self.admission_permits.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                let request = package_test_request_start_envelope(&header, payload);
                let error = self.saturation_error();
                emit_package_test_start_error(&host, &request, &error, &sender);
                return;
            }
        };
        let pending_start = self.begin_pending(header.request_id.clone());
        let start_permits = self.start_permits.clone();
        tokio::spawn(async move {
            let _admission_permit = admission_permit;
            let _start_permit = start_permits
                .acquire_owned()
                .await
                .expect("package-test start executor semaphore should remain open");
            run_package_test_start(&host, header, payload, sender, pending_start).await;
        });
    }

    pub(super) fn cancel_pending(&self, request_id: &str) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        let Some(state) = pending.get_mut(request_id) else {
            return false;
        };
        state.cancelled = true;
        true
    }

    #[cfg(test)]
    pub(crate) fn acquire_all_admission_for_test(&self) -> Vec<tokio::sync::OwnedSemaphorePermit> {
        (0..self.max_admission)
            .map(|_| {
                self.admission_permits
                    .clone()
                    .try_acquire_owned()
                    .expect("package-test start executor admission test permit should acquire")
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn acquire_all_start_for_test(&self) -> Vec<tokio::sync::OwnedSemaphorePermit> {
        (0..self.max_concurrency)
            .map(|_| {
                self.start_permits
                    .clone()
                    .try_acquire_owned()
                    .expect("package-test start executor start test permit should acquire")
            })
            .collect()
    }

    fn begin_pending(self: &Arc<Self>, request_id: String) -> PackageTestPendingStart {
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(request_id.clone(), PendingPackageTestStart::default());
        }
        PackageTestPendingStart {
            request_id,
            pending: self.pending.clone(),
            finished: false,
        }
    }

    fn saturation_error(&self) -> RuntimeError {
        RuntimeError::resource_limit_exceeded(
            "packageTestStartExecutor",
            "package-test start executor admission saturated",
            self.max_admission,
            self.max_admission
                .saturating_sub(self.admission_permits.available_permits()),
            1,
        )
    }
}

impl PackageTestPendingStart {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.pending
            .lock()
            .ok()
            .and_then(|pending| pending.get(&self.request_id).map(|state| state.cancelled))
            .unwrap_or(false)
    }

    pub(crate) fn finish(mut self) -> bool {
        self.finished = true;
        self.take_cancelled()
    }

    fn take_cancelled(&self) -> bool {
        self.pending
            .lock()
            .ok()
            .and_then(|mut pending| pending.remove(&self.request_id))
            .map(|state| state.cancelled)
            .unwrap_or(false)
    }
}

impl Drop for PackageTestPendingStart {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self
                .pending
                .lock()
                .map(|mut pending| pending.remove(&self.request_id));
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct PackageTestTemplateBuildLocks {
    locks: Arc<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    #[cfg(test)]
    template_build_count: AtomicUsize,
}

#[derive(Debug)]
pub(crate) struct PackageTestTemplateBuildGuard {
    key: String,
    locks: Arc<StdMutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    lock: Arc<tokio::sync::Mutex<()>>,
    permit: Option<OwnedMutexGuard<()>>,
}

impl PackageTestTemplateBuildLocks {
    async fn acquire(self: &Arc<Self>, key: String) -> PackageTestTemplateBuildGuard {
        let lock = self.lock_for_key(&key);
        let permit = lock.clone().lock_owned().await;
        PackageTestTemplateBuildGuard {
            key,
            locks: self.locks.clone(),
            lock,
            permit: Some(permit),
        }
    }

    fn lock_for_key(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .locks
            .lock()
            .expect("package-test template build lock map poisoned");
        locks
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    #[cfg(test)]
    fn record_template_build(&self) {
        self.template_build_count.fetch_add(1, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn template_build_count(&self) -> usize {
        self.template_build_count.load(Ordering::SeqCst)
    }
}

impl Drop for PackageTestTemplateBuildGuard {
    fn drop(&mut self) {
        let _ = self.permit.take();
        let Ok(mut locks) = self.locks.lock() else {
            return;
        };
        let should_remove = locks.get(&self.key).is_some_and(|lock| {
            Arc::ptr_eq(lock, &self.lock) && Arc::strong_count(&self.lock) == 2
        });
        if should_remove {
            locks.remove(&self.key);
        }
    }
}

fn default_package_test_start_concurrency() -> usize {
    let available_parallelism = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let fallback = resource_default_package_test_start_concurrency(available_parallelism);
    let env_value = match env::var(PACKAGE_TEST_START_CONCURRENCY_ENV) {
        Ok(value) => Some(value),
        Err(env::VarError::NotPresent) => None,
        Err(env::VarError::NotUnicode(_)) => {
            warn!(
                event = "runtime.package_test_start_concurrency_env_invalid",
                env = PACKAGE_TEST_START_CONCURRENCY_ENV,
                fallback,
                "package-test start concurrency env var must be unicode; using resource-aware default"
            );
            return fallback;
        }
    };
    let (concurrency, invalid_message) = package_test_start_concurrency_with_invalid_fallback(
        available_parallelism,
        env_value.as_deref(),
    );
    if let Some(message) = invalid_message {
        warn!(
            event = "runtime.package_test_start_concurrency_env_invalid",
            env = PACKAGE_TEST_START_CONCURRENCY_ENV,
            error = %message,
            fallback,
            "package-test start concurrency env var is invalid; using resource-aware default"
        );
    }
    concurrency
}

fn package_test_start_concurrency_with_invalid_fallback(
    available_parallelism: usize,
    env_value: Option<&str>,
) -> (usize, Option<String>) {
    let fallback = resource_default_package_test_start_concurrency(available_parallelism);
    match package_test_start_concurrency_for_values(available_parallelism, env_value) {
        Ok(value) => (value, None),
        Err(message) => (fallback, Some(message)),
    }
}

fn package_test_start_concurrency_for_values(
    available_parallelism: usize,
    env_value: Option<&str>,
) -> std::result::Result<usize, String> {
    match env_value {
        Some(value) => {
            parse_package_test_start_concurrency(value, PACKAGE_TEST_START_CONCURRENCY_ENV)
        }
        None => Ok(resource_default_package_test_start_concurrency(
            available_parallelism,
        )),
    }
}

fn resource_default_package_test_start_concurrency(available_parallelism: usize) -> usize {
    sanitize_package_test_start_concurrency(available_parallelism)
}

fn parse_package_test_start_concurrency(
    value: &str,
    source: &str,
) -> std::result::Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{source} must be a positive integer, got {value}"))?;
    if parsed == 0 {
        return Err(format!("{source} must be a positive integer, got {value}"));
    }
    if parsed > PACKAGE_TEST_START_MAX_SAFE_CONCURRENCY {
        return Err(format!(
            "{source} must be at most {PACKAGE_TEST_START_MAX_SAFE_CONCURRENCY}, got {value}"
        ));
    }
    Ok(parsed)
}

fn sanitize_package_test_start_concurrency(value: usize) -> usize {
    value.max(1).min(PACKAGE_TEST_START_MAX_SAFE_CONCURRENCY)
}

fn default_package_test_start_admission(max_concurrency: usize) -> usize {
    max_concurrency
        .saturating_mul(PACKAGE_TEST_START_ADMISSION_MULTIPLIER)
        .max(PACKAGE_TEST_START_MIN_ADMISSION)
}

#[cfg(test)]
impl RuntimeHost {
    pub(crate) fn acquire_all_package_test_start_admission_permits_for_test(
        &self,
    ) -> Vec<tokio::sync::OwnedSemaphorePermit> {
        self.package_test_start_executor
            .acquire_all_admission_for_test()
    }

    pub(crate) fn acquire_all_package_test_start_execution_permits_for_test(
        &self,
    ) -> Vec<tokio::sync::OwnedSemaphorePermit> {
        self.package_test_start_executor
            .acquire_all_start_for_test()
    }

    pub(crate) async fn acquire_package_test_template_build_lock_for_test(
        &self,
        cache_key: String,
    ) -> PackageTestTemplateBuildGuard {
        self.package_test_template_builds.acquire(cache_key).await
    }

    pub(crate) fn package_test_template_build_count_for_test(&self) -> usize {
        self.package_test_template_builds.template_build_count()
    }
}

pub(super) fn spawn_package_test_start(
    host: &RuntimeHost,
    header: PackageTestStartFrameHeader,
    payload: Vec<u8>,
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
) {
    host.package_test_start_executor
        .submit(host.clone(), header, payload, sender);
}

async fn run_package_test_start(
    host: &RuntimeHost,
    header: PackageTestStartFrameHeader,
    payload: Vec<u8>,
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
    pending_start: PackageTestPendingStart,
) {
    let request = package_test_request_start_envelope(&header, payload);
    if pending_start.is_cancelled() {
        let cancelled = pending_start.finish();
        debug_assert!(cancelled);
        emit_package_test_start_error(host, &request, &RuntimeError::cancelled(), &sender);
        return;
    }

    let loaded = match load_package_test_runtime_program(host, &header).await {
        Ok(loaded) => loaded,
        Err(error) => {
            let _ = pending_start.finish();
            emit_package_test_start_error(host, &request, &error, &sender);
            return;
        }
    };
    if pending_start.is_cancelled() {
        let cancelled = pending_start.finish();
        debug_assert!(cancelled);
        emit_package_test_start_error(host, &request, &RuntimeError::cancelled(), &sender);
        return;
    }

    let service = match package_test_service_context(host, &loaded, &header) {
        Ok(service) => service,
        Err(error) => {
            let _ = pending_start.finish();
            emit_package_test_start_error(host, &request, &error, &sender);
            return;
        }
    };
    if pending_start.is_cancelled() {
        let cancelled = pending_start.finish();
        debug_assert!(cancelled);
        emit_package_test_start_error(host, &request, &RuntimeError::cancelled(), &sender);
        return;
    }

    let operation = match package_test_operation(&loaded) {
        Ok(operation) => operation,
        Err(error) => {
            let _ = pending_start.finish();
            emit_package_test_start_error(host, &request, &error, &sender);
            return;
        }
    };
    let addr = loaded.executable_addr.clone();
    start_package_test_spawn_workers(host, service.clone(), sender.clone());

    host.spawn_resolved_package_test_request(
        ServiceOperationContext::new(service, operation, addr),
        request,
        sender,
        "runtime.package_test_error",
        pending_start,
    )
    .await;
}

pub(super) async fn load_package_test_runtime_program(
    host: &RuntimeHost,
    header: &PackageTestStartFrameHeader,
) -> Result<LoadedPackageTestRuntimeProgram> {
    validate_package_test_activation_id(&header.activation_id)?;
    let selection = PackageTestDispatchSelection {
        package_id: header.package_id.clone(),
        package_version: header.package_version.clone(),
        test_build_identity: header.test_build_identity.clone(),
        entrypoint_id: header.entrypoint_id.clone(),
        activation_id: header.activation_id.clone(),
    };
    let build_selection = selection.build_selection();
    let (artifact_roots, artifact_epoch) = {
        let load_state = host.artifact_load_state.lock().await;
        (load_state.artifact_roots.clone(), load_state.epoch)
    };
    let cache_key = PackageTestRuntimeTemplateCache::cache_key(&artifact_roots, &build_selection);
    if let Some(template) = host.artifact_caches.package_test_templates.get(&cache_key) {
        return template
            .load(&selection)
            .map_err(|error| RuntimeError::invalid_artifact(error.to_string()));
    }

    let _build_guard = host
        .package_test_template_builds
        .acquire(cache_key.clone())
        .await;
    if let Some(template) = host.artifact_caches.package_test_templates.get(&cache_key) {
        return template
            .load(&selection)
            .map_err(|error| RuntimeError::invalid_artifact(error.to_string()));
    }

    #[cfg(test)]
    host.package_test_template_builds.record_template_build();
    let template = Arc::new(
        PackageTestRuntimeBuilder::new(
            &artifact_roots,
            ArtifactGraphCache::new(&host.artifact_caches.files, &host.artifact_caches.packages),
        )
        .load_template(&build_selection)
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?,
    );
    template
        .load(&selection)
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    let template = if package_test_artifact_epoch_matches(host, artifact_epoch).await {
        host.artifact_caches
            .package_test_templates
            .insert_arc(cache_key, template)
    } else {
        template
    };
    let loaded = template
        .load(&selection)
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    host.artifact_caches.evict_lru_to_budget();
    Ok(loaded)
}

pub(super) fn package_test_service_context(
    host: &RuntimeHost,
    loaded: &LoadedPackageTestRuntimeProgram,
    header: &PackageTestStartFrameHeader,
) -> Result<Arc<ServiceRuntimeContext>> {
    let service_id = header.package_id.clone();
    let service_config_shape =
        config_and_effect_metadata_shape(&loaded.dispatch.entrypoint.config_and_effect_metadata)
            .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    let local_config = load_package_test_local_config(
        &loaded.dispatch.validated.artifact_root,
        &header.activation_id,
        loaded.production_unit.as_ref(),
        loaded.synthetic_service_unit.as_ref(),
        loaded.image.as_ref(),
        loaded.activation.as_ref(),
        service_config_shape,
    )
    .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    let service_db_id = package_test_service_db_id(&header.activation_id)?;
    let service_db = build_service_db_source(
        service_db_id,
        local_config.service_db.clone(),
        loaded.activation.as_ref(),
        &host.db_provider,
    )
    .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    Ok(Arc::new(ServiceRuntimeContext::new(
        service_id,
        host.default_http_response_max_bytes,
        Some(header.activation_id.clone()),
        None,
        loaded.image.clone(),
        RuntimeProgramIdentity::new(
            header.test_build_identity.clone(),
            loaded.identity.linked_image_identity.clone(),
        ),
        loaded.activation.clone(),
        package_test_revision_id(&header.test_build_identity),
        host.base_runtime_id.clone(),
        loaded.synthetic_service_unit.protocol_identity.clone(),
        loaded
            .dispatch
            .assembly
            .production_package_unit
            .build_identity
            .clone(),
        header.test_build_identity.clone(),
        header.test_build_identity.clone(),
        local_config.service_config,
        local_config.package_configs,
        service_db,
    )))
}

async fn package_test_artifact_epoch_matches(host: &RuntimeHost, artifact_epoch: u64) -> bool {
    host.artifact_load_state.lock().await.epoch == artifact_epoch
}

fn package_test_service_db_id(activation_id: &str) -> Result<String> {
    validate_package_test_activation_id(activation_id)?;
    let hash = Sha256::digest(activation_id.as_bytes());
    Ok(format!(
        "{PACKAGE_TEST_SERVICE_DB_PREFIX}{}",
        hex_prefix(&hash, 24)
    ))
}

fn validate_package_test_activation_id(value: &str) -> Result<()> {
    let Some(suffix) = value.strip_prefix(PACKAGE_TEST_ACTIVATION_ID_PREFIX) else {
        return Err(RuntimeError::invalid_artifact(format!(
            "package-test activationId must start with {PACKAGE_TEST_ACTIVATION_ID_PREFIX}, got {value}"
        )));
    };
    if suffix.is_empty() {
        return Err(RuntimeError::invalid_artifact(
            "package-test activationId must not have an empty run suffix".to_string(),
        ));
    }
    if suffix.contains("..") {
        return Err(RuntimeError::invalid_artifact(format!(
            "package-test activationId must not contain .., got {value}"
        )));
    }
    if suffix
        .bytes()
        .any(|byte| matches!(byte, b'/' | b'\\') || byte.is_ascii_control())
    {
        return Err(RuntimeError::invalid_artifact(format!(
            "package-test activationId suffix must be a single URL/path safe segment, got {value}"
        )));
    }
    if !suffix
        .bytes()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b':' | b'~' | b'-'))
    {
        return Err(RuntimeError::invalid_artifact(format!(
            "package-test activationId suffix must match [A-Za-z0-9._:~-]+, got {value}"
        )));
    }
    Ok(())
}

fn hex_prefix(bytes: &[u8], hex_chars: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(hex_chars);
    for byte in bytes {
        if output.len() >= hex_chars {
            break;
        }
        output.push(HEX[(byte >> 4) as usize] as char);
        if output.len() >= hex_chars {
            break;
        }
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_test_start_concurrency_default_uses_available_parallelism_without_8_cap() {
        assert_eq!(
            package_test_start_concurrency_for_values(0, None).unwrap(),
            1
        );
        assert_eq!(
            package_test_start_concurrency_for_values(1, None).unwrap(),
            1
        );
        assert_eq!(
            package_test_start_concurrency_for_values(8, None).unwrap(),
            8
        );
        assert_eq!(
            package_test_start_concurrency_for_values(14, None).unwrap(),
            14
        );
        assert_eq!(
            package_test_start_concurrency_for_values(100, None).unwrap(),
            100
        );
    }

    #[test]
    fn package_test_start_concurrency_env_override_accepts_high_values() {
        assert_eq!(
            package_test_start_concurrency_for_values(14, Some("64")).unwrap(),
            64
        );
        assert_eq!(
            package_test_start_concurrency_for_values(14, Some("100")).unwrap(),
            100
        );
        assert_eq!(
            package_test_start_concurrency_for_values(100, Some("1")).unwrap(),
            1
        );
    }

    #[test]
    fn package_test_start_concurrency_env_override_rejects_invalid_values() {
        assert!(package_test_start_concurrency_for_values(14, Some("0"))
            .expect_err("zero should be rejected")
            .contains("positive integer"));
        assert!(package_test_start_concurrency_for_values(14, Some("many"))
            .expect_err("non-integer should be rejected")
            .contains("positive integer"));
        assert!({
            let too_high = (PACKAGE_TEST_START_MAX_SAFE_CONCURRENCY + 1).to_string();
            package_test_start_concurrency_for_values(14, Some(&too_high))
        }
        .expect_err("unsafe high concurrency should be rejected")
        .contains("at most"));
    }

    #[test]
    fn package_test_start_concurrency_invalid_env_falls_back_to_resource_default() {
        let (concurrency, invalid_message) =
            package_test_start_concurrency_with_invalid_fallback(14, Some("0"));

        assert_eq!(concurrency, 14);
        assert!(invalid_message
            .expect("invalid env value should return diagnostic")
            .contains(PACKAGE_TEST_START_CONCURRENCY_ENV));
    }

    #[test]
    fn package_test_start_executor_admission_remains_bounded_and_scales_with_concurrency() {
        let minimum = PackageTestStartExecutor::new(1);
        assert_eq!(minimum.max_concurrency, 1);
        assert_eq!(minimum.max_admission, PACKAGE_TEST_START_MIN_ADMISSION);

        let high = PackageTestStartExecutor::new(100);
        assert_eq!(high.max_concurrency, 100);
        assert_eq!(
            high.max_admission,
            100 * PACKAGE_TEST_START_ADMISSION_MULTIPLIER
        );

        let sanitized = PackageTestStartExecutor::new(PACKAGE_TEST_START_MAX_SAFE_CONCURRENCY + 1);
        assert_eq!(
            sanitized.max_concurrency,
            PACKAGE_TEST_START_MAX_SAFE_CONCURRENCY
        );
        assert_eq!(
            sanitized.max_admission,
            PACKAGE_TEST_START_MAX_SAFE_CONCURRENCY * PACKAGE_TEST_START_ADMISSION_MULTIPLIER
        );
    }

    #[test]
    fn package_test_service_db_id_is_derived_from_activation_id() {
        let first = package_test_service_db_id("skiff-package-test-run-v1:example~com~~pkg:run:1")
            .expect("first activation id should project");
        let second = package_test_service_db_id("skiff-package-test-run-v1:example~com~~pkg:run:2")
            .expect("second activation id should project");

        assert_ne!(first, second);
        assert!(first.starts_with(PACKAGE_TEST_SERVICE_DB_PREFIX));
        assert_eq!(
            first.len(),
            PACKAGE_TEST_SERVICE_DB_PREFIX.len() + 24,
            "service id should stay short enough for Mongo database projection"
        );
    }

    #[test]
    fn package_test_service_db_id_rejects_invalid_activation_id() {
        let error = package_test_service_db_id(
            "skiff-package-test-build-v1:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        )
        .expect_err("testBuildIdentity is not a package-test activation id");

        assert!(error.to_string().contains("activationId"));
    }
}

fn package_test_operation(loaded: &LoadedPackageTestRuntimeProgram) -> Result<RuntimeOperation> {
    let executable = loaded
        .image
        .resolve_executable(&loaded.executable_addr)
        .map_err(|error| RuntimeError::invalid_artifact(error.to_string()))?;
    Ok(RuntimeOperation {
        operation_abi_id: None,
        operation: executable.executable.symbol.clone(),
        target: loaded.dispatch.entrypoint.entrypoint_id.clone(),
        mode: "unary".to_string(),
        parameters: executable
            .executable
            .params
            .iter()
            .map(|parameter| RuntimeOperationParameter {
                name: parameter.name.clone(),
                extra: serde_json::Map::new(),
            })
            .collect(),
        service_protocol_identity: Some(loaded.synthetic_service_unit.protocol_identity.clone()),
        extra: serde_json::Map::new(),
    })
}

fn start_package_test_spawn_workers(
    host: &RuntimeHost,
    service: Arc<ServiceRuntimeContext>,
    sender: mpsc::UnboundedSender<RouterWriterMessage>,
) -> usize {
    spawn_worker::start_spawn_workers_for_services(host.clone(), sender, vec![service])
}

fn emit_package_test_start_error(
    host: &RuntimeHost,
    request: &RequestEnvelope,
    error: &RuntimeError,
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
) {
    host.emit_request_route_error(request, error);
    host.send_request_error_response(request, error, sender);
}

fn package_test_request_start_envelope(
    header: &PackageTestStartFrameHeader,
    payload_bytes: Vec<u8>,
) -> RequestEnvelope {
    let mut extra = serde_json::Map::new();
    extra.insert(
        "caller".to_string(),
        serde_json::to_value(&header.caller).unwrap_or(Value::Null),
    );
    extra.insert(
        "packageId".to_string(),
        Value::String(header.package_id.clone()),
    );
    extra.insert(
        "packageVersion".to_string(),
        Value::String(header.package_version.clone()),
    );
    extra.insert(
        "testBuildIdentity".to_string(),
        Value::String(header.test_build_identity.clone()),
    );
    extra.insert(
        "entrypointId".to_string(),
        Value::String(header.entrypoint_id.clone()),
    );
    extra.insert(
        "activationId".to_string(),
        Value::String(header.activation_id.clone()),
    );
    if let Some(deadline) = &header.deadline {
        extra.insert(
            "deadline".to_string(),
            serde_json::to_value(deadline).unwrap_or(Value::Null),
        );
    }
    extra.insert(
        "trace".to_string(),
        serde_json::to_value(&header.trace).unwrap_or(Value::Null),
    );

    RequestEnvelope {
        request_id: header.request_id.clone(),
        mode: "unary".to_string(),
        target: header.entrypoint_id.clone(),
        operation_abi_id: None,
        selector: None,
        service_id: None,
        build_id: header.test_build_identity.clone(),
        service_protocol_identity: header.test_build_identity.clone(),
        contract_identity: None,
        activation_identity: Some(header.activation_id.clone()),
        binary_http: None,
        http_adapter: None,
        websocket_adapter: None,
        test_effects_enabled: header.test_effects_enabled,
        test_effect_doubles: header
            .test_effect_doubles
            .iter()
            .map(|(target, sequence)| {
                (
                    target.clone(),
                    sequence
                        .iter()
                        .map(|double| RequestEffectDouble {
                            expect_request: double.expect_request.clone(),
                            response: double.response.clone(),
                        })
                        .collect(),
                )
            })
            .collect(),
        payload_bytes,
        extra,
    }
}
