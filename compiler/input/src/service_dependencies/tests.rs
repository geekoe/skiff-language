use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::{json, Value};
use skiff_artifact_identity::{
    publication_abi_identity, runtime_program_dynamic_build_id,
    runtime_program_service_unit_identity_bytes, service_assembly_identity,
    service_build_identity_from_assembly_identity, service_unit_artifact_ref, service_unit_hash,
    ServiceAssemblyArtifactRef, ServiceUnitArtifactRef, SERVICE_BUILD_IDENTITY_PREFIX,
};
use skiff_artifact_model::{
    schema::{SERVICE_BUILD_SCHEMA_VERSION, SERVICE_VERSION_POINTER_SCHEMA_VERSION},
    ServiceUnit,
};

use super::resolve_service_dependencies;
use crate::ServiceDependency;

const SERVICE_ID: &str = "example.com/callee";
const SERVICE_VERSION: &str = "1.0.0";
const SERVICE_STORAGE_SEGMENT: &str = "example~com~~callee";

#[test]
fn canonical_dev_pointer_resolves_validated_closure_dynamic_build_id() {
    let fixture = ServiceDependencyFixture::new("canonical-dev");
    fixture.write_dev_pointer(&fixture.pointer);

    let resolved = resolve_service_dependencies(
        std::slice::from_ref(&fixture.dependency),
        &[fixture.root.path().to_path_buf()],
    )
    .expect("canonical dev service dependency should resolve");

    assert_eq!(resolved.constraints().len(), 1);
    assert_eq!(
        resolved.constraints()[0].build_id,
        fixture.expected_dynamic_build_id
    );
    assert_ne!(
        resolved.constraints()[0].build_id,
        fixture.pointer["buildId"].as_str().unwrap(),
        "compiler dependency identity must come from the validated closure, not the locator pointer"
    );
}

#[test]
fn canonical_release_pointer_resolves_the_same_validated_closure() {
    let fixture = ServiceDependencyFixture::new("canonical-release");
    fixture.write_release_pointers();

    let resolved = resolve_service_dependencies(
        std::slice::from_ref(&fixture.dependency),
        &[fixture.root.path().to_path_buf()],
    )
    .expect("canonical release service dependency should resolve");

    assert_eq!(
        resolved.constraints()[0].build_id,
        fixture.expected_dynamic_build_id
    );
}

#[test]
fn path_only_service_unit_pointer_fails_closed() {
    let fixture = ServiceDependencyFixture::new("path-only");
    let mut pointer = fixture.pointer.clone();
    pointer["serviceUnit"] = json!({
        "unitPath": fixture.service_unit_ref.unit_path,
    });
    fixture.write_dev_pointer(&pointer);

    let error = resolve_service_dependencies(
        std::slice::from_ref(&fixture.dependency),
        &[fixture.root.path().to_path_buf()],
    )
    .expect_err("path-only service unit pointers must fail closed")
    .to_string();

    assert!(error.contains("serviceUnit is invalid"), "{error}");
    assert!(error.contains("schemaVersion"), "{error}");
}

#[test]
fn snake_case_pointer_alias_fails_closed() {
    let fixture = ServiceDependencyFixture::new("snake-case-alias");
    let mut pointer = fixture.pointer.clone();
    pointer["service_id"] = Value::String(SERVICE_ID.to_string());
    fixture.write_dev_pointer(&pointer);

    let error = resolve_service_dependencies(
        std::slice::from_ref(&fixture.dependency),
        &[fixture.root.path().to_path_buf()],
    )
    .expect_err("snake_case pointer aliases must fail closed")
    .to_string();

    assert!(
        error.contains("legacy pointer field service_id is not supported"),
        "{error}"
    );
}

#[test]
fn dev_pointer_build_id_must_match_service_assembly_identity() {
    let fixture = ServiceDependencyFixture::new("dev-build-id-mismatch");
    let mut pointer = fixture.pointer.clone();
    pointer["buildId"] = Value::String(format!(
        "{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{}",
        "f".repeat(64)
    ));
    fixture.write_dev_pointer(&pointer);

    let error = resolve_service_dependencies(
        std::slice::from_ref(&fixture.dependency),
        &[fixture.root.path().to_path_buf()],
    )
    .expect_err("dev pointer buildId mismatch must fail closed")
    .to_string();

    assert!(
        error.contains("buildId must match serviceAssembly.assemblyIdentity"),
        "{error}"
    );
}

#[test]
fn legacy_service_index_is_not_a_resolution_fallback() {
    let fixture = ServiceDependencyFixture::new("legacy-index");
    fixture.write_json(
        &format!("indexes/services/{SERVICE_STORAGE_SEGMENT}/legacy.json"),
        &fixture.pointer,
    );

    let error = resolve_service_dependencies(
        std::slice::from_ref(&fixture.dependency),
        &[fixture.root.path().to_path_buf()],
    )
    .expect_err("legacy service indexes must not be resolved")
    .to_string();

    assert!(error.contains("was not found"), "{error}");
}

struct ServiceDependencyFixture {
    root: TempArtifactRoot,
    dependency: ServiceDependency,
    pointer: Value,
    service_assembly_ref: ServiceAssemblyArtifactRef,
    service_unit_ref: ServiceUnitArtifactRef,
    expected_dynamic_build_id: String,
}

impl ServiceDependencyFixture {
    fn new(label: &str) -> Self {
        let root = TempArtifactRoot::new(label);
        let mut service_unit = ServiceUnit::empty(SERVICE_ID, SERVICE_VERSION, "protocol:test");
        service_unit.publication_abi.abi_identity =
            publication_abi_identity(&service_unit.publication_abi)
                .expect("publication ABI identity");

        let service_hash = service_unit_hash(&service_unit).expect("service unit hash");
        let service_path = format!("units/services/{SERVICE_STORAGE_SEGMENT}/{service_hash}.json");
        write_json(root.path(), &service_path, &service_unit);
        let service_unit_ref = service_unit_artifact_ref(SERVICE_ID, service_path, &service_unit)
            .expect("canonical service unit reference");

        let mut assembly = json!({
            "schemaVersion": "skiff-assembly-v1",
            "kind": "service",
            "service": {
                "id": SERVICE_ID,
                "revisionId": "test-revision",
                "protocolIdentity": service_unit.protocol_identity,
                "api": { "bindings": {} },
            },
            "serviceUnit": service_unit_ref,
        });
        let assembly_identity =
            service_assembly_identity(&assembly).expect("service assembly identity");
        assembly["service"]["assemblyIdentity"] = Value::String(assembly_identity.clone());
        let assembly_hash = assembly_identity
            .rsplit(':')
            .next()
            .expect("assembly identity hash");
        let assembly_path =
            format!("assemblies/services/{SERVICE_STORAGE_SEGMENT}/{assembly_hash}.json");
        write_json(root.path(), &assembly_path, &assembly);
        let service_assembly_ref = ServiceAssemblyArtifactRef {
            assembly_identity,
            assembly_path,
        };

        let expected_dynamic_build_id = runtime_program_dynamic_build_id(
            &runtime_program_service_unit_identity_bytes(&service_unit)
                .expect("runtime service unit identity"),
            std::iter::empty(),
        );
        let pointer_build_id =
            service_build_identity_from_assembly_identity(&service_assembly_ref.assembly_identity)
                .expect("dev pointer build identity");
        let pointer = json!({
            "mode": "dev",
            "serviceId": SERVICE_ID,
            "serviceVersion": SERVICE_VERSION,
            "profile": "test",
            "buildId": pointer_build_id,
            "serviceAssembly": service_assembly_ref,
            "serviceUnit": service_unit_ref,
            "packageUnits": [],
        });

        Self {
            root,
            dependency: ServiceDependency {
                id: SERVICE_ID.to_string(),
                version: SERVICE_VERSION.to_string(),
                alias: "callee".to_string(),
            },
            pointer,
            service_assembly_ref,
            service_unit_ref,
            expected_dynamic_build_id,
        }
    }

    fn write_dev_pointer(&self, pointer: &Value) {
        self.write_json(
            &format!("dev/services/{SERVICE_STORAGE_SEGMENT}.json"),
            pointer,
        );
    }

    fn write_release_pointers(&self) {
        let release_build_id = format!("{SERVICE_BUILD_IDENTITY_PREFIX}:sha256:{}", "e".repeat(64));
        self.write_json(
            &format!("versions/services/{SERVICE_STORAGE_SEGMENT}/{SERVICE_VERSION}.json"),
            &json!({
                "schemaVersion": SERVICE_VERSION_POINTER_SCHEMA_VERSION,
                "serviceId": SERVICE_ID,
                "version": SERVICE_VERSION,
                "buildId": release_build_id,
            }),
        );
        self.write_json(
            &format!(
                "builds/services/{SERVICE_STORAGE_SEGMENT}/{}.json",
                "e".repeat(64)
            ),
            &json!({
                "schemaVersion": SERVICE_BUILD_SCHEMA_VERSION,
                "serviceId": SERVICE_ID,
                "serviceVersion": SERVICE_VERSION,
                "buildId": release_build_id,
                "serviceAssembly": self.service_assembly_ref,
                "serviceUnit": self.service_unit_ref,
                "packageUnits": [],
            }),
        );
    }

    fn write_json(&self, relative_path: &str, value: &impl Serialize) {
        write_json(self.root.path(), relative_path, value);
    }
}

fn write_json(root: &Path, relative_path: &str, value: &impl Serialize) {
    let path = root.join(relative_path);
    fs::create_dir_all(path.parent().expect("artifact path parent")).expect("artifact directory");
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("artifact JSON serialization"),
    )
    .expect("artifact write");
}

struct TempArtifactRoot {
    path: PathBuf,
}

impl TempArtifactRoot {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "skiff-compiler-service-dependency-{label}-{}-{nonce}",
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
