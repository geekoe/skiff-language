use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    PackageArtifact, PackageArtifactRef, ServiceDeployment, ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_linker::{link_deployment_execution_image, DeploymentExecutionImage, LinkLimits};
use skiff_runtime_loader::{load_deployment_bytecode_from_store, HydratedDeploymentBytecode};

const PROFILE: &str = "skiff-test";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

/// Each Phase 7 whole-system fixture publishes one deterministic immutable
/// activation from real `.skiff` source through the production compiler,
/// canonical artifact store and atomic linker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Unary,
    ServiceChild,
    InterfaceLocal,
    InterfaceRemote,
    Callback,
    Actor,
    Db,
    Recoverable,
    Throw,
    Slow,
    Task,
    TaskActorMethod,
    CrossRuntimeCallback,
    RequestGc,
    ActorCompaction,
}

impl Capability {
    pub fn package_id(self) -> &'static str {
        match self {
            Self::Unary => "test.skiff/bytecode-vm-phase-7-unary",
            Self::ServiceChild => "test.skiff/bytecode-vm-phase-7-service-child",
            Self::InterfaceLocal => "test.skiff/bytecode-vm-phase-7-interface-local",
            Self::InterfaceRemote => "test.skiff/bytecode-vm-phase-7-interface-remote",
            Self::Callback => "test.skiff/bytecode-vm-phase-7-callback",
            Self::Actor => "test.skiff/bytecode-vm-phase-7-actor",
            Self::Db => "test.skiff/bytecode-vm-phase-7-db",
            Self::Recoverable => "test.skiff/bytecode-vm-phase-7-recoverable",
            Self::Throw => "test.skiff/bytecode-vm-phase-7-throw",
            Self::Slow => "test.skiff/bytecode-vm-phase-7-slow",
            Self::Task => "test.skiff/bytecode-vm-phase-7-task",
            Self::TaskActorMethod => "test.skiff/bytecode-vm-phase-7-task-actor-method",
            Self::CrossRuntimeCallback => "test.skiff/bytecode-vm-phase-7-callback-cross-runtime",
            Self::RequestGc => "test.skiff/bytecode-vm-phase-7-request-gc",
            Self::ActorCompaction => "test.skiff/bytecode-vm-phase-7-actor-compaction",
        }
    }

    pub fn path(self) -> String {
        let directory = match self {
            Self::Unary => "http-unary-echo",
            Self::ServiceChild => "service-child",
            Self::InterfaceLocal => "interface-local",
            Self::InterfaceRemote => "interface-remote",
            Self::Callback => "callback",
            Self::Actor => "actor",
            Self::Db => "db",
            Self::Recoverable => "recoverable",
            Self::Throw => "throw",
            Self::Slow => "slow",
            Self::Task => "task",
            Self::TaskActorMethod => "task-actor-method",
            Self::CrossRuntimeCallback => "capability-callback-cross-runtime",
            Self::RequestGc => "capability-request-gc",
            Self::ActorCompaction => "capability-actor-compaction",
        };
        format!("runtime/host/tests/fixtures/bytecode-vm-phase-7/{directory}")
    }

    /// Provider package (id, directory) when this capability calls another
    /// service through the exact provider build.
    pub fn provider(self) -> Option<(&'static str, &'static str)> {
        match self {
            Self::ServiceChild => Some(("example.com/p7-payments", "service-child-provider")),
            Self::InterfaceRemote => Some((
                "example.com/p7-remote-provider",
                "interface-remote-provider",
            )),
            Self::Callback => Some(("example.com/p7-callback-provider", "callback-provider")),
            Self::Throw => Some(("example.com/p7-throw-provider", "throw-provider")),
            _ => None,
        }
    }

    pub fn ingress_path(self) -> &'static str {
        match self {
            Self::Unary => "/phase-7/unary",
            Self::ServiceChild => "/phase-7/service-child",
            Self::InterfaceLocal => "/phase-7/interface-local",
            Self::InterfaceRemote => "/phase-7/interface-remote",
            Self::Callback => "/phase-7/callback",
            Self::Actor => "/phase-7/actor",
            Self::Db => "/phase-7/db",
            Self::Recoverable => "/phase-7/recoverable",
            Self::Throw => "/phase-7/throw",
            Self::Slow => "/phase-7/slow",
            Self::Task => "/phase-7/task",
            Self::TaskActorMethod => "/phase-7/task-actor-method",
            Self::CrossRuntimeCallback => "/phase-7/callback-cross-runtime",
            Self::RequestGc => "/phase-7/request-gc",
            Self::ActorCompaction => "/phase-7/actor-compaction",
        }
    }
}

pub enum BuildOutcome {
    Published(PublishedFixture),
    Rejected {
        error_chain: String,
        package_pointer_absent: bool,
        release_pointer_absent: bool,
    },
}

pub struct PublishedFixture {
    _root: TempRoot,
    pub artifact_root: PathBuf,
    pub package: PackageArtifactRef,
    pub deployment: ServiceDeploymentRef,
}

impl PublishedFixture {
    pub fn store(&self) -> CanonicalArtifactStore {
        CanonicalArtifactStore::open(&self.artifact_root).expect("open published carrier store")
    }

    pub fn package_artifact(&self) -> Arc<PackageArtifact> {
        self.store()
            .read_package_artifact(&self.package)
            .expect("read published carrier package")
    }

    pub fn deployment_artifact(&self) -> Arc<ServiceDeployment> {
        self.store()
            .read_service_deployment(&self.deployment)
            .expect("read published carrier deployment")
    }

    pub fn bytecode(&self) -> Arc<ValidatedBytecodeArtifact> {
        let package = self.package_artifact();
        let bytecode = package
            .bytecode
            .as_ref()
            .expect("admitted carrier package has bytecode");
        self.store()
            .read_package_bytecode(&self.package, bytecode)
            .expect("read admitted carrier bytecode")
    }

    pub fn link(&self) -> Arc<DeploymentExecutionImage> {
        let hydrated = self.link_input();
        Arc::new(
            link_deployment_execution_image(hydrated, &production_link_limits())
                .expect("construct admitted carrier through the production atomic linker"),
        )
    }

    pub fn link_input(&self) -> HydratedDeploymentBytecode {
        let store = self.store();
        load_deployment_bytecode_from_store(&store, &self.deployment)
            .expect("hydrate admitted carrier through production loader")
    }
}

/// Builds the capability fixture through the production compiler and
/// publication store. Provider-backed capabilities publish the exact provider
/// build into the same immutable artifact root first.
pub fn build_capability(capability: Capability, prefix: &str) -> BuildOutcome {
    let repository = repository_root();
    let root = TempRoot::new(prefix);
    let root_path = root.path().to_path_buf();
    let sources =
        CompilerPlatformSources::new(&repository).expect("open compiler platform sources");
    seed_official_std_package(&sources, root.path()).expect("seed production std package");
    let fixture = repository.join(capability.path());
    if let Some((provider_id, provider_dir)) = capability.provider() {
        let provider = repository
            .join("runtime/host/tests/fixtures/bytecode-vm-phase-7")
            .join(provider_dir);
        match build_single_into(&provider, provider_id, "1.0.0", &root_path) {
            BuildOutcome::Published(_) => {}
            BuildOutcome::Rejected { error_chain, .. } => {
                return BuildOutcome::Rejected {
                    error_chain,
                    package_pointer_absent: false,
                    release_pointer_absent: false,
                }
            }
        }
    }
    let mut outcome = build_single_into(&fixture, capability.package_id(), "1.0.0", &root_path);
    if let BuildOutcome::Published(fixture) = &mut outcome {
        fixture._root = root;
    }
    outcome
}

fn build_single_into(
    fixture: &Path,
    package_id: &str,
    version: &str,
    root_path: &Path,
) -> BuildOutcome {
    let sources =
        CompilerPlatformSources::new(&repository_root()).expect("open compiler platform sources");
    let receipt = match build_authoring_object(
        &sources,
        AuthoringObject::Package,
        fixture,
        root_path,
        PROFILE,
        true,
    ) {
        Ok(receipt) => receipt,
        Err(error) => {
            let store =
                CanonicalArtifactStore::open(root_path).expect("open rejected carrier store");
            let package_pointer_absent = store
                .read_package_artifact_pointer(package_id, version)
                .expect("read rejected carrier package pointer")
                .is_none();
            let release_pointer_absent = store
                .read_release_pointer(PROFILE, package_id, version)
                .expect("read rejected carrier release pointer")
                .is_none();
            return BuildOutcome::Rejected {
                error_chain: error_chain(error.as_ref()),
                package_pointer_absent,
                release_pointer_absent,
            };
        }
    };
    let package = serde_json::from_value(
        receipt
            .pointer("/packageArtifactReceipt/artifact")
            .cloned()
            .expect("authoring receipt package artifact"),
    )
    .expect("typed package ref");
    let deployment = serde_json::from_value(
        receipt
            .pointer("/serviceDeploymentReceipt/deployment")
            .cloned()
            .expect("authoring receipt deployment"),
    )
    .expect("typed deployment ref");
    BuildOutcome::Published(PublishedFixture {
        _root: TempRoot::existing(root_path.to_path_buf()),
        artifact_root: root_path.to_path_buf(),
        package,
        deployment,
    })
}

struct TempRoot {
    path: PathBuf,
    cleanup: bool,
}

impl TempRoot {
    fn new(prefix: &str) -> Self {
        let ordinal = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p7-r1-{prefix}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create carrier root");
        Self {
            path,
            cleanup: true,
        }
    }

    fn existing(path: PathBuf) -> Self {
        Self {
            path,
            cleanup: false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        if self.cleanup {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("runtime/host is below repository root")
        .to_path_buf()
}

fn error_chain(mut error: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![error.to_string()];
    while let Some(source) = error.source() {
        parts.push(source.to_string());
        error = source;
    }
    parts.join(" :: ")
}

fn production_link_limits() -> LinkLimits {
    LinkLimits {
        max_packages: 256,
        max_root_specializations: 100_000,
        max_specializations: 1_000_000,
        max_code_words_per_function: 1_000_000,
        max_total_code_words: 100_000_000,
        max_relocations_per_function: 100_000,
        max_total_relocations: 10_000_000,
        max_image_table_entries: 1_000_000,
        max_total_image_table_entries: 10_000_000,
        max_total_function_table_entries: 10_000_000,
        max_type_nesting_depth: 64,
        max_expanded_type_nodes: 1_000_000,
        max_expanded_type_bytes: 64 * 1024 * 1024,
        max_constant_graph_nodes: 1_000_000,
        max_constant_graph_edges: 1_000_000,
    }
}
