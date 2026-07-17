use serde::Deserialize;

use crate::{validate_service_assembly_artifact_path, ArtifactRelativePath};

const FIXTURE_JSON: &str =
    include_str!("../../../cross-system-fixtures/artifact-reference-validation/cases.json");
const ASSEMBLY_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OTHER_HASH: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const SERVICE_STORAGE_SEGMENT: &str = "example~com~~svc";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactReferenceFixture {
    service_id: String,
    cases: Vec<ArtifactReferenceCase>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactReferenceCase {
    name: String,
    applies_to: Vec<String>,
    validation: String,
    path: String,
    valid: bool,
}

#[test]
fn shared_artifact_relative_path_cases_are_host_independent_and_strict() {
    let fixture = fixture();
    for case in fixture
        .cases
        .iter()
        .filter(|case| case.validation == "artifactRelativePath")
    {
        assert_cross_system_case(case);
        let path = render_path(&case.path);
        assert_eq!(
            ArtifactRelativePath::parse(&path, &case.name).is_ok(),
            case.valid,
            "shared fixture case {:?} disagreed for {path:?}",
            case.name,
        );
    }
}

#[test]
fn shared_service_assembly_coordinate_cases_validate_owner_and_identity_stem() {
    let fixture = fixture();
    let assembly_identity = format!("skiff-service-assembly-v1:sha256:{ASSEMBLY_HASH}");
    for case in fixture
        .cases
        .iter()
        .filter(|case| case.validation == "serviceAssemblyCoordinate")
    {
        assert_cross_system_case(case);
        let path = render_path(&case.path);
        assert_eq!(
            validate_service_assembly_artifact_path(
                &path,
                &fixture.service_id,
                &assembly_identity,
            )
            .is_ok(),
            case.valid,
            "shared fixture case {:?} disagreed for {path:?}",
            case.name,
        );
    }
}

fn fixture() -> ArtifactReferenceFixture {
    serde_json::from_str(FIXTURE_JSON).expect("artifact reference fixture must be valid JSON")
}

fn render_path(template: &str) -> String {
    template
        .replace("{serviceStorageSegment}", SERVICE_STORAGE_SEGMENT)
        .replace("{assemblyHash}", ASSEMBLY_HASH)
        .replace("{otherHash}", OTHER_HASH)
}

fn assert_cross_system_case(case: &ArtifactReferenceCase) {
    assert!(
        case.applies_to.iter().any(|system| system == "runtime")
            && case.applies_to.iter().any(|system| system == "router"),
        "shared fixture case {:?} must apply to runtime and router",
        case.name,
    );
}
