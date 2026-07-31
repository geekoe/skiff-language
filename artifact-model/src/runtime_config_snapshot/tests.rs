use serde_json::json;

use super::*;

#[test]
fn snapshot_id_is_strict_opaque_random_lexical_identity() {
    let valid = format!("{RUNTIME_CONFIG_SNAPSHOT_ID_PREFIX}:0123456789abcdef0123456789abcdef");
    let id = RuntimeConfigSnapshotId::parse(&valid).unwrap();
    assert_eq!(id.as_str(), valid);
    assert_eq!(id.random_suffix(), "0123456789abcdef0123456789abcdef");

    for invalid in [
        "",
        "skiff-runtime-config-snapshot-v1:sha256:0123456789abcdef0123456789abcdef",
        "skiff-runtime-config-snapshot-v1:0123456789abcdef",
        "skiff-runtime-config-snapshot-v1:0123456789abcdef0123456789abcdeg",
        "skiff-runtime-config-snapshot-v1:0123456789ABCDEF0123456789ABCDEF",
        "skiff-runtime-config-snapshot-v2:0123456789abcdef0123456789abcdef",
    ] {
        assert!(
            RuntimeConfigSnapshotId::parse(invalid).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn snapshot_ref_wire_is_strict() {
    let value = json!({
        "snapshotId": "skiff-runtime-config-snapshot-v1:0123456789abcdef0123456789abcdef"
    });
    let reference: RuntimeConfigSnapshotRef = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(reference).unwrap(), value);

    let mut unknown = value;
    unknown["assemblyIdentity"] = json!("must-not-appear");
    assert!(serde_json::from_value::<RuntimeConfigSnapshotRef>(unknown).is_err());
}
