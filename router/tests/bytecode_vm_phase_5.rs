use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;

use skiff_compiler::{
    authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::CanonicalArtifactStore;

const ROUTER_BIN_ENV: &str = "SKIFF_BYTECODE_VM_PHASE5_ROUTER_BIN";
const RUNTIME_BIN_ENV: &str = "SKIFF_BYTECODE_VM_PHASE5_RUNTIME_BIN";
const CARRIER_ENV: &str = "SKIFF_BYTECODE_VM_PHASE5_CARRIER_ROOT";
const PROFILE: &str = "skiff-test";
const SERVICE_ID: &str = "test.skiff/bytecode-vm-phase-5";
const VERSION: &str = "1.0.0";
const PROOF_SCHEMA: &str = "skiff-bytecode-vm-phase-5-router-proof-r1";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RouterProofEvidence {
    schema_version: String,
    verdict: String,
    external: ExternalEvidence,
    request: RequestEvidence,
    upstream: UpstreamEvidence,
    runtime_health: RuntimeHealthEvidence,
    timeout: CancellationEvidence,
    disconnect: CancellationEvidence,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExternalEvidence {
    method: String,
    path: String,
    status: u16,
    body: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequestEvidence {
    request_id: String,
    response_frame_types: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UpstreamEvidence {
    routes: Vec<RouteEvidence>,
    two_streams_open_before_release: bool,
    public_origins: Vec<String>,
    proxy_absolute_target_count: usize,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RouteEvidence {
    method: String,
    path: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeHealthEvidence {
    pending: HealthCounters,
    terminal: HealthCounters,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CancellationEvidence {
    request_id: String,
    cancel_reason: String,
    status: Option<u16>,
    error_code: Option<String>,
    provider_streams_closed: bool,
    pre_cancel_response_errors: usize,
    post_cancel_response_frames: usize,
    pending_health: HealthCounters,
    terminal_health: HealthCounters,
}

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HealthCounters {
    outbound_requests_pending: u64,
    outbound_stream_leases_active: u64,
    stream_runtime_streams_active: u64,
    flag_backed_cancel_waiters_active: u64,
    task_requests_active: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_5_router_full_chain_vcp() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("Router crate has repository parent");
        let script = repository.join("scripts/lib/bytecode-vm-phase-5-router-harness.mjs");
        assert!(script.is_file(), "Phase 5 Router harness is missing");

        // The production Router harness is a real process boundary that also
        // starts the production runtime binary and serves the Phase 5 carrier
        // artifact store. Make the test self-sufficient so it is green both
        // under the Phase 5 gate (which pre-populates `CARRIER_ENV` and points
        // `RUNTIME_BIN_ENV` at the shared target dir) and standalone: seed the
        // carrier store and build/locate the runtime binary when needed.
        let carrier = ensure_carrier_root(repository);
        let runtime_bin = ensure_runtime_bin(repository);
        let router_bin = PathBuf::from(env!("CARGO_BIN_EXE_skiff-router"));

        let output = Command::new(std::env::var_os("NODE").unwrap_or_else(|| "node".into()))
            .arg(&script)
            .current_dir(repository)
            .env(ROUTER_BIN_ENV, &router_bin)
            .env(RUNTIME_BIN_ENV, &runtime_bin)
            .env(CARRIER_ENV, &carrier.path)
            .output()
            .expect("start Phase 5 production Router harness");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert!(
        output.status.success(),
        "Phase 5 production Router harness failed at a real process boundary\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
        let evidence_lines = stdout
            .lines()
            .filter(|line| line.trim_start().starts_with('{'))
            .collect::<Vec<_>>();
        assert_eq!(
            evidence_lines.len(),
            1,
            "Phase 5 Router harness must emit exactly one JSON evidence line:\n{stdout}"
        );
        let evidence = serde_json::from_str::<RouterProofEvidence>(evidence_lines[0])
            .unwrap_or_else(|error| {
                panic!("Phase 5 Router evidence is not the exact typed DTO: {error}")
            });
        println!("phase-5-router-evidence={}", evidence_lines[0]);
        assert_exact_evidence(&evidence);
    }
}

fn assert_exact_evidence(evidence: &RouterProofEvidence) {
    assert_eq!(evidence.schema_version, PROOF_SCHEMA);
    assert_eq!(evidence.verdict, "PASS");
    assert_eq!(evidence.external.method, "POST");
    assert_eq!(evidence.external.path, "/phase-5/vcp");
    assert_eq!(evidence.external.status, 207);
    assert_eq!(
        evidence.external.body,
        "U=UNARY|A=LEFT-ALEFT-B|B=RIGHT-ARIGHT-B"
    );
    assert_eq!(
        evidence.request.response_frame_types,
        [
            "response.start",
            "response.chunk",
            "response.chunk",
            "response.chunk",
            "response.chunk",
            "response.chunk",
            "response.chunk",
            "response.end",
        ]
    );
    assert_eq!(
        evidence.upstream.routes,
        [
            RouteEvidence {
                method: "GET".to_string(),
                path: "/request".to_string(),
            },
            RouteEvidence {
                method: "GET".to_string(),
                path: "/stream/left".to_string(),
            },
            RouteEvidence {
                method: "GET".to_string(),
                path: "/stream/right".to_string(),
            },
        ]
    );
    assert!(evidence.upstream.two_streams_open_before_release);
    assert_eq!(
        evidence.upstream.public_origins,
        [
            "http://93.184.216.34",
            "http://93.184.216.34:8080",
            "http://93.184.216.34:8081",
        ]
    );
    assert_eq!(evidence.upstream.proxy_absolute_target_count, 9);
    assert_eq!(evidence.runtime_health.pending, pending_health());
    assert_eq!(evidence.runtime_health.terminal, HealthCounters::default());
    assert_eq!(evidence.timeout.cancel_reason, "timeout");
    assert_eq!(evidence.timeout.status, Some(504));
    assert_eq!(evidence.timeout.error_code.as_deref(), Some("TimeoutError"));
    assert!(evidence.timeout.provider_streams_closed);
    assert!(evidence.timeout.pre_cancel_response_errors <= 1);
    assert_eq!(evidence.timeout.post_cancel_response_frames, 0);
    assert_eq!(evidence.timeout.pending_health, pending_health());
    assert_eq!(evidence.timeout.terminal_health, HealthCounters::default());
    assert_eq!(evidence.disconnect.cancel_reason, "client_disconnect");
    assert_eq!(evidence.disconnect.status, None);
    assert_eq!(evidence.disconnect.error_code, None);
    assert!(evidence.disconnect.provider_streams_closed);
    assert_eq!(evidence.disconnect.pre_cancel_response_errors, 0);
    assert_eq!(evidence.disconnect.post_cancel_response_frames, 0);
    assert_eq!(evidence.disconnect.pending_health, pending_health());
    assert_eq!(
        evidence.disconnect.terminal_health,
        HealthCounters::default()
    );
    assert_ne!(evidence.request.request_id, evidence.timeout.request_id);
    assert_ne!(evidence.request.request_id, evidence.disconnect.request_id);
    assert_ne!(evidence.timeout.request_id, evidence.disconnect.request_id);
}

fn pending_health() -> HealthCounters {
    HealthCounters {
        task_requests_active: 1,
        ..HealthCounters::default()
    }
}

struct CarrierRoot {
    path: PathBuf,
    _temp: Option<TempRoot>,
}

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(prefix: &str) -> Self {
        let ordinal = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p5-r1-{prefix}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create carrier root");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn canonical(&self) -> PathBuf {
        fs::canonicalize(&self.path).expect("resolve canonical carrier root")
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Locate the production runtime binary, building it into the Cargo target
/// dir when neither the environment provides an existing file nor a build
/// artifact is present yet (e.g. a standalone `cargo test` run). The Phase 5
/// gate normally points `RUNTIME_BIN_ENV` at `$CARGO_TARGET_DIR/debug/runtime`
/// after `phase-5-runtime-process-binary`; rebuilding here is an idempotent
/// no-op once the shared binary exists.
fn ensure_runtime_bin(repository: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os(RUNTIME_BIN_ENV) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return fs::canonicalize(&path).expect("canonical runtime binary");
        }
    }
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository.join("target"));
    let bin = target_dir.join("debug").join("runtime");
    if !bin.is_file() {
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let status = Command::new(cargo)
            .args(["build", "-p", "runtime", "--bin", "runtime"])
            .env("CARGO_TARGET_DIR", &target_dir)
            .current_dir(repository)
            .status()
            .expect("start the production runtime build");
        assert!(
            status.success(),
            "building the production runtime binary failed"
        );
    }
    assert!(bin.is_file(), "runtime binary was not produced at {bin:?}");
    fs::canonicalize(&bin).expect("canonical runtime binary")
}

/// Seed the Phase 5 carrier artifact store with the positive fixture so the
/// Router harness can serve `test.skiff/bytecode-vm-phase-5@1.0.0`. Uses the
/// gate-provided carrier root when present and idempotently seeds it when the
/// release pointer is still absent; otherwise creates and seeds a temporary
/// root that lives for the duration of the harness run.
fn ensure_carrier_root(repository: &Path) -> CarrierRoot {
    if let Some(path) = std::env::var_os(CARRIER_ENV) {
        let path = PathBuf::from(path);
        seed_carrier(repository, &path);
        return CarrierRoot {
            path: fs::canonicalize(&path).expect("canonical carrier root"),
            _temp: None,
        };
    }
    let temp = TempRoot::new("phase5-router-carrier");
    seed_carrier(repository, temp.path());
    CarrierRoot {
        path: temp.canonical(),
        _temp: Some(temp),
    }
}

fn seed_carrier(repository: &Path, root: &Path) {
    let store = CanonicalArtifactStore::create(root).expect("open or create carrier store");
    if store
        .read_release_pointer(PROFILE, SERVICE_ID, VERSION)
        .is_ok_and(|pointer| pointer.is_some())
    {
        return;
    }
    let sources = CompilerPlatformSources::new(repository)
        .expect("open repository compiler platform sources");
    seed_official_std_package(&sources, root)
        .expect("seed production std package into the carrier store");
    let fixture = repository.join("runtime/host/tests/fixtures/bytecode-vm-phase-5/positive");
    build_authoring_object(
        &sources,
        AuthoringObject::Package,
        &fixture,
        root,
        PROFILE,
        true,
    )
    .expect("build the positive carrier fixture into the store");
}
