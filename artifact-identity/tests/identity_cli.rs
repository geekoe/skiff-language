use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};
use skiff_artifact_identity::{
    assign_package_unit_identities, package_build_identity, package_local_abi_identity,
    package_unit_content_hash, publication_abi_identity, service_assembly_identity,
    service_unit_hash, service_unit_identity,
};
use skiff_artifact_model::{
    CallableEffectSummary, CallableMayEffects, PackageDependencyConstraint, PackageUnit,
    ServiceUnit,
};

#[test]
fn runtime_program_build_id_cli_returns_dynamic_build_id() {
    let root = TempArtifactRoot::new("cli-success");
    let service = valid_service();
    let mut request = write_service_closure(root.path(), service, Vec::new());
    request["serviceVersion"] = json!("1.0.0");

    let output = run_cli_command(
        "runtime-program-build-id",
        json!({
            "services": [request],
        }),
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert_eq!(stdout["results"][0]["key"], "svc");
    assert!(stdout["results"][0]["dynamicBuildId"]
        .as_str()
        .expect("dynamic build id")
        .starts_with("skiff-service-build-v1:sha256:"));
    assert_eq!(
        stdout["results"][0]["serviceUnit"]["value"]["service"]["id"],
        "example.com/svc"
    );
    assert_eq!(
        stdout["results"][0]["serviceAssembly"]["value"]["kind"],
        "service"
    );
}

#[test]
fn runtime_program_build_id_cli_rejects_selected_service_version_mismatch() {
    let root = TempArtifactRoot::new("cli-version-mismatch");
    let mut request = write_service_closure(root.path(), valid_service(), Vec::new());
    request["serviceVersion"] = json!("2.0.0");

    let output = run_cli_command("runtime-program-build-id", json!({ "services": [request] }));

    assert!(!output.status.success());
    let stderr: Value = serde_json::from_slice(&output.stderr).expect("stderr JSON");
    assert_eq!(stderr["error"]["code"], "schema_invalid");
    assert!(stderr["error"]["message"]
        .as_str()
        .expect("error message")
        .contains("selected service version 2.0.0"));
}

#[test]
fn runtime_program_build_id_cli_keeps_optional_service_version_wire_strict() {
    let root = TempArtifactRoot::new("cli-version-wire");
    let request = write_service_closure(root.path(), valid_service(), Vec::new());

    for (label, invalid_version) in [
        ("null", Value::Null),
        ("empty", json!("")),
        ("number", json!(1)),
    ] {
        let mut invalid = request.clone();
        invalid["serviceVersion"] = invalid_version;
        let output = run_cli_command("runtime-program-build-id", json!({ "services": [invalid] }));
        assert!(!output.status.success(), "{label} must be rejected");
        let stderr: Value = serde_json::from_slice(&output.stderr).expect("stderr JSON");
        assert_eq!(stderr["error"]["code"], "schema_invalid", "{label}");
    }

    let mut unknown = request;
    unknown["service_version"] = json!("1.0.0");
    let output = run_cli_command("runtime-program-build-id", json!({ "services": [unknown] }));
    assert!(!output.status.success(), "unknown alias must be rejected");
    let stderr: Value = serde_json::from_slice(&output.stderr).expect("stderr JSON");
    assert_eq!(stderr["error"]["code"], "schema_invalid");
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
    let package = package_unit_with_build_seed("old");
    let request = write_service_closure(root.path(), service, vec![package]);

    let output = run_cli_command(
        "runtime-program-build-id",
        json!({
            "services": [request],
        }),
    );

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("stdout JSON");
    assert_eq!(
        stdout["results"][0]["packageUnits"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn runtime_program_build_id_cli_rejects_tampered_pointer_hash() {
    let root = TempArtifactRoot::new("cli-tamper");
    let mut request = write_service_closure(root.path(), valid_service(), Vec::new());
    request["serviceUnit"]["unitHash"] = Value::String("0".repeat(64));
    let output = run_cli_command("runtime-program-build-id", json!({ "services": [request] }));
    assert!(!output.status.success());
    let stderr: Value = serde_json::from_slice(&output.stderr).expect("stderr JSON");
    assert_eq!(stderr["error"]["code"], "schema_invalid");
}

#[test]
fn runtime_program_build_id_cli_rejects_assembly_service_unit_protocol_mismatch() {
    let root = TempArtifactRoot::new("cli-protocol-mismatch");
    let mut service = valid_service();
    service.protocol_identity = "other-protocol".to_string();
    let request = write_service_closure(root.path(), service, Vec::new());

    let output = run_cli_command("runtime-program-build-id", json!({ "services": [request] }));

    assert!(!output.status.success());
    let stderr: Value = serde_json::from_slice(&output.stderr).expect("stderr JSON");
    assert_eq!(stderr["error"]["code"], "schema_invalid");
    assert!(stderr["error"]["message"]
        .as_str()
        .expect("error message")
        .contains("protocolIdentity"));
}

#[test]
fn runtime_program_build_id_cli_reports_schema_invalid_json() {
    let root = TempArtifactRoot::new("cli-schema-invalid");
    let output = run_cli_command(
        "runtime-program-build-id",
        json!({
            "services": [{
                "key": "svc",
                "artifactRoot": root.path(),
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
            .contains("missing field"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn package_unit_identities_cli_returns_build_and_abi_identities() {
    let package_unit = valid_package_unit();
    let expected_build = package_build_identity(&package_unit).expect("package build identity");
    let expected_abi =
        package_local_abi_identity(&package_unit).expect("package local ABI identity");

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
        "skiff-package-build-v2:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "skiff-package-local-abi-v2:sha256:0000000000000000000000000000000000000000000000000000000000000000",
    );
    package.publication_abi.abi_identity =
        publication_abi_identity(&package.publication_abi).expect("publication ABI identity");
    package
}

fn package_unit_with_build_seed(seed: &str) -> PackageUnit {
    let mut package = PackageUnit::empty(
        "example.com/pkg",
        "1.0.0",
        "skiff-package-build-v2:sha256:0000000000000000000000000000000000000000000000000000000000000000",
        "skiff-package-local-abi-v2:sha256:0000000000000000000000000000000000000000000000000000000000000000",
    );
    package
        .config_and_effect_metadata
        .effects
        .operations
        .insert(
            "__testBuildSeed".to_string(),
            CallableEffectSummary::Analyzed {
                effects: CallableMayEffects {
                    writes_caller_reachable: seed == "new",
                    returns_caller_alias: false,
                    throws_caller_alias: false,
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_suspend: false,
                },
            },
        );
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

fn write_service_closure(root: &Path, service: ServiceUnit, packages: Vec<PackageUnit>) -> Value {
    let service_hash = service_unit_hash(&service).expect("service hash");
    let service_identity = service_unit_identity(&service).expect("service identity");
    let service_path = format!("units/services/example~com~~svc/{service_hash}.json");
    write_json_artifact(root, &service_path, &service);
    let service_ref = json!({
        "schemaVersion": "skiff-service-unit-v1",
        "unitIdentity": service_identity,
        "unitHash": service_hash,
        "unitPath": service_path,
    });
    let mut package_refs = Vec::new();
    for package in packages {
        let value = serde_json::to_value(&package).expect("package value");
        let hash = package_unit_content_hash(&value).expect("package content hash");
        let path = format!("units/packages/example~com~~pkg/{hash}.json");
        write_json_artifact(root, &path, &package);
        package_refs.push(json!({
            "schemaVersion": "skiff-package-unit-v1",
            "packageId": package.package_id,
            "version": package.version,
            "buildIdentity": package.build_identity,
            "abiIdentity": package.abi_identity,
            "unitHash": hash,
            "unitPath": path,
        }));
    }
    let mut assembly = json!({
        "schemaVersion": "skiff-assembly-v1",
        "kind": "service",
        "service": {
            "id": "example.com/svc",
            "revisionId": "revision",
            "protocolIdentity": "protocol",
            "api": { "bindings": {} },
        },
        "files": [], "packageConfigs": {}, "preludeIdentity": "prelude", "prelude": {},
        "configShape": {}, "configUses": [], "configActivation": {}, "configRequirements": {},
        "db": [], "operations": [], "gateway": {}, "timeout": null, "dependencyLock": [],
        "serviceUnit": service_ref, "sourceMap": {},
    });
    let assembly_identity = service_assembly_identity(&assembly).expect("assembly identity");
    assembly["service"]["assemblyIdentity"] = Value::String(assembly_identity.clone());
    let assembly_hash = assembly_identity.rsplit(':').next().expect("assembly hash");
    let assembly_path = format!("assemblies/services/example~com~~svc/{assembly_hash}.json");
    write_json_artifact(root, &assembly_path, &assembly);
    json!({
        "key": "svc",
        "artifactRoot": root,
        "serviceId": "example.com/svc",
        "serviceAssembly": { "assemblyIdentity": assembly_identity, "assemblyPath": assembly_path },
        "serviceUnit": assembly["serviceUnit"].clone(),
        "packageUnits": package_refs,
    })
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
