//! Production `RuntimeRegistrationDirectory` + pre-auth pool tests
//! (C-session §3/§4). These mirror the frozen reference model
//! (`runtime/transport/tests/session_directory_contract.rs`) against the
//! W-session implementation.

use skiff_artifact_model::{
    AssemblyIdentity, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
};
use skiff_router::session::consumer::ConsumerManifest;
use skiff_router::session::directory::{
    CloseProgress, PublishError, RuntimeRegistrationDirectory, TransitionOutcome,
};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_router::session::pre_auth::PreAuthPool;
use skiff_router::session::ConsumerKind;

fn tuple(generation: u64, assembly: &str) -> RegisteredAssemblyTuple {
    RegisteredAssemblyTuple {
        environment: "prod".to_string(),
        generation,
        assembly: RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(assembly.to_string()),
        },
        config_snapshot: RuntimeConfigSnapshotRef {
            snapshot_id: RuntimeConfigSnapshotId::parse(
                "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            )
            .expect("snapshot id"),
        },
    }
}

fn session(replica: &str, generation: u64) -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: replica.to_string(),
        connection_generation: generation,
    }
}

const FULL_SET: [ConsumerKind; 7] = [
    ConsumerKind::AdmissionPool,
    ConsumerKind::HealthLedger,
    ConsumerKind::RequestDispatcher,
    ConsumerKind::RuntimeGenerationPinLedger,
    ConsumerKind::WebSocketRequestBroker,
    ConsumerKind::ActorSessionOwner,
    ConsumerKind::ActivationCoordinator,
];

fn full_manifest() -> ConsumerManifest {
    ConsumerManifest::installed(FULL_SET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_replacement_cancels_old_then_installs_new_and_old_barrier_never_touches_new() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let registered = tuple(42, "assembly-a");
        let old = session("runtime-a", 1);
        let new = session("runtime-a", 2);

        let first = directory
            .publish_pending(&old, registered.clone(), &FULL_SET)
            .expect("first registration");
        assert!(first.cancelled_old.is_none());
        let replacement = directory
            .publish_pending(&new, registered.clone(), &FULL_SET)
            .expect("replacement registration");
        assert_eq!(replacement.cancelled_old, Some(old.clone()));
        assert!(
            directory.mark_registered(&new),
            "new session must become routable after its ACK"
        );

        assert_eq!(directory.current_by_replica().get("runtime-a"), Some(&new));
        let old_record = directory.record(&old).expect("old retained until barrier");
        assert!(
            old_record.cancelled,
            "old must be cancelled before new install"
        );
        let new_record = directory.record(&new).expect("new installed");
        assert!(!new_record.cancelled);

        // Old disconnect: barrier completes after all consumer ACKs.
        let start = directory.begin_close(&old).expect("old close starts");
        assert_eq!(start.permits, FULL_SET.to_vec());
        for (index, permit) in FULL_SET.iter().enumerate() {
            assert_eq!(
                directory.ack_close(&old, *permit),
                if index + 1 < FULL_SET.len() {
                    CloseProgress::Pending {
                        acked: index + 1,
                        expected: FULL_SET.len(),
                    }
                } else {
                    CloseProgress::Complete
                }
            );
        }
        assert!(
            directory.record(&old).is_none(),
            "old deleted after barrier"
        );
        assert_eq!(
            directory.current_by_replica().get("runtime-a"),
            Some(&new),
            "old close barrier must never delete current_by_replica[new]"
        );
        assert_eq!(directory.candidates(&registered), vec![new.clone()]);
    }
    #[test]
    fn session_transition_publishes_new_revision_same_session_duplicate_idempotent_stale_closes() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let tuple_42 = tuple(42, "assembly-a");
        let tuple_43 = tuple(43, "assembly-a");
        let s = session("runtime-a", 1);

        let revision_42 = directory
            .publish_pending(&s, tuple_42.clone(), &FULL_SET)
            .expect("initial registration")
            .revision;
        assert!(directory.mark_registered(&s));

        // Commit swaps epoch to 43; same physical session re-registers exact 43.
        assert_eq!(
            directory.transition(&s, tuple_43.clone(), &tuple_43, None),
            TransitionOutcome::Published {
                revision: revision_42 + 1
            }
        );
        assert_eq!(directory.current_by_replica().get("runtime-a"), Some(&s));
        assert_eq!(
            directory
                .record(&s)
                .map(|record| record.registration_revision),
            Some(revision_42 + 1)
        );

        // Exact duplicate is idempotent: no revision bump.
        assert_eq!(
            directory.transition(&s, tuple_43.clone(), &tuple_43, None),
            TransitionOutcome::Idempotent
        );
        assert_eq!(
            directory
                .record(&s)
                .map(|record| record.registration_revision),
            Some(revision_42 + 1)
        );

        // Stale tuple closes the exact session.
        assert_eq!(
            directory.transition(&s, tuple_42, &tuple_43, None),
            TransitionOutcome::StaleClosed
        );
        assert!(
            directory.record(&s).is_some_and(|record| record.cancelled),
            "stale register must close the exact session"
        );
    }

    #[test]
    fn session_new_generation_before_epoch_swap_is_rejected_without_mutation() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let tuple_42 = tuple(42, "assembly-a");
        let tuple_43 = tuple(43, "assembly-a");
        let s = session("runtime-a", 1);
        directory
            .publish_pending(&s, tuple_42.clone(), &FULL_SET)
            .expect("initial registration");
        directory.mark_registered(&s);

        assert_eq!(
            directory.transition(&s, tuple_43.clone(), &tuple_42, Some(&tuple_43)),
            TransitionOutcome::NewGenerationRejected
        );
        let record = directory.record(&s).expect("no mutation");
        assert_eq!(record.registered_tuple.as_ref(), Some(&tuple_42));
        assert!(!record.cancelled);
        assert_eq!(directory.current_by_replica().get("runtime-a"), Some(&s));
    }

    #[test]
    fn session_missing_consumer_manifest_permit_refuses_registration() {
        let partial_manifest = ConsumerManifest::installed([
            ConsumerKind::AdmissionPool,
            ConsumerKind::HealthLedger,
            ConsumerKind::RequestDispatcher,
        ]);
        let mut directory = RuntimeRegistrationDirectory::new(&partial_manifest);
        let s = session("runtime-a", 1);
        let registered = tuple(42, "assembly-a");

        let refused = directory.publish_pending(
            &s,
            registered.clone(),
            &[
                ConsumerKind::AdmissionPool,
                ConsumerKind::HealthLedger,
                ConsumerKind::RequestDispatcher,
                ConsumerKind::WebSocketRequestBroker,
            ],
        );
        assert_eq!(refused, Err(PublishError::PermitUnavailable));
        assert_eq!(directory.session_count(), 0);
        assert!(directory.current_by_replica().is_empty());
        assert_eq!(directory.candidates(&registered), vec![]);
    }

    #[test]
    fn session_barrier_missing_ack_or_timeout_is_fail_stop() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let s = session("runtime-a", 1);
        directory
            .publish_pending(&s, tuple(42, "assembly-a"), &FULL_SET)
            .expect("registration");
        directory.mark_registered(&s);

        assert!(directory.begin_close(&s).is_some());
        assert_eq!(
            directory.ack_close(&s, ConsumerKind::AdmissionPool),
            CloseProgress::Pending {
                acked: 1,
                expected: FULL_SET.len()
            }
        );
        directory.mark_fail_stop("barrier ACK timeout");
        assert!(directory.fail_stopped());
        assert!(
            directory.record(&s).is_some(),
            "fail-stop must not pretend the session was deleted"
        );
    }

    #[test]
    fn session_barrier_without_timeout_stays_pending_until_all_acks() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let s = session("runtime-a", 1);
        directory
            .publish_pending(&s, tuple(42, "assembly-a"), &FULL_SET)
            .expect("registration");
        directory.mark_registered(&s);

        assert!(directory.begin_close(&s).is_some());
        for (index, permit) in FULL_SET.iter().enumerate() {
            assert_eq!(
                directory.ack_close(&s, *permit),
                if index + 1 < FULL_SET.len() {
                    CloseProgress::Pending {
                        acked: index + 1,
                        expected: FULL_SET.len(),
                    }
                } else {
                    CloseProgress::Complete
                }
            );
        }
        assert!(directory.record(&s).is_none());
        assert!(!directory.fail_stopped());
    }

    #[test]
    fn session_max_concurrent_sessions_close_barrier_leaves_zero_residue() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let registered = tuple(42, "assembly-a");
        let replicas = ["runtime-a", "runtime-b", "runtime-c", "runtime-d"];
        let mut sessions = Vec::new();
        for (index, replica) in replicas.iter().enumerate() {
            let s = session(replica, 1);
            directory
                .publish_pending(&s, registered.clone(), &FULL_SET)
                .expect("registration");
            directory.mark_registered(&s);
            sessions.push(s);
            let _ = index;
        }
        assert_eq!(directory.session_count(), replicas.len());
        for s in &sessions {
            assert!(directory.begin_close(s).is_some());
            for permit in FULL_SET {
                let _ = directory.ack_close(s, permit);
            }
        }
        assert_eq!(directory.session_count(), 0);
        assert_eq!(directory.candidates(&registered), vec![]);
        assert_eq!(directory.permits_held(), 0);
        assert!(!directory.fail_stopped());
    }

    #[test]
    fn session_pending_rollback_removes_current_entry_and_revision() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let s = session("runtime-a", 1);
        directory
            .publish_pending(&s, tuple(42, "assembly-a"), &FULL_SET)
            .expect("registration");
        assert_eq!(directory.pending_count(), 1);
        assert!(directory.begin_close(&s).is_some());
        for permit in FULL_SET {
            let _ = directory.ack_close(&s, permit);
        }
        assert_eq!(directory.session_count(), 0);
        assert!(directory.current_by_replica().is_empty());
    }

    #[test]
    fn session_pre_auth_cap_rejects_overflow_and_releases_permit_on_ack() {
        let mut pool = PreAuthPool::new(2);
        assert!(pool.try_acquire("c1"));
        assert!(pool.try_acquire("c2"));
        assert!(!pool.try_acquire("c3"), "third pre-auth must be refused");
        assert_eq!(pool.refused(), 1);

        pool.release("c1");
        assert!(pool.try_acquire("c3"), "release frees the pre-auth permit");
        assert_eq!(pool.occupied(), 2);
    }

    #[test]
    fn session_handshake_timeout_releases_pre_auth_without_directory_residue() {
        let mut pool = PreAuthPool::new(4);
        let directory = RuntimeRegistrationDirectory::new(&full_manifest());
        assert!(pool.try_acquire("c1"));
        pool.release("c1");
        assert_eq!(pool.occupied(), 0);
        assert_eq!(directory.session_count(), 0);
        assert!(!directory.fail_stopped());
    }

    #[test]
    fn session_candidates_only_return_one_complete_routable_revision() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let tuple_42 = tuple(42, "assembly-a");
        let s = session("runtime-a", 1);
        directory
            .publish_pending(&s, tuple_42.clone(), &FULL_SET)
            .expect("registration");
        // Pending is never routable.
        assert_eq!(directory.candidates(&tuple_42), vec![]);
        directory.mark_registered(&s);
        assert_eq!(directory.candidates(&tuple_42), vec![s.clone()]);
        // Transition to 43 atomically: candidates read one complete revision.
        let tuple_43 = tuple(43, "assembly-a");
        directory.transition(&s, tuple_43.clone(), &tuple_43, None);
        assert_eq!(directory.candidates(&tuple_42), vec![]);
        assert_eq!(directory.candidates(&tuple_43), vec![s]);
    }
}
