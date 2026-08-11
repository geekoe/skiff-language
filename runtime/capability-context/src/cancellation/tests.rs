use super::*;

async fn wait_until_flag_backed_waiters(expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), async move {
        while flag_backed_cancel_waiters_active() != expected {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("flag-backed waiter count should settle");
}

#[tokio::test]
async fn token_waits_for_notify_backed_cancel() {
    let token = CancellationToken::new();
    let waiter = {
        let token = token.clone();
        tokio::spawn(async move { token.wait_cancelled().await })
    };

    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());

    token.cancel();
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("notify-backed cancellation should wake")
        .expect("wait task should succeed");
}

#[tokio::test]
async fn pre_cancelled_token_wait_returns_immediately() {
    let token = CancellationToken::new();
    token.cancel();

    tokio::time::timeout(Duration::from_millis(50), token.wait_cancelled())
        .await
        .expect("pre-cancelled token should not wait");
}

#[tokio::test]
async fn cancel_racing_with_waiter_registration_wakes() {
    for _ in 0..100 {
        let token = CancellationToken::new();
        let waiter = {
            let token = token.clone();
            tokio::spawn(async move { token.wait_cancelled().await })
        };
        tokio::task::yield_now().await;
        token.cancel();
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("racing cancellation should wake waiter")
            .expect("wait task should succeed");
    }
}

#[tokio::test]
async fn cancel_wakes_multiple_waiters() {
    let token = CancellationToken::new();
    let waiters = (0..16)
        .map(|_| {
            let token = token.clone();
            tokio::spawn(async move { token.wait_cancelled().await })
        })
        .collect::<Vec<_>>();

    tokio::task::yield_now().await;
    token.cancel();

    for waiter in waiters {
        tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("cancel should wake every waiter")
            .expect("wait task should succeed");
    }
}

#[tokio::test]
async fn token_waits_for_flag_backed_cancel_with_tracked_fallback() {
    let cancelled = Arc::new(AtomicBool::new(false));
    let token = CancellationToken::from_flag(cancelled.clone());
    let waiter = tokio::spawn(async move { token.wait_cancelled().await });

    wait_until_flag_backed_waiters(1).await;

    cancelled.store(true, Ordering::Release);
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("flag-backed cancellation should be polled")
        .expect("wait task should succeed");
    wait_until_flag_backed_waiters(0).await;
}

#[tokio::test]
async fn completion_signal_waits_for_mark_completed() {
    let completed = CompletionSignal::new();
    let waiter = {
        let completed = completed.clone();
        tokio::spawn(async move { completed.wait_completed().await })
    };

    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());

    completed.mark_completed();
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("completion signal should wake")
        .expect("wait task should succeed");
}

#[tokio::test]
async fn signals_wait_for_borrowed_flag() {
    let cancelled = AtomicBool::new(false);
    let signals = CancellationSignals::from_borrowed_flag(Some(&cancelled));

    assert!(!signals.is_cancelled());
    cancelled.store(true, Ordering::Release);

    tokio::time::timeout(Duration::from_secs(1), signals.wait_cancelled())
        .await
        .expect("borrowed flag should wake through compatibility polling");
}

#[tokio::test]
async fn signals_wait_for_notify_token_without_poll_delay() {
    let token = CancellationToken::new();
    let signals = CancellationSignals::from_tokens([token.clone()]);
    let waiter = tokio::spawn(async move { signals.wait_cancelled().await });

    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());

    token.cancel();
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("notify-backed token signal should wake")
        .expect("wait task should succeed");
}

#[tokio::test]
async fn signal_set_wakes_when_any_token_cancelled() {
    let first = CancellationToken::new();
    let second = CancellationToken::new();
    let signals = CancellationSignals::from_tokens([first, second.clone()]);
    let waiter = tokio::spawn(async move { signals.wait_cancelled().await });

    tokio::task::yield_now().await;
    second.cancel();

    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("any token cancellation should wake signal set")
        .expect("wait task should succeed");
}

#[test]
fn polling_fallback_allowlist_entries_have_counter_and_removal_owner() {
    assert_eq!(FLAG_BACKED_CANCELLATION_POLLING_FALLBACK_ALLOWLIST.len(), 5);
    for entry in FLAG_BACKED_CANCELLATION_POLLING_FALLBACK_ALLOWLIST {
        assert_eq!(entry.counter, "cancellation.flag_backed_waiters.active");
        assert!(!entry.bound.is_empty());
        assert!(!entry.removal.is_empty());
    }
}
