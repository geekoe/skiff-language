use std::{collections::BTreeMap, path::PathBuf};

use serde_json::{json, Value};
use skiff_artifact_model::{
    DeploymentArtifactIdentity, DeploymentRevision, PackageBuildId, PackageConfigAccess,
    PackageConfigRequirement, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
    ServiceDeploymentRef,
};

use super::{
    project_runtime_config_snapshot, project_runtime_config_snapshot_with_base,
    ConfigSnapshotDeploymentInput, ConfigSnapshotPackageInput,
};

const PROFILE: &str = "test";

#[test]
fn projection_validates_required_optional_types_unknown_packages_and_nested_conflicts() {
    let valid = input(
        "service-a",
        json!({
            "required": "yes",
            "nested": {"count": 2}
        }),
    );
    let snapshot = project_runtime_config_snapshot(PROFILE, snapshot_ref(), vec![valid]).unwrap();
    assert_eq!(snapshot.package_count(), 1);
    assert_eq!(snapshot.profile(), PROFILE);

    assert!(project_runtime_config_snapshot(
        "..",
        snapshot_ref(),
        vec![input("service-a", json!({"required": "yes"}))],
    )
    .unwrap_err()
    .to_string()
    .contains("profile"));

    let missing = input("service-a", json!({"nested": {"count": 2}}));
    assert!(
        project_runtime_config_snapshot(PROFILE, snapshot_ref(), vec![missing])
            .unwrap_err()
            .to_string()
            .contains("missing required")
    );

    let mismatch = input("service-a", json!({"required": 1, "nested": {"count": 2}}));
    assert!(
        project_runtime_config_snapshot(PROFILE, snapshot_ref(), vec![mismatch])
            .unwrap_err()
            .to_string()
            .contains("must be string")
    );

    let conflict = input(
        "service-a",
        json!({"required": "yes", "nested": "not-an-object"}),
    );
    assert!(
        project_runtime_config_snapshot(PROFILE, snapshot_ref(), vec![conflict])
            .unwrap_err()
            .to_string()
            .contains("nested below a non-object")
    );

    let mut unknown = input("service-a", json!({"required": "yes"}));
    unknown
        .config
        .insert("packages".to_string(), BTreeMap::new());
    assert!(
        project_runtime_config_snapshot(PROFILE, snapshot_ref(), vec![unknown])
            .unwrap_err()
            .to_string()
            .contains("outside the exact deployment closure")
    );
}

#[test]
fn same_build_is_isolated_across_deployments_and_duplicate_builds_are_rejected_within_one() {
    let left = input("service-a", json!({"required": "left"}));
    let right = input("service-b", json!({"required": "right"}));
    let snapshot =
        project_runtime_config_snapshot(PROFILE, snapshot_ref(), vec![right, left]).unwrap();
    let values = snapshot
        .deployments()
        .iter()
        .map(|deployment| {
            deployment.packages()[0].config()["required"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(values, vec!["left", "right"]);

    let mut duplicate = input("service-a", json!({"required": "left"}));
    duplicate.packages.push(duplicate.packages[0].clone());
    assert!(
        project_runtime_config_snapshot(PROFILE, snapshot_ref(), vec![duplicate])
            .unwrap_err()
            .to_string()
            .contains("Package build appears more than once")
    );

    let mut conflicting_builds = input("service-a", json!({"required": "left"}));
    conflicting_builds
        .packages
        .push(ConfigSnapshotPackageInput {
            package_id: "example.com/shared".to_string(),
            package_build_id: PackageBuildId::new("other-build"),
            requirements: Vec::new(),
        });
    assert!(
        project_runtime_config_snapshot(PROFILE, snapshot_ref(), vec![conflicting_builds])
            .unwrap_err()
            .to_string()
            .contains("Package ID appears more than once")
    );
}

#[test]
fn combined_projection_preserves_base_deployments_and_rejects_overlap() {
    let base_snapshot = project_runtime_config_snapshot(
        PROFILE,
        snapshot_ref(),
        vec![input("service-a", json!({"required": "base"}))],
    )
    .unwrap();
    let combined = project_runtime_config_snapshot_with_base(
        PROFILE,
        snapshot_ref(),
        Some(&base_snapshot),
        vec![input("service-b", json!({"required": "test"}))],
    )
    .unwrap();
    assert_eq!(combined.deployments().len(), 2);
    assert!(project_runtime_config_snapshot_with_base(
        PROFILE,
        snapshot_ref(),
        Some(&base_snapshot),
        vec![input("service-a", json!({"required": "overlap"}))],
    )
    .unwrap_err()
    .to_string()
    .contains("deployment appears more than once"));

    assert!(project_runtime_config_snapshot_with_base(
        "prod",
        snapshot_ref(),
        Some(&base_snapshot),
        vec![input("service-b", json!({"required": "test"}))],
    )
    .unwrap_err()
    .to_string()
    .contains("does not match target profile"));
}

fn input(service_id: &str, config: Value) -> ConfigSnapshotDeploymentInput {
    let Value::Object(config) = config else {
        panic!("config fixture must be object");
    };
    ConfigSnapshotDeploymentInput {
        deployment: deployment(service_id),
        source_path: PathBuf::from(format!("/{service_id}")),
        config: BTreeMap::from([(
            "example.com/shared".to_string(),
            config.into_iter().collect(),
        )]),
        packages: vec![ConfigSnapshotPackageInput {
            package_id: "example.com/shared".to_string(),
            package_build_id: PackageBuildId::new("build-shared"),
            requirements: vec![
                PackageConfigRequirement {
                    path: "required".to_string(),
                    access: PackageConfigAccess::Required {
                        value_type: "string".to_string(),
                    },
                },
                PackageConfigRequirement {
                    path: "nested.count".to_string(),
                    access: PackageConfigAccess::Optional {
                        value_type: "number".to_string(),
                    },
                },
            ],
        }],
    }
}

fn deployment(service_id: &str) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("dev"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "skiff-service-deployment-v2:sha256:{}",
            if service_id == "service-a" {
                "a".repeat(64)
            } else {
                "b".repeat(64)
            }
        )),
    }
}

fn snapshot_ref() -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(
            "skiff-runtime-config-snapshot-v1:0123456789abcdef0123456789abcdef",
        )
        .unwrap(),
    }
}
