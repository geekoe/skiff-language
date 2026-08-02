use super::*;
use crate::capability_context::TestHttpEntryRegistry;
use skiff_artifact_model::{DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef};

#[test]
fn cancellation_requires_session_and_correlation_and_is_first_reason_wins() {
    let registry = ActorOwnerInvocationRegistry::default();
    let registration = registry
        .register("inv:1".into(), "session:1".into(), "cancel:1".into())
        .unwrap();
    let token = registration.cancellation();
    assert!(!registry.cancel_for_session(
        "inv:1",
        "wrong",
        "cancel:1",
        ActorOwnerCancellationReason::Cancelled
    ));
    assert!(!registry.cancel_for_session(
        "inv:1",
        "session:1",
        "wrong",
        ActorOwnerCancellationReason::Cancelled
    ));
    assert!(!token.is_cancelled());
    assert!(registry.cancel_for_session(
        "inv:1",
        "session:1",
        "cancel:1",
        ActorOwnerCancellationReason::DeadlineExceeded
    ));
    assert!(registry.cancel_registered(
        registration.identity(),
        ActorOwnerCancellationReason::Cancelled
    ));
    assert_eq!(
        registry.finish(registration.identity()),
        Some(ActorOwnerCancellationReason::DeadlineExceeded)
    );
}

#[test]
fn stale_finish_and_deadline_cannot_affect_reused_invocation_id() {
    let registry = ActorOwnerInvocationRegistry::default();
    let old = registry
        .register(
            "inv:reuse".into(),
            "session:shared".into(),
            "cancel:shared".into(),
        )
        .unwrap();
    assert_eq!(registry.finish(old.identity()), None);

    let current = registry
        .register(
            "inv:reuse".into(),
            "session:shared".into(),
            "cancel:shared".into(),
        )
        .unwrap();
    let current_token = current.cancellation();

    assert_eq!(registry.finish(old.identity()), None);
    assert!(!registry.cancel_registered(
        old.identity(),
        ActorOwnerCancellationReason::DeadlineExceeded
    ));
    assert!(!current_token.is_cancelled());
    assert!(registry.contains("inv:reuse"));

    assert!(registry.cancel_registered(
        current.identity(),
        ActorOwnerCancellationReason::DeadlineExceeded
    ));
    assert_eq!(
        registry.finish(current.identity()),
        Some(ActorOwnerCancellationReason::DeadlineExceeded)
    );
}

#[test]
fn stale_session_cleanup_cannot_remove_reused_invocation_id() {
    let registry = ActorOwnerInvocationRegistry::default();
    let old = registry
        .register(
            "inv:reuse".into(),
            "session:old".into(),
            "cancel:old".into(),
        )
        .unwrap();
    assert_eq!(registry.cancel_session("session:old"), 1);

    let current = registry
        .register(
            "inv:reuse".into(),
            "session:current".into(),
            "cancel:current".into(),
        )
        .unwrap();
    let current_token = current.cancellation();
    assert_eq!(registry.cancel_session("session:old"), 0);
    assert!(!current_token.is_cancelled());
    assert!(registry.contains("inv:reuse"));

    assert_eq!(registry.cancel_session("session:current"), 1);
    assert!(current_token.is_cancelled());
    assert!(!registry.contains("inv:reuse"));
    assert_eq!(registry.finish(old.identity()), None);
    assert_eq!(registry.finish(current.identity()), None);
}

#[test]
fn stale_wire_cancel_cannot_cancel_reused_invocation_id() {
    let registry = ActorOwnerInvocationRegistry::default();
    let old = registry
        .register(
            "inv:reuse".into(),
            "session:old".into(),
            "cancel:old".into(),
        )
        .unwrap();
    assert_eq!(registry.cancel_session("session:old"), 1);

    let current = registry
        .register(
            "inv:reuse".into(),
            "session:current".into(),
            "cancel:current".into(),
        )
        .unwrap();
    let current_token = current.cancellation();
    assert!(!registry.cancel_for_session(
        "inv:reuse",
        "session:old",
        "cancel:old",
        ActorOwnerCancellationReason::DeadlineExceeded,
    ));
    assert!(!registry.cancel_for_session(
        "inv:reuse",
        "session:current",
        "cancel:old",
        ActorOwnerCancellationReason::DeadlineExceeded,
    ));
    assert!(!current_token.is_cancelled());
    assert_eq!(registry.finish(old.identity()), None);
    assert_eq!(registry.finish(current.identity()), None);
}

#[tokio::test]
async fn wire_cancel_revokes_only_the_exact_test_request_admission() {
    const SESSION: &str = "skiff-router-session-v1:opaque:wire-cancel";
    let test_entries = TestHttpEntryRegistry::default();
    let root = test_entries
        .begin_root_case(
            "wire-cancel-case",
            SESSION,
            "root".to_string(),
            "activation".to_string(),
            "http://127.0.0.1:44100/test-case",
            test_deployment(),
        )
        .unwrap();
    let old_execution = test_entries
        .begin_actor_method("wire-cancel-case", "root", SESSION, "inv:reuse".to_string())
        .unwrap();
    let stale_revoker = old_execution.revoker();
    let registry = ActorOwnerInvocationRegistry::default();
    let old_registration = registry
        .register_with_test_revoker(
            "inv:reuse".into(),
            SESSION.into(),
            "cancel:old".into(),
            Some(stale_revoker.clone()),
        )
        .unwrap();

    assert!(registry.cancel_for_session(
        "inv:reuse",
        SESSION,
        "cancel:old",
        ActorOwnerCancellationReason::Cancelled,
    ));
    assert!(test_entries
        .self_ingress_for_request(SESSION, "inv:reuse")
        .is_none());
    assert_eq!(
        registry.finish(old_registration.identity()),
        Some(ActorOwnerCancellationReason::Cancelled)
    );

    let new_execution = test_entries
        .begin_actor_method("wire-cancel-case", "root", SESSION, "inv:reuse".to_string())
        .unwrap();
    assert!(!stale_revoker.revoke());
    assert!(test_entries
        .self_ingress_for_request(SESSION, "inv:reuse")
        .is_some());

    let finalization = tokio::spawn(root.finalize());
    tokio::task::yield_now().await;
    drop(old_execution);
    assert!(!finalization.is_finished());
    drop(new_execution);
    finalization.await.unwrap().unwrap();
}

fn test_deployment() -> ServiceDeploymentRef {
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
