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
use sha2::{Digest, Sha256};
use skiff_artifact_identity::{
    gateway_entry_identity, service_deployment_ref, PackageBytecodeRecordPath,
};
use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentRevision, GatewayAdapterArg, GatewayAdapterKind,
    GatewayAdapterSource, GatewayDispatchMode, GatewayEntryIdentity, GatewayProtocolSurface,
    IngressProtocol, PackageArtifact, PackageArtifactRef, ServiceDeployment, ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};
use skiff_runtime_loader::FilesystemDeploymentBytecodeContentResolver;

use crate::host::router_session::ConnectionBootstrap;

pub(in crate::host::request_entry) const FIXTURE_RELATIVE: &str =
    "doc/implementation/bytecode-vm-convergence/fixtures/vcp1-trusted-scalar";
pub(in crate::host::request_entry) const PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-0";
pub(in crate::host::request_entry) const VERSION: &str = "1.0.0";
pub(in crate::host::request_entry) const PROFILE: &str = "skiff-test";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub(in crate::host::request_entry) struct PublishedFixture {
    pub(in crate::host::request_entry) artifact_root: TempRoot,
    fixture_root: PathBuf,
    authoring_receipt: Value,
    pub(in crate::host::request_entry) package_ref: PackageArtifactRef,
    pub(in crate::host::request_entry) package_artifact: Arc<PackageArtifact>,
    pub(in crate::host::request_entry) release_pointer: ReleasePointer,
    pub(in crate::host::request_entry) deployment: ServiceDeploymentRef,
    pub(in crate::host::request_entry) deployment_artifact: Arc<ServiceDeployment>,
    pub(in crate::host::request_entry) gateway_identity: GatewayEntryIdentity,
}

impl PublishedFixture {
    pub(in crate::host::request_entry) fn build(prefix: &str) -> Self {
        let repo_root = repository_root();
        let fixture_root = repo_root.join(FIXTURE_RELATIVE);
        let artifact_root = TempRoot::create(prefix);
        let platform_sources =
            CompilerPlatformSources::new(&repo_root).expect("open repository platform sources");
        let authoring_receipt = build_authoring_object(
            &platform_sources,
            AuthoringObject::Package,
            &fixture_root,
            artifact_root.path(),
            PROFILE,
            true,
        )
        .expect("canonical compiler authoring and publication accepts Phase 0 fixture");
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
            .read_release_pointer(PROFILE, PACKAGE_ID, VERSION)
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
                    && binding.selector.path == "/phase-0/vcp"
            })
            .expect("fixture publishes exact HTTP ingress");
        let gateway_identity = deployment_artifact
            .gateway_entries
            .get(&ingress.gateway_entry_key)
            .expect("fixture ingress pins a gateway entry")
            .gateway_entry_identity
            .clone();

        Self {
            artifact_root,
            fixture_root,
            authoring_receipt,
            package_ref,
            package_artifact,
            release_pointer,
            deployment,
            deployment_artifact,
            gateway_identity,
        }
    }

    pub(in crate::host::request_entry) fn connection_bootstrap(&self) -> ConnectionBootstrap {
        ConnectionBootstrap {
            resolver: FilesystemDeploymentBytecodeContentResolver::open(self.artifact_root.path())
                .expect("open production filesystem resolver"),
            activation: serde_json::from_value(serde_json::json!({ "profile": PROFILE }))
                .expect("decode canonical bootstrap activation"),
            max_response_bytes: 1024,
        }
    }

    /// Republishes the compiler-produced scalar package behind a structurally
    /// valid raw-HTTP deployment surface. This deliberately mismatched
    /// negative reaches request-time linked stream authority checks without
    /// asking the linker to reconstruct source return semantics.
    pub(in crate::host::request_entry) fn into_raw_http_scalar_negative(mut self) -> Self {
        let mut deployment = self.deployment_artifact.as_ref().clone();
        let gateway_key = deployment
            .ingress
            .iter()
            .find(|binding| {
                binding.selector.protocol == IngressProtocol::Http
                    && binding.selector.method.as_deref() == Some("POST")
                    && binding.selector.path == "/phase-0/vcp"
            })
            .expect("scalar negative fixture publishes exact HTTP ingress")
            .gateway_entry_key
            .clone();
        let entry = deployment
            .gateway_entries
            .get_mut(&gateway_key)
            .expect("scalar negative fixture ingress pins a gateway entry");
        let parameter = entry
            .adapter_plan
            .args
            .first()
            .expect("scalar fixture adapter has one exact handler parameter")
            .param
            .clone();
        entry.adapter_plan.kind = GatewayAdapterKind::RawHttp;
        entry.adapter_plan.args = vec![GatewayAdapterArg {
            param: parameter,
            source: GatewayAdapterSource::HttpRequest,
        }];
        let GatewayProtocolSurface::Http(http) = &mut entry.protocol_surface.protocol else {
            panic!("scalar negative fixture gateway remains HTTP")
        };
        http.adapter_kind = GatewayAdapterKind::RawHttp;
        http.dispatch_mode = GatewayDispatchMode::Unary;
        http.external_sources = vec![GatewayAdapterSource::HttpRequest];
        http.request_body_schema = None;
        http.response_schema = None;
        http.stream_item_schema = None;
        entry.gateway_entry_identity = gateway_entry_identity(&entry.protocol_surface)
            .expect("derive raw-HTTP scalar negative gateway identity");
        let gateway_identity = entry.gateway_entry_identity.clone();

        deployment.deployment_revision =
            DeploymentRevision::new("revision-phase-5-raw-http-scalar-negative");
        deployment.deployment_artifact_identity = DeploymentArtifactIdentity::new("unassigned");
        skiff_artifact_identity::assign_service_deployment_identity(&mut deployment)
            .expect("assign raw-HTTP scalar negative deployment identity");
        let deployment_ref = service_deployment_ref(&deployment);
        CanonicalArtifactStore::open(self.artifact_root.path())
            .expect("open scalar negative artifact store")
            .write_service_deployment(&deployment)
            .expect("publish raw-HTTP scalar negative deployment");

        self.deployment = deployment_ref;
        self.deployment_artifact = Arc::new(deployment);
        self.gateway_identity = gateway_identity;
        self
    }

    pub(in crate::host::request_entry) fn corrupt_bytecode_identity(
        &self,
    ) -> BytecodeIdentityCorruption {
        let bytecode = self
            .package_artifact
            .bytecode
            .as_ref()
            .expect("published fixture has bytecode");
        let record_path = PackageBytecodeRecordPath::new(&self.package_ref, bytecode)
            .expect("canonical bytecode record path");
        let absolute_path = self
            .artifact_root
            .path()
            .join(record_path.as_relative_path().as_path());
        let before = fs::read(&absolute_path).expect("read immutable bytecode record");
        let value = serde_json::from_slice::<Value>(&before)
            .expect("canonical bytecode record remains JSON before corruption");
        let identity = value
            .get("bytecodeIdentity")
            .and_then(Value::as_str)
            .expect("bytecode record contains bytecodeIdentity");
        assert_eq!(identity, bytecode.bytecode_identity);

        let positions = before
            .windows(identity.len())
            .enumerate()
            .filter_map(|(offset, window)| (window == identity.as_bytes()).then_some(offset))
            .collect::<Vec<_>>();
        let [identity_offset] = positions.as_slice() else {
            panic!("bytecodeIdentity must occur exactly once in canonical record")
        };
        let mut after = before.clone();
        let nibble_offset = identity_offset + identity.len() - 1;
        assert!(after[nibble_offset].is_ascii_hexdigit());
        after[nibble_offset] = if after[nibble_offset] == b'0' {
            b'1'
        } else {
            b'0'
        };
        assert_eq!(after.len(), before.len());
        serde_json::from_slice::<Value>(&after)
            .expect("corrupted bytecode identity preserves valid JSON");
        fs::write(&absolute_path, &after).expect("write corrupted bytecode record");

        BytecodeIdentityCorruption {
            record_path: record_path.as_str().to_string(),
            before_sha256: sha256_hex(&before),
            after_sha256: sha256_hex(&after),
        }
    }
}

pub(in crate::host::request_entry) struct BytecodeIdentityCorruption {
    pub(in crate::host::request_entry) record_path: String,
    pub(in crate::host::request_entry) before_sha256: String,
    pub(in crate::host::request_entry) after_sha256: String,
}

pub(in crate::host::request_entry) struct TempRoot {
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

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
