use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    BoundaryUnavailableReason, GatewayEntryIdentity, IngressProtocol, IngressSelector,
    PackageArtifact, PackageArtifactRef, ServiceDeployment, ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
    BytecodeEmissionError, CompilerPlatformSources, ContractDefinitionError,
    Phase1UnsupportedCapability,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_linker::{
    link_deployment_execution_image, DeploymentExecutionImage, LinkLimits,
};
use skiff_runtime_loader::load_deployment_bytecode_from_store;
use std::sync::Arc;

const PROFILE: &str = "skiff-test";
const ROUTER_CARRIER_ENV: &str = "SKIFF_BYTECODE_VM_PHASE5_CARRIER_ROOT";
const POSITIVE_PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-5";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

pub enum BuildOutcome {
    Published(PublishedFixture),
    Rejected {
        error_chain: String,
        package_pointer_absent: bool,
        release_pointer_absent: bool,
        rejection: Option<TypedRejection>,
    },
}

#[derive(Debug)]
pub enum TypedRejection {
    Phase1Capability {
        capability: Phase1UnsupportedCapability,
        module_path: String,
        function_key: Option<String>,
    },
    UnavailableServiceCalls {
        unavailable: BTreeMap<String, Vec<BoundaryUnavailableReason>>,
    },
}

pub struct PublishedFixture {
    _root: TempRoot,
    pub artifact_root: PathBuf,
    pub package: PackageArtifactRef,
    pub deployment: ServiceDeploymentRef,
}

pub struct GatewayFacts {
    pub ingress: IngressSelector,
    pub identity: GatewayEntryIdentity,
}

#[derive(Clone, Copy)]
pub struct FixtureSpec {
    relative: &'static str,
    package_id: &'static str,
    version: &'static str,
}

impl FixtureSpec {
    pub const fn positive() -> Self {
        Self {
            relative: "runtime/host/tests/fixtures/bytecode-vm-phase-5/positive",
            package_id: "test.skiff/bytecode-vm-phase-5",
            version: "1.0.0",
        }
    }

    pub const fn unsupported_sse() -> Self {
        Self {
            relative: "runtime/host/tests/fixtures/bytecode-vm-phase-5/unsupported-sse",
            package_id: "test.skiff/bytecode-vm-phase-5-unsupported-sse",
            version: "1.0.0",
        }
    }

    pub const fn unsupported_date_now() -> Self {
        Self {
            relative: "runtime/host/tests/fixtures/bytecode-vm-phase-5/unsupported-date-now",
            package_id: "test.skiff/bytecode-vm-phase-5-unsupported-date-now",
            version: "1.0.0",
        }
    }

    pub const fn illegal_stream_placement() -> Self {
        Self {
            relative: "runtime/host/tests/fixtures/bytecode-vm-phase-5/illegal-stream-placement",
            package_id: "test.skiff/bytecode-vm-phase-5-illegal-stream-placement",
            version: "1.0.0",
        }
    }

    pub fn build(self, prefix: &str) -> BuildOutcome {
        let repository = repository_root();
        let fixture = repository.join(self.relative);
        let root = self.carrier_root(prefix, &repository);
        if let Some((package, deployment)) = self.existing_carrier_refs(&root) {
            let artifact_root = root.path().to_path_buf();
            return BuildOutcome::Published(PublishedFixture {
                _root: root,
                artifact_root,
                package,
                deployment,
            });
        }
        let sources = CompilerPlatformSources::new(&repository)
            .expect("open repository compiler platform sources");
        seed_official_std_package(&sources, root.path())
            .expect("seed production std package into carrier store");
        let receipt = match build_authoring_object(
            &sources,
            AuthoringObject::Package,
            &fixture,
            root.path(),
            PROFILE,
            true,
        ) {
            Ok(receipt) => receipt,
            Err(error) => {
                let rejection = typed_rejection(error.as_ref());
                let store =
                    CanonicalArtifactStore::open(root.path()).expect("open rejected carrier store");
                let package_pointer_absent = store
                    .read_package_artifact_pointer(self.package_id, self.version)
                    .expect("read rejected carrier package pointer")
                    .is_none();
                let release_pointer_absent = store
                    .read_release_pointer(PROFILE, self.package_id, self.version)
                    .expect("read rejected carrier release pointer")
                    .is_none();
                return BuildOutcome::Rejected {
                    error_chain: error_chain(error.as_ref()),
                    package_pointer_absent,
                    release_pointer_absent,
                    rejection,
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
        let artifact_root = root.path().to_path_buf();
        BuildOutcome::Published(PublishedFixture {
            _root: root,
            artifact_root,
            package,
            deployment,
        })
    }

    fn carrier_root(self, prefix: &str, repository: &Path) -> TempRoot {
        if self.package_id != POSITIVE_PACKAGE_ID {
            return TempRoot::new(prefix);
        }
        let Some(path) = std::env::var_os(ROUTER_CARRIER_ENV).map(PathBuf::from) else {
            return TempRoot::new(prefix);
        };
        assert!(path.is_absolute(), "{ROUTER_CARRIER_ENV} must be absolute");
        assert!(
            !path.starts_with(repository) && !repository.starts_with(&path),
            "{ROUTER_CARRIER_ENV} must not overlap the candidate repository"
        );
        fs::create_dir_all(&path).expect("create retained Phase 5 Router carrier");
        TempRoot::retained(path)
    }

    fn existing_carrier_refs(
        self,
        root: &TempRoot,
    ) -> Option<(PackageArtifactRef, ServiceDeploymentRef)> {
        let store = CanonicalArtifactStore::open(root.path()).ok()?;
        let pointer = store
            .read_release_pointer(PROFILE, self.package_id, self.version)
            .expect("read retained Phase 5 release pointer")?;
        let deployment = pointer.deployment;
        let deployment_artifact = store
            .read_service_deployment(&deployment)
            .expect("read retained Phase 5 deployment");
        let package = deployment_artifact.implementation.clone();
        store
            .read_package_artifact(&package)
            .expect("read retained Phase 5 package");
        Some((package, deployment))
    }
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
            ingress: binding.selector.clone(),
            identity: gateway.gateway_entry_identity.clone(),
        }
    }

    pub fn link(&self) -> Arc<DeploymentExecutionImage> {
        let store = self.store();
        let hydrated = load_deployment_bytecode_from_store(&store, &self.deployment)
            .expect("hydrate admitted carrier through production loader");
        Arc::new(
            link_deployment_execution_image(hydrated, &production_link_limits())
                .expect("construct admitted carrier through the production atomic linker"),
        )
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
            "skiff-bcvm-p5-r1-{prefix}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create carrier root");
        Self {
            path,
            cleanup: true,
        }
    }

    fn retained(path: PathBuf) -> Self {
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

fn typed_rejection(mut error: &(dyn std::error::Error + 'static)) -> Option<TypedRejection> {
    loop {
        if let Some(BytecodeEmissionError::UnsupportedPhase1Capability {
            capability,
            module_path,
            function_key,
            ..
        }) = error.downcast_ref::<BytecodeEmissionError>()
        {
            return Some(TypedRejection::Phase1Capability {
                capability: *capability,
                module_path: module_path.clone(),
                function_key: function_key.clone(),
            });
        }
        if let Some(ContractDefinitionError::UnavailableServiceCalls { unavailable }) =
            error.downcast_ref::<ContractDefinitionError>()
        {
            return Some(TypedRejection::UnavailableServiceCalls {
                unavailable: unavailable.clone(),
            });
        }
        error = error.source()?;
    }
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
