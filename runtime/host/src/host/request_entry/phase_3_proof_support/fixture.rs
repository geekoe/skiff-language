use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;
use skiff_artifact_model::{
    GatewayEntryIdentity, IngressProtocol, IngressSelector, PackageArtifact, PackageArtifactRef,
    ServiceDeployment, ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};
use skiff_runtime_loader::FilesystemDeploymentBytecodeContentResolver;
use skiff_runtime_transport::protocol::{
    decode_bytecode_request_start_frame, encode_binary_frame, BytecodeHttpRequestFrameHeader,
    BytecodeRequestCallerFrameHeader, BytecodeRequestIngressFrameHeader,
    BytecodeRequestIngressProtocol, BytecodeRequestRoutingFrameHeader,
    BytecodeRequestStartFrameHeader, BytecodeRequestStartFrameWireHeader,
    BytecodeRequestTraceFrameHeader, RUNTIME_FRAME_SCHEMA_VERSION,
};

use crate::host::router_session::ConnectionBootstrap;

use super::{CanonicalSkbfRequest, Correlation};

pub(in crate::host::request_entry) const PHASE3_VCP_FIXTURE_RELATIVE: &str =
    "runtime/host/src/host/request_entry/phase_3_proof_support/fixtures/vcp3-union-catch";
pub(in crate::host::request_entry) const PHASE3_MISMATCH_FIXTURE_RELATIVE: &str =
    "runtime/host/src/host/request_entry/phase_3_proof_support/fixtures/vcp3-mismatch-catch";
pub(in crate::host::request_entry) const PHASE3_UNCAUGHT_FIXTURE_RELATIVE: &str =
    "runtime/host/src/host/request_entry/phase_3_proof_support/fixtures/vcp3-uncaught-throw";
pub(in crate::host::request_entry) const PHASE3_HOST_THROW_FIXTURE_RELATIVE: &str =
    "runtime/host/src/host/request_entry/phase_3_proof_support/fixtures/vcp3-host-throw";
pub(in crate::host::request_entry) const PHASE3_PENDING_THROW_FIXTURE_RELATIVE: &str =
    "runtime/host/src/host/request_entry/phase_3_proof_support/fixtures/vcp3-pending-throw";

const PROFILE: &str = "skiff-test";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// The outcome of one production authoring attempt. The success variant
/// carries the published fixture; the failure variant carries the complete
/// deterministic error chain so the harness can assert owner and message.
pub(in crate::host::request_entry) enum Phase3FixtureBuild {
    Published(Phase3PublishedFixture),
    Rejected { error_chain: String },
}

pub(in crate::host::request_entry) struct Phase3PublishedFixture {
    artifact_root: TempRoot,
    authoring_receipt: Value,
    package_ref: PackageArtifactRef,
    package_artifact: Arc<PackageArtifact>,
    release_pointer: ReleasePointer,
    deployment: ServiceDeploymentRef,
    deployment_artifact: Arc<ServiceDeployment>,
    gateway_identity: GatewayEntryIdentity,
    ingress_selector: IngressSelector,
    ingress_path: &'static str,
}

impl Phase3PublishedFixture {
    /// Builds the fixture through the production compiler/authoring/publication
    /// seam, exactly mirroring `Phase2PublishedFixture::build` (which is
    /// hard-wired to the Phase 2 fixtures and therefore cannot be reused here).
    pub(in crate::host::request_entry) fn build(
        prefix: &str,
        fixture_relative: &str,
        package_id: &str,
        version: &str,
        ingress_path: &'static str,
    ) -> Phase3FixtureBuild {
        let repo_root = repository_root();
        let fixture_root = repo_root.join(fixture_relative);
        let artifact_root = TempRoot::create(prefix);
        let platform_sources =
            CompilerPlatformSources::new(&repo_root).expect("open repository platform sources");
        let authoring_receipt = match build_authoring_object(
            &platform_sources,
            AuthoringObject::Package,
            &fixture_root,
            artifact_root.path(),
            PROFILE,
            true,
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                // Fail-closed publication fact: a rejected authoring attempt
                // must never leave a release pointer behind.
                if let Ok(store) = CanonicalArtifactStore::open(artifact_root.path()) {
                    assert!(
                        store
                            .read_release_pointer(PROFILE, package_id, version)
                            .expect("read release pointer after rejected authoring")
                            .is_none(),
                        "rejected authoring must not publish a release pointer"
                    );
                }
                return Phase3FixtureBuild::Rejected {
                    error_chain: error_chain_text(error.as_ref()),
                };
            }
        };

        let package_ref = serde_json::from_value::<PackageArtifactRef>(
            authoring_receipt
                .pointer("/packageArtifactReceipt/artifact")
                .cloned()
                .expect("authoring receipt contains package artifact"),
        )
        .expect("authoring package receipt remains typed");
        let deployment = serde_json::from_value::<ServiceDeploymentRef>(
            authoring_receipt
                .pointer("/serviceDeploymentReceipt/deployment")
                .cloned()
                .expect("authoring receipt contains service deployment"),
        )
        .expect("authoring deployment receipt remains typed");

        let store =
            CanonicalArtifactStore::open(artifact_root.path()).expect("open canonical store");
        let package_artifact = store
            .read_package_artifact(&package_ref)
            .expect("read canonical package artifact");
        let release_pointer = store
            .read_release_pointer(PROFILE, package_id, version)
            .expect("read canonical release pointer")
            .expect("canonical authoring publishes release pointer");
        assert_eq!(release_pointer.deployment, deployment);
        let deployment_artifact = store
            .read_service_deployment(&deployment)
            .expect("read canonical service deployment");
        let ingress = deployment_artifact
            .ingress
            .iter()
            .find(|binding| {
                binding.selector.protocol == IngressProtocol::Http
                    && binding.selector.method.as_deref() == Some("POST")
                    && binding.selector.path == ingress_path
            })
            .unwrap_or_else(|| panic!("fixture publishes exact HTTP ingress {ingress_path}"));
        let gateway_identity = deployment_artifact
            .gateway_entries
            .get(&ingress.gateway_entry_key)
            .expect("fixture ingress pins a gateway entry")
            .gateway_entry_identity
            .clone();
        let ingress_selector = ingress.selector.clone();

        Phase3FixtureBuild::Published(Self {
            artifact_root,
            authoring_receipt,
            package_ref,
            package_artifact,
            release_pointer,
            deployment,
            deployment_artifact,
            gateway_identity,
            ingress_selector,
            ingress_path,
        })
    }

    pub(in crate::host::request_entry) fn deployment_ref(&self) -> &ServiceDeploymentRef {
        &self.deployment
    }

    pub(in crate::host::request_entry) fn artifact_root_path(&self) -> &Path {
        self.artifact_root.path()
    }

    pub(in crate::host::request_entry) fn gateway_identity(&self) -> &GatewayEntryIdentity {
        &self.gateway_identity
    }

    pub(in crate::host::request_entry) fn ingress_selector(&self) -> &IngressSelector {
        &self.ingress_selector
    }

    pub(in crate::host::request_entry) fn ingress_path(&self) -> &'static str {
        self.ingress_path
    }

    pub(in crate::host::request_entry) fn connection_bootstrap(&self) -> ConnectionBootstrap {
        ConnectionBootstrap {
            resolver: FilesystemDeploymentBytecodeContentResolver::open(self.artifact_root.path())
                .expect("open production filesystem resolver"),
            activation: serde_json::from_value(serde_json::json!({ "profile": PROFILE }))
                .expect("decode canonical bootstrap activation"),
            max_response_bytes: 16 * 1024,
        }
    }

    fn http_header(
        &self,
        correlation: &Correlation,
        mode: &str,
    ) -> BytecodeRequestStartFrameHeader {
        BytecodeRequestStartFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.to_string(),
            frame_type: "request.start".to_string(),
            request_id: correlation.request_id.clone(),
            mode: mode.to_string(),
            caller: BytecodeRequestCallerFrameHeader {
                kind: "gateway".to_string(),
            },
            routing: BytecodeRequestRoutingFrameHeader {
                kind: "runtimeAssembly".to_string(),
                assembly_identity: None,
                assembly_generation: None,
                deployment: self.deployment.clone(),
                build_id: Some(
                    self.deployment
                        .deployment_artifact_identity
                        .as_str()
                        .to_string(),
                ),
                gateway_entry_identity: self.gateway_identity.clone(),
                ingress: BytecodeRequestIngressFrameHeader {
                    protocol: BytecodeRequestIngressProtocol::Http,
                    method: "POST".to_string(),
                    path: self.ingress_path.to_string(),
                },
            },
            client_session: None,
            deadline: None,
            trace: BytecodeRequestTraceFrameHeader {
                trace_id: format!("trace-{}", correlation.request_id),
                span_id: format!("span-{}", correlation.request_id),
                parent_span_id: None,
                sampled: None,
            },
            http_request: BytecodeHttpRequestFrameHeader {
                method: "POST".to_string(),
                url: format!("http://phase-3.example.test{}", self.ingress_path),
                path: self.ingress_path.to_string(),
                query: Vec::new(),
                headers: Vec::new(),
            },
            test_effects_enabled: false,
            test_case_capability: None,
            test_case_parent_request_id: None,
        }
    }

    pub(in crate::host::request_entry) fn canonical_request(
        &self,
        correlation: &Correlation,
        mode: &str,
        request_body: &[u8],
    ) -> CanonicalSkbfRequest {
        let frame = encode_binary_frame(&self.http_header(correlation, mode), request_body)
            .expect("encode canonical Phase 3 SKBF request");
        let (header, body) = decode_bytecode_request_start_frame(&frame)
            .expect("production decoder accepts canonical Phase 3 request");
        CanonicalSkbfRequest {
            frame,
            header,
            body,
        }
    }
}

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn create(prefix: &str) -> Self {
        let ordinal = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{prefix}-{}-{ordinal}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create temporary root");
        Self { path }
    }

    pub(in crate::host::request_entry) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host lives below repository root")
        .to_path_buf()
}

/// Deterministic text of the full error chain (outermost first). The harness
/// asserts emission-owner and message stability on this exact text.
fn error_chain_text(mut error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![error.to_string()];
    while let Some(source) = error.source() {
        parts.push(source.to_string());
        error = source;
    }
    parts.join(" :: ")
}
