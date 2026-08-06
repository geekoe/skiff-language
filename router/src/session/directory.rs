//! `RuntimeRegistrationDirectory` (authority design §3.2, C-session §3; M4:
//! capabilities-only registration).
//!
//! Invariant: one replica has exactly one current session; a cancelled
//! session is never selected; the barrier does not delete the exact session
//! before all consumer ACKs; the old finalizer never deletes a replacement;
//! all consumer pending returns to zero on success/error/disconnect/
//! saturation/shutdown.
//!
//! The directory owns session truth only: it never holds sockets, health
//! history, admission permits or active/draining routing eligibility.
//! Operations are short and never cross `.await`. The registered tuple is
//! retired (M4): the registration facts (loaded build ids / lazy-load
//! capability / artifact root) are the only registration identity.

use std::collections::{BTreeSet, HashMap};

use super::consumer::{ConsumerKind, ConsumerManifest};
use super::identity::RuntimeSessionEpoch;

/// Registration facts refreshed from every `runtime.capabilities` frame
/// (integration-contract-v2 §1/§3): the loaded build-id set and the
/// lazy-load advertisement. Each capabilities refresh overwrites the facts;
/// they are the unique registration identity (M4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistrationFacts {
    pub registered_build_ids: Vec<String>,
    pub lazy_load: bool,
    pub artifact_root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRecord {
    pub registration_revision: u64,
    pub routable: bool,
    pub cancelled: bool,
    pub consumer_permits: Vec<ConsumerKind>,
    pub barrier_acked: BTreeSet<ConsumerKind>,
    pub registration_facts: RegistrationFacts,
}

impl SessionRecord {
    fn new(revision: u64, permits: Vec<ConsumerKind>) -> Self {
        Self {
            registration_revision: revision,
            routable: false,
            cancelled: false,
            consumer_permits: permits,
            barrier_acked: BTreeSet::new(),
            registration_facts: RegistrationFacts::default(),
        }
    }

    /// Overwrites the capabilities-refresh registration facts (contract §3:
    /// every refresh replaces the previous facts).
    pub fn update_registration_facts(&mut self, facts: RegistrationFacts) {
        self.registration_facts = facts;
    }

    pub fn barrier_complete(&self) -> bool {
        self.consumer_permits
            .iter()
            .all(|permit| self.barrier_acked.contains(permit))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishPending {
    pub revision: u64,
    /// Old epoch for the same replica, marked cancelled before the new epoch
    /// became current. The caller must drive its close barrier.
    pub cancelled_old: Option<RuntimeSessionEpoch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishError {
    /// A required installed-consumer permit is missing from the manifest:
    /// registration is refused and the session is never published.
    PermitUnavailable,
    /// A record already exists for this exact session epoch.
    DuplicateSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseStart {
    pub permits: Vec<ConsumerKind>,
    pub routable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseProgress {
    Complete,
    Pending { acked: usize, expected: usize },
}

#[derive(Debug)]
pub struct RuntimeRegistrationDirectory {
    current_by_replica: HashMap<String, RuntimeSessionEpoch>,
    sessions_by_epoch: HashMap<RuntimeSessionEpoch, SessionRecord>,
    next_revision: u64,
    installed_consumers: BTreeSet<ConsumerKind>,
    fail_stop: bool,
}

impl RuntimeRegistrationDirectory {
    pub fn new(manifest: &ConsumerManifest) -> Self {
        Self {
            current_by_replica: HashMap::new(),
            sessions_by_epoch: HashMap::new(),
            next_revision: 0,
            installed_consumers: manifest.kinds().collect(),
            fail_stop: false,
        }
    }

    /// Pending publish: the capabilities bound and installed-consumer
    /// permits were acquired, but the record is not routable until
    /// `mark_registered` (registered ACK written). Same-replica replacement
    /// marks and cancels the old epoch BEFORE the new epoch becomes current.
    pub fn publish_pending(
        &mut self,
        session: &RuntimeSessionEpoch,
        permits: &[ConsumerKind],
    ) -> Result<PublishPending, PublishError> {
        debug_assert!(!self.fail_stop, "directory must not mutate after fail-stop");
        if !permits
            .iter()
            .all(|permit| self.installed_consumers.contains(permit))
        {
            return Err(PublishError::PermitUnavailable);
        }
        if self.sessions_by_epoch.contains_key(session) {
            return Err(PublishError::DuplicateSession);
        }

        let cancelled_old = self
            .current_by_replica
            .get(&session.replica_id)
            .filter(|old| **old != *session)
            .cloned();
        if let Some(old) = &cancelled_old {
            if let Some(old_record) = self.sessions_by_epoch.get_mut(old) {
                old_record.cancelled = true;
            }
        }

        self.next_revision += 1;
        let revision = self.next_revision;
        self.sessions_by_epoch.insert(
            session.clone(),
            SessionRecord::new(revision, permits.to_vec()),
        );
        self.current_by_replica
            .insert(session.replica_id.clone(), session.clone());
        Ok(PublishPending {
            revision,
            cancelled_old,
        })
    }

    /// Registered ACK written: pending publish becomes routable (no revision
    /// bump). Returns false when the exact record is gone or cancelled.
    pub fn mark_registered(&mut self, session: &RuntimeSessionEpoch) -> bool {
        let Some(record) = self.sessions_by_epoch.get_mut(session) else {
            return false;
        };
        if record.cancelled {
            return false;
        }
        record.routable = true;
        true
    }

    /// Begin the close protocol: mark cancelled and return the permits whose
    /// ACKs the barrier needs. No record means nothing to close.
    pub fn begin_close(&mut self, session: &RuntimeSessionEpoch) -> Option<CloseStart> {
        let record = self.sessions_by_epoch.get_mut(session)?;
        record.cancelled = true;
        Some(CloseStart {
            permits: record.consumer_permits.clone(),
            routable: record.routable,
        })
    }

    /// Consumer ACK via the reserved terminal slot. The exact record is
    /// deleted only after every permit ACKed. The entry in
    /// `current_by_replica` is removed only when it still points at this
    /// closing session (an old finalizer never deletes a replacement).
    pub fn ack_close(
        &mut self,
        session: &RuntimeSessionEpoch,
        consumer: ConsumerKind,
    ) -> CloseProgress {
        let Some(record) = self.sessions_by_epoch.get_mut(session) else {
            return CloseProgress::Complete;
        };
        record.barrier_acked.insert(consumer);
        if !record.barrier_complete() {
            return CloseProgress::Pending {
                acked: record.barrier_acked.len(),
                expected: record.consumer_permits.len(),
            };
        }
        let session = session.clone();
        self.sessions_by_epoch.remove(&session);
        if self
            .current_by_replica
            .get(&session.replica_id)
            .is_some_and(|current| current == &session)
        {
            self.current_by_replica.remove(&session.replica_id);
        }
        CloseProgress::Complete
    }

    /// Barrier ACK timeout or reserved-slot failure: process fail-stop.
    pub fn mark_fail_stop(&mut self, reason: &str) {
        self.fail_stop = true;
        let _ = reason;
    }

    pub fn fail_stopped(&self) -> bool {
        self.fail_stop
    }

    /// Every routable, non-cancelled, current session.
    pub fn candidates(&self) -> Vec<RuntimeSessionEpoch> {
        let mut sessions = self
            .sessions_by_epoch
            .iter()
            .filter(|(epoch, record)| {
                record.routable
                    && !record.cancelled
                    && self
                        .current_by_replica
                        .get(&epoch.replica_id)
                        .is_some_and(|current| current == *epoch)
            })
            .map(|(epoch, _)| epoch.clone())
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            left.replica_id
                .cmp(&right.replica_id)
                .then(left.connection_generation.cmp(&right.connection_generation))
        });
        sessions
    }

    /// Routable candidates whose registration facts can execute one exact
    /// deployment build id (integration-contract-v2 §1 rule 3): the build id
    /// is in the loaded set OR the session is a lazy-load capability holder
    /// sharing the router artifact root.
    pub fn candidates_by_build_id(
        &self,
        build_id: &str,
        router_artifact_root: Option<&str>,
    ) -> Vec<RuntimeSessionEpoch> {
        let mut sessions = self
            .sessions_by_epoch
            .iter()
            .filter(|(epoch, record)| {
                record.routable
                    && !record.cancelled
                    && self
                        .current_by_replica
                        .get(&epoch.replica_id)
                        .is_some_and(|current| current == *epoch)
                    && (record
                        .registration_facts
                        .registered_build_ids
                        .iter()
                        .any(|id| id == build_id)
                        || (record.registration_facts.lazy_load
                            && record.registration_facts.artifact_root.as_deref()
                                == router_artifact_root))
            })
            .map(|(epoch, _)| epoch.clone())
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            left.replica_id
                .cmp(&right.replica_id)
                .then(left.connection_generation.cmp(&right.connection_generation))
        });
        sessions
    }

    pub fn current_by_replica(&self) -> &HashMap<String, RuntimeSessionEpoch> {
        &self.current_by_replica
    }

    pub fn record(&self, session: &RuntimeSessionEpoch) -> Option<&SessionRecord> {
        self.sessions_by_epoch.get(session)
    }

    pub fn record_mut(&mut self, session: &RuntimeSessionEpoch) -> Option<&mut SessionRecord> {
        self.sessions_by_epoch.get_mut(session)
    }

    pub fn session_count(&self) -> usize {
        self.sessions_by_epoch.len()
    }

    pub fn routable_count(&self) -> usize {
        self.sessions_by_epoch
            .values()
            .filter(|record| record.routable)
            .count()
    }

    pub fn pending_count(&self) -> usize {
        self.sessions_by_epoch
            .values()
            .filter(|record| !record.routable)
            .count()
    }

    pub fn cancelled_count(&self) -> usize {
        self.sessions_by_epoch
            .values()
            .filter(|record| record.cancelled)
            .count()
    }

    pub fn barrier_pending_count(&self) -> usize {
        self.sessions_by_epoch
            .values()
            .filter(|record| record.cancelled && !record.barrier_complete())
            .count()
    }

    pub fn permits_held(&self) -> usize {
        self.sessions_by_epoch
            .values()
            .map(|record| record.consumer_permits.len())
            .sum()
    }
}
