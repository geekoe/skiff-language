use skiff_artifact_model::{DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef};

use super::*;

#[tokio::test]
async fn self_ingress_injects_exact_selector_and_releases_sequential_slot() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "case-a",
            "root-a".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let context = registry.self_ingress_for_request("root-a").unwrap();
    assert!(registry
        .begin_nested_http("activation-a", "before-active".to_string())
        .unwrap()
        .is_none());
    let first = context
        .prepare(&json!({
            "method": "POST",
            "url": "http://127.0.0.1:44100/entry",
            "headers": [{"name": "content-type", "value": "application/json"}],
        }))
        .unwrap()
        .unwrap();
    let child = registry
        .begin_nested_http("activation-a", "child-a".to_string())
        .unwrap()
        .unwrap();
    assert!(root.same_case_as(&child));
    assert!(registry.self_ingress_for_request("child-a").is_some());
    let concurrent_error = context
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/other",
        }))
        .err()
        .expect("second active self-ingress must fail");
    assert!(concurrent_error
        .to_string()
        .contains("already has an active self-ingress"));
    let headers = first.input().get("headers").unwrap().as_array().unwrap();
    assert!(headers
        .iter()
        .any(|header| { header == &json!({"name": "x-skiff-service", "value": "test.service"}) }));
    assert!(headers
        .iter()
        .any(|header| { header == &json!({"name": "x-skiff-version", "value": "1.0.0"}) }));
    drop(first);
    drop(child);
    let second = context
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/other",
        }))
        .unwrap()
        .unwrap();
    drop(second);
    root.finalize().await.unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[test]
fn non_self_origin_is_not_claimed_and_reserved_headers_fail_case_insensitively() {
    let registry = TestHttpEntryRegistry::default();
    let _root = registry
        .begin_root_case(
            "case-a",
            "root-a".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let context = registry.self_ingress_for_request("root-a").unwrap();
    assert!(context
        .prepare(&json!({
            "method": "GET",
            "url": "https://example.test/entry",
        }))
        .unwrap()
        .is_none());

    for name in [
        "X-SKIFF-SERVICE",
        " x-skiff-service ",
        "x-skiff-Version",
        "Host",
        "Content-Length",
        "Transfer-Encoding",
        "Connection",
        "Keep-Alive",
        "TE",
        "Trailer",
        "Upgrade",
    ] {
        let error = context
            .prepare(&json!({
                "method": "GET",
                "url": "http://127.0.0.1:44100/entry",
                "headers": [{"name": name, "value": "owned"}],
            }))
            .err()
            .expect("reserved self-ingress header must fail");
        assert!(error.to_string().contains("runtime-owned header"));
    }
}

#[test]
fn exact_capability_rejects_duplicate_but_allows_parallel_activation_cases() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "case-a",
            "root-a".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let duplicate = registry
        .begin_root_case(
            "case-a",
            "root-b".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44101/test-case",
            deployment(),
        )
        .err()
        .expect("duplicate parent must fail");
    assert!(duplicate.to_string().contains("already registered"));
    let parallel = registry
        .begin_root_case(
            "case-b",
            "root-b".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44101/test-case",
            deployment(),
        )
        .unwrap();
    assert!(registry
        .self_ingress_for_request("root-a")
        .unwrap()
        .matches(&json!({"url": "http://127.0.0.1:44100/entry"})));
    assert!(registry
        .self_ingress_for_request("root-b")
        .unwrap()
        .matches(&json!({"url": "http://127.0.0.1:44101/entry"})));
    drop(root);
    drop(parallel);
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn opaque_capability_isolates_concurrent_cases_on_one_activation() {
    let registry = TestHttpEntryRegistry::default();
    let root_a = registry
        .begin_root_case(
            "case-a",
            "root-a".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let root_b = registry
        .begin_root_case(
            "case-b",
            "root-b".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let child_a = registry
        .begin_derived("case-a", "child-a".to_string())
        .unwrap();
    let child_b = registry
        .begin_derived("case-b", "child-b".to_string())
        .unwrap();

    assert!(root_a.same_case_as(&child_a));
    assert!(root_b.same_case_as(&child_b));
    assert!(!child_a.same_case_as(&child_b));
    assert!(registry
        .self_ingress_for_request("root-a")
        .unwrap()
        .matches(&json!({"url": "http://127.0.0.1:44100/a"})));
    assert!(registry
        .self_ingress_for_request("child-b")
        .unwrap()
        .matches(&json!({"url": "http://127.0.0.1:44100/b"})));

    drop(child_a);
    drop(child_b);
    root_a.finalize().await.unwrap();
    root_b.finalize().await.unwrap();
    assert!(!registry.test_cases.contains_capability("case-a"));
    assert!(!registry.test_cases.contains_capability("case-b"));
}

#[test]
fn concurrent_self_ingress_on_one_activation_fails_closed_instead_of_crossing_cases() {
    let registry = TestHttpEntryRegistry::default();
    let root_a = registry
        .begin_root_case(
            "case-a",
            "root-a".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let root_b = registry
        .begin_root_case(
            "case-b",
            "root-b".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let active_a = registry
        .self_ingress_for_request("root-a")
        .unwrap()
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/a",
        }))
        .unwrap()
        .unwrap();
    let active_b = registry
        .self_ingress_for_request("root-b")
        .unwrap()
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/b",
        }))
        .unwrap()
        .unwrap();

    let error = registry
        .begin_nested_http("activation-shared", "ambiguous-child".to_string())
        .err()
        .expect("ambiguous nested ingress must fail closed");
    assert!(error
        .to_string()
        .contains("multiple test cases have active self-ingress"));
    assert_eq!(registry.test_cases.owner_counts(), (2, 2));

    drop(active_a);
    drop(active_b);
    drop(root_a);
    drop(root_b);
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn root_finalization_waits_for_recursive_derived_requests() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "case-recursive",
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let first = registry
        .begin_derived("case-recursive", "child-1".to_string())
        .unwrap();
    let state = Arc::clone(&root.state);
    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    assert!(registry.test_cases.contains_capability("case-recursive"));

    let recursive = registry
        .begin_derived("case-recursive", "child-2".to_string())
        .unwrap();
    assert!(first.same_case_as(&recursive));
    drop(first);
    assert!(registry.test_cases.contains_capability("case-recursive"));
    assert!(!finalization.is_finished());
    drop(recursive);

    finalization.await.unwrap().unwrap();
    assert_eq!(state.finalization_count.load(Ordering::Acquire), 1);
    assert!(!registry.test_cases.contains_capability("case-recursive"));
    assert!(!registry.test_cases.contains_request("child-1"));
    assert!(!registry.test_cases.contains_request("child-2"));
}

#[tokio::test]
async fn dropped_root_waiter_does_not_cancel_child_and_child_cleans_up() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "case-cancelled-root",
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let child = registry
        .begin_derived("case-cancelled-root", "child".to_string())
        .unwrap();
    let state = Arc::clone(&root.state);
    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    finalization.abort();
    let _ = finalization.await;

    assert!(registry
        .test_cases
        .contains_capability("case-cancelled-root"));
    assert!(registry.test_cases.contains_request("child"));
    drop(child);
    assert!(!registry
        .test_cases
        .contains_capability("case-cancelled-root"));
    assert!(!registry.test_cases.contains_request("child"));
    assert_eq!(state.finalization_count.load(Ordering::Acquire), 1);
}

#[tokio::test]
async fn unknown_and_active_duplicate_capabilities_fail_closed_without_leaks() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "case-once",
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    assert!(registry
        .begin_root_case(
            "case-once",
            "other-root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .err()
        .expect("duplicate capability must fail")
        .to_string()
        .contains("already registered"));
    assert!(registry
        .begin_derived("missing", "missing-child".to_string())
        .err()
        .expect("unknown capability must fail")
        .to_string()
        .contains("unknown or finalized"));
    let child = registry
        .begin_derived("case-once", "child".to_string())
        .unwrap();
    assert!(registry
        .begin_derived("case-once", "child".to_string())
        .err()
        .expect("duplicate derived request id must fail")
        .to_string()
        .contains("request id child was already registered"));
    assert!(registry
        .begin_root_case(
            "other-case",
            "child".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .err()
        .expect("request id already bound to a derived request must fail")
        .to_string()
        .contains("request id child was already registered"));

    drop(child);
    root.finalize().await.unwrap();
    assert!(registry
        .begin_derived("case-once", "late-child".to_string())
        .err()
        .expect("finalized capability must be unknown to derived requests")
        .to_string()
        .contains("unknown or finalized"));
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));

    // Router capabilities are random and not expected to be reissued.
    // The Runtime does not retain an unbounded tombstone merely to reject
    // a hypothetical root replay after the prior case is fully gone.
    registry
        .begin_root_case(
            "case-once",
            "fresh-root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap()
        .finalize()
        .await
        .unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[test]
fn root_drop_closes_case_but_keeps_derived_alive_until_terminal_drop() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "case-drop",
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let child = registry
        .begin_derived("case-drop", "child".to_string())
        .unwrap();
    let state = Arc::clone(&root.state);
    assert_eq!(root.finalization_count(), 0);
    drop(root);
    assert!(registry.test_cases.contains_capability("case-drop"));
    let recursive = registry
        .begin_derived("case-drop", "recursive".to_string())
        .unwrap();
    drop(child);
    assert!(registry.test_cases.contains_capability("case-drop"));
    drop(recursive);
    assert!(!registry.test_cases.contains_capability("case-drop"));
    assert_eq!(state.finalization_count.load(Ordering::Acquire), 1);
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[test]
fn dropping_host_registry_releases_owner_after_live_request_leases_end() {
    let registry = TestHttpEntryRegistry::default();
    let owner = registry.test_cases.owner_weak();
    let root = registry
        .begin_root_case(
            "case-host-drop",
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let child = registry
        .begin_derived("case-host-drop", "child".to_string())
        .unwrap();

    drop(registry);
    assert!(owner.upgrade().is_some());
    drop(root);
    assert!(owner.upgrade().is_some());
    drop(child);
    assert!(owner.upgrade().is_none());
}

fn deployment() -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "test.service".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("revision-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
            "skiff-deployment-artifact-v4:sha256:{}",
            "a".repeat(64)
        )),
    }
}
