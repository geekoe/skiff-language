//! Single heap-access mechanism for every evaluator execution.
//!
//! All executions (ordinary requests, actor instances, callback owners,
//! providers, producers, task targets) share one lease protocol: the
//! execution state owns an `Arc<tokio::sync::Mutex<RequestHeap>>` arena and
//! holds an owned guard for the synchronous segment. Ordinary requests use a
//! fresh private arena; actor instances pass their shared arena. The guard
//! must never survive a `Pending` poll: funnels release before awaiting and
//! reacquire after wake.

use std::{
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::Arc,
    task::Poll,
};

use skiff_runtime_model::request_heap::RequestHeap;

#[derive(Debug)]
pub struct HeapAccess {
    arena: Arc<tokio::sync::Mutex<RequestHeap>>,
    guard: Option<tokio::sync::OwnedMutexGuard<RequestHeap>>,
}

impl HeapAccess {
    /// Creates a fresh private arena for an ordinary execution and acquires
    /// the guard immediately. The new arena is uncontended, so the initial
    /// lock cannot fail.
    pub fn private(heap: RequestHeap) -> Self {
        let arena = Arc::new(tokio::sync::Mutex::new(heap));
        let guard = Arc::clone(&arena)
            .try_lock_owned()
            .expect("fresh private heap arena must be uncontended");
        Self {
            arena,
            guard: Some(guard),
        }
    }

    /// Wraps an already-acquired guard over an existing arena (actor segment,
    /// callback owner heap).
    pub fn with_guard(
        arena: Arc<tokio::sync::Mutex<RequestHeap>>,
        guard: tokio::sync::OwnedMutexGuard<RequestHeap>,
    ) -> Self {
        Self {
            arena,
            guard: Some(guard),
        }
    }

    #[inline(always)]
    pub fn heap_mut(&mut self) -> &mut RequestHeap {
        match self.guard.as_mut() {
            Some(guard) => guard,
            None => missing_guard(),
        }
    }

    /// Releases the arena guard (dropping it).
    pub fn release(&mut self) {
        self.guard = None;
    }

    /// Re-acquires the arena guard.
    pub async fn reacquire(&mut self) {
        if self.guard.is_none() {
            self.guard = Some(self.arena.clone().lock_owned().await);
        }
    }

    /// Recovers the owned heap from a private arena. The guard is dropped
    /// first; the arena must then have no other strong references.
    pub fn into_owned_heap(self) -> RequestHeap {
        let HeapAccess { arena, mut guard } = self;
        guard.take();
        Arc::try_unwrap(arena)
            .ok()
            .expect("private heap arena must be uniquely owned at execution end")
            .into_inner()
    }
}

impl Deref for HeapAccess {
    type Target = RequestHeap;

    fn deref(&self) -> &Self::Target {
        match self.guard.as_ref() {
            Some(guard) => guard,
            None => missing_guard(),
        }
    }
}

impl DerefMut for HeapAccess {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self.guard.as_mut() {
            Some(guard) => guard,
            None => missing_guard(),
        }
    }
}

#[cold]
#[inline(never)]
fn missing_guard() -> ! {
    panic!("HeapAccess heap access requires an acquired guard")
}

/// Polls the future once without yielding to the executor.
pub(crate) async fn poll_once<F>(mut future: Pin<&mut F>) -> Option<F::Output>
where
    F: Future,
{
    std::future::poll_fn(|context| {
        Poll::Ready(match future.as_mut().poll(context) {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        })
    })
    .await
}

/// Unified funnel body: poll once; a `Ready` keeps the guard, a `Pending`
/// releases the guard before awaiting and reacquires after wake.
///
/// The awaited future is boxed rather than `tokio::pin!`ed on the stack so a
/// large evaluator future never gets inlined into the funnel's (and therefore
/// the caller's) state machine.
pub(crate) async fn await_with_release<F>(heap: &mut HeapAccess, future: F) -> F::Output
where
    F: Future,
{
    let mut future = Box::pin(future);
    if let Some(output) = poll_once(future.as_mut()).await {
        return output;
    }
    heap.release();
    let output = future.await;
    heap.reacquire().await;
    output
}

#[cfg(test)]
mod tests;
