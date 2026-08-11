use super::Arguments;

#[test]
fn cli_accepts_one_profile_and_requires_it() {
    let base = [
        "--artifact-root",
        "/tmp/artifacts",
        "--source",
        r#"{"deployment":{"serviceId":"example.com/service","contractVersion":"1.0.0","deploymentRevision":"dev","deploymentArtifactIdentity":"skiff-service-deployment-v2:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"root":"/tmp/service"}"#,
    ];
    assert!(Arguments::parse(base.into_iter().map(str::to_string)).is_err());

    let arguments = Arguments::parse(
        base.into_iter()
            .chain(["--profile", "staging"])
            .map(str::to_string),
    )
    .unwrap();
    assert_eq!(arguments.profile, "staging");

    let duplicate = Arguments::parse(
        [
            "--artifact-root",
            "/tmp/artifacts",
            "--profile",
            "staging",
            "--profile",
            "prod",
        ]
        .into_iter()
        .map(str::to_string),
    )
    .unwrap_err();
    assert!(
        duplicate.contains("--profile was provided more than once"),
        "{duplicate}"
    );
}

#[test]
fn cli_rejects_retired_environment_flag() {
    let error = Arguments::parse(
        [
            "--artifact-root",
            "/tmp/artifacts",
            "--profile",
            "dev",
            "--environment",
            "dev",
        ]
        .into_iter()
        .map(str::to_string),
    )
    .unwrap_err();
    assert!(error.contains("unknown option --environment"), "{error}");
}

#[test]
fn cli_accepts_an_explicit_empty_service_source_set() {
    let arguments = Arguments::parse(
        [
            "--artifact-root",
            "/tmp/artifacts",
            "--profile",
            "dev",
        ]
        .into_iter()
        .map(str::to_string),
    )
    .unwrap();
    assert!(arguments.sources.is_empty());
}

#[test]
fn cli_rejects_retired_assembly_record_flag() {
    let error = Arguments::parse(
        [
            "--artifact-root",
            "/tmp/artifacts",
            "--assembly-record",
            "records/runtime-assembly.json",
            "--profile",
            "dev",
        ]
        .into_iter()
        .map(str::to_string),
    )
    .unwrap_err();
    assert!(error.contains("unknown option --assembly-record"), "{error}");
}
