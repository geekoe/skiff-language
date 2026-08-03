use skiff_artifact_model::{DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef};

use super::*;

const SESSION_A: &str = "skiff-router-session-v1:opaque:test-a";
const SESSION_B: &str = "skiff-router-session-v1:opaque:test-b";

#[tokio::test]
async fn self_ingress_injects_exact_selector_and_releases_sequential_slot() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "case-a",
            SESSION_A,
            "root-a".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let context = registry
        .self_ingress_for_request(SESSION_A, "root-a")
        .unwrap();
    assert!(registry
        .begin_nested_http("activation-a", SESSION_A, "before-active".to_string())
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
        .begin_nested_http("activation-a", SESSION_A, "child-a".to_string())
        .unwrap()
        .unwrap();
    assert!(root.same_case_as(&child));
    assert!(registry
        .self_ingress_for_request(SESSION_A, "child-a")
        .is_some());
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
    assert_eq!(
        prepared_header_value(&first, "x-skiff-test-case-capability"),
        Some("case-a")
    );
    assert_eq!(
        prepared_header_value(&first, "x-skiff-test-case-parent-request-id"),
        Some("root-a")
    );
    drop(first);
    let nested = child
        .admitted_context()
        .self_ingress()
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/nested",
        }))
        .unwrap()
        .unwrap();
    assert_eq!(
        prepared_header_value(&nested, "x-skiff-test-case-capability"),
        Some("case-a")
    );
    assert_eq!(
        prepared_header_value(&nested, "x-skiff-test-case-parent-request-id"),
        Some("child-a")
    );
    drop(nested);
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
            SESSION_A,
            "root-a".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let context = registry
        .self_ingress_for_request(SESSION_A, "root-a")
        .unwrap();
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
        "X-SKIFF-TEST-CASE-CAPABILITY",
        " x-skiff-test-case-parent-request-id ",
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
            SESSION_A,
            "root-a".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let duplicate = registry
        .begin_root_case(
            "case-a",
            SESSION_A,
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
            SESSION_A,
            "root-b".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44101/test-case",
            deployment(),
        )
        .unwrap();
    assert!(registry
        .self_ingress_for_request(SESSION_A, "root-a")
        .unwrap()
        .matches(&json!({"url": "http://127.0.0.1:44100/entry"})));
    assert!(registry
        .self_ingress_for_request(SESSION_A, "root-b")
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
            SESSION_A,
            "root-a".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let root_b = registry
        .begin_root_case(
            "case-b",
            SESSION_A,
            "root-b".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let child_a = registry
        .begin_derived("case-a", SESSION_A, "child-a".to_string())
        .unwrap();
    let child_b = registry
        .begin_derived("case-b", SESSION_A, "child-b".to_string())
        .unwrap();

    assert!(root_a.same_case_as(&child_a));
    assert!(root_b.same_case_as(&child_b));
    assert!(!child_a.same_case_as(&child_b));
    assert!(registry
        .self_ingress_for_request(SESSION_A, "root-a")
        .unwrap()
        .matches(&json!({"url": "http://127.0.0.1:44100/a"})));
    assert!(registry
        .self_ingress_for_request(SESSION_A, "child-b")
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
            SESSION_A,
            "root-a".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let root_b = registry
        .begin_root_case(
            "case-b",
            SESSION_A,
            "root-b".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let active_a = registry
        .self_ingress_for_request(SESSION_A, "root-a")
        .unwrap()
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/a",
        }))
        .unwrap()
        .unwrap();
    let active_b = registry
        .self_ingress_for_request(SESSION_A, "root-b")
        .unwrap()
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/b",
        }))
        .unwrap()
        .unwrap();

    let error = registry
        .begin_nested_http(
            "activation-shared",
            SESSION_A,
            "ambiguous-child".to_string(),
        )
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
async fn nested_http_selects_only_the_current_router_session_when_reconnects_overlap() {
    let registry = TestHttpEntryRegistry::default();
    let root_a = registry
        .begin_root_case(
            "overlap-case-a",
            SESSION_A,
            "overlap-root-a".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let root_b = registry
        .begin_root_case(
            "overlap-case-b",
            SESSION_B,
            "overlap-root-b".to_string(),
            "activation-shared".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let active_a = root_a
        .admitted_context()
        .self_ingress()
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/a",
        }))
        .unwrap()
        .unwrap();
    let active_b = root_b
        .admitted_context()
        .self_ingress()
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/b",
        }))
        .unwrap()
        .unwrap();

    let child_b = registry
        .begin_nested_http(
            "activation-shared",
            SESSION_B,
            "overlap-child-b".to_string(),
        )
        .unwrap()
        .unwrap();
    let admitted_b = child_b.admitted_context();
    assert_eq!(admitted_b.capability(), "overlap-case-b");
    assert_eq!(admitted_b.router_session().as_str(), SESSION_B);

    drop(active_a);
    drop(active_b);
    drop(child_b);
    root_a.finalize().await.unwrap();
    root_b.finalize().await.unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn root_finalization_waits_for_recursive_derived_requests() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "case-recursive",
            SESSION_A,
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let first = registry
        .begin_derived("case-recursive", SESSION_A, "child-1".to_string())
        .unwrap();
    let state = Arc::clone(&root.state);
    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    assert!(registry.test_cases.contains_capability("case-recursive"));

    let recursive = registry
        .begin_derived("case-recursive", SESSION_A, "child-2".to_string())
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
            SESSION_A,
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let child = registry
        .begin_derived("case-cancelled-root", SESSION_A, "child".to_string())
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
            SESSION_A,
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    assert!(registry
        .begin_root_case(
            "case-once",
            SESSION_A,
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
        .begin_derived("missing", SESSION_A, "missing-child".to_string())
        .err()
        .expect("unknown capability must fail")
        .to_string()
        .contains("unknown or finalized"));
    let child = registry
        .begin_derived("case-once", SESSION_A, "child".to_string())
        .unwrap();
    assert!(registry
        .begin_derived("case-once", SESSION_A, "child".to_string())
        .err()
        .expect("duplicate derived request id must fail")
        .to_string()
        .contains("request id child was already registered"));
    assert!(registry
        .begin_root_case(
            "other-case",
            SESSION_A,
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
        .begin_derived("case-once", SESSION_A, "late-child".to_string())
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
            SESSION_A,
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
            SESSION_A,
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let child = registry
        .begin_derived("case-drop", SESSION_A, "child".to_string())
        .unwrap();
    let state = Arc::clone(&root.state);
    assert_eq!(root.finalization_count(), 0);
    drop(root);
    assert!(registry.test_cases.contains_capability("case-drop"));
    let recursive = registry
        .begin_derived("case-drop", SESSION_A, "recursive".to_string())
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
            SESSION_A,
            "root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let child = registry
        .begin_derived("case-host-drop", SESSION_A, "child".to_string())
        .unwrap();

    drop(registry);
    assert!(owner.upgrade().is_some());
    drop(root);
    assert!(owner.upgrade().is_some());
    drop(child);
    assert!(owner.upgrade().is_none());
}

#[tokio::test]
async fn actor_parent_authority_is_atomic_recursive_isolated_and_late_closed() {
    let registry = TestHttpEntryRegistry::default();
    let root_a = registry
        .begin_root_case(
            "actor-case-a",
            SESSION_A,
            "root-a".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let root_b = registry
        .begin_root_case(
            "actor-case-b",
            SESSION_A,
            "root-b".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44101/test-case",
            deployment(),
        )
        .unwrap();

    let first = registry
        .begin_actor_method("actor-case-a", "root-a", SESSION_A, "actor-a-1".to_string())
        .unwrap();
    assert!(registry
        .begin_actor_method(
            "actor-case-a",
            "root-b",
            SESSION_A,
            "cross-case".to_string(),
        )
        .err()
        .expect("cross-case parent must fail")
        .to_string()
        .contains("belongs to another case"));
    assert!(registry
        .begin_actor_method(
            "actor-case-a",
            "missing-parent",
            SESSION_A,
            "missing".to_string(),
        )
        .err()
        .expect("missing parent must fail")
        .to_string()
        .contains("parent request is unknown"));
    assert!(registry
        .begin_actor_method("actor-case-a", "root-a", SESSION_A, "actor-a-1".to_string(),)
        .err()
        .expect("duplicate actor invocation must fail")
        .to_string()
        .contains("already registered"));
    assert!(registry
        .self_ingress_for_request(SESSION_A, "cross-case")
        .is_none());
    assert!(registry
        .self_ingress_for_request(SESSION_A, "missing")
        .is_none());

    let finalization = tokio::spawn(root_a.finalize());
    tokio::task::yield_now().await;
    assert!(!finalization.is_finished());
    assert!(registry
        .begin_actor_method(
            "actor-case-a",
            "root-a",
            SESSION_A,
            "late-root-child".to_string(),
        )
        .err()
        .expect("closed root cannot admit late Actor")
        .to_string()
        .contains("parent request is unknown"));

    let recursive = registry
        .begin_actor_method(
            "actor-case-a",
            "actor-a-1",
            SESSION_A,
            "actor-a-2".to_string(),
        )
        .unwrap();
    drop(first);
    assert!(!finalization.is_finished());
    drop(recursive);
    finalization.await.unwrap().unwrap();
    root_b.finalize().await.unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn exact_router_session_binds_root_derived_actor_and_self_ingress_authority() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "session-case",
            SESSION_A,
            "session-root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let root_context = root.admitted_context();
    assert_eq!(root_context.capability(), "session-case");
    assert_eq!(root_context.router_session().as_str(), SESSION_A);
    assert_eq!(root_context.request_id(), "session-root");
    assert!(root_context
        .self_ingress()
        .matches(&json!({"url": "http://127.0.0.1:44100/entry"})));
    assert!(registry
        .self_ingress_for_request(SESSION_B, "session-root")
        .is_none());
    assert!(registry
        .begin_derived("session-case", SESSION_B, "cross-session-task".to_string())
        .err()
        .expect("cross-session capability replay must fail")
        .to_string()
        .contains("another router session"));
    assert!(registry
        .begin_actor_method(
            "session-case",
            "session-root",
            SESSION_B,
            "cross-session-actor".to_string(),
        )
        .err()
        .expect("cross-session parent replay must fail")
        .to_string()
        .contains("another router session"));

    let active = root_context
        .self_ingress()
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/entry",
        }))
        .unwrap()
        .unwrap();
    assert!(registry
        .begin_nested_http("activation-a", SESSION_B, "cross-session-http".to_string(),)
        .err()
        .expect("cross-session nested HTTP must fail")
        .to_string()
        .contains("another router session"));
    drop(active);

    let first = registry
        .begin_actor_method(
            "session-case",
            "session-root",
            SESSION_A,
            "session-actor-1".to_string(),
        )
        .unwrap();
    let actor_context = first.context().admitted_context();
    assert_eq!(actor_context.capability(), "session-case");
    assert_eq!(actor_context.router_session().as_str(), SESSION_A);
    assert_eq!(actor_context.request_id(), "session-actor-1");
    let actor_http = actor_context
        .self_ingress()
        .prepare(&json!({
            "method": "GET",
            "url": "http://127.0.0.1:44100/actor",
        }))
        .unwrap()
        .unwrap();
    assert_eq!(
        prepared_header_value(&actor_http, "x-skiff-test-case-capability"),
        Some("session-case")
    );
    assert_eq!(
        prepared_header_value(&actor_http, "x-skiff-test-case-parent-request-id"),
        Some("session-actor-1")
    );
    drop(actor_http);
    let recursive = registry
        .begin_actor_method(
            "session-case",
            "session-actor-1",
            SESSION_A,
            "session-actor-2".to_string(),
        )
        .unwrap();
    drop(recursive);
    drop(first);
    assert!(registry
        .begin_actor_method(
            "session-case",
            "session-actor-1",
            SESSION_A,
            "stale-parent-child".to_string(),
        )
        .err()
        .expect("released parent must not admit a recursive child")
        .to_string()
        .contains("parent request is unknown"));
    root.finalize().await.unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn disconnect_revokes_session_authority_but_live_leases_finish_normally() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "disconnect-case",
            SESSION_A,
            "disconnect-root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let actor = registry
        .begin_actor_method(
            "disconnect-case",
            "disconnect-root",
            SESSION_A,
            "disconnect-actor".to_string(),
        )
        .unwrap();

    registry.disconnect_session(SESSION_A).unwrap();
    registry.disconnect_session(SESSION_A).unwrap();
    assert!(registry
        .self_ingress_for_request(SESSION_A, "disconnect-root")
        .is_none());
    assert!(registry
        .begin_derived(
            "disconnect-case",
            SESSION_A,
            "post-disconnect-task".to_string(),
        )
        .err()
        .expect("disconnect must close capability admission")
        .to_string()
        .contains("disconnected"));
    assert!(registry
        .begin_actor_method(
            "disconnect-case",
            "disconnect-actor",
            SESSION_A,
            "post-disconnect-actor".to_string(),
        )
        .err()
        .expect("disconnect must revoke recursive parent membership")
        .to_string()
        .contains("parent request is unknown"));
    assert!(registry
        .begin_actor_method(
            "disconnect-case",
            "disconnect-actor",
            SESSION_B,
            "post-reconnect-actor".to_string(),
        )
        .err()
        .expect("a new connection must not replay old parent authority")
        .to_string()
        .contains("parent request is unknown"));

    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    assert!(!finalization.is_finished());
    drop(actor);
    finalization.await.unwrap().unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn disconnect_without_derived_requests_waits_for_the_live_root_lease() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "disconnect-live-root",
            SESSION_A,
            "disconnect-root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let state = Arc::clone(&root.state);

    registry.disconnect_session(SESSION_A).unwrap();
    assert!(registry
        .self_ingress_for_request(SESSION_A, "disconnect-root")
        .is_none());
    assert!(registry
        .test_cases
        .contains_capability("disconnect-live-root"));
    assert_eq!(state.finalization_count.load(Ordering::Acquire), 0);

    root.finalize().await.unwrap();
    assert_eq!(state.finalization_count.load(Ordering::Acquire), 1);
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn disconnected_root_drop_cannot_remove_reused_request_id_in_a_new_session() {
    let registry = TestHttpEntryRegistry::default();
    let old_root = registry
        .begin_root_case(
            "disconnected-old-case",
            SESSION_A,
            "reused-root-id".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    registry.disconnect_session(SESSION_A).unwrap();
    let new_root = registry
        .begin_root_case(
            "connected-new-case",
            SESSION_B,
            "reused-root-id".to_string(),
            "activation-b".to_string(),
            "http://127.0.0.1:44101/test-case",
            deployment(),
        )
        .unwrap();

    drop(old_root);
    assert!(!registry
        .test_cases
        .contains_capability("disconnected-old-case"));
    assert!(registry
        .self_ingress_for_request(SESSION_A, "reused-root-id")
        .is_none());
    assert!(registry
        .self_ingress_for_request(SESSION_B, "reused-root-id")
        .is_some());

    new_root.finalize().await.unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn cancel_or_deadline_revokes_parent_authority_before_terminal_lease_drop() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "revoke-case",
            SESSION_A,
            "revoke-root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let actor = registry
        .begin_actor_method(
            "revoke-case",
            "revoke-root",
            SESSION_A,
            "revoke-actor".to_string(),
        )
        .unwrap();

    assert!(!registry.revoke_request(SESSION_B, "revoke-actor"));
    assert!(registry
        .self_ingress_for_request(SESSION_A, "revoke-actor")
        .is_some());
    assert!(registry.revoke_request(SESSION_A, "revoke-actor"));
    assert!(!registry.revoke_request(SESSION_A, "revoke-actor"));
    assert!(registry
        .self_ingress_for_request(SESSION_A, "revoke-actor")
        .is_none());
    assert!(registry
        .begin_actor_method(
            "revoke-case",
            "revoke-actor",
            SESSION_A,
            "revoke-recursive".to_string(),
        )
        .err()
        .expect("revoked parent must reject recursive admission")
        .to_string()
        .contains("parent request is unknown"));

    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    assert!(!finalization.is_finished());
    drop(actor);
    finalization.await.unwrap().unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn revoked_request_id_reuse_is_safe_when_old_lease_drops_first() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "reuse-old-first",
            SESSION_A,
            "reuse-root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let old = registry
        .begin_derived("reuse-old-first", SESSION_A, "reused-id".to_string())
        .unwrap();
    let stale_revoker = old.revoker();
    assert!(stale_revoker.revoke());
    let new = registry
        .begin_derived("reuse-old-first", SESSION_A, "reused-id".to_string())
        .unwrap();
    assert!(
        !stale_revoker.revoke(),
        "stale exact revoke must miss reuse"
    );

    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    drop(old);
    assert!(registry
        .self_ingress_for_request(SESSION_A, "reused-id")
        .is_some());
    assert!(
        !finalization.is_finished(),
        "old generation release must retain the new lifecycle member"
    );
    drop(new);
    finalization.await.unwrap().unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn revoked_request_id_reuse_is_safe_when_new_lease_drops_first() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "reuse-new-first",
            SESSION_A,
            "reuse-root".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let old = registry
        .begin_derived("reuse-new-first", SESSION_A, "reused-id".to_string())
        .unwrap();
    assert!(old.revoker().revoke());
    let new = registry
        .begin_derived("reuse-new-first", SESSION_A, "reused-id".to_string())
        .unwrap();

    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    drop(new);
    assert!(!registry.test_cases.contains_request("reused-id"));
    assert!(
        !finalization.is_finished(),
        "new generation release must not erase the old lifecycle member"
    );
    drop(old);
    finalization.await.unwrap().unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn stale_identity_cannot_revoke_reuse_in_another_case_or_session() {
    let registry = TestHttpEntryRegistry::default();
    let root_a = registry
        .begin_root_case(
            "reuse-case-a",
            SESSION_A,
            "root-a".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let root_b = registry
        .begin_root_case(
            "reuse-case-b",
            SESSION_B,
            "root-b".to_string(),
            "activation-b".to_string(),
            "http://127.0.0.1:44101/test-case",
            deployment(),
        )
        .unwrap();
    let old = registry
        .begin_derived("reuse-case-a", SESSION_A, "cross-reuse".to_string())
        .unwrap();
    let stale_revoker = old.revoker();
    assert!(stale_revoker.revoke());
    let new = registry
        .begin_derived("reuse-case-b", SESSION_B, "cross-reuse".to_string())
        .unwrap();

    assert!(!stale_revoker.revoke());
    drop(old);
    assert!(registry
        .self_ingress_for_request(SESSION_A, "cross-reuse")
        .is_none());
    assert!(registry
        .self_ingress_for_request(SESSION_B, "cross-reuse")
        .is_some());
    root_a.finalize().await.unwrap();
    let finalization_b = tokio::spawn(root_b.finalize());
    tokio::task::yield_now().await;
    assert!(!finalization_b.is_finished());
    drop(new);
    finalization_b.await.unwrap().unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn actor_execution_owner_covers_terminal_tail_and_abort() {
    let registry = TestHttpEntryRegistry::default();
    let root = registry
        .begin_root_case(
            "actor-tail",
            SESSION_A,
            "root-tail".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let execution = registry
        .begin_actor_method(
            "actor-tail",
            "root-tail",
            SESSION_A,
            "actor-tail-1".to_string(),
        )
        .unwrap();
    let eval_context = execution.context();
    drop(eval_context);
    let (release, hold) = oneshot::channel::<()>();
    let owner_task = tokio::spawn(async move {
        let _execution = execution;
        let _ = hold.await;
    });
    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    assert!(!finalization.is_finished());
    release.send(()).unwrap();
    owner_task.await.unwrap();
    finalization.await.unwrap().unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));

    let root = registry
        .begin_root_case(
            "actor-abort",
            SESSION_A,
            "root-abort".to_string(),
            "activation-a".to_string(),
            "http://127.0.0.1:44100/test-case",
            deployment(),
        )
        .unwrap();
    let execution = registry
        .begin_actor_method(
            "actor-abort",
            "root-abort",
            SESSION_A,
            "actor-abort-1".to_string(),
        )
        .unwrap();
    let owner_task = tokio::spawn(async move {
        let _execution = execution;
        std::future::pending::<()>().await;
    });
    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    assert!(!finalization.is_finished());
    owner_task.abort();
    assert!(owner_task.await.unwrap_err().is_cancelled());
    finalization.await.unwrap().unwrap();
    assert_eq!(registry.test_cases.owner_counts(), (0, 0));
}

#[tokio::test]
async fn actor_execution_owner_releases_on_success_error_deadline_and_direct_drop() {
    for (index, terminal) in ["success", "error", "deadline", "drop"]
        .into_iter()
        .enumerate()
    {
        let registry = TestHttpEntryRegistry::default();
        let capability = format!("actor-terminal-{terminal}");
        let root_id = format!("root-{terminal}");
        let child_id = format!("actor-{terminal}-{index}");
        let root = registry
            .begin_root_case(
                &capability,
                SESSION_A,
                root_id.clone(),
                "activation-a".to_string(),
                "http://127.0.0.1:44100/test-case",
                deployment(),
            )
            .unwrap();
        let execution = registry
            .begin_actor_method(&capability, &root_id, SESSION_A, child_id)
            .unwrap();
        let finalization = tokio::spawn(root.finalize());
        tokio::task::yield_now().await;
        assert!(!finalization.is_finished(), "{terminal}");
        match terminal {
            "success" => {
                tokio::spawn(async move { drop(execution) }).await.unwrap();
            }
            "error" => {
                let result = tokio::spawn(async move {
                    let _execution = execution;
                    Err::<(), _>("actor error")
                })
                .await
                .unwrap();
                assert!(result.is_err());
            }
            "deadline" => {
                let timed = tokio::time::timeout(std::time::Duration::from_millis(1), async move {
                    let _execution = execution;
                    std::future::pending::<()>().await;
                })
                .await;
                assert!(timed.is_err());
            }
            "drop" => drop(execution),
            _ => unreachable!(),
        }
        finalization.await.unwrap().unwrap();
        assert_eq!(registry.test_cases.owner_counts(), (0, 0), "{terminal}");
    }
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

fn prepared_header_value<'a>(
    prepared: &'a PreparedTestHttpSelfIngress,
    expected_name: &str,
) -> Option<&'a str> {
    prepared
        .input()
        .get("headers")?
        .as_array()?
        .iter()
        .find(|header| {
            header
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| name.eq_ignore_ascii_case(expected_name))
        })?
        .get("value")?
        .as_str()
}
