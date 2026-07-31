use super::Arguments;

#[test]
fn cli_requires_target_environment_independently_of_source_profile() {
    let base = [
        "--artifact-root",
        "/tmp/artifacts",
        "--assembly-record",
        "records/runtime-assembly.json",
        "--profile",
        "dev",
        "--source",
        r#"{"deployment":{"serviceId":"example.com/service","contractVersion":"1.0.0","deploymentRevision":"dev","deploymentArtifactIdentity":"skiff-service-deployment-v2:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"root":"/tmp/service"}"#,
    ];
    assert!(Arguments::parse(base.into_iter().map(str::to_string)).is_err());

    let arguments = Arguments::parse(
        base.into_iter()
            .chain(["--environment", "staging"])
            .map(str::to_string),
    )
    .unwrap();
    assert_eq!(arguments.environment, "staging");
    assert_eq!(arguments.profile, "dev");
}

#[test]
fn cli_accepts_an_explicit_empty_service_source_set() {
    let arguments = Arguments::parse(
        [
            "--artifact-root",
            "/tmp/artifacts",
            "--assembly-record",
            "records/runtime-assembly.json",
            "--environment",
            "dev",
            "--profile",
            "dev",
        ]
        .into_iter()
        .map(str::to_string),
    )
    .unwrap();
    assert!(arguments.sources.is_empty());
}
