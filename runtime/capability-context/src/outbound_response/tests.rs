use super::*;

#[tokio::test]
async fn terminal_sender_take_commits_once_before_response_delivery() {
    let registry = OutboundRequestRegistry::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let lease = registry
        .insert_with_lease("pending".to_string(), sender, None, "caller_cancel")
        .expect("pending request");
    let terminal = lease.terminal_signal();

    let sender = registry
        .take_terminal_sender("pending")
        .expect("first terminal take must win");
    terminal.wait_terminal().await;
    assert!(terminal.is_terminal());
    assert_eq!(registry.pending_count(), 0);
    assert!(registry.take_terminal_sender("pending").is_none());
    assert!(!lease.cancel("late_cancel"));

    sender
        .send(OutboundResponse::End {
            payload: b"response".to_vec(),
        })
        .expect("terminal winner must retain the response sender");
    assert_eq!(
        receiver.recv().await,
        Some(OutboundResponse::End {
            payload: b"response".to_vec()
        })
    );
    drop(lease);
    assert_eq!(registry.active_lease_count(), 0);
}

#[test]
fn cancellation_winner_fences_terminal_sender_take() {
    let registry = OutboundRequestRegistry::default();
    let (sender, _receiver) = mpsc::unbounded_channel();
    let lease = registry
        .insert_with_lease("pending".to_string(), sender, None, "caller_cancel")
        .expect("pending request");

    assert!(lease.cancel("caller_cancel"));
    assert!(registry.take_terminal_sender("pending").is_none());
    assert_eq!(registry.pending_count(), 0);
}

#[test]
fn terminal_sender_take_and_cancellation_have_exactly_one_concurrent_winner() {
    use std::sync::Barrier;

    for iteration in 0..128 {
        let registry = OutboundRequestRegistry::default();
        let (sender, _receiver) = mpsc::unbounded_channel();
        let lease = registry
            .insert_with_lease(
                format!("pending-{iteration}"),
                sender,
                None,
                "caller_cancel",
            )
            .expect("pending request");
        let barrier = Barrier::new(3);

        let (response_won, cancellation_won) = std::thread::scope(|scope| {
            let response = scope.spawn(|| {
                barrier.wait();
                registry.take_terminal_sender(lease.request_id()).is_some()
            });
            let cancellation = scope.spawn(|| {
                barrier.wait();
                lease.cancel("caller_cancel")
            });
            barrier.wait();
            (
                response.join().expect("response contender"),
                cancellation.join().expect("cancellation contender"),
            )
        });

        assert_ne!(response_won, cancellation_won);
        assert_eq!(registry.pending_count(), 0);
    }
}

#[tokio::test]
async fn fail_all_delivers_error_and_commits_each_pending_request() {
    let registry = OutboundRequestRegistry::default();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let lease = registry
        .insert_with_lease("pending".to_string(), sender, None, "caller_cancel")
        .expect("pending request");
    let terminal = lease.terminal_signal();

    assert_eq!(
        registry.fail_all(ResponseError {
            code: "ConnectionClosed".to_string(),
            message: "router connection closed".to_string(),
            status: None,
            details: None,
        }),
        1
    );
    terminal.wait_terminal().await;
    assert!(matches!(
        receiver.recv().await,
        Some(OutboundResponse::Error(error))
            if error.code == "ConnectionClosed"
                && error.message == "router connection closed"
    ));
    assert_eq!(registry.pending_count(), 0);
    assert_eq!(
        registry.fail_all(ResponseError {
            code: "ConnectionClosed".to_string(),
            message: "router connection closed".to_string(),
            status: None,
            details: None,
        }),
        0
    );
    drop(lease);
    assert_eq!(registry.active_lease_count(), 0);
}
