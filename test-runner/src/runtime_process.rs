use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env, fs,
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::TcpStream,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::fd::AsRawFd;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use sha2::{Digest, Sha256};
use skiff_artifact_model::PackageDependencyConstraint;
use skiff_compiler::test_support::{
    default_std_dir, package_test_artifacts::TestPackageTestEntrypointSummary,
    write_package_test_artifact_root_with_runtime_path_registration,
    write_test_service_artifact_root_with_runtime_path_registration, TestPackageTestArtifactInput,
    TestServiceArtifactInput, TestServiceFileIrArtifact,
};

use super::{
    service_publish::{
        ServiceRuntimePublication, ServiceRuntimeSuiteCase, ServiceRuntimeSuitePublication,
    },
    types::TestEffectDouble,
    RuntimeTestArtifact, SkiffTestOptions,
};

const DEFAULT_CONTROL_BASE_URL: &str = "http://127.0.0.1:4001";
const TEST_ARTIFACT_ROOT_ENV: &str = "SKIFF_TEST_ARTIFACT_ROOT";
const TEST_SERVICE_VERSION: &str = "test";
const PAYLOAD_MAGIC: &[u8; 4] = b"SKPV";
const PAYLOAD_VERSION: u8 = 2;
const PAYLOAD_TAG_BOOL_TRUE: u8 = 2;
const PAYLOAD_TAG_STRING: u8 = 4;
const PAYLOAD_TAG_OBJECT: u8 = 7;
const TEST_REQUEST_PAYLOAD_PARAM: &str = "__skiffPayload";
const SERVICE_DEPENDENCY_SHARED_ARTIFACT_DIRS: &[&str] = &["bundles", "contracts", "resources"];
const SERVICE_DEPENDENCY_PACKAGE_ARTIFACT_DIRS: &[(&str, &str)] = &[
    ("assemblies", "packages"),
    ("files", "packages"),
    ("indexes", "packages"),
    ("units", "files"),
    ("units", "packages"),
];
const SERVICE_DEPENDENCY_SERVICE_ARTIFACT_DIRS: &[(&str, &str, bool)] = &[
    ("assemblies", "services", false),
    ("configs", "services", true),
    ("files", "services", false),
    ("indexes", "services", true),
    ("units", "services", false),
];
const PACKAGE_TEST_BUILD_IDENTITY_PREFIX: &str = "skiff-package-test-build-v1:sha256:";
const PACKAGE_TEST_ACTIVATION_ID_PREFIX: &str = "skiff-package-test-run-v1:";
const PACKAGE_TEST_SERVICE_DB_PREFIX: &str = "skiff.run/package-test-db-";
const SYNC_TEST_DB_CLEANUP_ENV: &str = "SKIFF_TEST_SYNC_CLEANUP";
const TEST_RUNNER_STATE_DIR: &str = ".skiff-test-runner";
const TEST_RUNNER_LOCK_FILE: &str = "artifact.lock";
const TEST_RUNNER_RUNS_DIR: &str = "runs";
const TEST_RUNNER_RELOAD_MARKER: &str = "reload-required";
const TEST_RUNNER_MANIFEST_SCHEMA_VERSION: &str = "skiff-test-runner-runtime-artifacts-v1";
const SYNTHETIC_SERVICE_ARTIFACT_PATH_PREFIX: &str = "example~com~~skiff-";

static SERVICE_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
static RUNTIME_ARTIFACT_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RuntimeExpectedError {
    code: String,
    message_contains: Option<String>,
}

impl RuntimeExpectedError {
    pub(crate) fn new(
        code: impl Into<String>,
        message_contains: Option<String>,
    ) -> Result<Self, String> {
        let code = code.into();
        if code.trim().is_empty() {
            return Err("runtimeLive.expectedError.code must be a non-empty string".to_string());
        }
        if message_contains
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(
                "runtimeLive.expectedError.messageContains must be a non-empty string".to_string(),
            );
        }
        Ok(Self {
            code,
            message_contains,
        })
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }
}

pub(super) fn synthetic_test_service_id(scope: &str) -> String {
    format!(
        "example.com/skiff-{scope}-{}-{}-{}",
        std::process::id(),
        current_nanos(),
        SERVICE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

pub(super) fn synthetic_test_target(
    service_id: &str,
    module_path: &str,
    function_name: &str,
) -> String {
    format!(
        "skiff.test.{}.{}.{}",
        test_target_component(service_id),
        test_target_component(module_path),
        test_target_component(function_name)
    )
}

fn synthetic_websocket_entry_id(service_id: &str) -> String {
    format!("test-dispatch.{}", service_id_artifact_path(service_id))
}

fn test_target_component(value: &str) -> String {
    value
        .replace('~', "~7e")
        .replace('.', "~")
        .replace('/', "~~")
}

struct RuntimeArtifactRun {
    _process_guard: MutexGuard<'static, ()>,
    _root_lock: RuntimeArtifactRootLock,
    registrar: Arc<RuntimeArtifactRunRegistrar>,
    control_base_url: String,
    live: bool,
    completed: AtomicBool,
}

impl RuntimeArtifactRun {
    fn start(
        process_guard: MutexGuard<'static, ()>,
        artifact_root: &Path,
        control_base_url: &str,
        options: &SkiffTestOptions,
    ) -> Result<Self, String> {
        let root_lock = RuntimeArtifactRootLock::acquire(artifact_root)?;
        preflight_runtime_artifact_cleanup(artifact_root, control_base_url, options.live)?;
        let run_id = runtime_artifact_run_id();
        let registrar = RuntimeArtifactRunRegistrar::create(artifact_root, run_id)?;
        Ok(Self {
            _process_guard: process_guard,
            _root_lock: root_lock,
            registrar,
            control_base_url: control_base_url.to_string(),
            live: options.live,
            completed: AtomicBool::new(false),
        })
    }

    fn registrar(&self) -> &RuntimeArtifactRunRegistrar {
        &self.registrar
    }

    fn register_path(&self, relative_path: impl AsRef<Path>) -> Result<(), String> {
        self.registrar.register_path(relative_path.as_ref())
    }

    fn mark_written(&self, relative_path: impl AsRef<Path>) -> Result<(), String> {
        self.registrar.mark_written(relative_path.as_ref())
    }

    fn finish(self) -> Result<(), String> {
        let mut reload =
            || reload_runtime_artifacts_for_live(&self.control_base_url, self.live).map(|_| ());
        let result = self.finish_with_reloader(&mut reload);
        if result.is_ok() {
            self.completed.store(true, Ordering::SeqCst);
        }
        result
    }

    fn finish_with_reloader(
        &self,
        reload: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        let manifest = self.registrar.snapshot()?;
        let cleanup = cleanup_manifest_paths(&self.registrar.artifact_root, &manifest)?;
        let reload_required = runtime_cleanup_needs_reload(
            self.registrar.reload_marker_path.exists(),
            std::slice::from_ref(&manifest),
            cleanup.deleted_any,
        );
        self.registrar
            .mark_finished_with_reload_required(reload_required)?;
        if reload_required {
            self.registrar.write_reload_marker()?;
            reload()?;
            self.registrar.remove_reload_marker()?;
        }
        self.registrar.remove_manifest()?;
        Ok(())
    }
}

impl Drop for RuntimeArtifactRun {
    fn drop(&mut self) {
        if self.completed.load(Ordering::SeqCst) {
            return;
        }
        self.registrar.mark_cleanup_required_best_effort();
    }
}

fn finish_runtime_artifacts<T>(
    runtime_artifacts: RuntimeArtifactRun,
    result: Result<T, String>,
) -> Result<T, String> {
    let cleanup = runtime_artifacts.finish();
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(message), Ok(())) => Err(message),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
        (Err(message), Err(cleanup_error)) => {
            Err(format!("{message}; cleanup failed: {cleanup_error}"))
        }
    }
}

fn run_with_runtime_artifacts<T>(
    runtime_artifacts: RuntimeArtifactRun,
    operation: impl FnOnce(&RuntimeArtifactRun) -> Result<T, String>,
) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        operation(&runtime_artifacts)
    })) {
        Ok(result) => finish_runtime_artifacts(runtime_artifacts, result),
        Err(payload) => {
            let _ = runtime_artifacts.finish();
            std::panic::resume_unwind(payload);
        }
    }
}

fn run_prepared_runtime_artifacts<T>(
    runtime_artifacts: RuntimeArtifactRun,
    operation: impl FnOnce(&RuntimeArtifactRun) -> Result<T, String>,
) -> Result<(RuntimeArtifactRun, T), String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        operation(&runtime_artifacts)
    })) {
        Ok(Ok(value)) => Ok((runtime_artifacts, value)),
        Ok(Err(message)) => Err(
            finish_runtime_artifacts(runtime_artifacts, Err::<(), _>(message))
                .expect_err("error result cannot produce a value"),
        ),
        Err(payload) => {
            let _ = runtime_artifacts.finish();
            std::panic::resume_unwind(payload);
        }
    }
}

struct RuntimeArtifactRootLock {
    file: File,
}

impl RuntimeArtifactRootLock {
    fn acquire(artifact_root: &Path) -> Result<Self, String> {
        let state_dir = test_runner_state_dir(artifact_root);
        fs::create_dir_all(&state_dir).map_err(|error| {
            format!(
                "failed to create test-runner artifact state directory {}: {error}",
                state_dir.display()
            )
        })?;
        let lock_path = state_dir.join(TEST_RUNNER_LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|error| {
                format!(
                    "failed to open test-runner artifact lock {}: {error}",
                    lock_path.display()
                )
            })?;
        lock_runtime_artifact_file(&file, &lock_path)?;
        Ok(Self { file })
    }
}

impl Drop for RuntimeArtifactRootLock {
    fn drop(&mut self) {
        unlock_runtime_artifact_file(&self.file);
    }
}

#[cfg(unix)]
fn lock_runtime_artifact_file(file: &File, lock_path: &Path) -> Result<(), String> {
    let status = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if status == 0 {
        return Ok(());
    }
    Err(format!(
        "failed to lock test-runner artifact lock {}: {}",
        lock_path.display(),
        std::io::Error::last_os_error()
    ))
}

#[cfg(not(unix))]
fn lock_runtime_artifact_file(_file: &File, _lock_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn unlock_runtime_artifact_file(file: &File) {
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(not(unix))]
fn unlock_runtime_artifact_file(_file: &File) {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeArtifactRunManifest {
    schema_version: String,
    run_id: String,
    status: String,
    reload_required: bool,
    paths: Vec<RuntimeArtifactManifestPath>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeArtifactManifestPath {
    path: String,
    written: bool,
}

struct RuntimeArtifactRunRegistrar {
    artifact_root: PathBuf,
    manifest_path: PathBuf,
    reload_marker_path: PathBuf,
    state: Mutex<RuntimeArtifactRunManifest>,
}

impl RuntimeArtifactRunRegistrar {
    fn create(artifact_root: &Path, run_id: String) -> Result<Arc<Self>, String> {
        let runs_dir = test_runner_runs_dir(artifact_root);
        fs::create_dir_all(&runs_dir).map_err(|error| {
            format!(
                "failed to create test-runner run manifest directory {}: {error}",
                runs_dir.display()
            )
        })?;
        let manifest_path = runs_dir.join(format!("{run_id}.json"));
        let manifest = RuntimeArtifactRunManifest {
            schema_version: TEST_RUNNER_MANIFEST_SCHEMA_VERSION.to_string(),
            run_id,
            status: "active".to_string(),
            reload_required: false,
            paths: Vec::new(),
        };
        write_runtime_artifact_manifest(&manifest_path, &manifest)?;
        Ok(Arc::new(Self {
            artifact_root: artifact_root.to_path_buf(),
            manifest_path,
            reload_marker_path: test_runner_reload_marker_path(artifact_root),
            state: Mutex::new(manifest),
        }))
    }

    fn register_path(&self, relative_path: &Path) -> Result<(), String> {
        let relative_path = artifact_manifest_relative_path(relative_path)?;
        validate_runtime_cleanup_path(&relative_path)?;
        let mut manifest = self
            .state
            .lock()
            .map_err(|_| "runtime artifact manifest lock was poisoned".to_string())?;
        if !manifest
            .paths
            .iter()
            .any(|entry| entry.path == relative_path)
        {
            manifest.paths.push(RuntimeArtifactManifestPath {
                path: relative_path,
                written: false,
            });
        }
        manifest.reload_required = true;
        self.write_reload_marker()?;
        write_runtime_artifact_manifest(&self.manifest_path, &manifest)
    }

    fn mark_written(&self, relative_path: &Path) -> Result<(), String> {
        let relative_path = artifact_manifest_relative_path(relative_path)?;
        let mut manifest = self
            .state
            .lock()
            .map_err(|_| "runtime artifact manifest lock was poisoned".to_string())?;
        if let Some(entry) = manifest
            .paths
            .iter_mut()
            .find(|entry| entry.path == relative_path)
        {
            entry.written = true;
        }
        manifest.reload_required = true;
        self.write_reload_marker()?;
        write_runtime_artifact_manifest(&self.manifest_path, &manifest)
    }

    fn snapshot(&self) -> Result<RuntimeArtifactRunManifest, String> {
        self.state
            .lock()
            .map_err(|_| "runtime artifact manifest lock was poisoned".to_string())
            .map(|manifest| manifest.clone())
    }

    fn mark_finished_with_reload_required(&self, reload_required: bool) -> Result<(), String> {
        let mut manifest = self
            .state
            .lock()
            .map_err(|_| "runtime artifact manifest lock was poisoned".to_string())?;
        manifest.status = "finished".to_string();
        manifest.reload_required = reload_required;
        write_runtime_artifact_manifest(&self.manifest_path, &manifest)
    }

    fn write_reload_marker(&self) -> Result<(), String> {
        write_reload_marker(&self.reload_marker_path)
    }

    fn remove_reload_marker(&self) -> Result<(), String> {
        remove_file_if_exists(&self.reload_marker_path).map_err(|error| {
            format!(
                "failed to remove test-runner reload marker {}: {error}",
                self.reload_marker_path.display()
            )
        })
    }

    fn remove_manifest(&self) -> Result<(), String> {
        remove_file_if_exists(&self.manifest_path).map_err(|error| {
            format!(
                "failed to remove test-runner run manifest {}: {error}",
                self.manifest_path.display()
            )
        })
    }

    fn mark_cleanup_required_best_effort(&self) {
        if let Ok(mut manifest) = self.state.lock() {
            manifest.status = "cleanup-required".to_string();
            manifest.reload_required = true;
            let _ = write_runtime_artifact_manifest(&self.manifest_path, &manifest);
        }
        let _ = self.write_reload_marker();
    }
}

fn preflight_runtime_artifact_cleanup(
    artifact_root: &Path,
    control_base_url: &str,
    live: bool,
) -> Result<(), String> {
    let manifests = read_runtime_artifact_manifests(artifact_root)?;
    let reload_marker = test_runner_reload_marker_path(artifact_root);
    let mut deleted_any = false;
    let manifests_to_cleanup = manifests
        .iter()
        .filter(|manifest| manifest.manifest.status != "finished")
        .collect::<Vec<_>>();
    if !manifests_to_cleanup.is_empty() {
        write_reload_marker(&reload_marker)?;
    }
    for manifest in manifests_to_cleanup {
        let cleanup = cleanup_manifest_paths(artifact_root, &manifest.manifest)?;
        deleted_any |= cleanup.deleted_any;
    }
    let legacy_cleanup = cleanup_legacy_test_runner_artifacts(artifact_root, &reload_marker)?;
    deleted_any |= legacy_cleanup.deleted_any;
    if manifests.is_empty() && !reload_marker.exists() && !deleted_any {
        return Ok(());
    }
    if runtime_cleanup_needs_reload(
        reload_marker.exists(),
        &manifests
            .iter()
            .map(|manifest| manifest.manifest.clone())
            .collect::<Vec<_>>(),
        deleted_any,
    ) {
        reload_runtime_artifacts_for_live(control_base_url, live)?;
        remove_file_if_exists(&reload_marker).map_err(|error| {
            format!(
                "failed to remove test-runner reload marker {}: {error}",
                reload_marker.display()
            )
        })?;
    }
    for manifest in manifests {
        remove_file_if_exists(&manifest.path).map_err(|error| {
            format!(
                "failed to remove test-runner run manifest {}: {error}",
                manifest.path.display()
            )
        })?;
    }
    Ok(())
}

fn cleanup_legacy_test_runner_artifacts(
    artifact_root: &Path,
    reload_marker: &Path,
) -> Result<RuntimeArtifactCleanupResult, String> {
    let mut result = RuntimeArtifactCleanupResult::default();
    let mut service_paths = legacy_synthetic_service_paths(artifact_root)?;
    service_paths.sort();
    service_paths.dedup();
    let mut pending_deletes = Vec::new();
    for service_path in &service_paths {
        pending_deletes.push(
            artifact_root
                .join("dev")
                .join("services")
                .join(format!("{service_path}.json")),
        );
        for (directory, child) in [
            ("configs", "services"),
            ("dev", "service-test-activations"),
            ("configs", "service-test-activations"),
            ("assemblies", "services"),
            ("units", "services"),
            ("indexes", "services"),
            ("files", "services"),
            ("versions", "services"),
            ("builds", "services"),
        ] {
            pending_deletes.push(artifact_root.join(directory).join(child).join(service_path));
        }
    }
    for path in [
        artifact_root.join("dev").join("package-tests"),
        artifact_root.join("assemblies").join("package-tests"),
        artifact_root.join("configs").join("package-tests"),
    ] {
        if path.exists() {
            pending_deletes.push(path);
        }
    }
    if pending_deletes.is_empty() {
        return Ok(result);
    }
    write_reload_marker(reload_marker)?;
    for path in pending_deletes {
        if remove_runtime_artifact_path_if_exists(&path)? {
            result.deleted_any = true;
        }
        prune_empty_artifact_dirs(artifact_root, path.parent());
    }
    Ok(result)
}

fn legacy_synthetic_service_paths(artifact_root: &Path) -> Result<Vec<String>, String> {
    let mut service_paths = Vec::new();
    let dev_services = artifact_root.join("dev").join("services");
    if dev_services.is_dir() {
        for entry in fs::read_dir(&dev_services).map_err(|error| {
            format!(
                "failed to read dev service pointers {}: {error}",
                dev_services.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to read dev service pointer in {}: {error}",
                    dev_services.display()
                )
            })?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if is_synthetic_service_artifact_path(stem) {
                service_paths.push(stem.to_string());
            }
        }
    }
    for (directory, child) in [
        ("configs", "services"),
        ("dev", "service-test-activations"),
        ("configs", "service-test-activations"),
        ("assemblies", "services"),
        ("units", "services"),
        ("indexes", "services"),
        ("files", "services"),
        ("versions", "services"),
        ("builds", "services"),
    ] {
        let root = artifact_root.join(directory).join(child);
        if !root.is_dir() {
            continue;
        }
        for entry in fs::read_dir(&root).map_err(|error| {
            format!(
                "failed to read legacy synthetic service directory {}: {error}",
                root.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "failed to read legacy synthetic service entry in {}: {error}",
                    root.display()
                )
            })?;
            let Some(name) = entry.file_name().to_str().map(ToString::to_string) else {
                continue;
            };
            if is_synthetic_service_artifact_path(&name) {
                service_paths.push(name);
            }
        }
    }
    Ok(service_paths)
}

#[derive(Clone)]
struct RuntimeArtifactManifestFile {
    path: PathBuf,
    manifest: RuntimeArtifactRunManifest,
}

fn read_runtime_artifact_manifests(
    artifact_root: &Path,
) -> Result<Vec<RuntimeArtifactManifestFile>, String> {
    let runs_dir = test_runner_runs_dir(artifact_root);
    if !runs_dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests = Vec::new();
    for entry in fs::read_dir(&runs_dir).map_err(|error| {
        format!(
            "failed to read test-runner run manifest directory {}: {error}",
            runs_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read test-runner run manifest entry in {}: {error}",
                runs_dir.display()
            )
        })?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let text = fs::read_to_string(entry.path()).map_err(|error| {
            format!(
                "failed to read test-runner run manifest {}: {error}",
                entry.path().display()
            )
        })?;
        let manifest =
            serde_json::from_str::<RuntimeArtifactRunManifest>(&text).map_err(|error| {
                format!(
                    "failed to parse test-runner run manifest {}: {error}",
                    entry.path().display()
                )
            })?;
        if manifest.schema_version == TEST_RUNNER_MANIFEST_SCHEMA_VERSION {
            manifests.push(RuntimeArtifactManifestFile {
                path: entry.path(),
                manifest,
            });
        }
    }
    manifests.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(manifests)
}

#[derive(Default)]
struct RuntimeArtifactCleanupResult {
    deleted_any: bool,
}

fn cleanup_manifest_paths(
    artifact_root: &Path,
    manifest: &RuntimeArtifactRunManifest,
) -> Result<RuntimeArtifactCleanupResult, String> {
    let mut result = RuntimeArtifactCleanupResult::default();
    for entry in &manifest.paths {
        if cleanup_registered_artifact_path(artifact_root, &entry.path)? {
            result.deleted_any = true;
        }
    }
    Ok(result)
}

fn cleanup_registered_artifact_path(
    artifact_root: &Path,
    relative_path: &str,
) -> Result<bool, String> {
    if validate_runtime_cleanup_path(relative_path).is_err() {
        return Ok(false);
    }
    let path = artifact_root.join(Path::new(relative_path));
    if !path.exists() {
        return Ok(false);
    };
    if path.is_dir() {
        return remove_runtime_artifact_path_if_exists(&path);
    }
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return Ok(false);
    };
    if metadata.is_dir() {
        return remove_runtime_artifact_path_if_exists(&path);
    }
    remove_file_if_exists(&path).map_err(|error| {
        format!(
            "failed to remove test-owned runtime artifact {}: {error}",
            path.display()
        )
    })?;
    prune_empty_artifact_dirs(artifact_root, path.parent());
    Ok(true)
}

fn remove_runtime_artifact_path_if_exists(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            fs::remove_dir_all(path).map_err(|error| {
                format!(
                    "failed to remove test-owned runtime artifact directory {}: {error}",
                    path.display()
                )
            })?;
            Ok(true)
        }
        Ok(_) => {
            fs::remove_file(path).map_err(|error| {
                format!(
                    "failed to remove test-owned runtime artifact {}: {error}",
                    path.display()
                )
            })?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "failed to inspect test-owned runtime artifact {}: {error}",
            path.display()
        )),
    }
}

fn prune_empty_artifact_dirs(artifact_root: &Path, mut directory: Option<&Path>) {
    while let Some(path) = directory {
        if path == artifact_root {
            break;
        }
        match fs::remove_dir(path) {
            Ok(()) => directory = path.parent(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => directory = path.parent(),
            Err(_) => break,
        }
    }
}

fn runtime_cleanup_needs_reload(
    reload_marker_exists: bool,
    manifests: &[RuntimeArtifactRunManifest],
    deleted_runtime_paths: bool,
) -> bool {
    reload_marker_exists
        || deleted_runtime_paths
        || manifests.iter().any(|manifest| manifest.reload_required)
}

fn validate_runtime_cleanup_path(relative_path: &str) -> Result<(), String> {
    let normalized = artifact_manifest_relative_path(Path::new(relative_path))?;
    let parts = normalized.split('/').collect::<Vec<_>>();
    if allowed_synthetic_service_runtime_path(&parts) || allowed_package_test_runtime_path(&parts) {
        Ok(())
    } else {
        Err(format!(
            "runtime artifact cleanup path {relative_path} is not test-owned"
        ))
    }
}

fn allowed_synthetic_service_runtime_path(parts: &[&str]) -> bool {
    match parts {
        ["dev", "services", file] => file
            .strip_suffix(".json")
            .is_some_and(is_synthetic_service_artifact_path),
        ["configs", "services", service, rest @ ..] => {
            is_synthetic_service_artifact_path(service) && !rest.is_empty()
        }
        ["dev", "service-test-activations", service, file] => {
            is_synthetic_service_artifact_path(service) && json_file(file)
        }
        ["configs", "service-test-activations", service, pointer_hash, activation_hash, "config.yml"] => {
            is_synthetic_service_artifact_path(service)
                && lowercase_sha256(pointer_hash)
                && lowercase_sha256(activation_hash)
        }
        [directory, "services", service, file]
            if matches!(
                *directory,
                "assemblies" | "units" | "indexes" | "versions" | "builds"
            ) =>
        {
            is_synthetic_service_artifact_path(service) && json_file(file)
        }
        ["files", "services", service, rest @ ..] => {
            is_synthetic_service_artifact_path(service) && !rest.is_empty()
        }
        _ => false,
    }
}

fn allowed_package_test_runtime_path(parts: &[&str]) -> bool {
    match parts {
        ["configs", "package-tests", activation_id, "config.yml"] => {
            validate_package_test_activation_id(activation_id).is_ok()
        }
        ["dev", "package-tests", package_path, file]
        | ["assemblies", "package-tests", package_path, file] => {
            !package_path.is_empty() && json_file(file)
        }
        _ => false,
    }
}

fn is_synthetic_service_artifact_path(value: &str) -> bool {
    value.starts_with(SYNTHETIC_SERVICE_ARTIFACT_PATH_PREFIX)
}

fn json_file(value: &str) -> bool {
    value.ends_with(".json") && value.len() > ".json".len()
}

fn lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn artifact_manifest_relative_path(path: &Path) -> Result<String, String> {
    if path.is_absolute() {
        return Err(format!(
            "runtime artifact manifest path {} must be relative",
            path.display()
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str().ok_or_else(|| {
                    format!(
                        "runtime artifact manifest path {} must be UTF-8",
                        path.display()
                    )
                })?;
                if part.is_empty() {
                    return Err(format!(
                        "runtime artifact manifest path {} contains an empty segment",
                        path.display()
                    ));
                }
                parts.push(part.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "runtime artifact manifest path {} must not contain ..",
                    path.display()
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "runtime artifact manifest path {} must be relative",
                    path.display()
                ));
            }
        }
    }
    if parts.is_empty() {
        return Err("runtime artifact manifest path must not be empty".to_string());
    }
    Ok(parts.join("/"))
}

fn write_runtime_artifact_manifest(
    manifest_path: &Path,
    manifest: &RuntimeArtifactRunManifest,
) -> Result<(), String> {
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create test-runner manifest directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let text = serde_json::to_string_pretty(manifest)
        .map_err(|error| format!("failed to serialize test-runner manifest: {error}"))?;
    let temporary_path = manifest_path.with_extension("json.tmp");
    let mut file = File::create(&temporary_path).map_err(|error| {
        format!(
            "failed to create test-runner manifest {}: {error}",
            temporary_path.display()
        )
    })?;
    file.write_all(format!("{text}\n").as_bytes())
        .map_err(|error| {
            format!(
                "failed to write test-runner manifest {}: {error}",
                temporary_path.display()
            )
        })?;
    file.sync_all().map_err(|error| {
        format!(
            "failed to sync test-runner manifest {}: {error}",
            temporary_path.display()
        )
    })?;
    fs::rename(&temporary_path, manifest_path).map_err(|error| {
        format!(
            "failed to install test-runner manifest {}: {error}",
            manifest_path.display()
        )
    })?;
    sync_parent_dir_best_effort(manifest_path);
    Ok(())
}

fn write_reload_marker(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create test-runner reload marker directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let mut file = File::create(path).map_err(|error| {
        format!(
            "failed to create test-runner reload marker {}: {error}",
            path.display()
        )
    })?;
    file.write_all(b"reload required\n").map_err(|error| {
        format!(
            "failed to write test-runner reload marker {}: {error}",
            path.display()
        )
    })?;
    file.sync_all().map_err(|error| {
        format!(
            "failed to sync test-runner reload marker {}: {error}",
            path.display()
        )
    })?;
    sync_parent_dir_best_effort(path);
    Ok(())
}

fn sync_parent_dir_best_effort(path: &Path) {
    let Some(parent) = path.parent() else {
        return;
    };
    if let Ok(directory) = File::open(parent) {
        let _ = directory.sync_all();
    }
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn test_runner_state_dir(artifact_root: &Path) -> PathBuf {
    artifact_root.join(TEST_RUNNER_STATE_DIR)
}

fn test_runner_runs_dir(artifact_root: &Path) -> PathBuf {
    test_runner_state_dir(artifact_root).join(TEST_RUNNER_RUNS_DIR)
}

fn test_runner_reload_marker_path(artifact_root: &Path) -> PathBuf {
    test_runner_state_dir(artifact_root).join(TEST_RUNNER_RELOAD_MARKER)
}

fn runtime_artifact_run_id() -> String {
    format!(
        "{}-{}-{}",
        std::process::id(),
        current_nanos(),
        SERVICE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

struct RuntimeTempDir {
    path: PathBuf,
}

impl RuntimeTempDir {
    fn create(label: &str) -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "skiff-test-runner-{label}-{}-{}-{}",
            std::process::id(),
            current_nanos(),
            SERVICE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).map_err(|error| {
            format!(
                "failed to create temporary runtime artifact directory {}: {error}",
                path.display()
            )
        })?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RuntimeTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn service_id_artifact_path(service_id: &str) -> String {
    service_id.replace('.', "~").replace('/', "~~")
}

/// Project a service id to the Mongo database name the runtime stores its data
/// under. Must stay byte-for-byte identical to the runtime projection in
/// `runtime/src/host/service_db.rs` (`service_id_storage_database_name`)
/// so test teardown drops exactly the database the test wrote to.
fn service_id_storage_database_name(service_id: &str) -> String {
    service_id.replace('.', "~").replace('/', "~~")
}

/// Drop the per-test service Mongo databases created by a run so they do not
/// accumulate run-over-run. Each test runs under a unique service id (=>
/// unique database), so this only drops databases the run itself created. By
/// default, cleanup waits for `mongosh` and reports cleanup errors to the
/// caller. Set `SKIFF_TEST_SYNC_CLEANUP=0` to restore background best-effort
/// cleanup for local speed.
///
/// The test path already depends on a live local dev stack (router + runtime +
/// Mongo), so it relies on `mongosh` being available for cleanup.
pub(super) fn drop_test_service_databases(
    mongo_url: &str,
    service_ids: &[String],
) -> Result<(), String> {
    if service_ids.is_empty() {
        return Ok(());
    }
    let database_names = service_ids
        .iter()
        .map(|service_id| service_id_storage_database_name(service_id))
        .collect::<Vec<_>>();
    let script = test_database_drop_script(&database_names)?;
    if !sync_test_database_cleanup_enabled() {
        let child = Command::new("mongosh")
            .arg(mongo_url)
            .arg("--quiet")
            .arg("--eval")
            .arg(&script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else {
            return Ok(());
        };
        thread::spawn(move || {
            let _ = child.wait();
        });
        return Ok(());
    }
    let output = Command::new("mongosh")
        .arg(mongo_url)
        .arg("--quiet")
        .arg("--eval")
        .arg(&script)
        .output()
        .map_err(|error| format!("failed to run mongosh for test database drop: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "mongosh exited with status {} while dropping {} test database(s){}",
        output.status,
        database_names.len(),
        command_output_section("stderr", &output.stderr),
    ))
}

fn test_database_drop_script(database_names: &[String]) -> Result<String, String> {
    let database_names_json = serde_json::to_string(database_names)
        .map_err(|error| format!("failed to encode database names for drop: {error}"))?;
    Ok(format!(
        "for (const name of {database_names_json}) {{ db.getSiblingDB(name).dropDatabase(); }}"
    ))
}

fn sync_test_database_cleanup_enabled() -> bool {
    sync_test_database_cleanup_enabled_from_env_value(
        env::var(SYNC_TEST_DB_CLEANUP_ENV).ok().as_deref(),
    )
}

fn sync_test_database_cleanup_enabled_from_env_value(value: Option<&str>) -> bool {
    value
        .map(test_database_cleanup_env_value_enabled)
        .unwrap_or(true)
}

fn test_database_cleanup_env_value_enabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(super) fn execute_runtime_process_test(
    artifacts: &[RuntimeTestArtifact],
    package_aliases: &BTreeMap<String, Vec<String>>,
    operation: &str,
    operation_module: &str,
    target: &str,
    service_id: &str,
    values: JsonValue,
    service_db_mongo_url: Option<&str>,
    doubles: Option<HashMap<String, Vec<TestEffectDouble>>>,
    options: &SkiffTestOptions,
) -> Result<(), String> {
    let runtime_artifact_guard = RUNTIME_ARTIFACT_LOCK
        .lock()
        .map_err(|_| "runtime artifact lock was poisoned".to_string())?;
    let control_base_url = router_control_base_url(options)?;
    let health = health_check(&control_base_url)?;
    let artifact_root = test_artifact_root(&health)?;
    let runtime_artifacts = RuntimeArtifactRun::start(
        runtime_artifact_guard,
        &artifact_root,
        &control_base_url,
        options,
    )?;
    run_with_runtime_artifacts(runtime_artifacts, |runtime_artifacts| {
        let published = write_test_service_artifact_root_with_runtime_path_registration(
            TestServiceArtifactInput {
                artifact_root,
                service_id: service_id.to_string(),
                version: TEST_SERVICE_VERSION.to_string(),
                artifacts: artifacts.iter().map(test_artifact_for_compiler).collect(),
                package_aliases: package_aliases.clone(),
                operation_name: operation.to_string(),
                operation_module: operation_module.to_string(),
                target: target.to_string(),
                test_config: values,
                service_db_mongo_url: service_db_mongo_url.map(ToString::to_string),
            },
            |path| runtime_artifacts.register_path(path),
        )
        .map_err(|error| format!("failed to write runtime test artifacts: {error}"))?;
        for path in &published.runtime_visible_paths {
            runtime_artifacts.mark_written(path)?;
        }

        let dispatch_url = control_url_with_path(&control_base_url, "/__skiff/test-dispatch")?;
        let mut dispatch_body = json!({
                "buildId": published.build_id,
                "mode": "unary",
                "serviceId": published.service_id,
                "serviceProtocolIdentity": published.service_protocol_identity,
                "operation": published.operation_name,
                "operationAbiId": published.operation_abi_id,
                "target": published.target,
                "payloadBase64": "",
                "testEffectsEnabled": !options.live,
                "testEffectDoubles": doubles.unwrap_or_default(),
        });
        dispatch_body["websocketEntryId"] =
            JsonValue::String(synthetic_websocket_entry_id(&published.service_id));
        let response = post_dispatch_json(&dispatch_url, &dispatch_body)
            .map_err(|error| format!("router test dispatch failed: {error}"))?;
        runtime_dispatch_result(&response, None)
    })
}

pub(super) fn execute_dev_synced_service_test(
    publication: &ServiceRuntimePublication,
    package_aliases: &BTreeMap<String, Vec<String>>,
    values: JsonValue,
    service_db_mongo_url: Option<&str>,
    doubles: Option<HashMap<String, Vec<TestEffectDouble>>>,
    request_payload: Option<&str>,
    expected_error: Option<&RuntimeExpectedError>,
    options: &SkiffTestOptions,
) -> Result<(), String> {
    let runtime_artifact_guard = RUNTIME_ARTIFACT_LOCK
        .lock()
        .map_err(|_| "runtime artifact lock was poisoned".to_string())?;
    let control_base_url = router_control_base_url(options)?;
    let health = health_check(&control_base_url)?;
    let artifact_root = test_artifact_root(&health)?;
    let runtime_artifacts = RuntimeArtifactRun::start(
        runtime_artifact_guard,
        &artifact_root,
        &control_base_url,
        options,
    )?;
    run_with_runtime_artifacts(runtime_artifacts, |runtime_artifacts| {
        write_runtime_config(
            publication.root(),
            &values,
            service_db_mongo_url,
            package_aliases.keys().map(String::as_str),
            options.live.then_some(RuntimeLiveMetadata {
                service_id: &publication.service_id,
                version: TEST_SERVICE_VERSION,
            }),
            false,
        )?;
        sync_service_dependency_artifact_roots(
            &artifact_root,
            options,
            Some(&publication.dependency_service_ids),
        )?;
        sync_test_owned_dev_service_artifacts(
            publication.root(),
            &artifact_root,
            &publication.service_id,
            options,
            runtime_artifacts.registrar(),
        )?;
        let pointer = read_dev_reload_pointer(&artifact_root, &publication.service_id)?;

        let reload_response = reload_runtime_artifacts(&control_base_url, options)?;
        let build_id = reloaded_dynamic_build_id(
            &reload_response,
            &publication.service_id,
            &pointer.build_id,
        )?;

        let dispatch_url = control_url_with_path(&control_base_url, "/__skiff/test-dispatch")?;
        let payload_base64 = runtime_test_payload_base64(request_payload)?;
        let mut dispatch_body = json!({
                "buildId": build_id,
                "mode": "unary",
                "serviceId": publication.service_id,
                "serviceProtocolIdentity": pointer.protocol_identity,
                "operation": publication.operation_name,
                "operationAbiId": publication.operation_abi_id,
                "target": publication.target,
                "payloadBase64": payload_base64,
                "testEffectsEnabled": !options.live,
                "testEffectDoubles": doubles.unwrap_or_default(),
        });
        dispatch_body["websocketEntryId"] =
            JsonValue::String(synthetic_websocket_entry_id(&publication.service_id));
        let response = post_dispatch_json(&dispatch_url, &dispatch_body)
            .map_err(|error| format!("router test dispatch failed: {error}"))?;
        runtime_dispatch_result(&response, expected_error)?;
        if expected_error.is_none() && request_payload.is_some() {
            runtime_dispatch_bool_true_payload(&response)?;
        }
        Ok(())
    })
}

pub(super) struct ServiceTestSuiteActivationInput {
    pub(super) case: ServiceRuntimeSuiteCase,
    pub(super) values: JsonValue,
    pub(super) service_db_mongo_url: Option<String>,
}

pub(super) struct PreparedServiceTestSuite {
    runtime_artifacts: RuntimeArtifactRun,
    pub(super) control_base_url: String,
    pub(super) service_id: String,
    pub(super) build_id: String,
    pub(super) service_protocol_identity: String,
}

pub(super) struct PreparedServiceTestSuiteDispatchContext<'a> {
    control_base_url: &'a str,
    service_id: &'a str,
    build_id: &'a str,
    service_protocol_identity: &'a str,
}

impl PreparedServiceTestSuite {
    pub(super) fn dispatch_context(&self) -> PreparedServiceTestSuiteDispatchContext<'_> {
        PreparedServiceTestSuiteDispatchContext {
            control_base_url: &self.control_base_url,
            service_id: &self.service_id,
            build_id: &self.build_id,
            service_protocol_identity: &self.service_protocol_identity,
        }
    }

    pub(super) fn finish(self) -> Result<(), String> {
        self.runtime_artifacts.finish()
    }
}

pub(super) fn prepare_dev_synced_service_test_suite(
    publication: &ServiceRuntimeSuitePublication,
    package_aliases: &BTreeMap<String, Vec<String>>,
    activations: &[ServiceTestSuiteActivationInput],
    options: &SkiffTestOptions,
) -> Result<PreparedServiceTestSuite, String> {
    let runtime_artifact_guard = RUNTIME_ARTIFACT_LOCK
        .lock()
        .map_err(|_| "runtime artifact lock was poisoned".to_string())?;
    let control_base_url = router_control_base_url(options)?;
    let health = health_check(&control_base_url)?;
    let artifact_root = test_artifact_root(&health)?;
    let runtime_artifacts = RuntimeArtifactRun::start(
        runtime_artifact_guard,
        &artifact_root,
        &control_base_url,
        options,
    )?;
    let (runtime_artifacts, (build_id, service_protocol_identity)) =
        run_prepared_runtime_artifacts(runtime_artifacts, |runtime_artifacts| {
            sync_service_dependency_artifact_roots(
                &artifact_root,
                options,
                Some(&publication.dependency_service_ids),
            )?;
            sync_test_owned_dev_service_artifacts(
                publication.root(),
                &artifact_root,
                &publication.service_id,
                options,
                runtime_artifacts.registrar(),
            )?;
            let pointer = read_dev_reload_pointer(&artifact_root, &publication.service_id)?;
            write_service_test_suite_activation_artifacts(
                &artifact_root,
                &publication.service_id,
                &pointer.build_id,
                package_aliases.keys().map(String::as_str),
                activations,
                options,
                runtime_artifacts.registrar(),
            )?;
            let reload_response = reload_runtime_artifacts(&control_base_url, options)?;
            let build_id = reloaded_dynamic_build_id(
                &reload_response,
                &publication.service_id,
                &pointer.build_id,
            )?;
            Ok((build_id, pointer.protocol_identity))
        })?;

    Ok(PreparedServiceTestSuite {
        runtime_artifacts,
        control_base_url,
        service_id: publication.service_id.clone(),
        build_id,
        service_protocol_identity,
    })
}

pub(super) fn execute_dev_synced_service_test_case_with_context(
    prepared: &PreparedServiceTestSuiteDispatchContext<'_>,
    case: &ServiceRuntimeSuiteCase,
    doubles: Option<HashMap<String, Vec<TestEffectDouble>>>,
    request_payload: Option<&str>,
    expected_error: Option<&RuntimeExpectedError>,
    options: &SkiffTestOptions,
) -> Result<(), String> {
    let dispatch_url = control_url_with_path(prepared.control_base_url, "/__skiff/test-dispatch")?;
    let payload_base64 = runtime_test_payload_base64(request_payload)?;
    let mut dispatch_body = json!({
            "buildId": prepared.build_id,
            "mode": "unary",
            "serviceId": prepared.service_id,
            "serviceProtocolIdentity": prepared.service_protocol_identity,
            "operation": case.operation_name,
            "operationAbiId": case.operation_abi_id,
            "target": case.target,
            "activationIdentity": case.activation_identity,
            "payloadBase64": payload_base64,
            "testEffectsEnabled": !options.live,
            "testEffectDoubles": doubles.unwrap_or_default(),
    });
    dispatch_body["websocketEntryId"] =
        JsonValue::String(synthetic_websocket_entry_id(&case.storage_service_id));
    let response = post_dispatch_json(&dispatch_url, &dispatch_body)
        .map_err(|error| format!("router test dispatch failed: {error}"))?;
    runtime_dispatch_result(&response, expected_error)?;
    if expected_error.is_none() && request_payload.is_some() {
        runtime_dispatch_bool_true_payload(&response)?;
    }
    Ok(())
}

fn write_service_test_suite_activation_artifacts<'a>(
    artifact_root: &Path,
    service_id: &str,
    pointer_build_id: &str,
    package_aliases: impl IntoIterator<Item = &'a str>,
    activations: &[ServiceTestSuiteActivationInput],
    options: &SkiffTestOptions,
    runtime_artifacts: &RuntimeArtifactRunRegistrar,
) -> Result<(), String> {
    let service_path = service_id_artifact_path(service_id);
    let pointer_hash = pointer_build_id_hash(pointer_build_id);
    let package_aliases = package_aliases.into_iter().collect::<Vec<_>>();
    let mut cases = Vec::with_capacity(activations.len());
    for activation in activations {
        let activation_hash = sha256_hex(activation.case.activation_identity.as_bytes());
        let config_path = PathBuf::from("configs")
            .join("service-test-activations")
            .join(&service_path)
            .join(&pointer_hash)
            .join(&activation_hash)
            .join("config.yml");
        let runtime_live = options.live.then_some(RuntimeLiveMetadata {
            service_id,
            version: TEST_SERVICE_VERSION,
        });
        runtime_artifacts.register_path(&config_path)?;
        write_runtime_config_at_artifact_path(
            artifact_root,
            &config_path,
            &activation.values,
            activation.service_db_mongo_url.as_deref(),
            package_aliases.iter().copied(),
            runtime_live,
        )?;
        runtime_artifacts.mark_written(&config_path)?;
        let mut case = JsonMap::new();
        case.insert(
            "activationIdentity".to_string(),
            json!(activation.case.activation_identity),
        );
        case.insert("operationTarget".to_string(), json!(activation.case.target));
        case.insert(
            "storageServiceId".to_string(),
            json!(activation.case.storage_service_id),
        );
        case.insert(
            "configPath".to_string(),
            json!(artifact_relative_path(&config_path)),
        );
        if let Some(mongo_url) = activation.service_db_mongo_url.as_deref() {
            case.insert(
                "serviceDb".to_string(),
                json!({
                    "mongoUrl": mongo_url,
                }),
            );
        }
        cases.push(JsonValue::Object(case));
    }
    let sidecar = json!({
        "schemaVersion": "skiff-service-test-activations-v1",
        "serviceId": service_id,
        "pointerBuildId": pointer_build_id,
        "cases": cases,
    });
    let sidecar_path = PathBuf::from("dev")
        .join("service-test-activations")
        .join(&service_path)
        .join(format!("{pointer_hash}.json"));
    runtime_artifacts.register_path(&sidecar_path)?;
    write_json_at_artifact_path(artifact_root, &sidecar_path, &sidecar)?;
    runtime_artifacts.mark_written(&sidecar_path)
}

fn write_runtime_config_at_artifact_path<'a>(
    artifact_root: &Path,
    relative_path: &Path,
    values: &JsonValue,
    service_db_mongo_url: Option<&str>,
    package_aliases: impl IntoIterator<Item = &'a str>,
    runtime_live: Option<RuntimeLiveMetadata<'_>>,
) -> Result<(), String> {
    let config = config_wrapped_for_router_with_package_defaults(
        values,
        service_db_mongo_url,
        package_aliases,
        runtime_live,
        false,
    );
    let path = artifact_root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create service-test runtime config directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let text = serde_yaml::to_string(&config)
        .map_err(|error| format!("failed to serialize service-test runtime config: {error}"))?;
    fs::write(&path, text).map_err(|error| {
        format!(
            "failed to write service-test runtime config {}: {error}",
            path.display()
        )
    })
}

fn write_json_at_artifact_path(
    artifact_root: &Path,
    relative_path: &Path,
    value: &JsonValue,
) -> Result<(), String> {
    let path = artifact_root.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create service-test activation directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed to serialize service-test activations: {error}"))?;
    fs::write(&path, text).map_err(|error| {
        format!(
            "failed to write service-test activations {}: {error}",
            path.display()
        )
    })
}

fn artifact_relative_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn pointer_build_id_hash(pointer_build_id: &str) -> String {
    pointer_build_id
        .strip_prefix("skiff-service-build-v1:sha256:")
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        })
        .unwrap_or("0000000000000000000000000000000000000000000000000000000000000000")
        .to_string()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) struct PreparedPackageTestArtifact {
    runtime_artifacts: RuntimeArtifactRun,
    pub(super) artifact_root: PathBuf,
    pub(super) control_base_url: String,
    pub(super) package_id: String,
    pub(super) package_version: String,
    pub(super) test_build_identity: String,
    package_config_aliases: Vec<String>,
    pub(super) entrypoints: Vec<TestPackageTestEntrypointSummary>,
}

pub(super) struct PreparedPackageTestDispatchContext<'a> {
    artifact_root: &'a Path,
    control_base_url: &'a str,
    package_id: &'a str,
    package_version: &'a str,
    test_build_identity: &'a str,
    package_config_aliases: &'a [String],
    runtime_artifacts: &'a RuntimeArtifactRunRegistrar,
}

impl PreparedPackageTestArtifact {
    pub(super) fn dispatch_context(&self) -> PreparedPackageTestDispatchContext<'_> {
        PreparedPackageTestDispatchContext {
            artifact_root: &self.artifact_root,
            control_base_url: &self.control_base_url,
            package_id: &self.package_id,
            package_version: &self.package_version,
            test_build_identity: &self.test_build_identity,
            package_config_aliases: &self.package_config_aliases,
            runtime_artifacts: self.runtime_artifacts.registrar(),
        }
    }

    pub(super) fn finish(self) -> Result<(), String> {
        self.runtime_artifacts.finish()
    }
}

pub(super) struct PackageTestEntrypointDispatchReport {
    pub(super) service_db_service_id: Option<String>,
    pub(super) result: Result<(), String>,
}

pub(super) fn prepare_dev_synced_package_test(
    mut artifact_input: TestPackageTestArtifactInput,
    options: &SkiffTestOptions,
) -> Result<PreparedPackageTestArtifact, String> {
    let runtime_artifact_guard = RUNTIME_ARTIFACT_LOCK
        .lock()
        .map_err(|_| "runtime artifact lock was poisoned".to_string())?;
    let control_base_url = router_control_base_url(options)?;
    let health = health_check(&control_base_url)?;
    let artifact_root = test_artifact_root(&health)?;
    let runtime_artifacts = RuntimeArtifactRun::start(
        runtime_artifact_guard,
        &artifact_root,
        &control_base_url,
        options,
    )?;
    let package_config_aliases = package_config_aliases(&artifact_input.package_dependencies);
    artifact_input.artifact_root = artifact_root.clone();
    let (runtime_artifacts, written) =
        run_prepared_runtime_artifacts(runtime_artifacts, |runtime_artifacts| {
            sync_service_dependency_artifact_roots(&artifact_root, options, None)?;
            let written = write_package_test_artifact_root_with_runtime_path_registration(
                artifact_input,
                |path| runtime_artifacts.register_path(path),
            )
            .map_err(|error| format!("failed to write package test artifacts: {error}"))?;
            for path in &written.runtime_visible_paths {
                runtime_artifacts.mark_written(path)?;
            }
            ensure_package_test_dependency_configs_are_objects(
                &artifact_root,
                &written.assembly_path,
            )?;
            reload_runtime_artifacts(&control_base_url, options)?;
            Ok(written)
        })?;

    Ok(PreparedPackageTestArtifact {
        runtime_artifacts,
        artifact_root,
        control_base_url,
        package_id: written.package_id,
        package_version: written.package_version,
        test_build_identity: written.test_build_identity,
        package_config_aliases,
        entrypoints: written.entrypoints,
    })
}

pub(super) fn execute_dev_synced_package_test_entrypoint(
    prepared: &PreparedPackageTestArtifact,
    entrypoint: &TestPackageTestEntrypointSummary,
    test_config: JsonValue,
    service_db_mongo_url: Option<&str>,
    doubles: Option<HashMap<String, Vec<TestEffectDouble>>>,
    request_payload: Option<&str>,
    expected_error: Option<&RuntimeExpectedError>,
    options: &SkiffTestOptions,
) -> PackageTestEntrypointDispatchReport {
    execute_dev_synced_package_test_entrypoint_with_context(
        &prepared.dispatch_context(),
        entrypoint,
        test_config,
        service_db_mongo_url,
        doubles,
        request_payload,
        expected_error,
        options,
    )
}

pub(super) fn execute_dev_synced_package_test_entrypoint_with_context(
    prepared: &PreparedPackageTestDispatchContext<'_>,
    entrypoint: &TestPackageTestEntrypointSummary,
    test_config: JsonValue,
    service_db_mongo_url: Option<&str>,
    doubles: Option<HashMap<String, Vec<TestEffectDouble>>>,
    request_payload: Option<&str>,
    expected_error: Option<&RuntimeExpectedError>,
    options: &SkiffTestOptions,
) -> PackageTestEntrypointDispatchReport {
    let activation_id =
        match package_test_activation_id(prepared.package_id, prepared.test_build_identity) {
            Ok(activation_id) => activation_id,
            Err(message) => {
                return PackageTestEntrypointDispatchReport {
                    service_db_service_id: None,
                    result: Err(message),
                };
            }
        };
    let service_db_service_id = if service_db_mongo_url.is_some() && !options.live {
        match package_test_service_db_service_id(&activation_id) {
            Ok(service_id) => Some(service_id),
            Err(message) => {
                return PackageTestEntrypointDispatchReport {
                    service_db_service_id: None,
                    result: Err(message),
                };
            }
        }
    } else {
        None
    };
    let result = execute_dev_synced_package_test_entrypoint_with_activation(
        prepared,
        entrypoint,
        &activation_id,
        test_config,
        service_db_mongo_url,
        doubles,
        request_payload,
        expected_error,
        options,
    );
    PackageTestEntrypointDispatchReport {
        service_db_service_id,
        result,
    }
}

fn execute_dev_synced_package_test_entrypoint_with_activation(
    prepared: &PreparedPackageTestDispatchContext<'_>,
    entrypoint: &TestPackageTestEntrypointSummary,
    activation_id: &str,
    test_config: JsonValue,
    service_db_mongo_url: Option<&str>,
    doubles: Option<HashMap<String, Vec<TestEffectDouble>>>,
    request_payload: Option<&str>,
    expected_error: Option<&RuntimeExpectedError>,
    options: &SkiffTestOptions,
) -> Result<(), String> {
    let config_path = package_test_config_path(activation_id);
    prepared.runtime_artifacts.register_path(&config_path)?;
    write_package_test_runtime_config(
        prepared.artifact_root,
        activation_id,
        prepared.package_config_aliases.iter().map(String::as_str),
        &test_config,
        service_db_mongo_url,
    )?;
    prepared.runtime_artifacts.mark_written(&config_path)?;

    let dispatch_url = control_url_with_path(prepared.control_base_url, "/__skiff/test-dispatch")?;
    let payload_base64 = runtime_test_payload_base64(request_payload)?;
    let dispatch_body = package_test_dispatch_body(PackageTestDispatchInput {
        package_id: prepared.package_id,
        package_version: prepared.package_version,
        test_build_identity: prepared.test_build_identity,
        entrypoint_id: &entrypoint.entrypoint_id,
        activation_id,
        payload_base64: &payload_base64,
        test_effects_enabled: !options.live,
        test_effect_doubles: doubles.unwrap_or_default(),
        timeout_ms: None,
    });
    let response = post_dispatch_json(&dispatch_url, &dispatch_body)
        .map_err(|error| format!("router package-test dispatch failed: {error}"))?;
    runtime_dispatch_result(&response, expected_error)
}

#[allow(dead_code)]
pub(super) fn execute_dev_synced_package_test(
    artifact_input: TestPackageTestArtifactInput,
    values: JsonValue,
    service_db_mongo_url: Option<&str>,
    doubles: Option<HashMap<String, Vec<TestEffectDouble>>>,
    request_payload: Option<&str>,
    expected_error: Option<&RuntimeExpectedError>,
    options: &SkiffTestOptions,
) -> Result<(), String> {
    let prepared = prepare_dev_synced_package_test(artifact_input, options)?;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let entrypoint = prepared
            .entrypoints
            .first()
            .ok_or_else(|| "package test artifact writer returned no entrypoints".to_string())?;
        execute_dev_synced_package_test_entrypoint(
            &prepared,
            entrypoint,
            values,
            service_db_mongo_url,
            doubles,
            request_payload,
            expected_error,
            options,
        )
        .result
    }));
    match result {
        Ok(result) => finish_runtime_artifacts(prepared.runtime_artifacts, result),
        Err(payload) => {
            let _ = prepared.runtime_artifacts.finish();
            std::panic::resume_unwind(payload);
        }
    }
}

#[allow(dead_code)]
pub(crate) struct PackageTestDispatchInput<'a> {
    pub(crate) package_id: &'a str,
    pub(crate) package_version: &'a str,
    pub(crate) test_build_identity: &'a str,
    pub(crate) entrypoint_id: &'a str,
    pub(crate) activation_id: &'a str,
    pub(crate) payload_base64: &'a str,
    pub(crate) test_effects_enabled: bool,
    pub(crate) test_effect_doubles: HashMap<String, Vec<TestEffectDouble>>,
    pub(crate) timeout_ms: Option<u64>,
}

#[allow(dead_code)]
pub(crate) fn package_test_dispatch_body(input: PackageTestDispatchInput<'_>) -> JsonValue {
    let mut body = JsonMap::new();
    body.insert("kind".to_string(), json!("packageTest"));
    body.insert("packageId".to_string(), json!(input.package_id));
    body.insert("packageVersion".to_string(), json!(input.package_version));
    body.insert(
        "testBuildIdentity".to_string(),
        json!(input.test_build_identity),
    );
    body.insert("entrypointId".to_string(), json!(input.entrypoint_id));
    body.insert("activationId".to_string(), json!(input.activation_id));
    body.insert("payloadBase64".to_string(), json!(input.payload_base64));
    body.insert(
        "testEffectsEnabled".to_string(),
        json!(input.test_effects_enabled),
    );
    body.insert(
        "testEffectDoubles".to_string(),
        json!(input.test_effect_doubles),
    );
    if let Some(timeout_ms) = input.timeout_ms {
        body.insert("timeoutMs".to_string(), json!(timeout_ms));
    }
    JsonValue::Object(body)
}

fn test_artifact_for_compiler(artifact: &RuntimeTestArtifact) -> TestServiceFileIrArtifact {
    TestServiceFileIrArtifact {
        source_path: artifact.source_path.clone(),
        module_path: artifact.module_path.clone(),
        role: artifact.role.clone(),
        package_id: artifact.package_id.clone(),
        file_ir: artifact.file_ir.clone(),
    }
}

struct DevReloadPointer {
    build_id: String,
    protocol_identity: String,
}

fn write_runtime_config<'a>(
    root: &Path,
    values: &JsonValue,
    service_db_mongo_url: Option<&str>,
    package_aliases: impl IntoIterator<Item = &'a str>,
    runtime_live: Option<RuntimeLiveMetadata<'_>>,
    include_empty_package_configs: bool,
) -> Result<(), String> {
    let config = config_wrapped_for_router_with_package_defaults(
        values,
        service_db_mongo_url,
        package_aliases,
        runtime_live,
        include_empty_package_configs,
    );
    let path = root.join("config.yml");
    if config.as_object().is_some_and(JsonMap::is_empty) {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    let text = serde_yaml::to_string(&config)
        .map_err(|error| format!("failed to serialize temporary runtime config: {error}"))?;
    fs::write(&path, text).map_err(|error| {
        format!(
            "failed to write temporary runtime config {}: {error}",
            path.display()
        )
    })
}

fn write_package_test_runtime_config<'a>(
    artifact_root: &Path,
    activation_id: &str,
    _package_aliases: impl IntoIterator<Item = &'a str>,
    test_config: &JsonValue,
    service_db_mongo_url: Option<&str>,
) -> Result<(), String> {
    let config_dir = package_test_config_dir(artifact_root, activation_id);
    fs::create_dir_all(&config_dir).map_err(|error| {
        format!(
            "failed to create package-test runtime config directory {}: {error}",
            config_dir.display()
        )
    })?;
    let config = config_wrapped_for_package_test(test_config, service_db_mongo_url);
    let path = config_dir.join("config.yml");
    if config.as_object().is_some_and(JsonMap::is_empty) {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    let text = serde_yaml::to_string(&config)
        .map_err(|error| format!("failed to serialize temporary runtime config: {error}"))?;
    fs::write(&path, text).map_err(|error| {
        format!(
            "failed to write temporary runtime config {}: {error}",
            path.display()
        )
    })
}

fn package_test_config_dir(artifact_root: &Path, activation_id: &str) -> PathBuf {
    artifact_root.join(package_test_config_dir_path(activation_id))
}

fn package_test_config_dir_path(activation_id: &str) -> PathBuf {
    PathBuf::from("configs")
        .join("package-tests")
        .join(activation_id)
}

fn package_test_config_path(activation_id: &str) -> PathBuf {
    package_test_config_dir_path(activation_id).join("config.yml")
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeLiveMetadata<'a> {
    pub(crate) service_id: &'a str,
    pub(crate) version: &'a str,
}

pub(crate) fn config_wrapped_for_router<'a>(
    test_config: &JsonValue,
    service_db_mongo_url: Option<&str>,
    package_aliases: impl IntoIterator<Item = &'a str>,
    runtime_live: Option<RuntimeLiveMetadata<'_>>,
) -> JsonValue {
    config_wrapped_for_router_with_package_defaults(
        test_config,
        service_db_mongo_url,
        package_aliases,
        runtime_live,
        false,
    )
}

fn config_wrapped_for_package_test(
    test_config: &JsonValue,
    service_db_mongo_url: Option<&str>,
) -> JsonValue {
    let mut service = test_config.as_object().cloned().unwrap_or_default();
    service.remove("serviceDb");
    let packages = service.remove("packages");

    let mut config = JsonMap::new();
    if let Some(mongo_url) = service_db_mongo_url {
        config.insert(
            "serviceDb".to_string(),
            json!({
                "mongoUrl": mongo_url,
            }),
        );
    }
    if !service.is_empty() {
        config.insert("service".to_string(), JsonValue::Object(service));
    }
    if let Some(packages) = packages {
        config.insert("packages".to_string(), packages);
    }
    JsonValue::Object(config)
}

fn config_wrapped_for_router_with_package_defaults<'a>(
    test_config: &JsonValue,
    service_db_mongo_url: Option<&str>,
    package_aliases: impl IntoIterator<Item = &'a str>,
    runtime_live: Option<RuntimeLiveMetadata<'_>>,
    include_empty_package_configs: bool,
) -> JsonValue {
    let mut service = test_config.as_object().cloned().unwrap_or_default();
    if let Some(metadata) = runtime_live {
        insert_runtime_live_metadata(&mut service, metadata);
    }
    if let Some(mongo_url) = service_db_mongo_url {
        service.insert(
            "serviceDb".to_string(),
            json!({
                "mongoUrl": mongo_url,
            }),
        );
    }

    let mut config = JsonMap::new();
    if !service.is_empty() {
        config.insert("service".to_string(), JsonValue::Object(service));
    }

    let mut package_config = test_config.as_object().cloned().unwrap_or_default();
    package_config.remove("serviceDb");
    let package_aliases = package_aliases.into_iter().collect::<Vec<_>>();
    if include_empty_package_configs || !package_config.is_empty() {
        let packages = package_aliases
            .into_iter()
            .map(|alias| (alias.to_string(), JsonValue::Object(package_config.clone())))
            .collect::<JsonMap<_, _>>();
        if !packages.is_empty() {
            config.insert("packages".to_string(), JsonValue::Object(packages));
        }
    }

    JsonValue::Object(config)
}

fn package_config_aliases(dependencies: &[PackageDependencyConstraint]) -> Vec<String> {
    let mut aliases = dependencies
        .iter()
        .filter_map(|dependency| {
            let alias = dependency.alias.trim();
            (!alias.is_empty()).then(|| alias.to_string())
        })
        .collect::<Vec<_>>();
    aliases.sort();
    aliases.dedup();
    aliases
}

fn ensure_package_test_dependency_configs_are_objects(
    artifact_root: &Path,
    assembly_path: &str,
) -> Result<(), String> {
    let assembly = read_json_artifact(artifact_root, assembly_path, "package test assembly")?;
    let mut unit_paths = Vec::new();
    let production_unit_path = assembly
        .pointer("/productionPackageUnit/unitPath")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            format!(
                "package test assembly {} productionPackageUnit.unitPath must be a string",
                artifact_path_for_message(artifact_root, assembly_path)
            )
        })?;
    unit_paths.push(production_unit_path.to_string());
    if let Some(dependencies) = assembly
        .get("dependencyPackageUnits")
        .and_then(JsonValue::as_array)
    {
        for (index, dependency) in dependencies.iter().enumerate() {
            let unit_path = dependency
                .get("unitPath")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    format!(
                        "package test assembly {} dependencyPackageUnits[{index}].unitPath must be a string",
                        artifact_path_for_message(artifact_root, assembly_path)
                    )
                })?;
            unit_paths.push(unit_path.to_string());
        }
    }
    unit_paths.sort();
    unit_paths.dedup();
    for unit_path in unit_paths {
        ensure_package_unit_dependency_configs_are_objects(artifact_root, &unit_path)?;
    }
    Ok(())
}

fn ensure_package_unit_dependency_configs_are_objects(
    artifact_root: &Path,
    unit_path: &str,
) -> Result<(), String> {
    let mut unit = read_json_artifact(artifact_root, unit_path, "package unit")?;
    let Some(dependencies) = unit
        .get_mut("dependencies")
        .and_then(JsonValue::as_array_mut)
    else {
        return Ok(());
    };

    let mut changed = false;
    for (index, dependency) in dependencies.iter_mut().enumerate() {
        let dependency = dependency.as_object_mut().ok_or_else(|| {
            format!(
                "package unit {} dependencies[{index}] must be a JSON object",
                artifact_path_for_message(artifact_root, unit_path)
            )
        })?;
        match dependency.get("config") {
            Some(JsonValue::Object(_)) => {}
            Some(JsonValue::Null) | None => {
                dependency.insert("config".to_string(), JsonValue::Object(JsonMap::new()));
                changed = true;
            }
            Some(_) => {
                return Err(format!(
                    "package unit {} dependencies[{index}].config must be a JSON object",
                    artifact_path_for_message(artifact_root, unit_path)
                ));
            }
        }
    }
    if changed {
        write_json_artifact(artifact_root, unit_path, &unit, "package unit")?;
    }
    Ok(())
}

fn read_json_artifact(
    artifact_root: &Path,
    relative_path: &str,
    label: &str,
) -> Result<JsonValue, String> {
    let path = artifact_path(artifact_root, relative_path, label)?;
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read {label} {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("failed to parse {label} {}: {error}", path.display()))
}

fn write_json_artifact(
    artifact_root: &Path,
    relative_path: &str,
    value: &JsonValue,
    label: &str,
) -> Result<(), String> {
    let path = artifact_path(artifact_root, relative_path, label)?;
    let text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("failed to serialize {label} {}: {error}", path.display()))?;
    fs::write(&path, text)
        .map_err(|error| format!("failed to write {label} {}: {error}", path.display()))
}

fn artifact_path(
    artifact_root: &Path,
    relative_path: &str,
    label: &str,
) -> Result<PathBuf, String> {
    let relative = Path::new(relative_path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(format!(
            "{label} path {relative_path} must be artifact-root relative"
        ));
    }
    Ok(artifact_root.join(relative))
}

fn artifact_path_for_message(artifact_root: &Path, relative_path: &str) -> String {
    artifact_root.join(relative_path).display().to_string()
}

fn insert_runtime_live_metadata(
    service: &mut JsonMap<String, JsonValue>,
    metadata: RuntimeLiveMetadata<'_>,
) {
    let runtime_live = service
        .entry("runtimeLive".to_string())
        .or_insert_with(|| JsonValue::Object(JsonMap::new()));
    let Some(runtime_live) = runtime_live.as_object_mut() else {
        return;
    };
    runtime_live.insert(
        "serviceId".to_string(),
        JsonValue::String(metadata.service_id.to_string()),
    );
    runtime_live.insert(
        "version".to_string(),
        JsonValue::String(metadata.version.to_string()),
    );
}

fn dev_reload_error_message(error: String, live: bool) -> String {
    let message = format!("router artifact reload failed: {error}");
    if live {
        return message;
    }
    missing_required_config_from_dev_reload(&message)
        .map(|missing| format!("path {missing} required value is missing or null"))
        .unwrap_or(message)
}

fn missing_required_config_from_dev_reload(message: &str) -> Option<&str> {
    message
        .find("final resolvedConfig ")
        .and_then(|index| message[index..].strip_prefix("final resolvedConfig "))
        .and_then(|rest| rest.strip_suffix(" is required"))
        .map(str::trim)
        .filter(|missing| !missing.is_empty())
}

fn sync_dev_service_artifacts(
    service_root: &Path,
    artifact_root: &Path,
    options: &SkiffTestOptions,
) -> Result<(), String> {
    let script = dev_sync_script_path()?;
    let package_dirs = options.package_resolution_dirs_for(service_root);
    let mut command = Command::new("node");
    command
        .arg(script)
        .arg("--root")
        .arg(service_root)
        .arg("--artifact-root")
        .arg(artifact_root)
        .arg("--no-reload");
    for package_dir in package_dirs.package_dirs {
        command.arg("--packages-dir").arg(package_dir);
    }
    for service_artifact_root in &options.service_artifact_roots {
        command
            .arg("--service-artifact-root")
            .arg(service_artifact_root);
    }
    let output = command
        .output()
        .map_err(|error| format!("failed to run skiff-dev-sync for runtime test: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "skiff-dev-sync failed for runtime test with status {}{}{}",
        output.status,
        command_output_section("stdout", &output.stdout),
        command_output_section("stderr", &output.stderr),
    ))
}

fn sync_test_owned_dev_service_artifacts(
    service_root: &Path,
    artifact_root: &Path,
    service_id: &str,
    options: &SkiffTestOptions,
    runtime_artifacts: &RuntimeArtifactRunRegistrar,
) -> Result<(), String> {
    let temp = RuntimeTempDir::create("dev-sync-artifacts")?;
    sync_dev_service_artifacts(service_root, temp.path(), options)?;
    let runtime_visible_paths = synthetic_service_runtime_visible_paths(temp.path(), service_id)?;
    for path in &runtime_visible_paths {
        runtime_artifacts.register_path(path)?;
    }
    let requested_service_paths = BTreeSet::from([service_id_artifact_path(service_id)]);
    copy_service_dependency_artifact_root(
        temp.path(),
        artifact_root,
        Some(&requested_service_paths),
    )?;
    for path in &runtime_visible_paths {
        if artifact_root.join(path).exists() {
            runtime_artifacts.mark_written(path)?;
        }
    }
    Ok(())
}

fn synthetic_service_runtime_visible_paths(
    artifact_root: &Path,
    service_id: &str,
) -> Result<Vec<PathBuf>, String> {
    let service_path = service_id_artifact_path(service_id);
    let mut paths = Vec::new();
    let pointer = PathBuf::from("dev")
        .join("services")
        .join(format!("{service_path}.json"));
    if artifact_root.join(&pointer).is_file() {
        paths.push(pointer);
    }
    for directory in [
        "assemblies",
        "configs",
        "files",
        "indexes",
        "units",
        "versions",
        "builds",
    ] {
        let root = PathBuf::from(directory)
            .join("services")
            .join(&service_path);
        collect_existing_artifact_files(artifact_root, &root, &mut paths)?;
    }
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(format!(
            "dev sync for {service_id} did not produce any test-owned runtime-visible service artifacts"
        ));
    }
    Ok(paths)
}

fn collect_existing_artifact_files(
    artifact_root: &Path,
    relative_dir: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let dir = artifact_root.join(relative_dir);
    if !dir.exists() {
        return Ok(());
    }
    if !dir.is_dir() {
        return Err(format!(
            "expected artifact path {} to be a directory",
            dir.display()
        ));
    }
    collect_existing_artifact_files_inner(artifact_root, relative_dir, output)
}

fn collect_existing_artifact_files_inner(
    artifact_root: &Path,
    relative_dir: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let dir = artifact_root.join(relative_dir);
    for entry in fs::read_dir(&dir).map_err(|error| {
        format!(
            "failed to read artifact directory {}: {error}",
            dir.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read artifact directory entry in {}: {error}",
                dir.display()
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect artifact path {}: {error}",
                entry.path().display()
            )
        })?;
        let child = relative_dir.join(entry.file_name());
        if file_type.is_dir() {
            collect_existing_artifact_files_inner(artifact_root, &child, output)?;
        } else if file_type.is_file() {
            output.push(child);
        }
    }
    Ok(())
}

fn sync_service_dependency_artifact_roots(
    artifact_root: &Path,
    options: &SkiffTestOptions,
    service_ids: Option<&[String]>,
) -> Result<(), String> {
    let requested_service_paths = requested_service_artifact_paths(service_ids);
    let mut copied_service_paths = BTreeSet::new();
    for dependency_root in &options.service_artifact_roots {
        if !dependency_root.is_dir() {
            return Err(format!(
                "service artifact root {} is not a directory",
                dependency_root.display()
            ));
        }
        copied_service_paths.extend(copy_service_dependency_artifact_root(
            dependency_root,
            artifact_root,
            requested_service_paths.as_ref(),
        )?);
    }
    if let Some(requested) = requested_service_paths {
        let missing = requested
            .difference(&copied_service_paths)
            .cloned()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(format!(
                "service artifact root(s) did not contain dev pointers for required service dependencies: {}",
                missing.join(", ")
            ));
        }
    }
    Ok(())
}

fn copy_service_dependency_artifact_root(
    dependency_root: &Path,
    artifact_root: &Path,
    requested_service_paths: Option<&BTreeSet<String>>,
) -> Result<BTreeSet<String>, String> {
    for directory in SERVICE_DEPENDENCY_SHARED_ARTIFACT_DIRS {
        let source = dependency_root.join(directory);
        if !source.exists() {
            continue;
        }
        if !source.is_dir() {
            return Err(format!(
                "service artifact root entry {} is not a directory",
                source.display()
            ));
        }
        copy_artifact_tree(
            &source,
            &artifact_root.join(directory),
            is_mutable_top_level_artifact_dir(directory),
        )?;
    }
    for (directory, child) in SERVICE_DEPENDENCY_PACKAGE_ARTIFACT_DIRS {
        let source = dependency_root.join(directory).join(child);
        if !source.exists() {
            continue;
        }
        if !source.is_dir() {
            return Err(format!(
                "service artifact root entry {} is not a directory",
                source.display()
            ));
        }
        copy_artifact_tree(
            &source,
            &artifact_root.join(directory).join(child),
            is_mutable_top_level_artifact_dir(directory),
        )?;
    }
    let service_paths =
        dependency_service_artifact_paths(dependency_root, requested_service_paths)?;
    let mut copied_service_paths = BTreeSet::new();
    for service_path in service_paths {
        copy_service_dependency_pointer(dependency_root, artifact_root, &service_path)?;
        copied_service_paths.insert(service_path.clone());
        for (directory, child, overwrite_existing) in SERVICE_DEPENDENCY_SERVICE_ARTIFACT_DIRS {
            let source = dependency_root
                .join(directory)
                .join(child)
                .join(&service_path);
            if !source.exists() {
                continue;
            }
            if !source.is_dir() {
                return Err(format!(
                    "service artifact root entry {} is not a directory",
                    source.display()
                ));
            }
            copy_artifact_tree(
                &source,
                &artifact_root
                    .join(directory)
                    .join(child)
                    .join(&service_path),
                *overwrite_existing,
            )?;
        }
    }
    Ok(copied_service_paths)
}

fn requested_service_artifact_paths(service_ids: Option<&[String]>) -> Option<BTreeSet<String>> {
    service_ids.map(|ids| ids.iter().map(|id| service_id_artifact_path(id)).collect())
}

fn dependency_service_artifact_paths(
    dependency_root: &Path,
    requested_service_paths: Option<&BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    let dev_services = dependency_root.join("dev").join("services");
    if !dev_services.exists() {
        return Ok(Vec::new());
    }
    if !dev_services.is_dir() {
        return Err(format!(
            "service artifact root entry {} is not a directory",
            dev_services.display()
        ));
    }
    if let Some(requested) = requested_service_paths {
        let mut service_paths = requested
            .iter()
            .filter(|service_path| dev_services.join(format!("{service_path}.json")).is_file())
            .cloned()
            .collect::<Vec<_>>();
        service_paths.sort();
        return Ok(service_paths);
    }

    let mut service_paths = Vec::new();
    for entry in fs::read_dir(&dev_services).map_err(|error| {
        format!(
            "failed to read service artifact directory {}: {error}",
            dev_services.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read service artifact entry in {}: {error}",
                dev_services.display()
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect service artifact entry {}: {error}",
                entry.path().display()
            )
        })?;
        if !file_type.is_file()
            || entry.path().extension().and_then(|ext| ext.to_str()) != Some("json")
        {
            continue;
        }
        let service_path = entry
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| {
                format!(
                    "service artifact pointer {} does not have a UTF-8 file stem",
                    entry.path().display()
                )
            })?
            .to_string();
        service_paths.push(service_path);
    }
    service_paths.sort();
    service_paths.dedup();
    Ok(service_paths)
}

fn copy_service_dependency_pointer(
    dependency_root: &Path,
    artifact_root: &Path,
    service_path: &str,
) -> Result<(), String> {
    let source = dependency_root
        .join("dev")
        .join("services")
        .join(format!("{service_path}.json"));
    let target = artifact_root
        .join("dev")
        .join("services")
        .join(format!("{service_path}.json"));
    copy_artifact_file(&source, &target, true)
}

fn is_mutable_top_level_artifact_dir(directory: &str) -> bool {
    matches!(directory, "dev" | "configs" | "indexes")
}

fn copy_artifact_tree(
    source: &Path,
    target: &Path,
    overwrite_existing: bool,
) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|error| {
        format!(
            "failed to create service artifact directory {}: {error}",
            target.display()
        )
    })?;
    for entry in fs::read_dir(source).map_err(|error| {
        format!(
            "failed to read service artifact directory {}: {error}",
            source.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "failed to read service artifact entry in {}: {error}",
                source.display()
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            format!(
                "failed to inspect service artifact entry {}: {error}",
                entry.path().display()
            )
        })?;
        let target_path = target.join(entry.file_name());
        if file_type.is_dir() {
            copy_artifact_tree(&entry.path(), &target_path, overwrite_existing)?;
        } else if file_type.is_file() {
            copy_artifact_file(&entry.path(), &target_path, overwrite_existing)?;
        }
    }
    Ok(())
}

fn copy_artifact_file(
    source: &Path,
    target: &Path,
    overwrite_existing: bool,
) -> Result<(), String> {
    if same_artifact_file(source, target) {
        return Ok(());
    }
    if target.exists() {
        if overwrite_existing {
            fs::copy(source, target).map_err(|error| {
                format!(
                    "failed to overwrite service artifact {} from {}: {error}",
                    target.display(),
                    source.display()
                )
            })?;
            return Ok(());
        }
        let existing = fs::read(target).map_err(|error| {
            format!(
                "failed to read existing service artifact {}: {error}",
                target.display()
            )
        })?;
        let incoming = fs::read(source).map_err(|error| {
            format!(
                "failed to read service artifact {}: {error}",
                source.display()
            )
        })?;
        if existing == incoming {
            return Ok(());
        }
        return Err(format!(
            "service dependency artifact conflict copying {} to {}",
            source.display(),
            target.display()
        ));
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create service artifact directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::copy(source, target).map_err(|error| {
        format!(
            "failed to copy service artifact {} to {}: {error}",
            source.display(),
            target.display()
        )
    })?;
    Ok(())
}

fn same_artifact_file(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    fs::canonicalize(left)
        .ok()
        .zip(fs::canonicalize(right).ok())
        .is_some_and(|(left, right)| left == right)
}

fn dev_sync_script_path() -> Result<PathBuf, String> {
    let std_dir = default_std_dir();
    let skiff_root = std_dir
        .parent()
        .ok_or_else(|| format!("std dir {} had no parent", std_dir.display()))?;
    Ok(skiff_root.join("scripts").join("skiff-dev-sync.mjs"))
}

fn command_output_section(label: &str, bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let text = String::from_utf8_lossy(bytes);
    format!("\n{label}:\n{text}")
}

fn read_dev_reload_pointer(
    artifact_root: &Path,
    service_id: &str,
) -> Result<DevReloadPointer, String> {
    let path = artifact_root
        .join("dev")
        .join("services")
        .join(format!("{}.json", service_id_artifact_path(service_id)));
    let text = fs::read_to_string(&path).map_err(|error| {
        format!(
            "failed to read dev reload pointer {}: {error}",
            path.display()
        )
    })?;
    let value = serde_json::from_str::<JsonValue>(&text).map_err(|error| {
        format!(
            "failed to parse dev reload pointer {}: {error}",
            path.display()
        )
    })?;
    let build_id = value
        .get("buildId")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            format!(
                "dev reload pointer {} did not include buildId",
                path.display()
            )
        })?;
    let protocol_identity = value
        .get("protocolIdentity")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            format!(
                "dev reload pointer {} did not include protocolIdentity",
                path.display()
            )
        })?;
    Ok(DevReloadPointer {
        build_id: build_id.to_string(),
        protocol_identity: protocol_identity.to_string(),
    })
}

fn reloaded_dynamic_build_id(
    reload_response: &JsonValue,
    service_id: &str,
    pointer_build_id: &str,
) -> Result<String, String> {
    let builds = reload_response
        .pointer("/artifact/serviceBuilds")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            "router reload response did not include artifact.serviceBuilds; restart the router with the matching Skiff source"
                .to_string()
        })?;
    builds
        .iter()
        .find(|build| {
            build.get("serviceId").and_then(JsonValue::as_str) == Some(service_id)
                && build.get("pointerBuildId").and_then(JsonValue::as_str)
                    == Some(pointer_build_id)
        })
        .and_then(|build| build.get("buildId").and_then(JsonValue::as_str))
        .map(ToString::to_string)
        .ok_or_else(|| {
            format!(
                "router reload response did not include dynamic buildId for {service_id} pointer build {pointer_build_id}"
            )
        })
}

fn reload_runtime_artifacts(
    control_base_url: &str,
    options: &SkiffTestOptions,
) -> Result<JsonValue, String> {
    reload_runtime_artifacts_for_live(control_base_url, options.live)
}

fn reload_runtime_artifacts_for_live(
    control_base_url: &str,
    live: bool,
) -> Result<JsonValue, String> {
    let reload_url = control_url_with_path(control_base_url, "/__skiff/reload-artifacts")?;
    post_json(&reload_url, &json!({})).map_err(|error| dev_reload_error_message(error, live))
}

fn router_control_base_url(options: &SkiffTestOptions) -> Result<String, String> {
    let configured = options
        .router_reload_url
        .clone()
        .or_else(|| std::env::var("SKIFF_DEV_RELOAD_URL").ok())
        .unwrap_or_else(|| DEFAULT_CONTROL_BASE_URL.to_string());
    control_base_url(&configured)
}

fn health_check(control_base_url: &str) -> Result<JsonValue, String> {
    let health_url = control_url_with_path(control_base_url, "/__router/health")?;
    http_request("GET", &health_url, None)
        .map_err(|error| format!("router health check failed: {error}"))
}

fn control_base_url(url: &str) -> Result<String, String> {
    let parsed = HttpUrl::parse(url)?;
    Ok(format!("http://{}:{}", parsed.host, parsed.port))
}

fn test_artifact_root(health: &JsonValue) -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os(TEST_ARTIFACT_ROOT_ENV) {
        let root = PathBuf::from(root);
        if root.as_os_str().is_empty() {
            return Err(format!("{TEST_ARTIFACT_ROOT_ENV} must not be empty"));
        }
        return Ok(root);
    }
    health
        .pointer("/artifact/artifactRoots/0")
        .and_then(JsonValue::as_str)
        .filter(|root| !root.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            format!(
                "router health did not include artifact.artifactRoots[0]; set {TEST_ARTIFACT_ROOT_ENV} to the runtime artifact root"
            )
        })
}

fn runtime_dispatch_result(
    response: &JsonValue,
    expected_error: Option<&RuntimeExpectedError>,
) -> Result<(), String> {
    let header = response
        .get("header")
        .ok_or_else(|| "router test dispatch response did not include header".to_string())?;
    match header.get("type").and_then(JsonValue::as_str) {
        Some("response.end") => match expected_error {
            Some(expected) => Err(format!(
                "runtime completed successfully but expected error {}",
                expected.code()
            )),
            None => Ok(()),
        },
        Some("response.error") => match expected_error {
            Some(expected) => runtime_dispatch_expected_error(header, expected),
            None => Err(runtime_error_message(header)),
        },
        Some(other) => Err(format!(
            "runtime returned unexpected response frame type {other}"
        )),
        None => Err("runtime response frame did not include type".to_string()),
    }
}

fn runtime_dispatch_expected_error(
    header: &JsonValue,
    expected: &RuntimeExpectedError,
) -> Result<(), String> {
    let error = header
        .get("error")
        .ok_or_else(|| "runtime returned response.error without error payload".to_string())?;
    let code = error
        .get("code")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "runtime returned response.error without error.code".to_string())?;
    let message = error
        .get("message")
        .and_then(JsonValue::as_str)
        .unwrap_or("runtime request failed");
    if code != expected.code {
        return Err(format!(
            "runtime returned error {code}: {message}; expected {}",
            expected.code
        ));
    }
    if let Some(expected_message) = &expected.message_contains {
        if !message.contains(expected_message) {
            return Err(format!(
                "runtime returned expected error code {code} but message {message:?} did not contain {expected_message:?}"
            ));
        }
    }
    Ok(())
}

fn runtime_dispatch_bool_true_payload(response: &JsonValue) -> Result<(), String> {
    let payload_base64 = response
        .get("payloadBase64")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "router test dispatch response did not include payloadBase64".to_string())?;
    let payload = BASE64
        .decode(payload_base64)
        .map_err(|error| format!("runtime response payloadBase64 is invalid: {error}"))?;
    let expected = [
        PAYLOAD_MAGIC.as_slice(),
        &[PAYLOAD_VERSION, PAYLOAD_TAG_BOOL_TRUE],
    ]
    .concat();
    if payload == expected {
        Ok(())
    } else {
        Err("runtime response payload was not the encoded boolean true".to_string())
    }
}

fn runtime_test_payload_base64(request_payload: Option<&str>) -> Result<String, String> {
    let Some(request_payload) = request_payload else {
        return Ok(String::new());
    };
    Ok(BASE64.encode(runtime_test_string_record_payload(
        TEST_REQUEST_PAYLOAD_PARAM,
        request_payload,
    )?))
}

fn package_test_activation_id(
    package_id: &str,
    test_build_identity: &str,
) -> Result<String, String> {
    let build_hash = package_test_build_identity_hash(test_build_identity)?;
    Ok(format!(
        "skiff-package-test-run-v1:{}:{}:{}:{}:{}",
        service_id_artifact_path(package_id),
        &build_hash[..12],
        std::process::id(),
        current_nanos(),
        SERVICE_ID_COUNTER.fetch_add(1, Ordering::Relaxed),
    ))
}

fn package_test_build_identity_hash(test_build_identity: &str) -> Result<&str, String> {
    let hash = test_build_identity
        .strip_prefix(PACKAGE_TEST_BUILD_IDENTITY_PREFIX)
        .ok_or_else(|| {
            format!(
                "testBuildIdentity must use {PACKAGE_TEST_BUILD_IDENTITY_PREFIX}<64 lowercase hex>, got {test_build_identity}"
            )
        })?;
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(format!(
            "testBuildIdentity must use {PACKAGE_TEST_BUILD_IDENTITY_PREFIX}<64 lowercase hex>, got {test_build_identity}"
        ));
    }
    Ok(hash)
}

fn package_test_service_db_service_id(activation_id: &str) -> Result<String, String> {
    validate_package_test_activation_id(activation_id)?;
    let hash = Sha256::digest(activation_id.as_bytes());
    Ok(format!(
        "{PACKAGE_TEST_SERVICE_DB_PREFIX}{}",
        hex_prefix(&hash, 24)
    ))
}

fn validate_package_test_activation_id(value: &str) -> Result<(), String> {
    let Some(suffix) = value.strip_prefix(PACKAGE_TEST_ACTIVATION_ID_PREFIX) else {
        return Err(format!(
            "package-test activationId must start with {PACKAGE_TEST_ACTIVATION_ID_PREFIX}, got {value}"
        ));
    };
    if suffix.is_empty() {
        return Err("package-test activationId must not have an empty run suffix".to_string());
    }
    if suffix.contains("..") {
        return Err(format!(
            "package-test activationId must not contain .., got {value}"
        ));
    }
    if suffix
        .bytes()
        .any(|byte| matches!(byte, b'/' | b'\\') || byte.is_ascii_control())
    {
        return Err(format!(
            "package-test activationId suffix must be a single URL/path safe segment, got {value}"
        ));
    }
    if !suffix
        .bytes()
        .all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b':' | b'~' | b'-'))
    {
        return Err(format!(
            "package-test activationId suffix must match [A-Za-z0-9._:~-]+, got {value}"
        ));
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

fn runtime_test_string_record_payload(field_name: &str, value: &str) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(64 + field_name.len() + value.len());
    output.extend_from_slice(PAYLOAD_MAGIC);
    output.push(PAYLOAD_VERSION);
    output.push(PAYLOAD_TAG_OBJECT);
    write_payload_len(&mut output, 1)?;
    write_payload_raw_string(&mut output, field_name)?;
    output.push(PAYLOAD_TAG_STRING);
    write_payload_raw_string(&mut output, value)?;
    Ok(output)
}

fn write_payload_raw_string(output: &mut Vec<u8>, value: &str) -> Result<(), String> {
    write_payload_len(output, value.len())?;
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

fn write_payload_len(output: &mut Vec<u8>, len: usize) -> Result<(), String> {
    let len =
        u32::try_from(len).map_err(|_| "runtime test payload length exceeds u32".to_string())?;
    output.extend_from_slice(&len.to_le_bytes());
    Ok(())
}

fn runtime_error_message(header: &JsonValue) -> String {
    let Some(error) = header.get("error") else {
        return "runtime returned response.error without error payload".to_string();
    };
    let message = error
        .get("message")
        .and_then(JsonValue::as_str)
        .unwrap_or("runtime request failed");
    let code = error.get("code").and_then(JsonValue::as_str);
    match code {
        Some(code) => format!("{code}: {message}"),
        None => message.to_string(),
    }
}

fn post_dispatch_json(url: &str, body: &JsonValue) -> Result<JsonValue, String> {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        match post_json(url, body) {
            Ok(value) => {
                if !is_transient_dispatch_response(&value) || Instant::now() >= deadline {
                    return Ok(value);
                }
            }
            Err(error) => {
                if !is_transient_dispatch_error(&error) || Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn is_transient_dispatch_response(value: &JsonValue) -> bool {
    let Some(error) = value.get("header").and_then(|header| header.get("error")) else {
        return false;
    };
    let message = error
        .get("message")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    message.contains("no artifact roots are configured")
}

fn is_transient_dispatch_error(error: &str) -> bool {
    error.contains(
        "ProviderUnavailableError: No runtime is registered for the requested service operation",
    ) || error.contains("ProviderUnavailableError: Runtime disconnected before responding")
}

fn post_json(url: &str, body: &JsonValue) -> Result<JsonValue, String> {
    let text = serde_json::to_string(body)
        .map_err(|error| format!("failed to serialize request body: {error}"))?;
    http_request("POST", url, Some(&text))
}

fn http_request(method: &str, url: &str, body: Option<&str>) -> Result<JsonValue, String> {
    let url = HttpUrl::parse(url)?;
    let mut stream = TcpStream::connect((&*url.host, url.port))
        .map_err(|error| format!("failed to connect to {}:{}: {error}", url.host, url.port))?;
    let request = if let Some(body) = body {
        format!(
            "{method} {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            url.path,
            url.host_header(),
            body.as_bytes().len(),
            body
        )
    } else {
        format!(
            "{method} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            url.path,
            url.host_header(),
        )
    };
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("failed to write request: {error}"))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|error| format!("failed to read response: {error}"))?;
    parse_http_json_response(&response)
}

fn parse_http_json_response(response: &[u8]) -> Result<JsonValue, String> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| "HTTP response did not include header terminator".to_string())?;
    let head = String::from_utf8(response[..header_end].to_vec())
        .map_err(|error| format!("HTTP response headers were not UTF-8: {error}"))?;
    let mut lines = head.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| "HTTP response did not include status line".to_string())?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| format!("invalid HTTP status line {status_line}"))?;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value
                .split(',')
                .any(|item| item.trim().eq_ignore_ascii_case("chunked"))
        {
            chunked = true;
        }
    }
    let raw_body = &response[header_end + 4..];
    let body = if chunked {
        decode_chunked_body(raw_body)?
    } else {
        raw_body.to_vec()
    };
    let text = String::from_utf8(body)
        .map_err(|error| format!("HTTP response body was not UTF-8: {error}"))?;
    let value = if text.trim().is_empty() {
        JsonValue::Null
    } else {
        serde_json::from_str::<JsonValue>(&text)
            .map_err(|error| format!("HTTP response body was not JSON: {error}: {text}"))?
    };
    if (200..300).contains(&status) {
        return Ok(value);
    }
    Err(router_error_message(status, &value))
}

fn decode_chunked_body(body: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    let mut index = 0;
    loop {
        let Some(line_end) = find_crlf(&body[index..]) else {
            return Err("chunked HTTP response ended before chunk size".to_string());
        };
        let size_line = std::str::from_utf8(&body[index..index + line_end])
            .map_err(|error| format!("chunk size was not UTF-8: {error}"))?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|error| format!("invalid chunk size {size_hex}: {error}"))?;
        index += line_end + 2;
        if size == 0 {
            break;
        }
        if body.len() < index + size + 2 {
            return Err("chunked HTTP response ended inside chunk".to_string());
        }
        decoded.extend_from_slice(&body[index..index + size]);
        index += size;
        if body.get(index..index + 2) != Some(b"\r\n") {
            return Err("chunked HTTP response chunk missing CRLF".to_string());
        }
        index += 2;
    }
    Ok(decoded)
}

fn find_crlf(bytes: &[u8]) -> Option<usize> {
    bytes.windows(2).position(|window| window == b"\r\n")
}

fn router_error_message(status: u16, value: &JsonValue) -> String {
    let error = value.get("error").unwrap_or(value);
    let code = error.get("code").and_then(JsonValue::as_str);
    let message = error.get("message").and_then(JsonValue::as_str);
    match (code, message) {
        (Some(code), Some(message)) => format!("HTTP {status} {code}: {message}"),
        (_, Some(message)) => format!("HTTP {status}: {message}"),
        _ => format!("HTTP {status}: {value}"),
    }
}

fn control_url_with_path(url: &str, path: &str) -> Result<String, String> {
    let parsed = HttpUrl::parse(url)?;
    Ok(format!("http://{}:{}{}", parsed.host, parsed.port, path))
}

struct HttpUrl {
    host: String,
    port: u16,
    path: String,
}

impl HttpUrl {
    fn parse(value: &str) -> Result<Self, String> {
        let without_scheme = value
            .strip_prefix("http://")
            .ok_or_else(|| format!("only http:// control URLs are supported, got {value}"))?;
        let (authority, path) = without_scheme
            .split_once('/')
            .map(|(authority, path)| (authority, format!("/{path}")))
            .unwrap_or((without_scheme, "/".to_string()));
        if authority.is_empty() {
            return Err(format!("control URL {value} did not include a host"));
        }
        let (host, port) = if let Some((host, port)) = authority.rsplit_once(':') {
            let port = port
                .parse::<u16>()
                .map_err(|error| format!("control URL port {port} is invalid: {error}"))?;
            (host.to_string(), port)
        } else {
            (authority.to_string(), 80)
        };
        if host.is_empty() {
            return Err(format!("control URL {value} did not include a host"));
        }
        Ok(Self { host, port, path })
    }

    fn host_header(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

fn current_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeSet, HashMap},
        path::{Path, PathBuf},
    };

    use serde_json::json;

    use super::{
        cleanup_manifest_paths, copy_artifact_file, copy_service_dependency_artifact_root,
        current_nanos, is_transient_dispatch_error, is_transient_dispatch_response,
        package_test_dispatch_body, package_test_service_db_service_id,
        preflight_runtime_artifact_cleanup, runtime_cleanup_needs_reload,
        service_id_storage_database_name, sync_test_database_cleanup_enabled_from_env_value,
        synthetic_service_runtime_visible_paths, synthetic_test_target,
        test_database_cleanup_env_value_enabled, test_database_drop_script,
        test_runner_reload_marker_path, test_runner_runs_dir, validate_runtime_cleanup_path,
        write_runtime_artifact_manifest, PackageTestDispatchInput, RuntimeArtifactManifestPath,
        RuntimeArtifactRun, RuntimeArtifactRunManifest, SkiffTestOptions, RUNTIME_ARTIFACT_LOCK,
        TEST_RUNNER_MANIFEST_SCHEMA_VERSION,
    };

    #[test]
    fn package_test_dispatch_body_uses_package_only_identity() {
        let body = package_test_dispatch_body(PackageTestDispatchInput {
            package_id: "example.com/pkg",
            package_version: "1.0.0",
            test_build_identity: "skiff-package-test-build-v1:sha256:test",
            entrypoint_id: "skiff-package-test-entrypoint-v1:sha256:entry",
            activation_id: "skiff-package-test-run-v1:example~com~~pkg:test:run:0",
            payload_base64: "",
            test_effects_enabled: true,
            test_effect_doubles: HashMap::new(),
            timeout_ms: Some(2000),
        });

        for service_only in [
            "serviceId",
            "buildId",
            "serviceProtocolIdentity",
            "operation",
            "operationAbiId",
            "target",
            "mode",
        ] {
            assert!(
                body.get(service_only).is_none(),
                "package-test dispatch body must not contain service-only field {service_only}"
            );
        }
        assert_eq!(body["kind"], json!("packageTest"));
        assert_eq!(body["packageId"], json!("example.com/pkg"));
        assert_eq!(body["packageVersion"], json!("1.0.0"));
        assert_eq!(
            body["testBuildIdentity"],
            json!("skiff-package-test-build-v1:sha256:test")
        );
        assert_eq!(
            body["entrypointId"],
            json!("skiff-package-test-entrypoint-v1:sha256:entry")
        );
        assert_eq!(
            body["activationId"],
            json!("skiff-package-test-run-v1:example~com~~pkg:test:run:0")
        );
        assert_eq!(body["timeoutMs"], json!(2000));
    }

    #[test]
    fn package_test_config_wrapper_separates_activation_service_and_packages() {
        let config = super::config_wrapped_for_package_test(
            &json!({
                "app": { "secret": "service-secret" },
                "serviceDb": { "mongoUrl": "mongodb://ignored-from-service-config" },
                "packages": {
                    "deplib": { "dep": { "secret": "dep-secret" } }
                }
            }),
            Some("mongodb://127.0.0.1:27017/?directConnection=true"),
        );

        assert_eq!(
            config.pointer("/serviceDb/mongoUrl"),
            Some(&json!("mongodb://127.0.0.1:27017/?directConnection=true"))
        );
        assert_eq!(
            config.pointer("/service/app/secret"),
            Some(&json!("service-secret"))
        );
        assert!(config.pointer("/service/serviceDb").is_none());
        assert_eq!(
            config.pointer("/packages/deplib/dep/secret"),
            Some(&json!("dep-secret"))
        );
    }

    #[test]
    fn package_test_service_db_cleanup_id_is_activation_derived() {
        let first =
            package_test_service_db_service_id("skiff-package-test-run-v1:example~com~~pkg:run:1")
                .expect("first activation id should project");
        let second =
            package_test_service_db_service_id("skiff-package-test-run-v1:example~com~~pkg:run:2")
                .expect("second activation id should project");

        assert_ne!(first, second);
        assert!(first.starts_with(super::PACKAGE_TEST_SERVICE_DB_PREFIX));
        assert_eq!(
            first.len(),
            super::PACKAGE_TEST_SERVICE_DB_PREFIX.len() + 24
        );
    }

    #[test]
    fn service_id_storage_database_name_matches_runtime_projection() {
        // Must stay identical to the runtime projection in
        // runtime/src/host/service_db.rs::service_id_storage_database_name
        // so test teardown drops exactly the database the runtime wrote to.
        assert_eq!(
            service_id_storage_database_name("example.com/skiff-package-test-1-2-3"),
            "example~com~~skiff-package-test-1-2-3"
        );
        assert_eq!(
            service_id_storage_database_name("skiff.run/account"),
            "skiff~run~~account"
        );
    }

    #[test]
    fn test_database_cleanup_env_accepts_truthy_values() {
        assert!(
            sync_test_database_cleanup_enabled_from_env_value(None),
            "unset cleanup env should default to synchronous cleanup"
        );
        for value in ["1", "true", "TRUE", "yes", "on", " on "] {
            assert!(
                test_database_cleanup_env_value_enabled(value),
                "{value:?} should enable sync cleanup"
            );
            assert!(
                sync_test_database_cleanup_enabled_from_env_value(Some(value)),
                "{value:?} should enable sync cleanup"
            );
        }
        for value in ["", "0", "false", "no", "off", "sync"] {
            assert!(
                !test_database_cleanup_env_value_enabled(value),
                "{value:?} should not enable sync cleanup"
            );
            assert!(
                !sync_test_database_cleanup_enabled_from_env_value(Some(value)),
                "{value:?} should disable sync cleanup"
            );
        }
    }

    #[test]
    fn test_database_drop_script_uses_projected_database_names() {
        let script = test_database_drop_script(&[
            "example~com~~pkg-test-1".to_string(),
            "skiff~run~~package-test-db-abc".to_string(),
        ])
        .expect("drop script should encode database names");

        assert_eq!(
            script,
            "for (const name of [\"example~com~~pkg-test-1\",\"skiff~run~~package-test-db-abc\"]) { db.getSiblingDB(name).dropDatabase(); }"
        );
    }

    #[test]
    fn synthetic_test_target_is_scoped_by_service_and_module() {
        assert_eq!(
            synthetic_test_target(
                "example.com/skiff-package-test-1",
                "api.__test",
                "__skiff_test_0"
            ),
            "skiff.test.example~com~~skiff-package-test-1.api~__test.__skiff_test_0"
        );
        assert_ne!(
            synthetic_test_target("example.com/one", "api.__test", "__skiff_test_0"),
            synthetic_test_target("example.com/two", "api.__test", "__skiff_test_0")
        );
        assert_ne!(
            synthetic_test_target("example.com/one", "api.one", "__skiff_test_0"),
            synthetic_test_target("example.com/one", "api.two", "__skiff_test_0")
        );
    }

    #[test]
    fn transient_dispatch_errors_are_limited_to_routing_races() {
        assert!(is_transient_dispatch_error(
            "HTTP 503 ProviderUnavailableError: No runtime is registered for the requested service operation"
        ));
        assert!(is_transient_dispatch_error(
            "HTTP 503 ProviderUnavailableError: Runtime disconnected before responding"
        ));
        assert!(!is_transient_dispatch_error(
            "HTTP 400 TestDispatchProtocolMismatch: stale active manifest"
        ));
        assert!(!is_transient_dispatch_error(
            "HTTP 503 ProviderUnavailableError: Multiple runtime activations match request; activationIdentity is required"
        ));

        assert!(!is_transient_dispatch_error(
            "HTTP 503 ProviderUnavailableError: connection failed"
        ));
    }

    #[test]
    fn transient_dispatch_responses_are_limited_to_runtime_control_races() {
        assert!(is_transient_dispatch_response(&json!({
            "header": {
                "error": {
                    "message": "no artifact roots are configured for lazy loading serviceId example.com/test buildId skiff-service-build-v1:sha256:abc"
                }
            }
        })));
        assert!(!is_transient_dispatch_response(&json!({
            "header": {
                "error": {
                    "message": "no artifact pointer matched serviceId example.com/test"
                }
            }
        })));
        assert!(!is_transient_dispatch_response(&json!({
            "header": {
                "error": {
                    "message": "no runtime program matched serviceId example.com/test dynamic buildId skiff-service-build-v1:sha256:abc"
                }
            }
        })));
    }

    #[test]
    fn runtime_cleanup_allowlist_rejects_escape_and_real_service_namespaces() {
        assert!(validate_runtime_cleanup_path("/tmp/skiff-artifact.json").is_err());
        assert!(validate_runtime_cleanup_path("dev/services/../escape.json").is_err());
        assert!(validate_runtime_cleanup_path("dev/services/skiff~run~~account.json").is_err());
        assert!(
            validate_runtime_cleanup_path("configs/services/skiff~run~~account/config.yml")
                .is_err()
        );
        assert!(validate_runtime_cleanup_path(
            "dev/services/example~com~~skiff-service-test-1.json"
        )
        .is_ok());
        assert!(
            validate_runtime_cleanup_path(&format!("units/files/{}.json", "a".repeat(64))).is_err()
        );
        assert!(validate_runtime_cleanup_path("units/files/not-a-hash.json").is_err());
    }

    #[test]
    fn stale_manifest_cleanup_deletes_registered_path_even_when_written_false() {
        let dir = temp_runtime_artifact_dir("stale-written-false");
        let relative = "dev/services/example~com~~skiff-stale-service.json";
        write_test_file(&dir, relative);
        let manifest = test_manifest("stale", false, [(relative, false)]);

        let cleanup = cleanup_manifest_paths(&dir, &manifest).expect("cleanup should run");

        assert!(cleanup.deleted_any);
        assert!(!dir.join(relative).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reload_marker_forces_reload_decision_without_deleted_files() {
        let manifest = test_manifest("finished", false, std::iter::empty::<(&str, bool)>());

        assert!(runtime_cleanup_needs_reload(true, &[manifest], false));
    }

    #[test]
    fn synthetic_service_cleanup_removes_exact_paths_but_not_real_service_paths() {
        let dir = temp_runtime_artifact_dir("synthetic-service-cleanup");
        let synthetic = "dev/services/example~com~~skiff-service-test-clean.json";
        let real = "dev/services/skiff~run~~account.json";
        let file_ir = format!("units/files/{}.json", "b".repeat(64));
        write_test_file(&dir, synthetic);
        write_test_file(&dir, real);
        write_test_file(&dir, &file_ir);
        let manifest = test_manifest(
            "mixed",
            true,
            [(synthetic, true), (real, true), (file_ir.as_str(), true)],
        );

        let cleanup = cleanup_manifest_paths(&dir, &manifest).expect("cleanup should run");

        assert!(cleanup.deleted_any);
        assert!(!dir.join(synthetic).exists());
        assert!(dir.join(file_ir).is_file());
        assert!(dir.join(real).is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn preflight_cleanup_does_not_drop_cleanup_required_manifest_without_deleting_paths() {
        let dir = temp_runtime_artifact_dir("preflight-cleanup-required");
        let relative = "dev/services/example~com~~skiff-cleanup-required.json";
        write_test_file(&dir, relative);
        let runs_dir = test_runner_runs_dir(&dir);
        std::fs::create_dir_all(&runs_dir).expect("runs dir should be created");
        let mut manifest = test_manifest("cleanup-required", true, [(relative, false)]);
        manifest.status = "cleanup-required".to_string();
        write_runtime_artifact_manifest(&runs_dir.join("cleanup-required.json"), &manifest)
            .expect("manifest should be written");
        preflight_runtime_artifact_cleanup(&dir, "http://127.0.0.1:1", false)
            .expect_err("reload should fail after cleanup");

        assert!(!dir.join(relative).exists());
        assert!(test_runner_reload_marker_path(&dir).exists());
        assert!(runs_dir.join("cleanup-required.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn synthetic_service_runtime_visible_paths_collects_dev_sync_outputs() {
        let dir = temp_runtime_artifact_dir("synthetic-service-paths");
        let service_id = "example.com/skiff-service-test-paths";
        let service_path = "example~com~~skiff-service-test-paths";
        let expected = [
            format!("dev/services/{service_path}.json"),
            format!("assemblies/services/{service_path}/assembly.json"),
            format!("units/services/{service_path}/unit.json"),
            format!("indexes/services/{service_path}/index.json"),
            format!("configs/services/{service_path}/config.dev.yml"),
        ];
        for path in &expected {
            write_test_file(&dir, path);
        }
        write_test_file(&dir, "dev/services/skiff~run~~account.json");

        let paths = synthetic_service_runtime_visible_paths(&dir, service_id)
            .expect("synthetic service paths should be discovered")
            .into_iter()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .collect::<BTreeSet<_>>();

        for path in expected {
            assert!(paths.contains(&path), "missing {path}");
        }
        assert!(!paths.contains("dev/services/skiff~run~~account.json"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn package_suite_cleanup_removes_pointer_assembly_and_activation_config() {
        let dir = temp_runtime_artifact_dir("package-suite-cleanup");
        let activation_id = "skiff-package-test-run-v1:example~com~~pkg:abcdef123456:1";
        let paths = vec![
            "dev/package-tests/example~com~~pkg/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json".to_string(),
            "assemblies/package-tests/example~com~~pkg/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.json".to_string(),
            format!("configs/package-tests/{activation_id}/config.yml"),
        ];
        for path in &paths {
            write_test_file(&dir, path);
        }
        let manifest = test_manifest(
            "package",
            true,
            paths.iter().map(|path| (path.as_str(), true)),
        );

        let cleanup = cleanup_manifest_paths(&dir, &manifest).expect("cleanup should run");

        assert!(cleanup.deleted_any);
        for path in manifest.paths {
            assert!(!dir.join(path.path).exists());
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn service_dependency_sync_does_not_copy_package_test_directories() {
        let dir = temp_runtime_artifact_dir("dependency-package-test-skip");
        let dependency_root = dir.join("dependency");
        let artifact_root = dir.join("artifact");
        write_test_file(
            &dependency_root,
            "dev/package-tests/example~com~~pkg/hash.json",
        );
        write_test_file(
            &dependency_root,
            "assemblies/package-tests/example~com~~pkg/hash.json",
        );
        write_test_file(
            &dependency_root,
            "configs/package-tests/skiff-package-test-run-v1:example~com~~pkg:run/config.yml",
        );

        copy_service_dependency_artifact_root(&dependency_root, &artifact_root, None)
            .expect("dependency sync should run");

        assert!(!artifact_root.join("dev/package-tests").exists());
        assert!(!artifact_root.join("assemblies/package-tests").exists());
        assert!(!artifact_root.join("configs/package-tests").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn service_dependency_sync_copies_resource_artifact_dir() {
        let dir = temp_runtime_artifact_dir("dependency-resources-copy");
        let dependency_root = dir.join("dependency");
        let artifact_root = dir.join("artifact");
        write_test_file(
            &dependency_root,
            "resources/sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );

        copy_service_dependency_artifact_root(&dependency_root, &artifact_root, None)
            .expect("dependency sync should run");

        assert!(artifact_root
            .join(
                "resources/sha256/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )
            .is_file());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn dispatch_error_path_can_finish_and_cleanup_registered_artifacts() {
        let dir = temp_runtime_artifact_dir("dispatch-error-cleanup");
        let process_guard = RUNTIME_ARTIFACT_LOCK
            .lock()
            .expect("runtime artifact lock should lock");
        let options = SkiffTestOptions::default();
        let run = RuntimeArtifactRun::start(process_guard, &dir, "http://127.0.0.1:1", &options)
            .expect("runtime artifact run should start");
        let relative = "dev/services/example~com~~skiff-dispatch-error.json";
        run.register_path(relative)
            .expect("registered path should be accepted");
        write_test_file(&dir, relative);
        run.mark_written(relative)
            .expect("registered path should be marked written");
        let mut reloads = 0usize;
        let mut reload = || {
            reloads += 1;
            Ok(())
        };

        run.finish_with_reloader(&mut reload)
            .expect("cleanup should finish");
        run.completed
            .store(true, std::sync::atomic::Ordering::SeqCst);

        assert_eq!(reloads, 1);
        assert!(!dir.join(relative).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn service_dependency_sync_filters_requested_service_pointers() {
        let dir = std::env::temp_dir().join(format!(
            "skiff-service-dependency-sync-test-{}-{}",
            std::process::id(),
            current_nanos()
        ));
        let dependency_root = dir.join("dependency");
        let artifact_root = dir.join("artifact");
        std::fs::create_dir_all(dependency_root.join("dev").join("services"))
            .expect("dependency dev services dir should be created");
        std::fs::write(
            dependency_root
                .join("dev")
                .join("services")
                .join("example~com~~needed.json"),
            r#"{"serviceId":"example.com/needed"}"#,
        )
        .expect("needed pointer should be written");
        std::fs::write(
            dependency_root
                .join("dev")
                .join("services")
                .join("example~com~~unrelated.json"),
            r#"{"serviceId":"example.com/unrelated"}"#,
        )
        .expect("unrelated pointer should be written");

        let requested = BTreeSet::from(["example~com~~needed".to_string()]);
        let copied = super::copy_service_dependency_artifact_root(
            &dependency_root,
            &artifact_root,
            Some(&requested),
        )
        .expect("dependency artifacts should copy");

        assert_eq!(copied, requested);
        assert!(artifact_root
            .join("dev")
            .join("services")
            .join("example~com~~needed.json")
            .is_file());
        assert!(!artifact_root
            .join("dev")
            .join("services")
            .join("example~com~~unrelated.json")
            .exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn copy_artifact_file_skips_self_copy_without_truncating() {
        let dir = std::env::temp_dir().join(format!(
            "skiff-copy-self-test-{}-{}",
            std::process::id(),
            current_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        let path = dir.join("artifact.json");
        std::fs::write(&path, r#"{"ok":true}"#).expect("artifact should be written");

        copy_artifact_file(&path, &path, true).expect("self copy should be skipped");

        assert_eq!(
            std::fs::read_to_string(&path).expect("artifact should remain readable"),
            r#"{"ok":true}"#
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    fn temp_runtime_artifact_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "skiff-runtime-process-{label}-{}-{}",
            std::process::id(),
            current_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp artifact dir should be created");
        dir
    }

    fn write_test_file(root: &Path, relative_path: &str) {
        let path = root.join(relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("test artifact parent should be created");
        }
        std::fs::write(path, b"test").expect("test artifact should be written");
    }

    fn test_manifest<'a>(
        run_id: &str,
        reload_required: bool,
        paths: impl IntoIterator<Item = (&'a str, bool)>,
    ) -> RuntimeArtifactRunManifest {
        RuntimeArtifactRunManifest {
            schema_version: TEST_RUNNER_MANIFEST_SCHEMA_VERSION.to_string(),
            run_id: run_id.to_string(),
            status: "active".to_string(),
            reload_required,
            paths: paths
                .into_iter()
                .map(|(path, written)| RuntimeArtifactManifestPath {
                    path: path.to_string(),
                    written,
                })
                .collect(),
        }
    }
}
