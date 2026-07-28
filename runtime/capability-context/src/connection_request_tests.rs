use std::{sync::Arc, time::Duration};

use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use tokio::sync::mpsc;

use crate::{
    CancellationSource, ConnectionRequestCancelReason, ConnectionRequestRegistry,
    ConnectionRequestSession, ConnectionRequestTerminal, ExecutionScope,
};

fn session(value: &str) -> ConnectionRequestSession {
    ConnectionRequestSession::new(value).expect("session token")
}

fn request_scope(cancellation: &CancellationSource) -> ExecutionScope {
    ExecutionScope::request(cancellation.token(), None)
}

fn derived_scope(
    cancellation: &CancellationSource,
    deadline: tokio::time::Instant,
) -> ExecutionScope {
    request_scope(cancellation)
        .derive(
            deadline.into_std(),
            InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
            },
        )
        .expect("derived request scope")
}

#[tokio::test]
async fn f445h_i6_connection_request_scope_ancestor_stop_wins_and_late_response_is_fenced() {
    let registry = Arc::new(ConnectionRequestRegistry::new(8));
    let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel();
    let cancellation = CancellationSource::new();
    let session = session("session-a");
    let observed_registry = registry.clone();
    let mut pending = registry
        .install(
            session.clone(),
            request_scope(&cancellation),
            Arc::new(move |request_id, reason| {
                cancel_tx
                    .send((
                        request_id.to_string(),
                        reason,
                        observed_registry.pending_count(),
                        observed_registry.active_lease_count(),
                        observed_registry.active_timer_count(),
                    ))
                    .map_err(|_| ())
            }),
        )
        .expect("request lease");
    let request_id = pending.request_id().to_string();

    cancellation.cancel();
    assert_eq!(
        pending.wait().await,
        ConnectionRequestTerminal::AncestorCancelled
    );
    assert_eq!(
        cancel_rx.recv().await,
        Some((
            request_id.clone(),
            ConnectionRequestCancelReason::CallerCancel,
            0,
            0,
            0,
        ))
    );
    assert!(!registry.complete(
        &session,
        &request_id,
        ConnectionRequestTerminal::Success(b"late".to_vec())
    ));
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);
    assert_eq!(registry.active_timer_count(), 0);
}

#[tokio::test]
async fn f445h_i6_connection_request_scope_failed_hint_does_not_change_local_terminal() {
    let registry = ConnectionRequestRegistry::new(8);
    let cancellation = CancellationSource::new();
    let scope = request_scope(&cancellation);
    let scope_observer = scope.clone();
    let session = session("session-hint-failure");
    let mut pending = registry
        .install(session, scope, Arc::new(|_, _| Err(())))
        .expect("request lease");

    cancellation.cancel();
    assert_eq!(
        pending.wait().await,
        ConnectionRequestTerminal::AncestorCancelled
    );
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);
    assert_eq!(registry.active_timer_count(), 0);
    assert_eq!(
        scope_observer.lifecycle_snapshot(),
        crate::ExecutionScopeLifecycleSnapshot::default()
    );
}

#[tokio::test]
async fn f445h_i6_connection_request_scope_derived_deadline_cleans_all_owners_before_hint() {
    let registry = Arc::new(ConnectionRequestRegistry::new(8));
    let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel();
    let session = session("session-deadline");
    let cancellation = CancellationSource::new();
    let observed_registry = registry.clone();
    let mut pending = registry
        .install(
            session,
            derived_scope(
                &cancellation,
                tokio::time::Instant::now() + Duration::from_millis(1),
            ),
            Arc::new(move |request_id, reason| {
                cancel_tx
                    .send((
                        request_id.to_string(),
                        reason,
                        observed_registry.pending_count(),
                        observed_registry.active_lease_count(),
                        observed_registry.active_timer_count(),
                    ))
                    .map_err(|_| ())
            }),
        )
        .expect("request lease");
    let request_id = pending.request_id().to_string();

    assert_eq!(
        pending.wait().await,
        ConnectionRequestTerminal::DeadlineExceeded
    );
    assert_eq!(
        cancel_rx.recv().await,
        Some((
            request_id,
            ConnectionRequestCancelReason::DeadlineExceeded,
            0,
            0,
            0,
        ))
    );
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);
    assert_eq!(registry.active_timer_count(), 0);
}

#[tokio::test]
async fn f445h_i6_connection_request_scope_session_fence_rejects_reconnect_and_settles_disconnect()
{
    let registry = ConnectionRequestRegistry::new(8);
    let old_session = session("runtime-a/session-1");
    let new_session = session("runtime-a/session-2");
    let cancellation = CancellationSource::new();
    let mut pending = registry
        .install(
            old_session.clone(),
            request_scope(&cancellation),
            Arc::new(|_, _| Ok(())),
        )
        .expect("request lease");
    let request_id = pending.request_id().to_string();

    assert!(!registry.complete(
        &new_session,
        &request_id,
        ConnectionRequestTerminal::Success(b"forged".to_vec())
    ));
    assert_eq!(
        registry.disconnect_session(&old_session),
        1,
        "the original session owns exactly one pending request"
    );
    assert_eq!(
        pending.wait().await,
        ConnectionRequestTerminal::TransportUnavailable
    );
    assert!(!registry.complete(
        &new_session,
        &request_id,
        ConnectionRequestTerminal::Success(b"late".to_vec())
    ));
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);
    assert_eq!(registry.active_timer_count(), 0);
}

#[tokio::test]
async fn f445h_i6_connection_request_scope_response_first_beats_ready_deadline_and_duplicates() {
    let registry = ConnectionRequestRegistry::new(8);
    let session = session("session-response-first");
    let cancellation = CancellationSource::new();
    let scope = derived_scope(&cancellation, tokio::time::Instant::now());
    let scope_observer = scope.clone();
    let mut pending = registry
        .install(
            session.clone(),
            scope,
            Arc::new(|_, _| panic!("response winner must not emit an internal stop hint")),
        )
        .expect("pending request");
    let request_id = pending.request_id().to_string();

    assert!(registry.complete(
        &session,
        &request_id,
        ConnectionRequestTerminal::Success(b"response".to_vec())
    ));
    assert!(!registry.complete(
        &session,
        &request_id,
        ConnectionRequestTerminal::Success(b"duplicate".to_vec())
    ));
    assert_eq!(
        pending.wait().await,
        ConnectionRequestTerminal::Success(b"response".to_vec())
    );
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);
    assert_eq!(registry.active_timer_count(), 0);
    assert_eq!(
        scope_observer.lifecycle_snapshot(),
        crate::ExecutionScopeLifecycleSnapshot::default()
    );
}

#[tokio::test]
async fn f445h_i6_connection_request_scope_correlation_ids_are_never_reused() {
    let registry = ConnectionRequestRegistry::new(8);
    let session = session("session-monotonic");
    let first_cancellation = CancellationSource::new();
    let first = registry
        .install(
            session.clone(),
            request_scope(&first_cancellation),
            Arc::new(|_, _| Ok(())),
        )
        .expect("first request");
    let first_id = first.request_id().to_string();
    drop(first);
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);

    let mut second = registry
        .install(
            session.clone(),
            request_scope(&CancellationSource::new()),
            Arc::new(|_, _| Ok(())),
        )
        .expect("second request");
    let second_id = second.request_id().to_string();
    assert_ne!(first_id, second_id);
    assert!(registry.complete(
        &session,
        &second_id,
        ConnectionRequestTerminal::Success(b"ok".to_vec())
    ));
    assert_eq!(
        second.wait().await,
        ConnectionRequestTerminal::Success(b"ok".to_vec())
    );
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(registry.active_lease_count(), 0);
    assert_eq!(registry.active_timer_count(), 0);
}

#[tokio::test]
async fn f445h_i6_connection_request_scope_registry_drop_settles_waiter_and_releases_scope() {
    let registry = ConnectionRequestRegistry::new(8);
    let cancellation = CancellationSource::new();
    let scope = request_scope(&cancellation);
    let scope_observer = scope.clone();
    let mut pending = registry
        .install(session("session-drop"), scope, Arc::new(|_, _| Ok(())))
        .expect("accepted request");
    assert_eq!(registry.pending_count(), 1);
    assert_eq!(registry.active_lease_count(), 1);
    drop(registry);

    assert_eq!(
        pending.wait().await,
        ConnectionRequestTerminal::TransportUnavailable
    );
    assert_eq!(
        scope_observer.lifecycle_snapshot(),
        crate::ExecutionScopeLifecycleSnapshot::default()
    );
}
