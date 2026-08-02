//! Dual-mode heap access for the evaluator.
//!
//! Ordinary requests keep the historical exclusive borrow of the caller
//! `RequestHeap`; actor instances will use a shared arena behind
//! `tokio::sync::Mutex`. In Shared mode the owned guard must never survive a
//! `Pending` poll: funnels release before awaiting and reacquire after wake.

use std::{
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::Arc,
    task::Poll,
};

use skiff_runtime_model::request_heap::RequestHeap;

#[derive(Debug)]
pub enum HeapAccess<'a> {
    Exclusive(&'a mut RequestHeap),
    Shared {
        arena: Arc<tokio::sync::Mutex<RequestHeap>>,
        guard: Option<tokio::sync::OwnedMutexGuard<RequestHeap>>,
    },
}

impl HeapAccess<'_> {
    #[inline(always)]
    pub fn heap_mut(&mut self) -> &mut RequestHeap {
        match self {
            HeapAccess::Exclusive(heap) => heap,
            HeapAccess::Shared { guard, .. } => match guard.as_mut() {
                Some(guard) => guard,
                None => missing_shared_guard(),
            },
        }
    }

    /// Releases the Shared arena guard (dropping it). Exclusive mode is a no-op.
    pub fn release(&mut self) {
        if let HeapAccess::Shared { guard, .. } = self {
            *guard = None;
        }
    }

    /// Re-acquires the Shared arena guard. Exclusive mode is a no-op.
    pub async fn reacquire(&mut self) {
        if let HeapAccess::Shared { arena, guard } = self {
            if guard.is_none() {
                *guard = Some(arena.clone().lock_owned().await);
            }
        }
    }

    pub fn is_shared(&self) -> bool {
        matches!(self, HeapAccess::Shared { .. })
    }
}

impl Deref for HeapAccess<'_> {
    type Target = RequestHeap;

    fn deref(&self) -> &Self::Target {
        match self {
            HeapAccess::Exclusive(heap) => heap,
            HeapAccess::Shared { guard, .. } => match guard.as_ref() {
                Some(guard) => guard,
                None => missing_shared_guard(),
            },
        }
    }
}

impl DerefMut for HeapAccess<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            HeapAccess::Exclusive(heap) => heap,
            HeapAccess::Shared { guard, .. } => match guard.as_mut() {
                Some(guard) => guard,
                None => missing_shared_guard(),
            },
        }
    }
}

#[cold]
#[inline(never)]
fn missing_shared_guard() -> ! {
    panic!("HeapAccess::Shared heap access requires an acquired guard")
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

/// Shared-mode funnel body: poll once; a `Ready` keeps the guard, a `Pending`
/// releases the guard before awaiting and reacquires after wake.
pub(crate) async fn await_shared_with_release<F>(heap: &mut HeapAccess<'_>, future: F) -> F::Output
where
    F: Future,
{
    tokio::pin!(future);
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
