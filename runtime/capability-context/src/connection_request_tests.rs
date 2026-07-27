use std::{sync::Arc, time::Duration};

use tokio::sync::mpsc;

use crate::{
    CancellationSource, ConnectionRequestCancelReason, ConnectionRequestRegistry,
    ConnectionRequestSession, ConnectionRequestTerminal,
};

fn session(value: &str) -> ConnectionRequestSession {
    ConnectionRequestSession::new(value).expect("session token")
}

#[tokio::test]
async fn connection_request_cancel_wins_and_late_response_cannot_reopen_pending() {
    let registry = Arc::new(ConnectionRequestRegistry::new(8));
    let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel();
    let cancellation = CancellationSource::new();
    let session = session("session-a");
    let observed_registry = registry.clone();
    let mut pending = registry
        .install(
            session.clone(),
            cancellation.token(),
            None,
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
async fn connection_request_deadline_cleans_pending_and_emits_dedicated_cancel() {
    let registry = Arc::new(ConnectionRequestRegistry::new(8));
    let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel();
    let session = session("session-deadline");
    let observed_registry = registry.clone();
    let mut pending = registry
        .install(
            session,
            CancellationSource::new().token(),
            Some(tokio::time::Instant::now() + Duration::from_millis(1)),
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
async fn connection_request_session_fence_rejects_reconnect_and_settles_disconnect() {
    let registry = ConnectionRequestRegistry::new(8);
    let old_session = session("runtime-a/session-1");
    let new_session = session("runtime-a/session-2");
    let mut pending = registry
        .install(
            old_session.clone(),
            CancellationSource::new().token(),
            None,
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
async fn connection_request_correlation_ids_are_never_reused_within_registry_lifetime() {
    let registry = ConnectionRequestRegistry::new(8);
    let session = session("session-monotonic");
    let first = registry
        .install(
            session.clone(),
            CancellationSource::new().token(),
            None,
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
            CancellationSource::new().token(),
            None,
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
async fn connection_request_registry_drop_settles_accepted_waiter_as_transport_unavailable() {
    let registry = ConnectionRequestRegistry::new(8);
    let mut pending = registry
        .install(
            session("session-drop"),
            CancellationSource::new().token(),
            None,
            Arc::new(|_, _| Ok(())),
        )
        .expect("accepted request");
    assert_eq!(registry.pending_count(), 1);
    assert_eq!(registry.active_lease_count(), 1);
    drop(registry);

    assert_eq!(
        pending.wait().await,
        ConnectionRequestTerminal::TransportUnavailable
    );
}
