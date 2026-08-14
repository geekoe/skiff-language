use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::Value;
use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    GatewayEntryIdentity, IngressProtocol, IngressSelector, PackageArtifact, PackageArtifactRef,
    ServiceDeployment, ServiceDeploymentRef,
};
use skiff_compiler::{
    authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::CanonicalArtifactStore;
use skiff_runtime_bytecode_verifier::VerificationLimits;
use skiff_runtime_linker::{
    link_deployment_execution_image, DeploymentExecutionImage, DeploymentExecutionLimits,
    LinkLimits,
};
use skiff_runtime_loader::load_deployment_bytecode_from_store;
use std::sync::Arc;

const PROFILE: &str = "skiff-test";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

pub enum BuildOutcome {
    Published(PublishedFixture),
    Rejected {
        error_chain: String,
        release_pointer_absent: bool,
    },
}

pub struct PublishedFixture {
    _root: TempRoot,
    pub artifact_root: PathBuf,
    pub package: PackageArtifactRef,
    pub deployment: ServiceDeploymentRef,
    pub receipt: Value,
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
        let root = TempRoot::new(prefix);
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
                let store =
                    CanonicalArtifactStore::open(root.path()).expect("open rejected carrier store");
                let release_pointer_absent = store
                    .read_release_pointer(PROFILE, self.package_id, self.version)
                    .expect("read rejected carrier release pointer")
                    .is_none();
                return BuildOutcome::Rejected {
                    error_chain: error_chain(error.as_ref()),
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
        let artifact_root = root.path().to_path_buf();
        BuildOutcome::Published(PublishedFixture {
            _root: root,
            artifact_root,
            package,
            deployment,
            receipt,
        })
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
            link_deployment_execution_image(hydrated, &production_execution_limits())
                .expect("link and verify admitted carrier through production linker"),
        )
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(prefix: &str) -> Self {
        let ordinal = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p5-r1-{prefix}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create carrier root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
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

fn production_execution_limits() -> DeploymentExecutionLimits {
    DeploymentExecutionLimits::new(
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
        },
        VerificationLimits {
            max_functions: 100_000,
            max_total_instructions: 100_000_000,
            max_instructions_per_function: 1_000_000,
            max_frame_slots_per_function: 65_536,
            max_operand_depth: 65_536,
            max_control_flow_edges_per_function: 1_000_000,
            max_exception_regions_per_function: 1_000_000,
            max_switch_targets_per_function: 65_536,
            max_statement_events_per_pc: 100_000,
            max_statement_events_per_function: 1_000_000,
            max_total_statement_events: 10_000_000,
            max_source_map_entries_per_function: 1_000_000,
            max_image_table_entries: 1_000_000,
            max_arity: 256,
            max_callback_captures_per_callback: 4_096,
            max_type_nesting_depth: 64,
            max_value_lifecycle_nodes: 1_000_000,
            max_value_lifecycle_canonical_bytes: 64 * 1024 * 1024,
            max_constant_graph_edges: 1_000_000,
        },
    )
}
