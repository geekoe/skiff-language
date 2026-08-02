//! Bounded blocking loader pool (C-bootstrap §2.3/§3.8, plan §3.8/§7).
//!
//! Sync artifact/snapshot readers may only be invoked through this pool:
//! bounded concurrency (semaphore), per-operation deadline, fail-closed
//! saturation (no unbounded queue), and shutdown (new loads refused, drain
//! waits for in-flight occupancy to return to zero within a drain deadline).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Notify, Semaphore};

/// Pool configuration (process-level defaults from C-bootstrap §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockingLoaderOptions {
    pub concurrency: usize,
    pub read_deadline: Duration,
    pub drain_deadline: Duration,
}

impl Default for BlockingLoaderOptions {
    fn default() -> Self {
        Self {
            concurrency: 8,
            read_deadline: Duration::from_secs(5),
            drain_deadline: Duration::from_secs(5),
        }
    }
}

/// Fail-closed outcomes of one pool operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockingLoaderError<E> {
    /// Pool saturated: no permit available; the operation was never queued.
    Saturated,
    /// Per-operation deadline elapsed; the logical read aborted.
    Deadline,
    /// Pool is shut down; new loads are refused.
    Shutdown,
    /// Blocking task panicked or the join failed.
    Join(String),
    /// The underlying blocking operation failed.
    Operation(E),
}

impl<E> std::fmt::Display for BlockingLoaderError<E>
where
    E: std::fmt::Display,
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Saturated => write!(formatter, "blocking loader saturated"),
            Self::Deadline => write!(formatter, "blocking loader read deadline elapsed"),
            Self::Shutdown => write!(formatter, "blocking loader is shut down"),
            Self::Join(message) => write!(formatter, "blocking loader task join failed: {message}"),
            Self::Operation(error) => write!(formatter, "blocking operation failed: {error}"),
        }
    }
}

impl<E> std::error::Error for BlockingLoaderError<E> where
    E: std::fmt::Display + std::fmt::Debug + 'static
{
}

/// Health projection (`blockingLoader.{occupancy,queued,saturated,
/// deadlineAborts}` plus shutdown state). `queued` is always zero: saturation
/// fails closed and never queues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockingLoaderHealth {
    pub concurrency: usize,
    pub occupancy: usize,
    pub queued: u64,
    pub saturated: u64,
    pub deadline_aborts: u64,
    pub shutdown_refusals: u64,
    pub shutdown: bool,
}

/// Bounded blocking pool; `Send + Sync`, safe to share across the router.
#[derive(Debug)]
pub struct BlockingLoader {
    options: BlockingLoaderOptions,
    permits: Arc<Semaphore>,
    shutdown: AtomicBool,
    occupancy: AtomicUsize,
    saturated: AtomicU64,
    deadline_aborts: AtomicU64,
    shutdown_refusals: AtomicU64,
    drained: Notify,
}

impl BlockingLoader {
    pub fn new(options: BlockingLoaderOptions) -> Self {
        let concurrency = options.concurrency.max(1);
        Self {
            options: BlockingLoaderOptions {
                concurrency,
                ..options
            },
            permits: Arc::new(Semaphore::new(concurrency)),
            shutdown: AtomicBool::new(false),
            occupancy: AtomicUsize::new(0),
            saturated: AtomicU64::new(0),
            deadline_aborts: AtomicU64::new(0),
            shutdown_refusals: AtomicU64::new(0),
            drained: Notify::new(),
        }
    }

    /// Runs one blocking operation under a permit and a deadline.
    ///
    /// Saturation and shutdown fail closed before any work is spawned; a
    /// deadline elapse fails the logical read closed (the detached blocking
    /// thread finishes on its own; it never consumes another permit).
    pub async fn run<T, E, F>(&self, operation: F) -> Result<T, BlockingLoaderError<E>>
    where
        F: FnOnce() -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
    {
        if self.shutdown.load(Ordering::SeqCst) {
            self.shutdown_refusals.fetch_add(1, Ordering::Relaxed);
            return Err(BlockingLoaderError::Shutdown);
        }
        let permit = match Arc::clone(&self.permits).try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.saturated.fetch_add(1, Ordering::Relaxed);
                return Err(BlockingLoaderError::Saturated);
            }
        };
        self.occupancy.fetch_add(1, Ordering::AcqRel);
        let task = tokio::task::spawn_blocking(operation);
        let result = tokio::time::timeout(self.options.read_deadline, task).await;
        drop(permit);
        let finished = self.occupancy.fetch_sub(1, Ordering::AcqRel) - 1;
        if finished == 0 {
            self.drained.notify_waiters();
        }
        match result {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(operation_error))) => Err(BlockingLoaderError::Operation(operation_error)),
            Ok(Err(join_error)) => Err(BlockingLoaderError::Join(join_error.to_string())),
            Err(_) => {
                self.deadline_aborts.fetch_add(1, Ordering::Relaxed);
                Err(BlockingLoaderError::Deadline)
            }
        }
    }

    /// Shuts the pool down: refuses new loads and drains in-flight occupancy
    /// within the drain deadline.
    pub async fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let deadline = tokio::time::sleep(self.options.drain_deadline);
        tokio::pin!(deadline);
        while self.occupancy.load(Ordering::Acquire) > 0 {
            let notified = self.drained.notified();
            tokio::select! {
                _ = &mut deadline => break,
                _ = notified => {}
            }
        }
    }

    pub fn health(&self) -> BlockingLoaderHealth {
        BlockingLoaderHealth {
            concurrency: self.options.concurrency,
            occupancy: self.occupancy.load(Ordering::Acquire),
            queued: 0,
            saturated: self.saturated.load(Ordering::Relaxed),
            deadline_aborts: self.deadline_aborts.load(Ordering::Relaxed),
            shutdown_refusals: self.shutdown_refusals.load(Ordering::Relaxed),
            shutdown: self.shutdown.load(Ordering::SeqCst),
        }
    }
}
