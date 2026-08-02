use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};

use super::{await_shared_with_release, HeapAccess};

fn exclusive_access(heap: &mut RequestHeap) -> HeapAccess<'_> {
    HeapAccess::Exclusive(heap)
}

#[tokio::test]
async fn exclusive_release_and_reacquire_are_noops() {
    let mut heap = RequestHeap::new(RequestHeapLimits::default());
    let mut access = exclusive_access(&mut heap);
    assert!(!access.is_shared());

    let handle = access
        .heap_mut()
        .alloc_local_carrier_cell(RuntimeValue::Null.into())
        .expect("allocate in exclusive heap");
    assert!(access.heap_mut().get(handle).is_ok());

    access.release();
    access.reacquire().await;

    assert!(access.heap_mut().get(handle).is_ok());
}

#[tokio::test]
async fn shared_release_drops_guard_and_reacquire_restores_it() {
    let arena: Arc<tokio::sync::Mutex<RequestHeap>> = Arc::new(tokio::sync::Mutex::new(
        RequestHeap::new(RequestHeapLimits::default()),
    ));
    let guard = arena.clone().lock_owned().await;
    let mut access = HeapAccess::Shared {
        arena: arena.clone(),
        guard: Some(guard),
    };
    assert!(access.is_shared());

    let handle = access
        .heap_mut()
        .alloc_local_carrier_cell(RuntimeValue::Null.into())
        .expect("allocate in shared arena");
    assert!(access.heap_mut().get(handle).is_ok());
    assert!(
        arena.clone().try_lock_owned().is_err(),
        "guard must be held"
    );

    access.release();
    assert!(
        arena.clone().try_lock_owned().is_ok(),
        "released guard must leave the arena acquirable"
    );

    access.reacquire().await;
    assert!(access.heap_mut().get(handle).is_ok());
    assert!(
        arena.clone().try_lock_owned().is_err(),
        "reacquired guard must be held again"
    );
}

#[tokio::test]
async fn shared_funnel_ready_keeps_guard() {
    let arena: Arc<tokio::sync::Mutex<RequestHeap>> = Arc::new(tokio::sync::Mutex::new(
        RequestHeap::new(RequestHeapLimits::default()),
    ));
    let guard = arena.clone().lock_owned().await;
    let mut access = HeapAccess::Shared {
        arena: arena.clone(),
        guard: Some(guard),
    };

    let output = await_shared_with_release(&mut access, async { 7 }).await;

    assert_eq!(output, 7);
    let HeapAccess::Shared { guard, .. } = &access else {
        panic!("funnel access must remain Shared");
    };
    assert!(
        guard.is_some(),
        "a Ready first poll must not release the guard"
    );
    assert!(
        arena.clone().try_lock_owned().is_err(),
        "guard must still be held after a Ready funnel poll"
    );
}

/// Future that reports its first Pending poll and then waits for a release
/// signal before completing, so the test can observe the arena while the
/// funnel is suspended.
struct PendingGate {
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    release: tokio::sync::oneshot::Receiver<()>,
}

impl Future for PendingGate {
    type Output = u32;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(entered) = self.entered.take() {
            let _ = entered.send(());
            return Poll::Pending;
        }
        match Pin::new(&mut self.release).poll(context) {
            Poll::Ready(Ok(())) => Poll::Ready(42),
            Poll::Ready(Err(_)) => Poll::Ready(0),
            Poll::Pending => Poll::Pending,
        }
    }
}

#[tokio::test]
async fn shared_funnel_pending_releases_guard_and_reacquires_after_wake() {
    let arena: Arc<tokio::sync::Mutex<RequestHeap>> = Arc::new(tokio::sync::Mutex::new(
        RequestHeap::new(RequestHeapLimits::default()),
    ));
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let arena_for_task = arena.clone();

    let task = tokio::spawn(async move {
        let guard = arena_for_task.clone().lock_owned().await;
        let mut access = HeapAccess::Shared {
            arena: arena_for_task,
            guard: Some(guard),
        };
        let output = await_shared_with_release(
            &mut access,
            PendingGate {
                entered: Some(entered_tx),
                release: release_rx,
            },
        )
        .await;
        let HeapAccess::Shared { guard, .. } = &access else {
            panic!("funnel task access must remain Shared");
        };
        assert!(
            guard.is_some(),
            "guard must be reacquired before the funnel returns"
        );
        output
    });

    entered_rx
        .await
        .expect("funnel must report its first Pending poll");
    assert!(
        arena.clone().try_lock_owned().is_ok(),
        "Shared guard must be released while the funnel future is Pending"
    );
    release_tx.send(()).expect("release the pending funnel");

    assert_eq!(task.await.expect("funnel task"), 42);
    // The task dropped its access (and with it the reacquired guard) when it
    // completed; the reacquire-before-return invariant is asserted inside the
    // task where the access is still alive.
    assert!(
        arena.clone().try_lock_owned().is_ok(),
        "funnel task must release the arena when its access is dropped"
    );
}
