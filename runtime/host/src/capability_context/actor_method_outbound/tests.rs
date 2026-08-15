use super::*;

fn implementation() -> ActorImplementationIdentity {
    ActorImplementationIdentity::new(format!(
        "skiff-actor-implementation-v1:sha256:{}",
        "a".repeat(64)
    ))
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_registry_commits_only_exact_response_once() {
    let registry = ActorMethodOutboundRegistry::default();
    let mut lease = registry
        .register("invoke-1".into(), "cancel-1".into(), 1, implementation())
        .unwrap();
    assert_eq!(
        registry.cancellation_correlation("invoke-1").as_deref(),
        Some("cancel-1")
    );
    assert!(!registry.complete("invoke-2", ActorInvocationOutcome::Returned(vec![2])));
    assert!(registry.complete("invoke-1", ActorInvocationOutcome::Returned(vec![1])));
    assert_eq!(
        lease.receive().await.unwrap().unwrap(),
        ActorInvocationOutcome::Returned(vec![1])
    );
    assert!(!registry.complete("invoke-1", ActorInvocationOutcome::Returned(vec![3])));
    assert_eq!(registry.pending_count(), 0);
}

#[tokio::test]
async fn actor_pending_response_requires_exact_epoch_and_implementation() {
    let registry = ActorMethodOutboundRegistry::default();
    let mut lease = registry
        .register(
            "invoke-pending".into(),
            "cancel-pending".into(),
            1,
            implementation(),
        )
        .unwrap();
    assert_eq!(registry.pending_count(), 1);
    assert!(!registry.complete_failure(
        "invoke-pending",
        2,
        &implementation(),
        ActorInvocationTransportError {
            code: "StaleEpoch".into(),
            message: "stale".into(),
        }
    ));
    assert!(registry.complete_failure(
        "invoke-pending",
        1,
        &implementation(),
        ActorInvocationTransportError {
            code: "RuntimeExecutionFailed".into(),
            message: "boom".into(),
        }
    ));
    assert_eq!(
        lease.receive().await.unwrap().unwrap_err(),
        ActorInvocationTransportError {
            code: "RuntimeExecutionFailed".into(),
            message: "boom".into(),
        }
    );
    assert_eq!(registry.pending_count(), 0);
}

#[tokio::test]
async fn actor_method_error_routes_typed_outcome_once() {
    let registry = ActorMethodOutboundRegistry::default();
    let mut lease = registry
        .register(
            "invoke-error".into(),
            "cancel-error".into(),
            7,
            implementation(),
        )
        .unwrap();
    assert!(registry.complete_actor_error(
        "invoke-error",
        ActorInvocationError::ActorIncarnationReplaced {
            requested_epoch: 6,
            current_epoch: 7,
        }
    ));
    assert_eq!(
        lease.receive().await.unwrap().unwrap(),
        ActorInvocationOutcome::ActorError(ActorInvocationError::ActorIncarnationReplaced {
            requested_epoch: 6,
            current_epoch: 7,
        })
    );
    assert!(!registry.complete_actor_error(
        "invoke-error",
        ActorInvocationError::ActorUpgrading { retry_after_ms: 1 }
    ));
}

#[tokio::test]
async fn fail_all_delivers_connection_error_and_fences_late_response() {
    let registry = ActorMethodOutboundRegistry::default();
    let mut lease = registry
        .register("invoke-1".into(), "cancel-1".into(), 1, implementation())
        .unwrap();

    assert_eq!(
        registry.fail_all(ActorInvocationTransportError {
            code: "ConnectionClosed".to_string(),
            message: "router connection closed".to_string(),
        }),
        1
    );
    assert_eq!(
        lease.receive().await.unwrap().unwrap_err(),
        ActorInvocationTransportError {
            code: "ConnectionClosed".to_string(),
            message: "router connection closed".to_string(),
        }
    );
    assert_eq!(registry.pending_count(), 0);
    assert!(!registry.complete(
        "invoke-1",
        ActorInvocationOutcome::Returned(b"late".to_vec())
    ));
}

#[test]
fn f445h_i6_actor_scope_method_registry_drop_fences_late_response() {
    let registry = ActorMethodOutboundRegistry::default();
    let lease = registry
        .register("invoke-1".into(), "cancel-1".into(), 1, implementation())
        .unwrap();
    drop(lease);
    assert_eq!(registry.cancellation_correlation("invoke-1"), None);
    assert_eq!(registry.pending_count(), 0);
    assert!(!registry.complete("invoke-1", ActorInvocationOutcome::Returned(vec![1])));
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_transport_failure_keeps_exact_owner() {
    let registry = ActorMethodOutboundRegistry::default();
    let mut lease = registry
        .register("invoke-1".into(), "cancel-1".into(), 1, implementation())
        .unwrap();
    assert!(!registry.complete_failure(
        "invoke-1",
        2,
        &implementation(),
        ActorInvocationTransportError {
            code: "runtimeExecutionFailed".into(),
            message: "stale".into(),
        }
    ));
    assert!(registry.complete_failure(
        "invoke-1",
        1,
        &implementation(),
        ActorInvocationTransportError {
            code: "runtimeExecutionFailed".into(),
            message: "boom".into(),
        }
    ));
    assert_eq!(
        lease.receive().await.unwrap().unwrap_err(),
        ActorInvocationTransportError {
            code: "runtimeExecutionFailed".into(),
            message: "boom".into(),
        }
    );
}
