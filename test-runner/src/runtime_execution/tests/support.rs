use serde_json::Value;
use skiff_artifact_model::RuntimeConfigSnapshotRef;

pub(super) const PROFILE: &str = "package-tests";
pub(super) const ASSEMBLY_A: &str = concat!(
    "skiff-runtime-assembly-v3:sha256:",
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
);
pub(super) const ASSEMBLY_B: &str = concat!(
    "skiff-runtime-assembly-v3:sha256:",
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
);
pub(super) const SNAPSHOT_A: &str =
    "skiff-runtime-config-snapshot-v1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub(super) const SNAPSHOT_B: &str =
    "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub(super) const DEPLOYMENT_A: &str = concat!(
    "skiff-deployment-artifact-v4:sha256:",
    "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
);
pub(super) const DEPLOYMENT_B: &str = concat!(
    "skiff-deployment-artifact-v4:sha256:",
    "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
);

pub(super) fn snapshot_ref(snapshot_id: &str) -> RuntimeConfigSnapshotRef {
    serde_json::from_value(serde_json::json!({ "snapshotId": snapshot_id }))
        .expect("canonical test config snapshot ref")
}

/// Canonical router health body with the release pointer table projection.
/// `counters` and `capabilityConnections` are router-owned surfaces with no
/// test-runner contract; the decoder must tolerate their presence.
pub(super) fn health_body(profile: &str, build_ids: Vec<&str>) -> String {
    serde_json::json!({
        "ok": true,
        "activeAssembly": {
            "profile": profile,
            "releaseCount": build_ids.len(),
            "buildIds": build_ids,
        },
        "capabilityConnections": [],
        "replicas": [],
        "counters": Value::Object(Default::default()),
    })
    .to_string()
}

pub(super) fn valid_health() -> Value {
    serde_json::from_str(&health_body(PROFILE, vec![DEPLOYMENT_A, DEPLOYMENT_B])).unwrap()
}
