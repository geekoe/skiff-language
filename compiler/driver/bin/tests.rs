use serde_json::json;

use super::{render_authoring_receipt, run_with_args, USAGE};

#[test]
fn internal_actions_are_absent_from_public_help() {
    assert!(!USAGE.contains("platform-source"));
    assert!(!USAGE.contains("std-seed"));
    for object in ["package", "release"] {
        assert!(USAGE.contains(object));
    }
    assert!(!USAGE.contains("assembly"));
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
fn package_authoring_accepts_legacy_no_bytecode_flag() {
    let error = run_error(&[
        "package",
        "build",
        "/missing-authoring-root",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--platform-source-root",
        "/missing-skiff-platform-root",
        "--no-bytecode",
    ]);
    assert!(error.contains("compiler platform source"), "{error}");
}

#[test]
fn assembly_authoring_object_is_rejected() {
    let error = run_error(&[
        "assembly",
        "build",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--profile",
        "dev",
    ]);
    assert!(error.contains("unknown authoring object assembly; expected package"));
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

#[test]
fn release_actions_validate_required_options_and_reject_unknown_options() {
    let missing_artifact_root =
        run_error(&["release", "set", "--profile", "dev", "--deployment", "{}"]);
    assert_eq!(missing_artifact_root, "--artifact-root is required");

    let missing_profile = run_error(&[
        "release",
        "get",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--service",
        "example.echo",
        "--version",
        "1.0.0",
    ]);
    assert_eq!(missing_profile, "--profile is required");

    let missing_deployment = run_error(&[
        "release",
        "set",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--profile",
        "dev",
    ]);
    assert_eq!(
        missing_deployment,
        "release set requires --deployment '<exact ServiceDeploymentRef JSON>'"
    );

    let missing_service = run_error(&[
        "release",
        "unset",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--profile",
        "dev",
        "--version",
        "1.0.0",
    ]);
    assert_eq!(missing_service, "release unset requires --service <id>");

    let missing_version = run_error(&[
        "release",
        "get",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--profile",
        "dev",
        "--service",
        "example.echo",
    ]);
    assert_eq!(missing_version, "release get requires --version <v>");

    let unknown = run_error(&[
        "release",
        "set",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--profile",
        "dev",
        "--deployment",
        "{}",
        "--revision",
        "revision-1",
    ]);
    assert!(
        unknown.contains("unknown release set option --revision"),
        "{unknown}"
    );
}

#[test]
fn release_actions_reject_malformed_and_mismatched_expected_pointers() {
    const DEPLOYMENT: &str = "{\"serviceId\":\"example.echo\",\"contractVersion\":\"1.0.0\",\"deploymentRevision\":\"revision-1\",\"deploymentArtifactIdentity\":\"skiff-deployment-artifact-v4:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"}";
    let malformed = run_error(&[
        "release",
        "set",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--profile",
        "dev",
        "--deployment",
        DEPLOYMENT,
        "--expected",
        "not-json",
    ]);
    assert!(
        malformed.contains("--expected requires exact ReleasePointer JSON"),
        "{malformed}"
    );

    let mismatched = run_error(&[
        "release",
        "unset",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--profile",
        "dev",
        "--service",
        "example.echo",
        "--version",
        "1.0.0",
        "--expected",
        "{\"schemaVersion\":\"skiff-release-pointer-v1\",\"profile\":\"prod\",\"deployment\":{\"serviceId\":\"example.echo\",\"contractVersion\":\"1.0.0\",\"deploymentRevision\":\"revision-1\",\"deploymentArtifactIdentity\":\"skiff-deployment-artifact-v4:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\"},\"recordPath\":\"records/service-deployments/example~decho/1.0.0/revision-1/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.json\"}",
    ]);
    assert!(
        mismatched.contains("must target the same profile, service, and version"),
        "{mismatched}"
    );
}

#[test]
fn release_actions_reject_unknown_actions_and_duplicate_options() {
    let unknown_action = run_error(&["release", "swap", "--artifact-root", "/tmp/skiff-artifacts"]);
    assert_eq!(
        unknown_action,
        "unknown release action swap; expected set, unset, or get"
    );

    let duplicate = run_error(&[
        "release",
        "set",
        "--artifact-root",
        "/tmp/skiff-artifacts",
        "--artifact-root",
        "/tmp/other-artifacts",
        "--profile",
        "dev",
        "--deployment",
        "{}",
    ]);
    assert_eq!(duplicate, "--artifact-root was provided more than once");
}

fn run_error(args: &[&str]) -> String {
    run_with_args(args.iter().map(|argument| (*argument).to_owned()))
        .unwrap_err()
        .to_string()
}
