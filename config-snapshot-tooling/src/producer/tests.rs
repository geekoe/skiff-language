use std::collections::BTreeMap;

use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, RuntimeAssembly, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_runtime_config_snapshot::{
    RuntimeConfigSnapshotStore, RUNTIME_CONFIG_SNAPSHOT_RECORD_SCHEMA_VERSION,
};
use tempfile::tempdir;

use super::{produce_runtime_config_snapshot, ConfigSnapshotProductionInput};

#[test]
fn empty_assembly_produces_and_securely_publishes_an_empty_snapshot() {
    let artifact_root = tempdir().unwrap();
    let assembly = RuntimeAssembly {
            schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
            assembly_identity: AssemblyIdentity::new(
                "skiff-runtime-assembly-v3:sha256:23c593adcf1df8a6b4ffc3fc13586b3023ed0bf2ba6d91b817f942dea02bf8ee",
            ),
            roots: Vec::new(),
            resolved_deployments: Vec::new(),
            resolved_contracts: Vec::new(),
            resolved_packages: Vec::new(),
            package_link_plan: CanonicalPackageLinkPlan {
                code_slots: Vec::new(),
                package_links: Vec::new(),
            },
            service_binding_templates: Vec::new(),
            activation_templates: Vec::new(),
            gateway_ingress: Vec::new(),
        };
    let receipt = produce_runtime_config_snapshot(
        ConfigSnapshotProductionInput {
            profile: "dev".to_string(),
            assembly,
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
