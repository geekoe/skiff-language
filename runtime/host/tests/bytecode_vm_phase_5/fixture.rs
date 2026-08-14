use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::Value;
use skiff_artifact_model::{PackageArtifactRef, ServiceDeploymentRef};
use skiff_compiler::{
    authoring::{build_authoring_object, seed_official_std_package, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::CanonicalArtifactStore;

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
