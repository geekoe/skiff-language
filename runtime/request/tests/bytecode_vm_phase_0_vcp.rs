use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use skiff_artifact_model::ServiceDeploymentRef;
use skiff_compiler::{
    authoring::{build_authoring_object, AuthoringObject},
    CompilerPlatformSources,
};
use skiff_deployment::storage::CanonicalArtifactStore;

const FIXTURE_RELATIVE: &str =
    "doc/implementation/bytecode-vm-convergence/fixtures/vcp1-trusted-scalar";
const PACKAGE_ID: &str = "test.skiff/bytecode-vm-phase-0";
const VERSION: &str = "1.0.0";
const PROFILE: &str = "skiff-test";
const SCENARIO_ID: &str = "vcp-1-success";
const CORRELATION_ID: &str = "vcp-phase-0-request";
const RAW_SCHEMA: &str = "skiff-vcp-phase-0-raw-v1";
const HOST_COMPOSITION_BOUNDARY: &str = "runtime/host RuntimeHost::spawn_bytecode_request";

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

/// Expected-red proof of the first production composition boundary that the
/// request-crate VCP cannot cross.
///
/// Run this test with `SKIFF_VCP_PHASE0_RAW_EVENTS` set to preserve the raw
/// canonical authoring receipts. It deliberately fails after publication:
/// `skiff-runtime-host` depends on this crate, while the host-owned deployment
/// registry and request entry are crate-private. Moving the VCP to that owner
/// (and adding production observation) is required; recreating those steps in
/// this harness would be a second execution authority.
#[test]
fn phase_0_vcp_production_composition_expected_red() {
    let repo_root = repository_root();
    let fixture_root = repo_root.join(FIXTURE_RELATIVE);
    let mut artifact_root = TempArtifactRoot::create("skiff-vcp-phase-0-artifacts");
    let mut evidence = RawEvidence::from_environment();

    evidence.record(
        "proof-harness",
        "fixture.source",
        source_inventory(&fixture_root),
    );

    let platform_sources =
        CompilerPlatformSources::new(&repo_root).expect("open repository platform sources");
    let receipt = build_authoring_object(
        &platform_sources,
        AuthoringObject::Package,
        &fixture_root,
        artifact_root.path(),
        PROFILE,
        true,
    )
    .expect("canonical compiler authoring and publication must accept the VCP source");
    evidence.record(
        "skiff-compiler::authoring::build_authoring_object",
        "compiler.authoring.receipt",
        receipt.clone(),
    );

    let deployment = publication_deployment(&receipt);
    let store =
        CanonicalArtifactStore::open(artifact_root.path()).expect("open canonical artifact store");
    let release = store
        .read_release_pointer(PROFILE, PACKAGE_ID, VERSION)
        .expect("read canonical release pointer")
        .expect("canonical authoring must publish a release pointer");
    assert_eq!(release.deployment, deployment);
    evidence.record(
        "skiff-deployment::storage::CanonicalArtifactStore",
        "artifact-store.release-pointer",
        serde_json::to_value(&release).expect("serialize production release pointer"),
    );

    evidence.record(
        "proof-harness",
        "composition.blocked",
        json!({
            "boundary": HOST_COMPOSITION_BOUNDARY,
            "artifactRoot": artifact_root.path(),
            "deployment": deployment,
            "reason": "the production deployment cache/loader/route/request entry is owned by skiff-runtime-host; skiff-runtime-host already depends on skiff-runtime-request and its composition APIs are crate-private",
            "requiredOwner": "skiff-runtime-host",
        }),
    );

    let cleaned_root = artifact_root.path().to_path_buf();
    artifact_root
        .cleanup()
        .expect("remove expected-red canonical artifact store");
    evidence.record(
        "proof-harness",
        "proof-harness.cleanup",
        json!({
            "artifactRoot": cleaned_root,
            "existsAfter": cleaned_root.exists(),
            "scope": "harness-owned temporary artifact store; not production request/VM cleanup evidence",
        }),
    );

    panic!(
        "P0-V expected-red at {HOST_COMPOSITION_BOUNDARY}: relocate the VCP to the host owner; do not construct linked/verified/image/entry/target/fiber in runtime/request"
    );
}

fn publication_deployment(receipt: &Value) -> ServiceDeploymentRef {
    serde_json::from_value(
        receipt
            .get("serviceDeploymentReceipt")
            .and_then(|value| value.get("deployment"))
            .cloned()
            .expect("canonical service publication receipt must contain deployment"),
    )
    .expect("canonical publication deployment receipt must remain typed")
}

fn source_inventory(fixture_root: &Path) -> Value {
    let files = [
        "package.yml",
        "service.yml",
        "api.yml",
        "http.yml",
        "main.skiff",
    ];
    let files = files
        .into_iter()
        .map(|relative| {
            let path = fixture_root.join(relative);
            let bytes = fs::read(&path)
                .unwrap_or_else(|error| panic!("read VCP fixture {}: {error}", path.display()));
            json!({
                "path": path,
                "sha256": sha256_hex(&bytes),
                "byteLength": bytes.len(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "root": fixture_root,
        "packageId": PACKAGE_ID,
        "packageVersion": VERSION,
        "files": files,
    })
}

struct RawEvidence {
    output: Option<PathBuf>,
    ordinal: u64,
}

impl RawEvidence {
    fn from_environment() -> Self {
        let output = env::var_os("SKIFF_VCP_PHASE0_RAW_EVENTS").map(PathBuf::from);
        if let Some(path) = output.as_ref() {
            assert!(
                path.is_absolute(),
                "SKIFF_VCP_PHASE0_RAW_EVENTS must be an absolute path"
            );
        }
        if let Some(path) = output.as_ref().and_then(|path| path.parent()) {
            fs::create_dir_all(path).expect("create raw VCP evidence directory");
        }
        if let Some(path) = output.as_ref() {
            fs::write(path, []).expect("initialize raw VCP evidence stream");
        }
        Self { output, ordinal: 0 }
    }

    fn record(&mut self, source: &str, kind: &str, payload: Value) {
        let record = json!({
            "schemaVersion": RAW_SCHEMA,
            "scenarioId": SCENARIO_ID,
            "ordinal": self.ordinal,
            "source": source,
            "kind": kind,
            "correlationId": CORRELATION_ID,
            "payload": payload,
        });
        self.ordinal += 1;
        let encoded = serde_json::to_vec(&record).expect("serialize raw VCP fact");
        match &self.output {
            Some(path) => {
                let mut output = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .expect("open raw VCP evidence stream");
                output
                    .write_all(&encoded)
                    .and_then(|()| output.write_all(b"\n"))
                    .expect("append raw VCP fact");
                output.sync_data().expect("sync raw VCP fact");
            }
            None => println!(
                "SKIFF_VCP_PHASE0_RAW_EVENT={}",
                String::from_utf8(encoded).expect("raw VCP fact is UTF-8 JSON")
            ),
        }
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("runtime request crate has repository parent")
        .to_path_buf()
}

struct TempArtifactRoot {
    path: PathBuf,
    removed: bool,
}

impl TempArtifactRoot {
    fn create(prefix: &str) -> Self {
        let unique = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!(
            "{prefix}-{}-{}-{}",
            std::process::id(),
            unique,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create VCP artifact root");
        Self {
            path,
            removed: false,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn cleanup(&mut self) -> std::io::Result<()> {
        fs::remove_dir_all(&self.path)?;
        self.removed = true;
        Ok(())
    }
}

impl Drop for TempArtifactRoot {
    fn drop(&mut self) {
        if !self.removed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
