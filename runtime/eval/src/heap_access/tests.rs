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

use super::{await_with_release, HeapAccess};

#[tokio::test]
async fn private_arena_holds_guard_until_release() {
    let mut access = HeapAccess::private(RequestHeap::new(RequestHeapLimits::default()));

    let handle = access
        .heap_mut()
        .alloc_local_carrier_cell(RuntimeValue::Null.into())
        .expect("allocate in private arena");
    assert!(access.heap_mut().get(handle).is_ok());

    access.release();
    access.reacquire().await;

    assert!(access.heap_mut().get(handle).is_ok());
}

#[tokio::test]
async fn private_into_owned_heap_recovers_the_heap() {
    let mut access = HeapAccess::private(RequestHeap::new(RequestHeapLimits::default()));
    let handle = access
        .heap_mut()
        .alloc_local_carrier_cell(RuntimeValue::Number(7.0).into())
        .expect("allocate in private arena");
    let heap = access.into_owned_heap();
    assert!(heap.get(handle).is_ok());
}

#[tokio::test]
async fn shared_release_drops_guard_and_reacquire_restores_it() {
    let arena: Arc<tokio::sync::Mutex<RequestHeap>> = Arc::new(tokio::sync::Mutex::new(
        RequestHeap::new(RequestHeapLimits::default()),
    ));
    let guard = arena.clone().lock_owned().await;
    let mut access = HeapAccess::with_guard(arena.clone(), guard);

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
async fn funnel_ready_keeps_guard() {
    let mut access = HeapAccess::private(RequestHeap::new(RequestHeapLimits::default()));
    let arena = arena_of(&access);

    let output = await_with_release(&mut access, async { 7 }).await;

    assert_eq!(output, 7);
    assert!(
        access.guard.is_some(),
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

async fn run_pending_funnel<F>(arena: Arc<tokio::sync::Mutex<RequestHeap>>, make_access: F) -> u32
where
    F: FnOnce(Arc<tokio::sync::Mutex<RequestHeap>>) -> HeapAccess + Send + 'static,
{
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let arena_for_task = arena.clone();

    let task = tokio::spawn(async move {
        let mut access = make_access(arena_for_task);
        let output = await_with_release(
            &mut access,
            PendingGate {
                entered: Some(entered_tx),
                release: release_rx,
            },
        )
        .await;
        assert!(
            access.guard.is_some(),
            "guard must be reacquired before the funnel returns"
        );
        output
    });

    entered_rx
        .await
        .expect("funnel must report its first Pending poll");
    assert!(
        arena.clone().try_lock_owned().is_ok(),
        "guard must be released while the funnel future is Pending"
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
    42
}

#[tokio::test]
async fn private_funnel_pending_releases_guard_and_reacquires_after_wake() {
    let arena: Arc<tokio::sync::Mutex<RequestHeap>> = Arc::new(tokio::sync::Mutex::new(
        RequestHeap::new(RequestHeapLimits::default()),
    ));
    run_pending_funnel(arena, |arena| {
        let guard = Arc::clone(&arena)
            .try_lock_owned()
            .expect("fresh private arena should lock");
        HeapAccess::with_guard(arena, guard)
    })
    .await;
}

#[tokio::test]
async fn shared_funnel_pending_releases_guard_and_reacquires_after_wake() {
    let arena: Arc<tokio::sync::Mutex<RequestHeap>> = Arc::new(tokio::sync::Mutex::new(
        RequestHeap::new(RequestHeapLimits::default()),
    ));
    run_pending_funnel(arena, |arena| {
        let guard = Arc::clone(&arena)
            .try_lock_owned()
            .expect("fresh shared arena should lock");
        HeapAccess::with_guard(arena, guard)
    })
    .await;
}

fn arena_of(access: &HeapAccess) -> Arc<tokio::sync::Mutex<RequestHeap>> {
    Arc::clone(&access.arena)
}
