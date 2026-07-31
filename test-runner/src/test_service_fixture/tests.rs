use std::collections::BTreeSet;

use super::{
    test_service_case_ids, test_service_case_ids_with_digest, test_service_execution_nonce,
    PublicationId, TestServiceCaseOrigin,
};

#[test]
fn test_service_execution_nonces_are_unique_under_parallel_allocation() {
    let handles = (0..32)
        .map(|_| std::thread::spawn(test_service_execution_nonce))
        .collect::<Vec<_>>();
    let nonces = handles
        .into_iter()
        .map(|handle| handle.join().expect("nonce worker").expect("nonce"))
        .collect::<BTreeSet<_>>();

    assert_eq!(nonces.len(), 32);
}

#[test]
fn case_ids_use_the_canonical_package_digest_coordinate() {
    let [service_id] = test_service_case_ids("agine.ai/api-tests", "execution-a", 1)
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        service_id,
        "test.skiff/p-df02597bee463b7b/e-ca2a9581bcfa0549/case-0"
    );
    assert!(PublicationId::parse(&service_id).is_ok());
}

#[test]
fn repeated_test_invocations_receive_distinct_service_ids() {
    let first = test_service_case_ids("agine.ai/api-tests", "execution-a", 2).unwrap();
    let second = test_service_case_ids("agine.ai/api-tests", "execution-b", 2).unwrap();
    assert_eq!(
        first
            .iter()
            .map(|service_id| service_id.split("/case-").next().unwrap())
            .collect::<BTreeSet<_>>()
            .len(),
        1,
        "all cases in one invocation share one execution identity"
    );
    assert!(first.iter().all(|service_id| !second.contains(service_id)));
    assert_eq!(
        first[0]
            .split("/e-")
            .next()
            .expect("package service-id prefix"),
        second[0]
            .split("/e-")
            .next()
            .expect("package service-id prefix")
    );
}

#[test]
fn case_id_collision_reports_both_origins_without_digest_input() {
    let origins = [
        TestServiceCaseOrigin {
            package_id: "first.example/package",
            case_index: 7,
        },
        TestServiceCaseOrigin {
            package_id: "second.example/package",
            case_index: 7,
        },
    ];
    let digest_input = "test-only-secret-digest-input";
    let error = test_service_case_ids_with_digest(
        &origins,
        "execution",
        |_| {
            let _captured_without_diagnostic_exposure = digest_input;
            "0000000000000000".to_string()
        },
        |_| "1111111111111111".to_string(),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("first.example/package"));
    assert!(error.contains("second.example/package"));
    assert!(error.contains("case index 7"));
    assert!(!error.contains(digest_input));
}
