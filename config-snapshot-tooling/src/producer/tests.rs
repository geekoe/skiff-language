use std::{collections::BTreeMap, path::PathBuf};

use skiff_artifact_model::{DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef};
use skiff_runtime_config_snapshot::{
    RuntimeConfigSnapshotStore, RUNTIME_CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION,
};
use tempfile::tempdir;

use super::{produce_runtime_config_snapshot, ConfigSnapshotProductionInput, ServiceConfigSource};

#[test]
fn empty_deployments_produce_and_securely_publish_an_empty_snapshot() {
    let artifact_root = tempdir().unwrap();
    let receipt = produce_runtime_config_snapshot(
        ConfigSnapshotProductionInput {
            profile: "dev".to_string(),
            deployments: BTreeMap::new(),
            package_artifacts: BTreeMap::new(),
            sources: Vec::new(),
        },
        artifact_root.path(),
    )
    .unwrap();

    assert_eq!(receipt.deployment_count, 0);
    assert_eq!(receipt.package_count, 0);
    let store =
        RuntimeConfigSnapshotStore::open(artifact_root.path().join("runtime-config")).unwrap();
    let snapshot = store.read(&receipt.snapshot).unwrap();
    assert_eq!(snapshot.profile(), "dev");
    assert!(snapshot.deployments().is_empty());
    let record: serde_json::Value = serde_json::from_slice(
        &std::fs::read(artifact_root.path().join(receipt.record_path)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        record["schemaVersion"],
        RUNTIME_CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION
    );
    assert_eq!(record["deployments"], serde_json::json!([]));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        assert_eq!(
            std::fs::metadata(store.root().join("snapshots").join(format!(
                "{}.json",
                receipt.snapshot.snapshot_id.random_suffix()
            )))
            .unwrap()
            .permissions()
            .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn source_set_must_match_supplied_deployments() {
    let artifact_root = tempdir().unwrap();
    let error = produce_runtime_config_snapshot(
        ConfigSnapshotProductionInput {
            profile: "dev".to_string(),
            deployments: BTreeMap::new(),
            package_artifacts: BTreeMap::new(),
            sources: vec![ServiceConfigSource {
                deployment: deployment_ref(),
                root: PathBuf::from("/tmp/service"),
            }],
        },
        artifact_root.path(),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("exactly match supplied ServiceDeployment records"),
        "{error}"
    );
}

fn deployment_ref() -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "example.com/service".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("dev"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(
            "skiff-service-deployment-v4:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ),
    }
}
