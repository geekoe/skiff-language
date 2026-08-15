use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    sync::Arc,
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    GatewayEntryIdentity, IngressProtocol, PackageArtifact, PackageArtifactRef, ServiceDeployment,
    ServiceDeploymentRef,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    Service,
    InterfaceLocal,
    InterfaceRemote,
    Callback,
    Recoverable,
    Db,
    Task,
    Actor,
    Containment,
}

impl Capability {
    pub fn package_id(self, negative: bool) -> &'static str {
        match (self, negative) {
            (Self::Service, false) => "test.skiff/bytecode-vm-phase-6-service",
            (Self::Service, true) => "test.skiff/bytecode-vm-phase-6-service-negative",
            (Self::InterfaceLocal, false) => "test.skiff/bytecode-vm-phase-6-interface-local",
            (Self::InterfaceLocal, true) => {
                "test.skiff/bytecode-vm-phase-6-interface-local-bad-signature"
            }
            (Self::InterfaceRemote, false) => "test.skiff/bytecode-vm-phase-6-interface",
            (Self::InterfaceRemote, true) => "test.skiff/bytecode-vm-phase-6-interface-negative",
            (Self::Callback, false) => "test.skiff/bytecode-vm-phase-6-callback",
            (Self::Callback, true) => "test.skiff/bytecode-vm-phase-6-callback-negative",
            (Self::Recoverable, false) => "test.skiff/bytecode-vm-phase-6-recoverable",
            (Self::Recoverable, true) => "test.skiff/bytecode-vm-phase-6-recoverable-negative",
            (Self::Db, false) => "test.skiff/bytecode-vm-phase-6-db",
            (Self::Db, true) => "test.skiff/bytecode-vm-phase-6-db-negative",
            (Self::Task, false) => "test.skiff/bytecode-vm-phase-6-task",
            (Self::Task, true) => "test.skiff/bytecode-vm-phase-6-task-negative",
            (Self::Actor, false) => "test.skiff/bytecode-vm-phase-6-actor",
            (Self::Actor, true) => "test.skiff/bytecode-vm-phase-6-actor-negative",
            (Self::Containment, false) => "test.skiff/bytecode-vm-phase-6-containment-positive",
            (Self::Containment, true) => "test.skiff/bytecode-vm-phase-6-containment-negative",
        }
    }

    pub fn path(self, negative: bool) -> String {
        let directory = match self {
            Self::Service if negative => "service-negative",
            Self::Service => "service-positive",
            Self::InterfaceLocal if negative => "interface-local-bad-signature",
            Self::InterfaceLocal => "interface-local-success",
            Self::InterfaceRemote if negative => "interface-negative",
            Self::InterfaceRemote => "interface-positive",
            Self::Callback if negative => "callback-negative",
            Self::Callback => "callback-positive",
            Self::Recoverable if negative => "recoverable-negative",
            Self::Recoverable => "recoverable-positive",
            Self::Db if negative => "db-negative",
            Self::Db => "db-positive",
            Self::Task if negative => "task-negative",
            Self::Task => "task-positive",
            Self::Actor if negative => "actor-negative",
            Self::Actor => "actor-positive",
            Self::Containment if negative => "containment-negative",
            Self::Containment => "containment-positive",
        };
        format!("runtime/host/tests/fixtures/bytecode-vm-phase-6/{directory}")
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

pub struct GatewayFacts {
    pub identity: GatewayEntryIdentity,
}

pub fn build_capability(capability: Capability, negative: bool, prefix: &str) -> BuildOutcome {
    let repository = repository_root();
    if capability == Capability::Service && !negative {
        return build_service_positive(&repository, prefix);
    }
    if capability == Capability::Callback && !negative {
        return build_callback_positive(&repository, prefix);
    }
    let fixture = repository.join(capability.path(negative));
    let root = TempRoot::new(prefix);
    let root_path = root.path().to_path_buf();
    build_single(
        &fixture,
        capability.package_id(negative),
        "1.0.0",
        &root_path,
        root,
    )
}

pub fn build_interface_local_named(
    directory: &str,
    package_id: &str,
    prefix: &str,
) -> BuildOutcome {
    let repository = repository_root();
    let fixture = repository
        .join("runtime/host/tests/fixtures/bytecode-vm-phase-6")
        .join(directory);
    let root = TempRoot::new(prefix);
    let root_path = root.path().to_path_buf();
    build_single(&fixture, package_id, "1.0.0", &root_path, root)
}

fn build_service_positive(repository: &Path, prefix: &str) -> BuildOutcome {
    let root = TempRoot::new(prefix);
    let sources = CompilerPlatformSources::new(repository).expect("open compiler platform sources");
    seed_official_std_package(&sources, root.path()).expect("seed production std package");
    let provider =
        repository.join("runtime/host/tests/fixtures/bytecode-vm-phase-6/service-provider");
    match build_single_into(&provider, "example.com/payments", "1.0.0", root.path()) {
        BuildOutcome::Published(_) => {}
        BuildOutcome::Rejected { error_chain, .. } => {
            return BuildOutcome::Rejected {
                error_chain,
                package_pointer_absent: false,
                release_pointer_absent: false,
            }
        }
    }
    let consumer =
        repository.join("runtime/host/tests/fixtures/bytecode-vm-phase-6/service-positive");
    let mut outcome = build_single_into(
        &consumer,
        "test.skiff/bytecode-vm-phase-6-service",
        "1.0.0",
        root.path(),
    );
    if let BuildOutcome::Published(fixture) = &mut outcome {
        fixture._root = root;
    }
    outcome
}

fn build_callback_positive(repository: &Path, prefix: &str) -> BuildOutcome {
    let root = TempRoot::new(prefix);
    let sources = CompilerPlatformSources::new(repository).expect("open compiler platform sources");
    seed_official_std_package(&sources, root.path()).expect("seed production std package");
    let provider =
        repository.join("runtime/host/tests/fixtures/bytecode-vm-phase-6/callback-provider");
    match build_single_into(
        &provider,
        "example.com/phase-6-callback-provider",
        "1.0.0",
        root.path(),
    ) {
        BuildOutcome::Published(_) => {}
        BuildOutcome::Rejected { error_chain, .. } => {
            return BuildOutcome::Rejected {
                error_chain,
                package_pointer_absent: false,
                release_pointer_absent: false,
            }
        }
    }
    let consumer =
        repository.join("runtime/host/tests/fixtures/bytecode-vm-phase-6/callback-positive");
    let mut outcome = build_single_into(
        &consumer,
        "test.skiff/bytecode-vm-phase-6-callback",
        "1.0.0",
        root.path(),
    );
    if let BuildOutcome::Published(fixture) = &mut outcome {
        fixture._root = root;
    }
    outcome
}

fn build_single(
    fixture: &Path,
    package_id: &str,
    version: &str,
    root_path: &Path,
    root: TempRoot,
) -> BuildOutcome {
    let sources = CompilerPlatformSources::new(&repository_root())
        .expect("open repository compiler platform sources");
    seed_official_std_package(&sources, root_path).expect("seed production std package");
    let mut outcome = build_single_into(fixture, package_id, version, root_path);
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
    let repository = repository_root();
    let sources =
        CompilerPlatformSources::new(&repository).expect("open compiler platform sources");
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

    pub fn gateway(&self, path: &str) -> GatewayFacts {
        let deployment = self.deployment_artifact();
        let binding = deployment
            .ingress
            .iter()
            .find(|binding| {
                binding.selector.protocol == IngressProtocol::Http
                    && binding.selector.method.as_deref() == Some("POST")
                    && binding.selector.path == path
            })
            .unwrap_or_else(|| panic!("published carrier has POST {path}"));
        let gateway = deployment
            .gateway_entries
            .get(&binding.gateway_entry_key)
            .expect("published carrier ingress has gateway entry");
        GatewayFacts {
            identity: gateway.gateway_entry_identity.clone(),
        }
    }

    pub(super) fn link(&self) -> Arc<DeploymentExecutionImage> {
        let hydrated = self.link_input();
        Arc::new(
            link_deployment_execution_image(hydrated, &production_link_limits())
                .expect("construct admitted carrier through the production atomic linker"),
        )
    }

    pub(super) fn link_input(&self) -> HydratedDeploymentBytecode {
        let store = self.store();
        load_deployment_bytecode_from_store(&store, &self.deployment)
            .expect("hydrate admitted carrier through production loader")
    }
}

struct TempRoot {
    path: PathBuf,
    cleanup: bool,
}

impl TempRoot {
    fn new(prefix: &str) -> Self {
        let ordinal = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p6-r1-{prefix}-{}-{ordinal}",
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
