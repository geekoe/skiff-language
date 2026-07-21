use serde_json::Value;

use super::*;

#[path = "../../../cross-system-fixtures/package-service-ecosystem/activation_raw_corpus.rs"]
mod activation_raw_corpus;

#[test]
fn cross_language_golden_request_and_control_wire_decode_strictly() {
    let request_fixture = include_str!(
        "../../../cross-system-fixtures/package-service-ecosystem/activation-request.json"
    );
    let request: AssemblyActivationRequest =
        serde_json::from_str(request_fixture).expect("canonical activation request fixture");
    assert_eq!(request.expected_generation, 41);
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        serde_json::from_str::<Value>(request_fixture).unwrap()
    );

    let control_fixture =
        include_str!("../../../cross-system-fixtures/package-service-ecosystem/control-wire.json");
    let controls: Vec<AssemblyActivationControl> =
        serde_json::from_str(control_fixture).expect("canonical control fixture");
    assert_eq!(controls.len(), 6);
    assert_eq!(
        serde_json::to_value(controls).unwrap(),
        serde_json::from_str::<Value>(control_fixture).unwrap()
    );
}

#[test]
fn shared_raw_assembly_activation_request_and_control_corpus_has_exact_outcomes() {
    let cases = activation_raw_corpus::activation_raw_cases();

    let mut checked = 0;
    for case in cases
        .iter()
        .filter(|case| case.target == "request" || case.target == "control")
    {
        let bytes = case.bytes();
        let accepted = match case.target.as_str() {
            "request" => serde_json::from_slice::<AssemblyActivationRequest>(&bytes).is_ok(),
            "control" => serde_json::from_slice::<AssemblyActivationControl>(&bytes).is_ok(),
            _ => unreachable!(),
        };
        assert_eq!(
            accepted,
            case.outcome == "accept",
            "shared raw case {}",
            case.name
        );
        checked += 1;
    }
    assert!(
        checked >= 40,
        "request/control raw corpus must stay exhaustive"
    );
}

#[test]
fn assembly_activation_typed_leafs_reject_non_ascii_and_expected_max() {
    assert!(validate_activation_token("visible!~", "token").is_ok());
    assert!(validate_activation_token("not visible", "token").is_err());
    assert!(validate_activation_token("caf\u{e9}", "token").is_err());
    assert!(validate_activation_environment("prod.us-1",).is_ok());
    assert!(validate_activation_environment(".").is_err());
    assert!(validate_expected_activation_generation(
        crate::MAX_EXPECTED_ACTIVATION_GENERATION,
        "expectedGeneration"
    )
    .is_ok());
    assert!(validate_expected_activation_generation(
        crate::MAX_SAFE_ACTIVATION_GENERATION,
        "expectedGeneration"
    )
    .is_err());
}
