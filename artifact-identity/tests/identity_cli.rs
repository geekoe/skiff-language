use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};
use skiff_artifact_identity::{
    assign_package_unit_identities, package_abi_identity, package_build_identity,
    publication_abi_identity, runtime_program_dynamic_build_id_from_artifact_refs,
    runtime_program_dynamic_build_id_from_artifact_root, PackageUnitArtifactRef,
};
use skiff_artifact_model::{
    EffectMetadata, MetadataValue, PackageDependencyConstraint, PackageUnit, ServiceUnit,
};

#[test]
fn runtime_program_build_id_cli_returns_dynamic_build_id() {
    let root = TempArtifactRoot::new("cli-success");
    let service = valid_service();
    let expected = runtime_program_dynamic_build_id_from_artifact_root(root.path(), &service)
        .expect("expected dynamic build id");

    let output = run_cli_command(
        "runtime-program-build-id",
        json!({
            "artifactRoot": root.path(),
            "services": [{
                "key": "svc",
                "serviceUnit": service,
            }],
        }),
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert_eq!(stdout["results"][0]["key"], "svc");
    assert_eq!(stdout["results"][0]["dynamicBuildId"], expected);
}

#[test]
fn runtime_program_build_id_cli_uses_pinned_package_units() {
    let root = TempArtifactRoot::new("cli-pinned-package-units");
    let mut service = valid_service();
    service
        .package_dependencies
        .push(PackageDependencyConstraint {
            id: "example.com/pkg".to_string(),
            version: "1.0.0".to_string(),
            alias: "pkg".to_string(),
            config: Value::Object(Default::default()),
        });
    let old_package = package_unit_with_build_seed("old");
    let new_package = package_unit_with_build_seed("new");
    write_json_artifact(root.path(), "units/packages/pkg-old.json", &old_package);
    write_json_artifact(root.path(), "units/packages/pkg-new.json", &new_package);
    write_json_artifact(
        root.path(),
        "indexes/packages/example~com~~pkg/versions/1.0.0.json",
        &json!({
            "schemaVersion": "skiff-package-unit-index-v1",
            "packageId": "example.com/pkg",
            "version": "1.0.0",
            "packageUnit": {
                "unitPath": "units/packages/pkg-new.json"
            }
        }),
    );
    let package_ref = PackageUnitArtifactRef {
        package_id: old_package.package_id.clone(),
        version: old_package.version.clone(),
        build_identity: old_package.build_identity.clone(),
        abi_identity: old_package.abi_identity.clone(),
        unit_hash: None,
        unit_path: PathBuf::from("units/packages/pkg-old.json"),
    };
    let expected = runtime_program_dynamic_build_id_from_artifact_refs(
        root.path(),
        &service,
        std::slice::from_ref(&package_ref),
    )
    .expect("pinned dynamic build id");
    let mutable_index_build_id =
        runtime_program_dynamic_build_id_from_artifact_root(root.path(), &service)
            .expect("mutable index dynamic build id");
    assert_ne!(expected, mutable_index_build_id);

    let output = run_cli_command(
        "runtime-program-build-id",
        json!({
            "artifactRoot": root.path(),
            "services": [{
                "key": "svc",
                "serviceUnit": service,
                "packageUnits": [{
                    "packageId": package_ref.package_id,
                    "version": package_ref.version,
                    "buildIdentity": package_ref.build_identity,
                    "abiIdentity": package_ref.abi_identity,
                    "unitPath": package_ref.unit_path
                }],
            }],
        }),
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert_eq!(stdout["results"][0]["dynamicBuildId"], expected);
}

#[test]
fn runtime_program_build_id_cli_matches_cross_system_fixture() {
    let fixture = dynamic_build_id_fixture();
    let root = TempArtifactRoot::new("cli-cross-system-fixture");
    write_fixture_artifact_root(root.path(), &fixture);
    let service_unit = fixture
        .artifact_root
        .get(&fixture.service_unit_path)
        .expect("fixture service unit should exist")
        .clone();

    let output = run_cli_command(
        "runtime-program-build-id",
        json!({
            "artifactRoot": root.path(),
            "services": [{
                "key": "fixture",
                "serviceUnit": service_unit,
            }],
        }),
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert_eq!(stdout["results"][0]["key"], "fixture");
    assert_eq!(
        stdout["results"][0]["dynamicBuildId"],
        fixture.expected_dynamic_build_id
    );
}

#[test]
fn runtime_program_build_id_cli_reports_schema_invalid_json() {
    let root = TempArtifactRoot::new("cli-schema-invalid");
    let output = run_cli_command(
        "runtime-program-build-id",
        json!({
            "artifactRoot": root.path(),
            "services": [{
                "key": "svc",
                "serviceUnit": {
                    "schemaVersion": "skiff-service-unit-v1",
                    "service": { "id": "example.com/svc" },
                    "version": "1.0.0",
                    "protocolIdentity": "protocol",
                    "files": [],
                    "gateway": {},
                    "config": {},
                },
            }],
        }),
    );

    assert!(!output.status.success());
    let stderr: Value = serde_json::from_slice(&output.stderr).expect("stderr JSON");
    assert_eq!(stderr["error"]["code"], "schema_invalid");
    assert!(
        stderr["error"]["message"]
            .as_str()
            .expect("message")
            .contains("publicationAbi"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn package_unit_identities_cli_returns_build_and_abi_identities() {
    let package_unit = valid_package_unit();
    let expected_build = package_build_identity(&package_unit).expect("package build identity");
    let expected_abi = package_abi_identity(&package_unit).expect("package ABI identity");

    let output = run_cli_command(
        "package-unit-identities",
        json!({
            "packageUnit": package_unit,
        }),
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert_eq!(stdout["buildIdentity"], expected_build);
    assert_eq!(stdout["abiIdentity"], expected_abi);
}

fn run_cli_command(command: &str, input: Value) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_skiff-artifact-identity"))
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn identity CLI");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.to_string().as_bytes())
        .expect("write stdin");
    child.wait_with_output().expect("identity CLI output")
}

fn valid_service() -> ServiceUnit {
    let mut service = ServiceUnit::empty("example.com/svc", "1.0.0", "protocol");
    service.publication_abi.abi_identity =
        publication_abi_identity(&service.publication_abi).expect("publication ABI identity");
    service
}

fn valid_package_unit() -> PackageUnit {
    let mut package = PackageUnit::empty(
        "example.com/pkg",
        "1.0.0",
        "skiff-package-build-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "skiff-package-abi-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000",
    );
    package.publication_abi.abi_identity =
        publication_abi_identity(&package.publication_abi).expect("publication ABI identity");
    package
}

fn package_unit_with_build_seed(seed: &str) -> PackageUnit {
    let mut package = PackageUnit::empty(
        "example.com/pkg",
        "1.0.0",
        "skiff-package-build-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "skiff-package-abi-v1:sha256:0000000000000000000000000000000000000000000000000000000000000000",
    );
    let mut effect = EffectMetadata::default();
    effect
        .metadata
        .insert("seed".to_string(), MetadataValue::String(seed.to_string()));
    package
        .config_and_effect_metadata
        .effects
        .insert("__testBuildSeed".to_string(), effect);
    assign_package_unit_identities(&mut package).expect("package identities");
    package
}

fn write_json_artifact(root: &Path, relative_path: &str, value: &impl serde::Serialize) {
    let path = root.join(relative_path);
    fs::create_dir_all(path.parent().expect("artifact path should have parent"))
        .expect("artifact dir should be created");
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("artifact JSON should serialize"),
    )
    .expect("artifact should be written");
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DynamicBuildIdFixture {
    service_unit_path: String,
    expected_dynamic_build_id: String,
    artifact_root: BTreeMap<String, Value>,
}

fn dynamic_build_id_fixture() -> DynamicBuildIdFixture {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("artifact-identity crate should live under the skiff repository root")
        .join("cross-system-fixtures/dynamic-build-id-parity/case.json");
    let text = fs::read_to_string(&path).expect("dynamic build id fixture should be readable");
    serde_json::from_str(&text).expect("dynamic build id fixture should parse")
}

fn write_fixture_artifact_root(root: &Path, fixture: &DynamicBuildIdFixture) {
    for (relative_path, value) in &fixture.artifact_root {
        let path = root.join(relative_path);
        fs::create_dir_all(path.parent().expect("fixture path should have parent"))
            .expect("fixture directory should be created");
        fs::write(
            &path,
            serde_json::to_vec_pretty(value).expect("fixture JSON should serialize"),
        )
        .expect("fixture artifact should be written");
    }
}

struct TempArtifactRoot {
    path: PathBuf,
}

impl TempArtifactRoot {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skiff-artifact-identity-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temp artifact root");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempArtifactRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
