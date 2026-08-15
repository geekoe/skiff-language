//! Request-level aggregate memory authority for all owner-local heaps.
//!
//! This ledger is intentionally not coupled to an allocator. The VM heap and
//! boundary materializers report reservation/commit/release deltas through
//! this exact accounting surface; the scheduler keeps one ledger per request.

use std::{
    fmt,
    num::NonZeroU64,
    sync::{Arc, Mutex, MutexGuard},
};

use skiff_runtime_model::{
    memory_ledger::{MemoryLease, MemoryLeaseHost, MemoryLeaseToken},
    vm_heap::{HeapDomainId, HeapEpoch},
};
use thiserror::Error;

/// Structured failure from the request memory ledger.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryLedgerError {
    #[error(
        "request memory hard cap exceeded: hard_cap {hard_cap}, reserved {reserved}, committed {committed}, requested {requested}"
    )]
    HardCapExceeded {
        hard_cap: usize,
        reserved: usize,
        committed: usize,
        requested: usize,
    },
    #[error("request memory ledger is terminal")]
    Terminal,
    #[error("request memory ledger release underflow: committed {committed}, amount {amount}")]
    ReleaseUnderflow { committed: usize, amount: usize },
    #[error("request memory ledger has live allocations at terminal: reserved {reserved}, committed {committed}")]
    LiveAllocationsAtTerminal { reserved: usize, committed: usize },
    #[error("request memory ledger domain space is exhausted")]
    DomainSpaceExhausted,
    #[error("request memory ledger epoch space is exhausted")]
    EpochSpaceExhausted,
}

/// Immutable observable state of one request memory ledger.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryLedgerSnapshot {
    pub hard_cap: usize,
    pub reserved: usize,
    pub committed: usize,
    pub peak_reserved: usize,
    pub peak_committed: usize,
    pub peak_total: usize,
    pub terminal: bool,
}

struct LedgerState {
    hard_cap: usize,
    reserved: usize,
    committed: usize,
    peak_reserved: usize,
    peak_committed: usize,
    peak_total: usize,
    terminal: bool,
    next_token: u64,
    next_domain: u64,
    next_epoch: u32,
}

impl LedgerState {
    fn new(hard_cap: usize) -> Self {
        Self {
            hard_cap,
            reserved: 0,
            committed: 0,
            peak_reserved: 0,
            peak_committed: 0,
            peak_total: 0,
            terminal: false,
            next_token: 1,
            next_domain: 1,
            next_epoch: 0,
        }
    }

    fn snapshot(&self) -> MemoryLedgerSnapshot {
        MemoryLedgerSnapshot {
            hard_cap: self.hard_cap,
            reserved: self.reserved,
            committed: self.committed,
            peak_reserved: self.peak_reserved,
            peak_committed: self.peak_committed,
            peak_total: self.peak_total,
            terminal: self.terminal,
        }
    }
}

struct LedgerInner {
    state: Mutex<LedgerState>,
}

impl MemoryLeaseHost for LedgerInner {
    fn release_lease(&self, _token: MemoryLeaseToken, amount: usize) {
        let mut state = lock_unpoisoned(&self.state);
        let Some(committed) = state.committed.checked_sub(amount) else {
            debug_assert!(
                false,
                "request memory ledger released an amount not committed"
            );
            return;
        };
        state.committed = committed;
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Cloneable request-scoped authority for aggregate memory accounting.
///
/// All clones share the same hard cap, peak counters and terminal state. The
/// ledger does not mint a global heap domain; `mint_child_heap` derives both
/// domain and epoch from this request's own monotonic state.
#[derive(Clone)]
pub struct RequestMemoryLedger {
    inner: Arc<LedgerInner>,
}

impl RequestMemoryLedger {
    pub fn new(hard_cap: usize) -> Self {
        Self {
            inner: Arc::new(LedgerInner {
                state: Mutex::new(LedgerState::new(hard_cap)),
            }),
        }
    }

    /// Reserves capacity for one future commit.
    ///
    /// The returned reservation is affine: it must be committed or abandoned.
    pub fn reserve(&self, amount: usize) -> Result<MemoryReservation, MemoryLedgerError> {
        let mut state = self.lock();
        if state.terminal {
            return Err(MemoryLedgerError::Terminal);
        }
        let current_total = state.reserved.checked_add(state.committed).ok_or_else(|| {
            MemoryLedgerError::HardCapExceeded {
                hard_cap: state.hard_cap,
                reserved: state.reserved,
                committed: state.committed,
                requested: amount,
            }
        })?;
        let total = current_total.checked_add(amount).ok_or_else(|| {
            MemoryLedgerError::HardCapExceeded {
                hard_cap: state.hard_cap,
                reserved: state.reserved,
                committed: state.committed,
                requested: amount,
            }
        })?;
        if total > state.hard_cap {
            return Err(MemoryLedgerError::HardCapExceeded {
                hard_cap: state.hard_cap,
                reserved: state.reserved,
                committed: state.committed,
                requested: amount,
            });
        }
        let reserved = state
            .reserved
            .checked_add(amount)
            .expect("current total and amount fit, so reserved plus amount must fit");
        let token = match state.next_token.checked_add(1) {
            Some(next) => {
                let token = MemoryLeaseToken::new(
                    NonZeroU64::new(state.next_token)
                        .expect("request memory ledger token starts at one"),
                );
                state.next_token = next;
                token
            }
            None => {
                return Err(MemoryLedgerError::Terminal);
            }
        };
        state.reserved = reserved;
        state.peak_reserved = state.peak_reserved.max(reserved);
        state.peak_total = state.peak_total.max(total);
        Ok(MemoryReservation {
            ledger: Arc::clone(&self.inner),
            token,
            amount,
            consumed: false,
        })
    }

    /// Releases a directly accounted committed amount.
    pub fn release(&self, amount: usize) -> Result<(), MemoryLedgerError> {
        let mut state = self.lock();
        let committed =
            state
                .committed
                .checked_sub(amount)
                .ok_or(MemoryLedgerError::ReleaseUnderflow {
                    committed: state.committed,
                    amount,
                })?;
        state.committed = committed;
        Ok(())
    }

    /// Mints a child heap identity and a committed memory lease in one bounded
    /// request-scoped operation.
    ///
    /// Epoch zero is the initial owner-local epoch. Whole-heap replacement can
    /// mint a new carrier with a later epoch; this foundational seam keeps the
    /// domain identity request-scoped and monotonic.
    pub fn mint_child_heap(
        &self,
        reserved_bytes: usize,
    ) -> Result<(HeapDomainId, HeapEpoch, MemoryLease), MemoryLedgerError> {
        let (domain, epoch) = {
            let mut state = self.lock();
            if state.terminal {
                return Err(MemoryLedgerError::Terminal);
            }
            let raw_domain = state.next_domain;
            let next_domain = state
                .next_domain
                .checked_add(1)
                .ok_or(MemoryLedgerError::DomainSpaceExhausted)?;
            let domain =
                HeapDomainId::try_new(raw_domain).ok_or(MemoryLedgerError::DomainSpaceExhausted)?;
            let epoch = HeapEpoch::new(state.next_epoch);
            state.next_domain = next_domain;
            state.next_epoch = state
                .next_epoch
                .checked_add(1)
                .ok_or(MemoryLedgerError::EpochSpaceExhausted)?;
            (domain, epoch)
        };
        let reservation = self.reserve(reserved_bytes)?;
        let lease = reservation.commit();
        Ok((domain, epoch, lease))
    }

    pub fn snapshot(&self) -> MemoryLedgerSnapshot {
        self.lock().snapshot()
    }

    /// Marks the request ledger terminal after all memory has been released.
    ///
    /// This is the observable zero point: after a successful call the
    /// snapshot reports `terminal == true` with reserved and committed both
    /// zero. Live allocations fail closed and keep the ledger open.
    pub fn mark_terminal(&self) -> Result<MemoryLedgerSnapshot, MemoryLedgerError> {
        let mut state = self.lock();
        if state.reserved != 0 || state.committed != 0 {
            return Err(MemoryLedgerError::LiveAllocationsAtTerminal {
                reserved: state.reserved,
                committed: state.committed,
            });
        }
        state.terminal = true;
        Ok(state.snapshot())
    }

    fn lock(&self) -> MutexGuard<'_, LedgerState> {
        lock_unpoisoned(&self.inner.state)
    }
}

impl std::fmt::Debug for RequestMemoryLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RequestMemoryLedger")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

/// One validated capacity hold returned by [`RequestMemoryLedger::reserve`].
#[must_use = "a memory reservation must be committed or abandoned"]
pub struct MemoryReservation {
    ledger: Arc<LedgerInner>,
    token: MemoryLeaseToken,
    amount: usize,
    consumed: bool,
}

impl fmt::Debug for MemoryReservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryReservation")
            .field("token", &self.token)
            .field("amount", &self.amount)
            .field("consumed", &self.consumed)
            .finish()
    }
}

impl MemoryReservation {
    pub const fn amount(&self) -> usize {
        self.amount
    }

    /// Converts the reservation into one affine committed lease.
    pub fn commit(mut self) -> MemoryLease {
        debug_assert!(!self.consumed, "a memory reservation commits exactly once");
        self.consumed = true;
        {
            let mut state = lock_unpoisoned(&self.ledger.state);
            debug_assert!(state.reserved >= self.amount);
            state.reserved -= self.amount;
            state.committed = state
                .committed
                .checked_add(self.amount)
                .expect("request memory ledger committed total cannot overflow");
            state.peak_committed = state.peak_committed.max(state.committed);
            let total = state
                .committed
                .checked_add(state.reserved)
                .expect("request memory ledger total cannot overflow");
            state.peak_total = state.peak_total.max(total);
        }
        MemoryLease::new(
            Arc::clone(&self.ledger) as Arc<dyn MemoryLeaseHost>,
            self.token,
            self.amount,
        )
    }

    /// Releases the capacity hold without committing it.
    pub fn abandon(mut self) {
        debug_assert!(!self.consumed, "a memory reservation abandons exactly once");
        self.consumed = true;
        let mut state = lock_unpoisoned(&self.ledger.state);
        debug_assert!(state.reserved >= self.amount);
        state.reserved -= self.amount;
    }
}

impl Drop for MemoryReservation {
    fn drop(&mut self) {
        if self.consumed {
            return;
        }
        let mut state = lock_unpoisoned(&self.ledger.state);
        debug_assert!(state.reserved >= self.amount);
        state.reserved -= self.amount;
    }
}

#[cfg(test)]
mod tests {
    use super::{MemoryLedgerError, RequestMemoryLedger};

    #[test]
    fn reserve_commit_release_tracks_exact_counts_and_peak() {
        let ledger = RequestMemoryLedger::new(100);
        let reservation = ledger.reserve(30).unwrap();
        assert_eq!(ledger.snapshot().reserved, 30);
        let lease = reservation.commit();
        let snapshot = ledger.snapshot();
        assert_eq!(snapshot.reserved, 0);
        assert_eq!(snapshot.committed, 30);
        assert_eq!(snapshot.peak_reserved, 30);
        assert_eq!(snapshot.peak_committed, 30);
        assert_eq!(snapshot.peak_total, 30);

        lease.release();
        assert_eq!(ledger.snapshot().committed, 0);
        assert_eq!(ledger.snapshot().peak_committed, 30);
    }

    #[test]
    fn abandoned_reservation_returns_capacity_without_committing() {
        let ledger = RequestMemoryLedger::new(10);
        let reservation = ledger.reserve(6).unwrap();
        assert_eq!(ledger.snapshot().reserved, 6);
        reservation.abandon();
        let snapshot = ledger.snapshot();
        assert_eq!(snapshot.reserved, 0);
        assert_eq!(snapshot.committed, 0);
        assert_eq!(snapshot.peak_reserved, 6);
    }

    #[test]
    fn dropped_reservation_returns_capacity_without_committing() {
        let ledger = RequestMemoryLedger::new(10);
        drop(ledger.reserve(4).unwrap());
        let snapshot = ledger.snapshot();
        assert_eq!(snapshot.reserved, 0);
        assert_eq!(snapshot.committed, 0);
        assert_eq!(snapshot.peak_total, 4);
    }

    #[test]
    fn hard_cap_rejects_without_mutating_state() {
        let ledger = RequestMemoryLedger::new(5);
        let error = ledger.reserve(6).unwrap_err();
        assert!(matches!(
            error,
            MemoryLedgerError::HardCapExceeded {
                hard_cap: 5,
                reserved: 0,
                committed: 0,
                requested: 6,
            }
        ));
        assert_eq!(ledger.snapshot().reserved, 0);
        assert_eq!(ledger.snapshot().committed, 0);
    }

    #[test]
    fn hard_cap_aggregates_committed_and_reserved() {
        let ledger = RequestMemoryLedger::new(10);
        let lease = ledger.reserve(8).unwrap().commit();
        let error = ledger.reserve(3).unwrap_err();
        assert!(matches!(
            error,
            MemoryLedgerError::HardCapExceeded {
                hard_cap: 10,
                reserved: 0,
                committed: 8,
                requested: 3,
            }
        ));

        let second = ledger.reserve(2).unwrap();
        assert_eq!(ledger.snapshot().peak_total, 10);
        drop(second);
        drop(lease);
        assert_eq!(ledger.snapshot().committed, 0);
        assert_eq!(ledger.snapshot().reserved, 0);
    }

    #[test]
    fn terminal_requires_zero_and_observes_zero() {
        let ledger = RequestMemoryLedger::new(8);
        let reservation = ledger.reserve(4).unwrap();
        let lease = reservation.commit();
        assert!(matches!(
            ledger.mark_terminal(),
            Err(MemoryLedgerError::LiveAllocationsAtTerminal {
                reserved: 0,
                committed: 4,
            })
        ));
        lease.release();
        let terminal = ledger.mark_terminal().unwrap();
        assert!(terminal.terminal);
        assert_eq!(terminal.reserved, 0);
        assert_eq!(terminal.committed, 0);
        assert!(matches!(
            ledger.reserve(1),
            Err(MemoryLedgerError::Terminal)
        ));
    }

    #[test]
    fn mint_child_heap_uses_request_scoped_domain_epoch_and_lease() {
        let ledger = RequestMemoryLedger::new(100);
        let (first_domain, first_epoch, first_lease) = ledger.mint_child_heap(12).unwrap();
        let (second_domain, second_epoch, second_lease) = ledger.mint_child_heap(8).unwrap();

        assert_eq!(first_domain.get(), 1);
        assert_eq!(second_domain.get(), 2);
        assert_eq!(first_epoch.get(), 0);
        assert_eq!(second_epoch.get(), 1);
        assert_eq!(first_lease.amount(), 12);
        assert_eq!(second_lease.amount(), 8);
        assert_eq!(ledger.snapshot().committed, 20);

        drop(first_lease);
        drop(second_lease);
        assert_eq!(ledger.snapshot().committed, 0);
    }
}
