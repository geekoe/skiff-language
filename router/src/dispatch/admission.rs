//! `RuntimeAdmissionPool`: per-session capacity permits and the selection
//! cursor/policy (plan §3.2, C-dispatch §3).
//!
//! Invariant: a reservation/permit is strictly paired with exactly one
//! release (reserve -> terminal or revalidate failure); `permitsHeld` is the
//! sum of per-session in-flight counts and returns to zero when every pending
//! terminates. The pool does not own session truth, request pending or the
//! active routing epoch: it only answers selection/reserve/release questions
//! from typed candidate leases.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::session::identity::RuntimeSessionEpoch;

use super::candidate::RegisteredSessionLease;

/// Frozen admission counters (C-dispatch §7.6 `admission.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdmissionCounters {
    pub releases: u64,
    pub queue_full_rejects: u64,
    pub revalidate_failures: u64,
    pub reselects: u64,
    pub no_candidate_rejects: u64,
    pub duplicate_request_id_rejects: u64,
}

#[derive(Debug)]
struct AdmissionInner {
    max_concurrency: usize,
    in_flight: HashMap<RuntimeSessionEpoch, usize>,
    cursor: usize,
    counters: AdmissionCounters,
}

impl AdmissionInner {
    fn capacity_available(&self, lease: &RegisteredSessionLease) -> bool {
        self.in_flight
            .get(&lease.session_epoch)
            .copied()
            .unwrap_or(0)
            < self.max_concurrency
    }

    fn capacity_available_for(&self, session: &RuntimeSessionEpoch) -> bool {
        self.in_flight.get(session).copied().unwrap_or(0) < self.max_concurrency
    }
}

/// Per-session capacity permit pool (plan §3.2).
#[derive(Clone)]
pub struct RuntimeAdmissionPool {
    inner: Arc<Mutex<AdmissionInner>>,
}

impl RuntimeAdmissionPool {
    pub fn new(max_concurrency: usize) -> Self {
        assert!(
            max_concurrency >= 1,
            "RuntimeAdmissionPool maxConcurrency must be >= 1"
        );
        Self {
            inner: Arc::new(Mutex::new(AdmissionInner {
                max_concurrency,
                in_flight: HashMap::new(),
                cursor: 0,
                counters: AdmissionCounters::default(),
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, AdmissionInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn reserve_locked(
        &self,
        inner: &mut AdmissionInner,
        lease: &RegisteredSessionLease,
    ) -> Reservation {
        let session = lease.session_epoch.clone();
        *inner.in_flight.entry(session.clone()).or_insert(0) += 1;
        Reservation {
            inner: self.inner.clone(),
            session,
            released: false,
        }
    }

    pub fn max_concurrency(&self) -> usize {
        self.lock().max_concurrency
    }

    pub fn permits_held(&self) -> usize {
        self.lock().in_flight.values().sum()
    }

    pub fn in_flight(&self, session: &RuntimeSessionEpoch) -> usize {
        self.lock().in_flight.get(session).copied().unwrap_or(0)
    }

    pub fn cursor(&self) -> usize {
        self.lock().cursor
    }

    pub fn counters(&self) -> AdmissionCounters {
        self.lock().counters
    }

    /// Selects a candidate and reserves one permit.
    ///
    /// Policy (frozen by C-dispatch §3 / the reference machine): `preferred`
    /// is taken when it is a candidate with capacity; otherwise the scan
    /// starts at the round-robin cursor and skips full sessions. The cursor
    /// advances to the next position after the selected candidate.
    pub fn select(
        &self,
        leases: &[RegisteredSessionLease],
        preferred: Option<&RuntimeSessionEpoch>,
    ) -> Option<SelectedLease> {
        let mut inner = self.lock();
        if leases.is_empty() {
            return None;
        }
        if let Some(index) = preferred.and_then(|session| {
            leases
                .iter()
                .position(|lease| &lease.session_epoch == session)
        }) {
            if inner.capacity_available(&leases[index]) {
                inner.cursor = (index + 1) % leases.len();
                let reservation = self.reserve_locked(&mut inner, &leases[index]);
                return Some(SelectedLease {
                    lease: leases[index].clone(),
                    reservation,
                });
            }
        }
        for offset in 0..leases.len() {
            let index = (inner.cursor + offset) % leases.len();
            if inner.capacity_available(&leases[index]) {
                inner.cursor = (index + 1) % leases.len();
                let reservation = self.reserve_locked(&mut inner, &leases[index]);
                return Some(SelectedLease {
                    lease: leases[index].clone(),
                    reservation,
                });
            }
        }
        None
    }

    /// Reselects after a revalidate failure (C-dispatch §3 step 5).
    ///
    /// Scans from the head of the candidate list, skipping the failed session
    /// and full sessions; the cursor intentionally does not advance
    /// (reference-machine semantics).
    pub fn select_after_revalidate_failure(
        &self,
        leases: &[RegisteredSessionLease],
        excluded: &RuntimeSessionEpoch,
    ) -> Option<SelectedLease> {
        let mut inner = self.lock();
        for lease in leases {
            if lease.session_epoch == *excluded {
                continue;
            }
            if inner.capacity_available(lease) {
                let reservation = self.reserve_locked(&mut inner, lease);
                return Some(SelectedLease {
                    lease: lease.clone(),
                    reservation,
                });
            }
        }
        None
    }

    /// Reserves a permit for one exact session (derived function spawn on the
    /// parent session, C-dispatch §5.2).
    pub fn reserve_exact(&self, session: &RuntimeSessionEpoch) -> Option<Reservation> {
        let mut inner = self.lock();
        if !inner.capacity_available_for(session) {
            return None;
        }
        *inner.in_flight.entry(session.clone()).or_insert(0) += 1;
        Some(Reservation {
            inner: self.inner.clone(),
            session: session.clone(),
            released: false,
        })
    }

    pub fn record_queue_full(&self) {
        self.lock().counters.queue_full_rejects += 1;
    }

    pub fn record_revalidate_failure(&self) {
        self.lock().counters.revalidate_failures += 1;
    }

    pub fn record_reselect(&self) {
        self.lock().counters.reselects += 1;
    }

    pub fn record_no_candidate(&self) {
        self.lock().counters.no_candidate_rejects += 1;
    }

    pub fn record_duplicate_request_id(&self) {
        self.lock().counters.duplicate_request_id_rejects += 1;
    }
}

impl fmt::Debug for RuntimeAdmissionPool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.lock();
        let permits_held: usize = inner.in_flight.values().sum();
        formatter
            .debug_struct("RuntimeAdmissionPool")
            .field("max_concurrency", &inner.max_concurrency)
            .field("permits_held", &permits_held)
            .field("cursor", &inner.cursor)
            .field("counters", &inner.counters)
            .finish_non_exhaustive()
    }
}

/// A lease selected by the pool together with its reserved permit.
#[derive(Debug)]
pub struct SelectedLease {
    pub lease: RegisteredSessionLease,
    pub reservation: Reservation,
}

/// Reserved capacity permit before enqueue (C-dispatch §7.2).
///
/// Either `commit()` (pending now holds the permit) or `release()` (revalidate
/// failure / deadline recheck failure). Dropping without either releases as a
/// safety net so a permit can never leak.
#[derive(Debug)]
pub struct Reservation {
    inner: Arc<Mutex<AdmissionInner>>,
    session: RuntimeSessionEpoch,
    released: bool,
}

impl Reservation {
    pub fn session(&self) -> &RuntimeSessionEpoch {
        &self.session
    }

    pub fn commit(mut self) -> Permit {
        self.released = true;
        Permit {
            inner: self.inner.clone(),
            session: self.session.clone(),
            released: false,
        }
    }

    pub fn release(mut self) {
        if self.released {
            return;
        }
        self.released = true;
        release_permit(&self.inner, &self.session);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.released {
            release_permit(&self.inner, &self.session);
        }
    }
}

/// Held capacity permit owned by one pending (C-dispatch §7.2).
///
/// Released exactly once when the pending terminates. Dropping without
/// explicit `release()` is a safety net and still counts the release.
#[derive(Debug)]
pub struct Permit {
    inner: Arc<Mutex<AdmissionInner>>,
    session: RuntimeSessionEpoch,
    released: bool,
}

impl Permit {
    pub fn session(&self) -> &RuntimeSessionEpoch {
        &self.session
    }

    pub fn release(mut self) {
        if self.released {
            return;
        }
        self.released = true;
        release_permit(&self.inner, &self.session);
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if !self.released {
            release_permit(&self.inner, &self.session);
        }
    }
}

fn release_permit(inner: &Arc<Mutex<AdmissionInner>>, session: &RuntimeSessionEpoch) {
    let mut inner = inner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = inner
        .in_flight
        .get_mut(session)
        .expect("permit release must pair with a reserve");
    debug_assert!(*entry >= 1, "permit release must not underflow");
    *entry -= 1;
    if *entry == 0 {
        inner.in_flight.remove(session);
    }
    inner.counters.releases += 1;
}

/// Observable permit ledger (test/integration assertions; not health).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PermitLedger {
    pub permits_held: usize,
    pub releases: u64,
    pub per_session: HashMap<RuntimeSessionEpoch, usize>,
}

impl PermitLedger {
    pub fn from_pool(pool: &RuntimeAdmissionPool) -> Self {
        let inner = pool.lock();
        Self {
            permits_held: inner.in_flight.values().sum(),
            releases: inner.counters.releases,
            per_session: inner.in_flight.clone(),
        }
    }
}
