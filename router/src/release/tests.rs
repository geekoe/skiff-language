use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use skiff_artifact_identity::assign_service_deployment_identity;
use skiff_artifact_model::{
    DeploymentDiagnosticText, DeploymentRevision, PackageArtifactRef, PackageBuildId,
    PackageLocalAbiIdentity, ServiceContractRef, ServiceDeployment, ServiceDeploymentRef,
    ServiceProtocolIdentity, SERVICE_DEPLOYMENT_SCHEMA_VERSION,
};
use skiff_deployment::storage::{CanonicalArtifactStore, ReleasePointer};

use super::{ReleaseResolver, StoreReleaseResolver};

const SERVICE_ID: &str = "example.com/release-fixture";
const CONTRACT_VERSION: &str = "1.2.3";
const PROFILE: &str = "prod";

struct Fixture {
    store: CanonicalArtifactStore,
    deployment: ServiceDeployment,
    reference: ServiceDeploymentRef,
}

struct Guard(PathBuf);

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_root(name: &str) -> (PathBuf, Guard) {
    let path = std::env::temp_dir().join(format!(
        "skiff-release-resolver-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let guard = Guard(path.clone());
    (path, guard)
}

fn deployment_record() -> ServiceDeployment {
    let mut deployment = ServiceDeployment {
        schema_version: SERVICE_DEPLOYMENT_SCHEMA_VERSION.to_string(),
        contract: ServiceContractRef {
            service_id: SERVICE_ID.to_string(),
            contract_version: CONTRACT_VERSION.to_string(),
            service_protocol_identity: ServiceProtocolIdentity::new("protocol"),
        },
        deployment_revision: DeploymentRevision::new("1"),
        deployment_artifact_identity: skiff_artifact_model::DeploymentArtifactIdentity::new(
            "placeholder",
        ),
        implementation: PackageArtifactRef {
            package_id: SERVICE_ID.to_string(),
            package_version: "0.1.0".to_string(),
            package_build_id: PackageBuildId::new("build"),
            package_local_abi_identity: PackageLocalAbiIdentity::new("abi"),
        },
        operation_bindings: Vec::new(),
        package_bindings: Vec::new(),
        service_selectors: Vec::new(),
        gateway_entries: BTreeMap::new(),
        ingress: Vec::new(),
        diagnostic_text: DeploymentDiagnosticText {
            display_name: "release-fixture".to_string(),
            notes: BTreeMap::new(),
        },
    };
    assign_service_deployment_identity(&mut deployment).expect("assign deployment identity");
    deployment
}

fn fixture() -> (Fixture, Guard) {
    let (root, guard) = temp_root("fixture");
    let store = CanonicalArtifactStore::create(&root).expect("create artifact store");
    let deployment = deployment_record();
    let reference = skiff_artifact_identity::service_deployment_ref(&deployment);
    let fixture = Fixture {
        store,
        deployment,
        reference,
    };
    (fixture, guard)
}

fn write_pointer(store: &CanonicalArtifactStore, reference: &ServiceDeploymentRef) {
    let pointer = ReleasePointer::new(PROFILE, reference.clone()).expect("release pointer");
    store
        .write_release_pointer(&pointer)
        .expect("write release pointer");
}

#[test]
fn resolves_exact_deployment_reference_when_pointer_is_set() {
    let (fixture, _guard) = fixture();
    fixture
        .store
        .write_service_deployment(&fixture.deployment)
        .expect("write deployment record");
    write_pointer(&fixture.store, &fixture.reference);

    let resolver = StoreReleaseResolver::new(fixture.store.clone());
    let resolved = resolver
        .resolve(PROFILE, SERVICE_ID, CONTRACT_VERSION)
        .expect("release resolves");
    assert_eq!(resolved, Some(fixture.reference.clone()));
    assert_eq!(
        resolved
            .as_ref()
            .expect("resolved")
            .deployment_artifact_identity,
        fixture.reference.deployment_artifact_identity
    );
}

#[test]
fn unset_pointer_resolves_to_none() {
    let (fixture, _guard) = fixture();
    fixture
        .store
        .write_service_deployment(&fixture.deployment)
        .expect("write deployment record");

    let resolver = StoreReleaseResolver::new(fixture.store.clone());
    let resolved = resolver
        .resolve(PROFILE, SERVICE_ID, CONTRACT_VERSION)
        .expect("release resolves");
    assert_eq!(resolved, None);
}

#[test]
fn unknown_service_or_version_resolves_to_none() {
    let (fixture, _guard) = fixture();
    fixture
        .store
        .write_service_deployment(&fixture.deployment)
        .expect("write deployment record");

    let resolver = StoreReleaseResolver::new(fixture.store.clone());
    assert_eq!(
        resolver
            .resolve(PROFILE, "example.com/other", CONTRACT_VERSION)
            .expect("ok"),
        None
    );
    assert_eq!(
        resolver.resolve(PROFILE, SERVICE_ID, "9.9.9").expect("ok"),
        None
    );
}

#[test]
fn missing_deployment_record_fails_closed() {
    let (fixture, _guard) = fixture();
    fixture
        .store
        .write_service_deployment(&fixture.deployment)
        .expect("write deployment record");
    write_pointer(&fixture.store, &fixture.reference);
    let record_path = skiff_artifact_identity::ServiceDeploymentRecordPath::new(&fixture.reference)
        .expect("record path");
    std::fs::remove_file(
        fixture
            .store
            .root()
            .join(record_path.as_relative_path().as_path()),
    )
    .expect("remove deployment record");

    let resolver = StoreReleaseResolver::new(fixture.store.clone());
    let error = resolver
        .resolve(PROFILE, SERVICE_ID, CONTRACT_VERSION)
        .expect_err("missing record must fail closed");
    assert!(error.contains("read release pointer"), "error: {error}");
}

#[test]
fn tampered_pointer_fails_closed() {
    let (fixture, _guard) = fixture();
    fixture
        .store
        .write_service_deployment(&fixture.deployment)
        .expect("write deployment record");
    let pointer = ReleasePointer::new(PROFILE, fixture.reference.clone()).expect("pointer");
    let path = std::env::temp_dir().join(format!(
        "skiff-release-resolver-tampered-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let store = CanonicalArtifactStore::create(&path).expect("create tampered store");
    store
        .write_service_deployment(&fixture.deployment)
        .expect("write deployment record");
    store
        .write_release_pointer(&pointer)
        .expect("write release pointer");

    let pointer_root = store.root();
    let pointer_path = pointer_root.join(
        skiff_artifact_identity::ReleasePointerPath::new(PROFILE, SERVICE_ID, CONTRACT_VERSION)
            .expect("pointer path")
            .as_relative_path()
            .as_path(),
    );
    let mut bytes = std::fs::read(&pointer_path).expect("read pointer file");
    let tampered = bytes
        .windows(
            pointer
                .deployment
                .deployment_artifact_identity
                .as_str()
                .len(),
        )
        .position(|window| {
            window
                == pointer
                    .deployment
                    .deployment_artifact_identity
                    .as_str()
                    .as_bytes()
        })
        .expect("find identity bytes");
    for byte in &mut bytes[tampered..tampered + 8] {
        *byte = b'X';
    }
    std::fs::write(&pointer_path, bytes).expect("write tampered pointer");

    let resolver = StoreReleaseResolver::new(store);
    let error = resolver
        .resolve(PROFILE, SERVICE_ID, CONTRACT_VERSION)
        .expect_err("tampered pointer must fail closed");
    assert!(error.contains("read release pointer"), "error: {error}");
}

#[test]
fn pointer_overwrite_resolves_new_reference() {
    let (fixture, _guard) = fixture();
    fixture
        .store
        .write_service_deployment(&fixture.deployment)
        .expect("write deployment record");
    write_pointer(&fixture.store, &fixture.reference);

    let mut replacement = fixture.deployment.clone();
    replacement.deployment_revision = DeploymentRevision::new("2");
    assign_service_deployment_identity(&mut replacement).expect("assign replacement identity");
    let replacement_ref = skiff_artifact_identity::service_deployment_ref(&replacement);
    assert_ne!(replacement_ref, fixture.reference);
    fixture
        .store
        .write_service_deployment(&replacement)
        .expect("write replacement record");
    write_pointer(&fixture.store, &replacement_ref);

    let resolver = StoreReleaseResolver::new(fixture.store.clone());
    assert_eq!(
        resolver
            .resolve(PROFILE, SERVICE_ID, CONTRACT_VERSION)
            .expect("release resolves"),
        Some(replacement_ref)
    );
}
