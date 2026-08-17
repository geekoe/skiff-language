use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use skiff_artifact_model::{PackageArtifactRef, ServiceDeploymentRef};
use skiff_compiler::{
    authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::CanonicalArtifactStore;

const PROFILE: &str = "skiff-test";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

pub enum BuildOutcome {
    Published(RouterFixture),
    Rejected {
        error_chain: String,
        package_pointer_absent: bool,
        release_pointer_absent: bool,
    },
}

pub struct RouterFixture {
    _root: TempRoot,
    pub artifact_root: PathBuf,
    pub package: PackageArtifactRef,
    pub deployment: ServiceDeploymentRef,
}

impl RouterFixture {
    pub fn store(&self) -> CanonicalArtifactStore {
        CanonicalArtifactStore::open(&self.artifact_root).expect("open published carrier store")
    }
}

/// Compiles the Phase 7 whole-system unary fixture through the production
/// compiler into a real immutable artifact root (package artifact + deployment
/// record + release pointer). No hand-built deployment is used: the Router
/// resolves the exact compiler-published carrier.
pub fn build_unary_fixture(prefix: &str) -> BuildOutcome {
    let repository = repository_root();
    let root = TempRoot::new(prefix);
    let root_path = root.path().to_path_buf();
    let sources =
        CompilerPlatformSources::new(&repository).expect("open compiler platform sources");
    seed_official_std_package(&sources, root.path()).expect("seed production std package");
    let fixture =
        repository.join("runtime/host/tests/fixtures/bytecode-vm-phase-7/http-unary-echo");
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
            let store = CanonicalArtifactStore::open(root.path()).expect("open rejected store");
            let package_pointer_absent = store
                .read_package_artifact_pointer("test.skiff/bytecode-vm-phase-7-unary", "1.0.0")
                .expect("read rejected package pointer")
                .is_none();
            let release_pointer_absent = store
                .read_release_pointer(PROFILE, "test.skiff/bytecode-vm-phase-7-unary", "1.0.0")
                .expect("read rejected release pointer")
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
    BuildOutcome::Published(RouterFixture {
        _root: root,
        artifact_root: root_path,
        package,
        deployment,
    })
}

pub fn published_unary(prefix: &str) -> RouterFixture {
    match build_unary_fixture(prefix) {
        BuildOutcome::Published(fixture) => fixture,
        BuildOutcome::Rejected { error_chain, .. } => panic!(
            "production Phase 7 unary source did not reach the executable carrier: {error_chain}"
        ),
    }
}

struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    fn new(prefix: &str) -> Self {
        let ordinal = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-bcvm-p7-router-r1-{prefix}-{}-{ordinal}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create carrier root");
        Self { path }
    }

    fn path(&self) -> &Path {
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
        .expect("router is below repository root")
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
