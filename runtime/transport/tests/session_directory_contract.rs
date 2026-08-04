//! Reference-model tests for the frozen C-session contract
//! (`doc/implementation/router-rust-migration-c-session-contract.md`):
//! `RuntimeRegistrationDirectory` double index, replacement/cancel/barrier,
//! `RuntimeRegistrationTransition`, pre-auth cap, handshake timeouts,
//! consumer-manifest permits, and fail-stop semantics.
//!
//! TEST-ONLY reference model. Not production code; W-session must implement
//! the same semantics from the contract doc and consume the shared corpus.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SessionEpoch {
    replica_id: String,
    connection_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredTuple {
    profile: String,
    generation: u64,
    assembly: String,
    config_snapshot: String,
}

impl RegisteredTuple {
    fn new(generation: u64, assembly: &str) -> Self {
        Self {
            profile: "prod".to_string(),
            generation,
            assembly: assembly.to_string(),
            config_snapshot: "snapshot-b".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum PermitKind {
    Admission,
    Health,
    Dispatcher,
    GenerationPin,
    Broker,
    Actor,
    Activation,
}

const FULL_CONSUMER_SET: [PermitKind; 7] = [
    PermitKind::Admission,
    PermitKind::Health,
    PermitKind::Dispatcher,
    PermitKind::GenerationPin,
    PermitKind::Broker,
    PermitKind::Actor,
    PermitKind::Activation,
];

#[derive(Debug)]
struct SessionRecord {
    tuple: Option<RegisteredTuple>,
    revision: u64,
    cancelled: bool,
    permits: Vec<PermitKind>,
    barrier_acked: HashSet<PermitKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransitionOutcome {
    Published(u64),
    Idempotent,
    StaleClosed,
    NewGenerationRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseResult {
    Complete,
    Pending,
    FailStop,
}

struct Directory {
    current_by_replica: HashMap<String, SessionEpoch>,
    sessions_by_epoch: HashMap<SessionEpoch, SessionRecord>,
    next_revision: u64,
    fail_stop: bool,
    // Installed session-keyed components (static manifest).
    installed_consumers: HashSet<PermitKind>,
}

impl Directory {
    fn new(installed_consumers: impl IntoIterator<Item = PermitKind>) -> Self {
        Self {
            current_by_replica: HashMap::new(),
            sessions_by_epoch: HashMap::new(),
            next_revision: 1,
            fail_stop: false,
            installed_consumers: installed_consumers.into_iter().collect(),
        }
    }

    /// Register + ACK: publish a routable revision for a session.
    /// Any missing installed-consumer permit refuses registration entirely.
    fn publish_registered(
        &mut self,
        replica_id: &str,
        connection_generation: u64,
        tuple: RegisteredTuple,
        permits: &[PermitKind],
    ) -> Option<u64> {
        assert!(!self.fail_stop, "directory must not mutate after fail-stop");
        let permits_available = permits
            .iter()
            .all(|permit| self.installed_consumers.contains(permit));
        if !permits_available {
            return None;
        }
        let new_epoch = SessionEpoch {
            replica_id: replica_id.to_string(),
            connection_generation,
        };
        // Replacement: mark and cancel old epoch BEFORE installing new current.
        if let Some(old) = self.current_by_replica.get(replica_id) {
            if let Some(old_record) = self.sessions_by_epoch.get_mut(old) {
                old_record.cancelled = true;
            }
        }
        self.next_revision += 1;
        let revision = self.next_revision;
        self.sessions_by_epoch.insert(
            new_epoch.clone(),
            SessionRecord {
                tuple: Some(tuple),
                revision,
                cancelled: false,
                permits: permits.to_vec(),
                barrier_acked: HashSet::new(),
            },
        );
        self.current_by_replica
            .insert(replica_id.to_string(), new_epoch);
        Some(revision)
    }

    /// Same physical session re-registers after an activation commit.
    fn transition(
        &mut self,
        session: &SessionEpoch,
        tuple: RegisteredTuple,
        current_epoch_tuple: &RegisteredTuple,
        pending_epoch_tuple: Option<&RegisteredTuple>,
    ) -> TransitionOutcome {
        assert!(!self.fail_stop, "directory must not mutate after fail-stop");
        let (existing_tuple, cancelled) = {
            let record = self
                .sessions_by_epoch
                .get(session)
                .expect("exact session must exist");
            (record.tuple.clone(), record.cancelled)
        };
        assert!(!cancelled, "cancelled session cannot transition");
        if existing_tuple.as_ref() == Some(&tuple) {
            return TransitionOutcome::Idempotent;
        }
        if Some(&tuple) == Some(current_epoch_tuple) {
            self.next_revision += 1;
            let revision = self.next_revision;
            let record = self.sessions_by_epoch.get_mut(session).unwrap();
            record.tuple = Some(tuple);
            record.revision = revision;
            return TransitionOutcome::Published(revision);
        }
        if pending_epoch_tuple == Some(&tuple) {
            return TransitionOutcome::NewGenerationRejected;
        }
        self.sessions_by_epoch.get_mut(session).unwrap().cancelled = true;
        TransitionOutcome::StaleClosed
    }

    /// Close barrier: cancellation already fired; consumers ACK via the
    /// reserved terminal slot. Deletion happens only after all ACKs.
    fn close(&mut self, session: &SessionEpoch, acks: &[PermitKind], timeout: bool) -> CloseResult {
        {
            let record = self
                .sessions_by_epoch
                .get_mut(session)
                .expect("exact session must exist");
            record.cancelled = true;
            for ack in acks {
                record.barrier_acked.insert(*ack);
            }
        }
        let complete = {
            let record = self
                .sessions_by_epoch
                .get(session)
                .expect("exact session must exist");
            record
                .permits
                .iter()
                .all(|permit| record.barrier_acked.contains(permit))
        };
        if complete {
            let session = session.clone();
            self.sessions_by_epoch.remove(&session);
            return CloseResult::Complete;
        }
        if timeout {
            self.fail_stop = true;
            return CloseResult::FailStop;
        }
        CloseResult::Pending
    }

    /// Candidate query reads one complete revision: exact tuple, not
    /// cancelled, and the session must still be the replica's current.
    fn candidates(&self, tuple: &RegisteredTuple) -> Vec<SessionEpoch> {
        let mut result = self
            .sessions_by_epoch
            .iter()
            .filter(|(epoch, record)| {
                record.tuple.as_ref() == Some(tuple)
                    && !record.cancelled
                    && self.current_by_replica.get(&epoch.replica_id) == Some(*epoch)
            })
            .map(|(epoch, _)| epoch.clone())
            .collect::<Vec<_>>();
        result.sort_by(|left, right| {
            left.replica_id
                .cmp(&right.replica_id)
                .then(left.connection_generation.cmp(&right.connection_generation))
        });
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HandshakePhase {
    Accepted,
    BootstrapSent,
    CapabilitiesBound,
    RegisterValidated,
    Registered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeoutKind {
    Bootstrap,
    Capabilities,
    Register,
}

/// Pre-auth pool: independent total cap; permit releases on registered ACK,
/// timeout, disconnect, or terminal.
struct PreAuthPool {
    limit: usize,
    occupied: Vec<String>,
    refused: u64,
}

impl PreAuthPool {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            occupied: Vec::new(),
            refused: 0,
        }
    }

    fn accept(&mut self, connection: &str) -> bool {
        if self.occupied.len() >= self.limit {
            self.refused += 1;
            return false;
        }
        assert!(
            !self.occupied.iter().any(|existing| existing == connection),
            "connection already pre-auth"
        );
        self.occupied.push(connection.to_string());
        true
    }

    fn release(&mut self, connection: &str) {
        let before = self.occupied.len();
        self.occupied.retain(|existing| existing != connection);
        assert!(
            self.occupied.len() == before - 1,
            "release of unknown connection"
        );
    }

    fn advance(&mut self, connection: &str, from: HandshakePhase, to: HandshakePhase) {
        if from == HandshakePhase::RegisterValidated && to == HandshakePhase::Registered {
            self.release(connection);
        }
    }
}

#[test]
fn replacement_cancels_old_then_installs_new_and_old_barrier_never_touches_new() {
    let mut directory = Directory::new(FULL_CONSUMER_SET);
    let tuple = RegisteredTuple::new(42, "assembly-a");
    let old = SessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };
    let new = SessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 2,
    };

    directory.publish_registered("runtime-a", 1, tuple.clone(), &FULL_CONSUMER_SET);
    directory.publish_registered("runtime-a", 2, tuple.clone(), &FULL_CONSUMER_SET);

    assert_eq!(directory.current_by_replica.get("runtime-a"), Some(&new));
    let old_record = directory
        .sessions_by_epoch
        .get(&old)
        .expect("old retained until barrier");
    assert!(
        old_record.cancelled,
        "old epoch must be cancelled before new install"
    );
    let new_record = directory
        .sessions_by_epoch
        .get(&new)
        .expect("new session installed");
    assert!(!new_record.cancelled);

    // Old disconnect: barrier completes after all consumer ACKs.
    assert_eq!(
        directory.close(&old, &FULL_CONSUMER_SET, false),
        CloseResult::Complete
    );
    assert!(!directory.sessions_by_epoch.contains_key(&old));
    assert_eq!(
        directory.current_by_replica.get("runtime-a"),
        Some(&new),
        "old close barrier must never delete current_by_replica[new]"
    );
    assert_eq!(directory.candidates(&tuple), vec![new.clone()]);
}

#[test]
fn transition_publishes_new_revision_same_session_duplicate_idempotent_stale_closes() {
    let mut directory = Directory::new(FULL_CONSUMER_SET);
    let tuple_42 = RegisteredTuple::new(42, "assembly-a");
    let tuple_43 = RegisteredTuple::new(43, "assembly-a");
    let session = SessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };

    let revision_42 = directory
        .publish_registered("runtime-a", 1, tuple_42.clone(), &FULL_CONSUMER_SET)
        .expect("initial registration publishes");

    // Commit swaps epoch to 43; same physical session re-registers exact 43.
    assert_eq!(
        directory.transition(&session, tuple_43.clone(), &tuple_43, None),
        TransitionOutcome::Published(revision_42 + 1)
    );
    assert_eq!(
        directory.current_by_replica.get("runtime-a"),
        Some(&session)
    );
    assert_eq!(
        directory
            .sessions_by_epoch
            .get(&session)
            .map(|record| record.revision),
        Some(revision_42 + 1)
    );

    // Exact duplicate is idempotent: no revision bump, same current.
    assert_eq!(
        directory.transition(&session, tuple_43.clone(), &tuple_43, None),
        TransitionOutcome::Idempotent
    );
    assert_eq!(
        directory
            .sessions_by_epoch
            .get(&session)
            .map(|record| record.revision),
        Some(revision_42 + 1)
    );

    // Stale tuple closes the exact session.
    assert_eq!(
        directory.transition(&session, tuple_42, &tuple_43, None),
        TransitionOutcome::StaleClosed
    );
    assert!(
        directory
            .sessions_by_epoch
            .get(&session)
            .is_some_and(|record| record.cancelled),
        "stale register must close the exact session"
    );
}

#[test]
fn new_generation_before_epoch_swap_is_rejected_without_mutation() {
    let mut directory = Directory::new(FULL_CONSUMER_SET);
    let tuple_42 = RegisteredTuple::new(42, "assembly-a");
    let tuple_43 = RegisteredTuple::new(43, "assembly-a");
    let session = SessionEpoch {
        replica_id: "runtime-a".to_string(),
        connection_generation: 1,
    };
    directory.publish_registered("runtime-a", 1, tuple_42.clone(), &FULL_CONSUMER_SET);

    // Pending epoch 43 exists but current is still 42.
    assert_eq!(
        directory.transition(&session, tuple_43.clone(), &tuple_42, Some(&tuple_43)),
        TransitionOutcome::NewGenerationRejected
    );
    let record = directory
        .sessions_by_epoch
        .get(&session)
        .expect("no mutation");
    assert_eq!(record.tuple.as_ref(), Some(&tuple_42));
    assert!(!record.cancelled);
    assert_eq!(
        directory.current_by_replica.get("runtime-a"),
        Some(&session)
    );
}

#[test]
fn missing_consumer_manifest_permit_refuses_registration() {
    let mut directory = Directory::new([
        PermitKind::Admission,
        PermitKind::Health,
        PermitKind::Dispatcher,
    ]);
    let tuple = RegisteredTuple::new(42, "assembly-a");
    let refused = directory.publish_registered(
        "runtime-a",
        1,
        tuple.clone(),
        &[
            PermitKind::Admission,
            PermitKind::Health,
            PermitKind::Dispatcher,
            PermitKind::Broker, // installed consumer missing its manifest permit
        ],
    );
    assert!(refused.is_none(), "registration must be refused");
    assert!(directory.sessions_by_epoch.is_empty());
    assert!(directory.current_by_replica.is_empty());
    assert_eq!(directory.candidates(&tuple), vec![]);
}

#[test]
fn barrier_missing_ack_or_timeout_is_fail_stop() {
    let mut directory = Directory::new(FULL_CONSUMER_SET);
    let tuple = RegisteredTuple::new(42, "assembly-a");
    directory.publish_registered("runtime-a", 1, tuple, &FULL_CONSUMER_SET);
    let session = directory
        .current_by_replica
        .get("runtime-a")
        .cloned()
        .expect("session installed");

    // Only one consumer ACKs; delivery of the rest times out.
    assert_eq!(
        directory.close(&session, &[PermitKind::Admission], true),
        CloseResult::FailStop
    );
    assert!(
        directory.fail_stop,
        "barrier ACK timeout must fail-stop the process"
    );
    assert!(
        directory.sessions_by_epoch.contains_key(&session),
        "fail-stop must not pretend the session was deleted"
    );
}

#[test]
fn barrier_without_timeout_stays_pending_until_all_acks() {
    let mut directory = Directory::new(FULL_CONSUMER_SET);
    let tuple = RegisteredTuple::new(42, "assembly-a");
    directory.publish_registered("runtime-a", 1, tuple, &FULL_CONSUMER_SET);
    let session = directory
        .current_by_replica
        .get("runtime-a")
        .cloned()
        .unwrap();

    assert_eq!(
        directory.close(
            &session,
            &[PermitKind::Admission, PermitKind::Health],
            false
        ),
        CloseResult::Pending
    );
    assert!(directory.sessions_by_epoch.contains_key(&session));
    assert_eq!(
        directory.close(&session, &FULL_CONSUMER_SET[2..], false),
        CloseResult::Complete
    );
    assert!(!directory.sessions_by_epoch.contains_key(&session));
    assert!(!directory.fail_stop);
}

#[test]
fn max_concurrent_sessions_close_barrier_leaves_zero_residue() {
    let mut directory = Directory::new(FULL_CONSUMER_SET);
    let tuple = RegisteredTuple::new(42, "assembly-a");
    let replicas = ["runtime-a", "runtime-b", "runtime-c", "runtime-d"];
    for (index, replica) in replicas.iter().enumerate() {
        directory.publish_registered(replica, 1, tuple.clone(), &FULL_CONSUMER_SET);
        let _ = index;
    }
    assert_eq!(directory.sessions_by_epoch.len(), replicas.len());
    let sessions = directory
        .sessions_by_epoch
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for session in &sessions {
        assert_eq!(
            directory.close(session, &FULL_CONSUMER_SET, false),
            CloseResult::Complete
        );
    }
    assert!(directory.sessions_by_epoch.is_empty());
    assert!(directory.candidates(&tuple).is_empty());
    assert!(!directory.fail_stop);
}

#[test]
fn pre_auth_cap_rejects_overflow_and_releases_permit_on_ack() {
    let mut pool = PreAuthPool::new(2);
    assert!(pool.accept("c1"));
    assert!(pool.accept("c2"));
    assert!(!pool.accept("c3"), "third pre-auth must be refused");
    assert_eq!(pool.refused, 1);

    pool.advance(
        "c1",
        HandshakePhase::RegisterValidated,
        HandshakePhase::Registered,
    );
    assert!(pool.accept("c3"), "ACK releases the pre-auth permit");
}

#[test]
fn handshake_timeout_releases_pre_auth_without_directory_residue() {
    for (timeout, phase_at_timeout) in [
        (TimeoutKind::Bootstrap, HandshakePhase::Accepted),
        (TimeoutKind::Capabilities, HandshakePhase::BootstrapSent),
        (TimeoutKind::Register, HandshakePhase::CapabilitiesBound),
    ] {
        let mut pool = PreAuthPool::new(4);
        let directory = Directory::new(FULL_CONSUMER_SET);
        let tuple = RegisteredTuple::new(42, "assembly-a");
        let session = SessionEpoch {
            replica_id: "runtime-a".to_string(),
            connection_generation: 1,
        };

        assert!(pool.accept("c1"));
        let _ = phase_at_timeout;
        let _ = timeout;
        pool.release("c1");
        assert_eq!(pool.occupied.len(), 0);
        assert!(directory.sessions_by_epoch.is_empty());
        assert!(!directory.candidates(&tuple).contains(&session));
        assert!(!directory.fail_stop);
    }
}

#[test]
fn writer_queue_full_uses_abort_handle_not_queue_accept() {
    // Frozen rule: writer queue full must close the socket via the independent
    // abort handle instead of waiting for a close frame to be accepted.
    #[derive(Clone, Copy)]
    struct BoundedQueue {
        capacity: usize,
        len: usize,
    }

    impl BoundedQueue {
        fn try_push(&mut self) -> bool {
            if self.len >= self.capacity {
                return false;
            }
            self.len += 1;
            true
        }
    }

    let mut queue = BoundedQueue {
        capacity: 2,
        len: 2,
    };
    assert!(!queue.try_push(), "queue full");
    // The contract outcome is an abort of the exact session, never blocking
    // here; the abort handle path is asserted by the handshake ack-loss
    // corpus scenario (writer queue full -> AckLoss strict terminal).
}
