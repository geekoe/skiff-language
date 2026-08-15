//! Per-incarnation Actor VM arena state machine.
//!
//! The arena is deliberately heap-neutral. It owns the shared incarnation
//! lifecycle facts that the request heap and scheduler must not reconstruct:
//! exact incarnation/arena epoch, segment active/suspended counts, root pins,
//! committed arena memory, the sole pending-cleanup slot and whole-instance
//! discard. A concrete Actor executor supplies the actual `RequestHeap`
//! behind this arena; this module never allocates VM values or inspects
//! `ValueSlot` bytes.

use std::{
    fmt,
    num::NonZeroU64,
    sync::{Arc, Mutex, MutexGuard},
};

/// Exact identity of one Actor VM arena incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActorVmArenaId(NonZeroU64);

impl ActorVmArenaId {
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub fn try_new(value: u64) -> Option<Self> {
        Some(Self(NonZeroU64::new(value)?))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Epoch of the physical arena backing one Actor incarnation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActorVmArenaEpoch(NonZeroU64);

impl ActorVmArenaEpoch {
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub fn try_new(value: u64) -> Option<Self> {
        Some(Self(NonZeroU64::new(value)?))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Opaque stable root identity inside one Actor arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActorVmArenaRootId(NonZeroU64);

impl ActorVmArenaRootId {
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    pub fn try_new(value: u64) -> Option<Self> {
        Some(Self(NonZeroU64::new(value)?))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// Structured failure from one Actor arena lifecycle transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ActorVmArenaError {
    #[error(
        "actor arena hard cap exceeded: hard_cap {hard_cap}, committed {committed}, requested {requested}"
    )]
    HardCapExceeded {
        hard_cap: usize,
        committed: usize,
        requested: usize,
    },
    #[error("actor arena is discarded")]
    Discarded,
    #[error("actor arena cannot discard with live segments or roots: active {active}, suspended {suspended}, roots {roots}")]
    LiveState {
        active: u64,
        suspended: u64,
        roots: u64,
    },
    #[error("actor arena pending cleanup slot is already occupied")]
    PendingCleanupOccupied,
    #[error("actor arena segment lease is already released")]
    LeaseAlreadyReleased,
    #[error("actor arena root lease is already released")]
    RootAlreadyReleased,
}

/// Immutable observable state of one Actor VM arena.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActorVmArenaSnapshot {
    pub id: ActorVmArenaId,
    pub incarnation: u64,
    pub epoch: ActorVmArenaEpoch,
    pub hard_cap: usize,
    pub committed: usize,
    pub active_segments: u64,
    pub suspended_segments: u64,
    pub roots: u64,
    pub pending_cleanup: bool,
    pub discarded: bool,
}

struct ActorVmArenaState {
    id: ActorVmArenaId,
    incarnation: u64,
    epoch: ActorVmArenaEpoch,
    hard_cap: usize,
    committed: usize,
    active_segments: u64,
    suspended_segments: u64,
    roots: u64,
    pending_cleanup: Option<Box<dyn FnOnce() + Send>>,
    discarded: bool,
}

impl ActorVmArenaState {
    fn snapshot(&self) -> ActorVmArenaSnapshot {
        ActorVmArenaSnapshot {
            id: self.id,
            incarnation: self.incarnation,
            epoch: self.epoch,
            hard_cap: self.hard_cap,
            committed: self.committed,
            active_segments: self.active_segments,
            suspended_segments: self.suspended_segments,
            roots: self.roots,
            pending_cleanup: self.pending_cleanup.is_some(),
            discarded: self.discarded,
        }
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Cloneable shared handle to one Actor incarnation arena.
///
/// All clones share the same hard cap, segment counters, root pins, pending
/// cleanup and discard state. The arena never owns a concrete VM heap; the
/// concrete Actor executor keeps that heap behind the same incarnation fence.
#[derive(Clone)]
pub struct ActorVmArena {
    inner: Arc<Mutex<ActorVmArenaState>>,
}

impl ActorVmArena {
    pub fn new(
        id: ActorVmArenaId,
        incarnation: u64,
        epoch: ActorVmArenaEpoch,
        hard_cap: usize,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ActorVmArenaState {
                id,
                incarnation,
                epoch,
                hard_cap,
                committed: 0,
                active_segments: 0,
                suspended_segments: 0,
                roots: 0,
                pending_cleanup: None,
                discarded: false,
            })),
        }
    }

    /// Reserves arena memory against the per-incarnation hard cap.
    ///
    /// The returned lease is affine: dropping it releases the committed amount
    /// exactly once. Arena growth is never charged to a caller request ledger
    /// by this module.
    pub fn reserve(&self, amount: usize) -> Result<ActorVmArenaMemoryLease, ActorVmArenaError> {
        let mut state = lock_unpoisoned(&self.inner);
        if state.discarded {
            return Err(ActorVmArenaError::Discarded);
        }
        let committed =
            state
                .committed
                .checked_add(amount)
                .ok_or(ActorVmArenaError::HardCapExceeded {
                    hard_cap: state.hard_cap,
                    committed: state.committed,
                    requested: amount,
                })?;
        if committed > state.hard_cap {
            return Err(ActorVmArenaError::HardCapExceeded {
                hard_cap: state.hard_cap,
                committed: state.committed,
                requested: amount,
            });
        }
        state.committed = committed;
        Ok(ActorVmArenaMemoryLease {
            arena: self.clone(),
            amount,
            released: false,
        })
    }

    /// Acquires one synchronous Actor method segment lease.
    ///
    /// A segment lease is released by dropping it or explicitly converted to
    /// the suspended-continuation state on an actual `Pending`.
    pub fn acquire_segment(&self) -> Result<ActorSegmentLease, ActorVmArenaError> {
        let mut state = lock_unpoisoned(&self.inner);
        if state.discarded {
            return Err(ActorVmArenaError::Discarded);
        }
        state.active_segments = state
            .active_segments
            .checked_add(1)
            .ok_or(ActorVmArenaError::Discarded)?;
        Ok(ActorSegmentLease {
            arena: self.clone(),
            released: false,
        })
    }

    /// Pins one stable Actor arena root for the lifetime of the returned lease.
    pub fn pin_root(
        &self,
        root: ActorVmArenaRootId,
    ) -> Result<ActorVmArenaRootLease, ActorVmArenaError> {
        let mut state = lock_unpoisoned(&self.inner);
        if state.discarded {
            return Err(ActorVmArenaError::Discarded);
        }
        state.roots = state
            .roots
            .checked_add(1)
            .ok_or(ActorVmArenaError::Discarded)?;
        Ok(ActorVmArenaRootLease {
            arena: self.clone(),
            root,
            released: false,
        })
    }

    /// Attaches the single pending cleanup authority for this incarnation.
    ///
    /// A second attachment fails closed. Callers must either run the cleanup
    /// through [`Self::discard`] or take it after an exact successful commit.
    pub fn attach_pending_cleanup(
        &self,
        cleanup: Box<dyn FnOnce() + Send>,
    ) -> Result<(), ActorVmArenaError> {
        let mut state = lock_unpoisoned(&self.inner);
        if state.discarded {
            return Err(ActorVmArenaError::Discarded);
        }
        if state.pending_cleanup.is_some() {
            return Err(ActorVmArenaError::PendingCleanupOccupied);
        }
        state.pending_cleanup = Some(cleanup);
        Ok(())
    }

    /// Removes the pending cleanup for an exact successful path.
    pub fn take_pending_cleanup(&self) -> Option<Box<dyn FnOnce() + Send>> {
        lock_unpoisoned(&self.inner).pending_cleanup.take()
    }

    /// Whole-instance discard: bounded and quiescence-checked.
    ///
    /// The arena may not have active or suspended segments or transient root
    /// pins. The pending cleanup, if any, is run exactly once as part of the
    /// discard transition.
    pub fn discard(&self) -> Result<(), ActorVmArenaError> {
        let mut state = lock_unpoisoned(&self.inner);
        if state.discarded {
            return Err(ActorVmArenaError::Discarded);
        }
        if state.active_segments != 0 || state.suspended_segments != 0 || state.roots != 0 {
            return Err(ActorVmArenaError::LiveState {
                active: state.active_segments,
                suspended: state.suspended_segments,
                roots: state.roots,
            });
        }
        state.discarded = true;
        let cleanup = state.pending_cleanup.take();
        drop(state);
        if let Some(cleanup) = cleanup {
            cleanup();
        }
        Ok(())
    }

    pub fn snapshot(&self) -> ActorVmArenaSnapshot {
        lock_unpoisoned(&self.inner).snapshot()
    }

    fn release_memory(&self, amount: usize) {
        let mut state = lock_unpoisoned(&self.inner);
        state.committed = state
            .committed
            .checked_sub(amount)
            .expect("actor arena memory lease releases a committed amount");
    }

    fn release_segment(&self) {
        let mut state = lock_unpoisoned(&self.inner);
        state.active_segments = state
            .active_segments
            .checked_sub(1)
            .expect("actor segment lease releases an active segment");
    }

    fn suspend_segment(&self) {
        let mut state = lock_unpoisoned(&self.inner);
        state.active_segments = state
            .active_segments
            .checked_sub(1)
            .expect("actor segment lease releases an active segment");
        state.suspended_segments = state
            .suspended_segments
            .checked_add(1)
            .expect("actor suspended segment counter must fit u64");
    }

    fn resume_segment(&self) {
        let mut state = lock_unpoisoned(&self.inner);
        state.suspended_segments = state
            .suspended_segments
            .checked_sub(1)
            .expect("actor suspended continuation lease releases a suspended segment");
        state.active_segments = state
            .active_segments
            .checked_add(1)
            .expect("actor active segment counter must fit u64");
    }

    fn release_suspended_segment(&self) {
        let mut state = lock_unpoisoned(&self.inner);
        state.suspended_segments = state
            .suspended_segments
            .checked_sub(1)
            .expect("actor suspended continuation lease releases a suspended segment");
    }

    fn release_root(&self) {
        let mut state = lock_unpoisoned(&self.inner);
        state.roots = state
            .roots
            .checked_sub(1)
            .expect("actor root lease releases a pinned root");
    }
}

impl fmt::Debug for ActorVmArena {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ActorVmArena")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

/// Affine committed memory lease owned by one Actor arena.
#[must_use = "arena memory must be released exactly once"]
pub struct ActorVmArenaMemoryLease {
    arena: ActorVmArena,
    amount: usize,
    released: bool,
}

impl ActorVmArenaMemoryLease {
    pub const fn amount(&self) -> usize {
        self.amount
    }

    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.arena.release_memory(self.amount);
    }
}

impl Drop for ActorVmArenaMemoryLease {
    fn drop(&mut self) {
        self.release_inner();
    }
}

/// One active synchronous Actor method segment.
#[must_use = "an actor segment lease must be released or suspended exactly once"]
pub struct ActorSegmentLease {
    arena: ActorVmArena,
    released: bool,
}

impl ActorSegmentLease {
    /// Converts this active segment into the suspended-continuation state.
    ///
    /// This is the only actual-`Pending` transition: the active lease is
    /// consumed and the arena still counts the suspended continuation.
    pub fn suspend(mut self) -> Result<ActorSuspendedContinuationLease, ActorVmArenaError> {
        if self.released {
            return Err(ActorVmArenaError::LeaseAlreadyReleased);
        }
        self.released = true;
        self.arena.suspend_segment();
        Ok(ActorSuspendedContinuationLease {
            arena: self.arena.clone(),
            released: false,
        })
    }

    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.arena.release_segment();
    }
}

impl Drop for ActorSegmentLease {
    fn drop(&mut self) {
        self.release_inner();
    }
}

/// One suspended Actor continuation counted by its arena.
#[must_use = "a suspended continuation must resume or be released exactly once"]
pub struct ActorSuspendedContinuationLease {
    arena: ActorVmArena,
    released: bool,
}

impl ActorSuspendedContinuationLease {
    /// Reacquires the segment lease after an actual-`Pending` resume.
    pub fn resume(mut self) -> Result<ActorSegmentLease, ActorVmArenaError> {
        if self.released {
            return Err(ActorVmArenaError::LeaseAlreadyReleased);
        }
        self.released = true;
        self.arena.resume_segment();
        Ok(ActorSegmentLease {
            arena: self.arena.clone(),
            released: false,
        })
    }

    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.arena.release_suspended_segment();
    }
}

impl Drop for ActorSuspendedContinuationLease {
    fn drop(&mut self) {
        self.release_inner();
    }
}

/// Affine pin for one stable Actor arena root.
#[must_use = "an arena root pin must be released exactly once"]
pub struct ActorVmArenaRootLease {
    arena: ActorVmArena,
    root: ActorVmArenaRootId,
    released: bool,
}

impl ActorVmArenaRootLease {
    pub const fn root(&self) -> ActorVmArenaRootId {
        self.root
    }

    pub fn release(mut self) {
        self.release_inner();
    }

    fn release_inner(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        self.arena.release_root();
    }
}

impl Drop for ActorVmArenaRootLease {
    fn drop(&mut self) {
        self.release_inner();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    use super::*;

    fn arena() -> ActorVmArena {
        ActorVmArena::new(
            ActorVmArenaId::try_new(1).unwrap(),
            7,
            ActorVmArenaEpoch::try_new(3).unwrap(),
            64,
        )
    }

    #[test]
    fn arena_reservation_is_bounded_by_per_incarnation_cap() {
        let arena = arena();
        let first = arena.reserve(32).expect("first reservation fits");
        assert!(matches!(
            arena.reserve(33),
            Err(ActorVmArenaError::HardCapExceeded {
                hard_cap: 64,
                committed: 32,
                requested: 33,
            })
        ));
        drop(first);
        assert_eq!(arena.snapshot().committed, 0);
    }

    #[test]
    fn segment_leases_release_active_count_and_suspend_on_pending() {
        let arena = arena();
        let active = arena.acquire_segment().expect("acquire");
        assert_eq!(arena.snapshot().active_segments, 1);
        let suspended = active.suspend().expect("actual pending suspends");
        assert_eq!(arena.snapshot().active_segments, 0);
        assert_eq!(arena.snapshot().suspended_segments, 1);
        let resumed = suspended.resume().expect("resume reacquires");
        assert_eq!(arena.snapshot().active_segments, 1);
        resumed.release();
        assert_eq!(arena.snapshot().active_segments, 0);
        assert_eq!(arena.snapshot().suspended_segments, 0);
    }

    #[test]
    fn discard_requires_quiescence_and_runs_pending_cleanup_once() {
        let arena = arena();
        let ran = Arc::new(AtomicBool::new(false));
        let trigger = Arc::clone(&ran);
        arena
            .attach_pending_cleanup(Box::new(move || trigger.store(true, Ordering::SeqCst)))
            .expect("one cleanup slot");
        assert_eq!(
            arena.attach_pending_cleanup(Box::new(|| {})),
            Err(ActorVmArenaError::PendingCleanupOccupied)
        );

        let root = arena
            .pin_root(ActorVmArenaRootId::try_new(11).unwrap())
            .expect("pin root");
        assert_eq!(
            arena.discard(),
            Err(ActorVmArenaError::LiveState {
                active: 0,
                suspended: 0,
                roots: 1,
            })
        );
        root.release();
        arena.discard().expect("quiescent arena discards");
        assert!(ran.load(Ordering::SeqCst));
        assert!(arena.snapshot().discarded);
    }

    #[test]
    fn pending_cleanup_can_be_taken_for_exact_success() {
        let arena = arena();
        arena
            .attach_pending_cleanup(Box::new(|| {}))
            .expect("attach");
        assert!(arena.take_pending_cleanup().is_some());
        assert!(!arena.snapshot().pending_cleanup);
    }
}
