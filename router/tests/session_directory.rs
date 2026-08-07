//! Production `RuntimeRegistrationDirectory` + pre-auth pool tests
//! (C-session §3/§4; M4: capabilities-only registration, no registered
//! tuple). These mirror the frozen reference model
//! (`runtime/transport/tests/session_directory_contract.rs`) against the
//! W-session implementation.

use skiff_router::session::consumer::ConsumerManifest;
use skiff_router::session::directory::{
    CloseProgress, PublishError, RegistrationFacts, RuntimeRegistrationDirectory,
};
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::pre_auth::PreAuthPool;
use skiff_router::session::ConsumerKind;

fn session(replica: &str, generation: u64) -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: replica.to_string(),
        connection_generation: generation,
    }
}

const FULL_SET: [ConsumerKind; 5] = [
    ConsumerKind::AdmissionPool,
    ConsumerKind::HealthLedger,
    ConsumerKind::RequestDispatcher,
    ConsumerKind::WebSocketRequestBroker,
    ConsumerKind::ActorSessionOwner,
];

fn full_manifest() -> ConsumerManifest {
    ConsumerManifest::installed(FULL_SET)
}

fn facts(build_ids: &[&str]) -> RegistrationFacts {
    RegistrationFacts {
        registered_build_ids: build_ids.iter().map(|id| id.to_string()).collect(),
        lazy_load: false,
        artifact_root: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_replacement_cancels_old_then_installs_new_and_old_barrier_never_touches_new() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let old = session("runtime-a", 1);
        let new = session("runtime-a", 2);

        let first = directory
            .publish_pending(&old, &FULL_SET)
            .expect("first registration");
        assert!(first.cancelled_old.is_none());
        let replacement = directory
            .publish_pending(&new, &FULL_SET)
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
        assert_eq!(directory.candidates(), vec![new.clone()]);
    }

    #[test]
    fn duplicate_session_epoch_is_rejected() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let s = session("runtime-a", 1);
        directory
            .publish_pending(&s, &FULL_SET)
            .expect("first publish");
        assert!(matches!(
            directory.publish_pending(&s, &FULL_SET),
            Err(PublishError::DuplicateSession)
        ));
    }

    #[test]
    fn missing_installed_consumer_permit_refuses_registration() {
        let manifest = ConsumerManifest::installed([ConsumerKind::HealthLedger]);
        let mut directory = RuntimeRegistrationDirectory::new(&manifest);
        let s = session("runtime-a", 1);
        assert!(matches!(
            directory.publish_pending(&s, &FULL_SET),
            Err(PublishError::PermitUnavailable)
        ));
        assert!(directory.record(&s).is_none());
    }

    #[test]
    fn mark_registered_makes_the_session_routable_and_candidates_follow_build_id() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let s = session("runtime-a", 1);
        directory.publish_pending(&s, &FULL_SET).expect("publish");
        assert!(
            !directory.record(&s).expect("record").routable,
            "pending publish is not routable before the ACK"
        );
        assert!(directory.candidates().is_empty());
        assert!(directory.mark_registered(&s));
        let record = directory.record(&s).expect("record");
        assert!(record.routable);
        assert_eq!(directory.candidates(), vec![s.clone()]);
        directory
            .record_mut(&s)
            .expect("record mut")
            .update_registration_facts(facts(&["build-a"]));
        assert_eq!(
            directory.candidates_by_build_id("build-a", None),
            vec![s.clone()]
        );
        assert!(directory.candidates_by_build_id("build-b", None).is_empty());
    }

    #[test]
    fn candidates_by_build_id_honors_lazy_load_with_matching_artifact_root() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let s = session("runtime-a", 1);
        directory.publish_pending(&s, &FULL_SET).expect("publish");
        directory.mark_registered(&s);
        directory
            .record_mut(&s)
            .expect("record mut")
            .update_registration_facts(RegistrationFacts {
                registered_build_ids: Vec::new(),
                lazy_load: true,
                artifact_root: Some("/shared/artifacts".to_string()),
            });
        assert_eq!(
            directory.candidates_by_build_id("build-x", Some("/shared/artifacts")),
            vec![s.clone()],
            "lazy-load with the shared artifact root qualifies"
        );
        assert!(
            directory
                .candidates_by_build_id("build-x", Some("/other/artifacts"))
                .is_empty(),
            "lazy-load with a mismatched artifact root does not qualify"
        );
    }

    #[test]
    fn close_barrier_returns_permits_to_zero() {
        let mut directory = RuntimeRegistrationDirectory::new(&full_manifest());
        let s = session("runtime-a", 1);
        directory.publish_pending(&s, &FULL_SET).expect("publish");
        directory.mark_registered(&s);
        assert_eq!(directory.permits_held(), FULL_SET.len());
        let start = directory.begin_close(&s).expect("close starts");
        assert_eq!(start.permits, FULL_SET.to_vec());
        for permit in FULL_SET.iter() {
            let _ = directory.ack_close(&s, *permit);
        }
        assert_eq!(directory.permits_held(), 0);
        assert!(directory.record(&s).is_none());
    }

    #[test]
    fn pre_auth_pool_acquire_release_and_refusal() {
        let mut pool = PreAuthPool::new(2);
        assert!(pool.try_acquire("c1"));
        assert!(pool.try_acquire("c2"));
        assert!(!pool.try_acquire("c3"), "limit reached");
        pool.release("c1");
        assert!(pool.try_acquire("c3"));
        assert_eq!(pool.occupied(), 2);
        assert_eq!(pool.refused(), 1);
    }
}
