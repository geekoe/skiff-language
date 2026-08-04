use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{symlink, PermissionsExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    new_runtime_config_snapshot_ref, RuntimeConfigDeployment, RuntimeConfigPackage,
    RuntimeConfigSnapshot, RuntimeConfigSnapshotError, RuntimeConfigSnapshotResolver,
    RuntimeConfigSnapshotStore, RUNTIME_CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION,
};
use serde_json::{json, Value};
use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentRevision, PackageBuildId, RuntimeConfigSnapshotId,
    RuntimeConfigSnapshotRef, ServiceDeploymentRef,
};

static TEST_ROOT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let sequence = TEST_ROOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "skiff-runtime-config-snapshot-test-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn deployment(service_id: &str) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("dev"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "skiff-deployment-artifact-v4:sha256:{}",
            if service_id == "a" {
                "a".repeat(64)
            } else {
                "b".repeat(64)
            }
        )),
    }
}

fn package(build: &str, config: Value) -> RuntimeConfigPackage {
    let Value::Object(config) = config else {
        panic!("test config must be object");
    };
    RuntimeConfigPackage::new(
        PackageBuildId::new(build),
        config.into_iter().collect::<BTreeMap<_, _>>(),
    )
    .unwrap()
}

fn snapshot() -> RuntimeConfigSnapshot {
    RuntimeConfigSnapshot::new(
        "dev",
        fixed_ref(),
        vec![
            RuntimeConfigDeployment::new(
                deployment("a"),
                vec![
                    package(
                        "build-a",
                        json!({"apiKey": "secret", "nested": {"z": 2, "a": 1}}),
                    ),
                    package("build-b", json!({"enabled": true})),
                ],
            )
            .unwrap(),
            RuntimeConfigDeployment::new(
                deployment("b"),
                vec![package("build-c", json!({"limit": 42}))],
            )
            .unwrap(),
        ],
    )
    .unwrap()
}

fn fixed_ref() -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:0123456789abcdef0123456789abcdef",
        )
        .unwrap(),
    }
}

#[test]
fn random_snapshot_refs_are_opaque_and_distinct() {
    let first = new_runtime_config_snapshot_ref();
    let second = new_runtime_config_snapshot_ref();
    assert_ne!(first, second);
    assert!(first
        .snapshot_id
        .as_str()
        .starts_with("skiff-runtime-config-snapshot-v1:"));
    assert!(!first.snapshot_id.as_str().contains("sha256"));
}

#[test]
fn record_wire_is_strict_sorted_bounded_and_contains_no_assembly_ref() {
    let record = snapshot();
    let value = serde_json::to_value(&record).unwrap();
    assert_eq!(
        value["schemaVersion"],
        RUNTIME_CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION
    );
    assert_eq!(value["profile"], "dev");
    assert_eq!(record.profile(), "dev");
    assert_eq!(
        record.deployments()[0].packages()[0].config_value(),
        json!({"apiKey": "secret", "nested": {"a": 1, "z": 2}})
    );
    assert!(value.get("assembly").is_none());
    assert!(value.get("assemblyRef").is_none());
    assert!(value.get("assemblyIdentity").is_none());
    assert_eq!(
        format!("{record:?}"),
        "RuntimeConfigSnapshot { snapshot_id: RuntimeConfigSnapshotId(\"skiff-runtime-config-snapshot-v1:0123456789abcdef0123456789abcdef\"), deployment_count: 2, package_count: 3 }"
    );
    assert!(!format!("{record:?}").contains("secret"));

    let mut unknown = value.clone();
    unknown["plaintext"] = json!("must reject");
    assert!(serde_json::from_value::<RuntimeConfigSnapshot>(unknown).is_err());

    let mut missing_profile = value.clone();
    missing_profile.as_object_mut().unwrap().remove("profile");
    assert!(serde_json::from_value::<RuntimeConfigSnapshot>(missing_profile).is_err());

    let mut legacy_schema = value.clone();
    legacy_schema["schemaVersion"] = json!("skiff-runtime-config-snapshot-record-v1");
    assert!(serde_json::from_value::<RuntimeConfigSnapshot>(legacy_schema).is_err());

    let mut invalid_profile = value.clone();
    invalid_profile["profile"] = json!("..");
    assert!(serde_json::from_value::<RuntimeConfigSnapshot>(invalid_profile).is_err());
    assert!(RuntimeConfigSnapshot::new("..", fixed_ref(), Vec::new()).is_err());

    let mut reversed = value.clone();
    reversed["deployments"].as_array_mut().unwrap().reverse();
    assert!(serde_json::from_value::<RuntimeConfigSnapshot>(reversed).is_err());

    let mut duplicate = value.clone();
    let first = duplicate["deployments"][0].clone();
    duplicate["deployments"].as_array_mut().unwrap().push(first);
    assert!(serde_json::from_value::<RuntimeConfigSnapshot>(duplicate).is_err());

    let mut package_duplicate = value;
    let first = package_duplicate["deployments"][0]["packages"][0].clone();
    package_duplicate["deployments"][0]["packages"]
        .as_array_mut()
        .unwrap()
        .push(first);
    assert!(serde_json::from_value::<RuntimeConfigSnapshot>(package_duplicate).is_err());

    let duplicate_nested_key = br#"{
        "schemaVersion":"skiff-runtime-config-snapshot-record-v3",
        "profile":"dev",
        "snapshot":{"snapshotId":"skiff-runtime-config-snapshot-v1:0123456789abcdef0123456789abcdef"},
        "deployments":[{
            "deployment":{
                "serviceId":"a",
                "contractVersion":"1.0.0",
                "deploymentRevision":"dev",
                "deploymentArtifactIdentity":"skiff-deployment-artifact-v4:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            },
            "packages":[{
                "packageBuildId":"build-a",
                "config":{"nested":{"same":1,"same":2}}
            }]
        }]
    }"#;
    assert!(serde_json::from_slice::<RuntimeConfigSnapshot>(duplicate_nested_key).is_err());
}

#[test]
fn record_rejects_depth_and_collection_limits() {
    let mut nested = json!("leaf");
    for _ in 0..crate::MAX_CONFIG_DEPTH {
        nested = json!({ "next": nested });
    }
    assert!(RuntimeConfigPackage::new(
        PackageBuildId::new("build-too-deep"),
        [("root".to_string(), nested)].into_iter().collect(),
    )
    .is_err());

    let packages = (0..=crate::MAX_PACKAGES_PER_DEPLOYMENT)
        .map(|index| {
            RuntimeConfigPackage::new(
                PackageBuildId::new(format!("build-{index:05}")),
                BTreeMap::new(),
            )
            .unwrap()
        })
        .collect();
    assert!(RuntimeConfigDeployment::new(deployment("a"), packages).is_err());

    let deployments = (0..=crate::MAX_DEPLOYMENTS_PER_SNAPSHOT)
        .map(|index| {
            RuntimeConfigDeployment::new(
                ServiceDeploymentRef {
                    service_id: format!("service-{index:05}"),
                    ..deployment("a")
                },
                Vec::new(),
            )
            .unwrap()
        })
        .collect();
    assert!(RuntimeConfigSnapshot::new("dev", fixed_ref(), deployments).is_err());
}

#[test]
fn secure_store_publishes_once_reads_strictly_and_resolves() {
    let temp = TestRoot::new();
    let root = temp.path().join("config-store");
    let store = RuntimeConfigSnapshotStore::create(&root).unwrap();
    assert_eq!(
        fs::metadata(&root).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(root.join("snapshots"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let record = snapshot();
    let path = store.publish(&record).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(store.read(record.snapshot_ref()).unwrap(), record);
    assert_eq!(store.resolve(record.snapshot_ref()).unwrap(), record);
    assert!(matches!(
        store.publish(&record),
        Err(RuntimeConfigSnapshotError::AlreadyExists { .. })
    ));

    let production = RuntimeConfigSnapshot::new(
        "prod",
        new_runtime_config_snapshot_ref(),
        record.deployments().to_vec(),
    )
    .unwrap();
    store.publish(&production).unwrap();
    assert_eq!(
        store.read(production.snapshot_ref()).unwrap().profile(),
        "prod"
    );
    assert!(!production
        .snapshot_ref()
        .snapshot_id
        .as_str()
        .contains("prod"));
}

#[test]
fn secure_store_rejects_symlinks_nonregular_files_permissions_and_duplicate_json_keys() {
    let temp = TestRoot::new();

    let real = temp.path().join("real");
    fs::create_dir(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();
    let linked = temp.path().join("linked");
    symlink(&real, &linked).unwrap();
    assert!(RuntimeConfigSnapshotStore::open(&linked).is_err());

    let root = temp.path().join("store");
    let store = RuntimeConfigSnapshotStore::create(&root).unwrap();
    let path = store.publish(&snapshot()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.read(&fixed_ref()).is_err());

    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(store.read(&fixed_ref()).is_err());

    fs::remove_dir(&path).unwrap();
    let target = temp.path().join("target");
    fs::write(&target, b"{}").unwrap();
    symlink(&target, &path).unwrap();
    assert!(store.read(&fixed_ref()).is_err());

    fs::remove_file(&path).unwrap();
    let duplicate = br#"{"schemaVersion":"skiff-runtime-config-snapshot-record-v3","profile":"dev","snapshot":{"snapshotId":"skiff-runtime-config-snapshot-v1:0123456789abcdef0123456789abcdef"},"deployments":[{"deployment":{"serviceId":"a","contractVersion":"1.0.0","deploymentRevision":"dev","deploymentArtifactIdentity":"skiff-deployment-artifact-v4:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"packages":[{"packageBuildId":"build-a","config":{"same":1,"same":2}}]}]}"#;
    fs::write(&path, duplicate).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(store.read(&fixed_ref()).is_err());
}

#[test]
fn secure_store_rejects_snapshot_id_path_mismatch_and_missing_records() {
    let temp = TestRoot::new();
    let store = RuntimeConfigSnapshotStore::create(temp.path().join("store")).unwrap();
    assert!(matches!(
        store.read(&fixed_ref()),
        Err(RuntimeConfigSnapshotError::NotFound { .. })
    ));

    let record = snapshot();
    let original = store.publish(&record).unwrap();
    let other_ref = RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:ffffffffffffffffffffffffffffffff",
        )
        .unwrap(),
    };
    let other_path =
        original.with_file_name(format!("{}.json", other_ref.snapshot_id.random_suffix()));
    fs::copy(&original, &other_path).unwrap();
    fs::set_permissions(&other_path, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(store.read(&other_ref).is_err());
}
