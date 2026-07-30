use std::path::Path;

use super::*;

#[path = "../../../../cross-system-fixtures/package-service-ecosystem/activation_raw_corpus.rs"]
mod activation_raw_corpus;

#[test]
fn activation_state_golden_decodes_strictly() {
    let state_fixture = include_str!(
        "../../../../cross-system-fixtures/package-service-ecosystem/activation-state.json"
    );
    let state: EnvironmentActivationState =
        serde_json::from_str(state_fixture).expect("canonical activation state fixture");
    assert_eq!(state.committed.generation, 41);
    assert_eq!(state.pending.as_ref().unwrap().candidate_generation, 42);
    assert_eq!(
        serde_json::to_value(&state).unwrap(),
        serde_json::from_str::<serde_json::Value>(state_fixture).unwrap()
    );
}

#[test]
fn shared_raw_activation_state_corpus_enters_production_parser() {
    let cases = activation_raw_corpus::activation_raw_cases();
    let path = Path::new("activation.json");
    let mut checked = 0;
    for case in cases.iter().filter(|case| case.target == "state") {
        let accepted = parse_state(path, &case.bytes()).is_ok();
        assert_eq!(
            accepted,
            case.outcome == "accept",
            "shared raw case {}",
            case.name
        );
        checked += 1;
    }
    assert!(checked >= 10, "state raw corpus must stay exhaustive");
}
