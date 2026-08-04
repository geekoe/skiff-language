use serde_json::json;

use super::{render_authoring_receipt, run_with_args, USAGE};

#[test]
fn internal_actions_are_absent_from_public_help() {
    assert!(!USAGE.contains("platform-source"));
    assert!(!USAGE.contains("std-seed"));
    for object in ["package", "assembly"] {
        assert!(USAGE.contains(object));
    }
}

#[test]
fn std_seed_action_requires_exactly_one_platform_source_root() {
    let missing = run_error(&["std-seed", "--artifact-root", "/tmp/skiff-artifacts"]);
    assert_eq!(missing, "--platform-source-root is required");

    let duplicate = run_error(&[
        "std-seed",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--platform-source-root",
        "/missing-platform-root-a",
        "--platform-source-root",
        "/missing-platform-root-b",
    ]);
    assert_eq!(
        duplicate,
        "--platform-source-root was provided more than once"
    );
}

#[test]
fn std_seed_action_requires_artifact_root_and_rejects_unknown_options() {
    let missing = run_error(&[
        "std-seed",
        "--platform-source-root",
        "/missing-platform-root",
    ]);
    assert_eq!(missing, "--artifact-root is required");

    let unknown = run_error(&[
        "std-seed",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--platform-source-root",
        "/missing-platform-root",
        "--profile",
        "dev",
    ]);
    assert_eq!(unknown, "unknown std-seed option --profile");
}

#[test]
fn authoring_actions_require_exactly_one_platform_source_root() {
    let missing = run_error(&[
        "package",
        "build",
        "/missing-authoring-root",
        "--artifact-root",
        "/tmp/skiff-artifacts",
    ]);
    assert_eq!(missing, "--platform-source-root is required");

    let duplicate = run_error(&[
        "package",
        "build",
        "/missing-authoring-root",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--platform-source-root",
        "/missing-platform-root-a",
        "--platform-source-root",
        "/missing-platform-root-b",
    ]);
    assert_eq!(
        duplicate,
        "--platform-source-root was provided more than once"
    );
}

#[test]
fn assembly_projection_rejects_positional_authoring_roots() {
    let error = run_error(&[
        "assembly",
        "build",
        "/legacy/assembly.yml",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--profile",
        "dev",
    ]);
    assert!(error.contains("unknown assembly option /legacy/assembly.yml"));
    assert!(!error.contains("No such file"));
}

#[test]
fn assembly_projection_requires_inline_exact_reference_json() {
    let error = run_error(&[
        "assembly",
        "build",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--profile",
        "dev",
        "--root-deployment",
        "/tmp/deployment.json",
    ]);
    assert!(error.contains("requires exact ServiceDeploymentRef JSON"));
    assert!(!error.contains("No such file"));
}

#[test]
fn authoring_actions_reject_relative_or_unreadable_platform_source_roots_first() {
    let relative = run_error(&[
        "package",
        "build",
        "/missing-authoring-root",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--platform-source-root",
        "relative/platform-root",
    ]);
    assert!(relative.contains("must be absolute"), "{relative}");

    let unreadable = run_error(&[
        "package",
        "build",
        "/missing-authoring-root",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--platform-source-root",
        "/missing-skiff-platform-root",
    ]);
    assert!(
        unreadable.contains("compiler platform source"),
        "{unreadable}"
    );
    assert!(!unreadable.contains("contract.yml"), "{unreadable}");
}

#[test]
fn human_service_api_output_never_hides_package_only_functions() {
    let rendered = render_authoring_receipt(&json!({
        "serviceApiReceipt": {
            "serviceId": "example.registry",
            "serviceProtocolIdentity": "protocol",
            "projection": {
                "functions": [
                    {
                        "publicPath": "read",
                        "callableId": "read-id",
                        "status": "available",
                        "serviceOperationId": "read-operation"
                    },
                    {
                        "publicPath": "inspect",
                        "callableId": "inspect-id",
                        "status": "available"
                    },
                    {
                        "publicPath": "unsafeWrite",
                        "callableId": "write-id",
                        "status": "unavailable",
                        "reasons": ["writesCallerReachable", "returnsCallerAlias"]
                    }
                ]
            }
        }
    }))
    .unwrap();
    assert_eq!(
            rendered,
            "Service API for example.registry\nAvailable: 1\nPackage-only: 2\n  available read\n  package-only inspect\n  package-only unsafeWrite\n    - \"writesCallerReachable\"\n    - \"returnsCallerAlias\""
        );
}

#[test]
fn human_service_api_output_is_explicit_for_zero_api() {
    let rendered = render_authoring_receipt(&json!({
        "serviceApiReceipt": {
            "serviceId": "example.empty",
            "serviceProtocolIdentity": "protocol",
            "projection": { "functions": [] }
        }
    }))
    .unwrap();
    assert_eq!(
        rendered,
        "Service API for example.empty\nAvailable: 0\nPackage-only: 0"
    );
}

fn run_error(args: &[&str]) -> String {
    run_with_args(args.iter().map(|argument| (*argument).to_owned()))
        .unwrap_err()
        .to_string()
}
