//! W-routing-query × W-session seam: the stateless query runs against the
//! real `RuntimeRegistrationDirectory` (multi-replica, cancellation,
//! replacement) with build-id based candidate projection (contract v2 §1/§3).
//! M4: registration is capabilities-only — no tuple, no transition.

use std::collections::HashMap;

use skiff_artifact_model::{DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef};
use skiff_router::routing::{
    CandidateQuery, DispatchCapabilities, DispatchMode, RegisteredSessionLease,
    RuntimeCandidateQuery,
};
use skiff_router::session::consumer::ConsumerManifest;
use skiff_router::session::directory::{RegistrationFacts, RuntimeRegistrationDirectory};
use skiff_router::session::identity::RuntimeSessionEpoch;
use skiff_router::session::layer::SessionRegistrationFacts;
use skiff_router::session::ConsumerKind;

const FULL_SET: [ConsumerKind; 6] = [
    ConsumerKind::AdmissionPool,
    ConsumerKind::HealthLedger,
    ConsumerKind::RequestDispatcher,
    ConsumerKind::RuntimeGenerationPinLedger,
    ConsumerKind::WebSocketRequestBroker,
    ConsumerKind::ActorSessionOwner,
];

fn deployment() -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: "example.com/service-1".to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("deployment-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(
            "skiff-deployment-artifact-v4:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
    }
}

fn session(replica: &str, connection_generation: u64) -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: replica.to_string(),
        connection_generation,
    }
}

fn full_capabilities() -> DispatchCapabilities {
    DispatchCapabilities {
        unary: true,
        server_stream: true,
    }
}

fn full_facts() -> SessionRegistrationFacts {
    SessionRegistrationFacts {
        dispatch: full_capabilities(),
        registration: RegistrationFacts {
            registered_build_ids: vec![query_build_id()],
            lazy_load: false,
            artifact_root: None,
        },
    }
}

fn query_build_id() -> String {
    deployment().deployment_artifact_identity.to_string()
}

fn register_and_ack(
    directory: &mut RuntimeRegistrationDirectory,
    session: &RuntimeSessionEpoch,
) {
    directory
        .publish_pending(session, &FULL_SET)
        .expect("registration");
    assert!(
        directory.mark_registered(session),
        "registered ACK must publish the session"
    );
}

fn query() -> CandidateQuery {
    CandidateQuery {
        mode: DispatchMode::Unary,
        build_id: query_build_id(),
    }
}

fn project(
    directory: &RuntimeRegistrationDirectory,
    facts: &HashMap<RuntimeSessionEpoch, SessionRegistrationFacts>,
) -> Vec<RegisteredSessionLease> {
    let view = RuntimeCandidateQuery::snapshot_directory_view(directory, facts, None);
    assert_eq!(
        view.revision, None,
        "production snapshot uses per-session revision semantics"
    );
    RuntimeCandidateQuery.query(&view, &query())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_query_directory_seam_projects_all_exact_replicas_deterministically() {
        let mut directory =
            RuntimeRegistrationDirectory::new(&ConsumerManifest::installed(FULL_SET));
        let a = session("runtime-a", 1);
        let b = session("runtime-b", 3);
        register_and_ack(&mut directory, &a);
        register_and_ack(&mut directory, &b);

        let capabilities = HashMap::from([
            (a.clone(), full_facts()),
            (b.clone(), full_facts()),
        ]);
        let leases = project(&directory, &capabilities);

        assert_eq!(
            leases
                .iter()
                .map(|lease| lease.session_epoch.replica_id.as_str())
                .collect::<Vec<_>>(),
            vec!["runtime-a", "runtime-b"],
            "snapshot order is deterministic by replica id"
        );
        for lease in &leases {
            assert_eq!(
                lease.registered_build_ids,
                vec![query_build_id()],
                "M4 lease carries the capabilities-loaded build ids"
            );
            // The directory's revision counter is global: the first publish is
            // revision 1, the second 2. The production snapshot uses per-session
            // revisions (no view-level marker), so both are current candidates.
            let expected_revision = if lease.session_epoch.replica_id == "runtime-a" {
                1
            } else {
                2
            };
            assert_eq!(lease.registration_revision, expected_revision);
            assert!(!lease.cancellation.cancelled);
            assert_eq!(lease.capabilities, full_capabilities());
        }
    }

    #[test]
    fn routing_query_directory_seam_excludes_cancelled_and_pending_sessions() {
        let mut directory =
            RuntimeRegistrationDirectory::new(&ConsumerManifest::installed(FULL_SET));
        let a = session("runtime-a", 1);
        let b = session("runtime-b", 1);
        register_and_ack(&mut directory, &a);
        register_and_ack(&mut directory, &b);
        assert!(
            directory.begin_close(&a).is_some(),
            "close protocol marks cancelled"
        );

        // A pending session (published but not ACKed) is never routable.
        let pending = session("runtime-c", 1);
        directory
            .publish_pending(&pending, &FULL_SET)
            .expect("pending registration");

        let capabilities = HashMap::from([
            (a.clone(), full_facts()),
            (b.clone(), full_facts()),
            (pending.clone(), full_facts()),
        ]);
        let leases = project(&directory, &capabilities);
        assert_eq!(
            leases
                .iter()
                .map(|lease| lease.session_epoch.replica_id.as_str())
                .collect::<Vec<_>>(),
            vec!["runtime-b"]
        );
    }

    #[test]
    fn routing_query_directory_replacement_never_projects_the_cancelled_old_session() {
        let mut directory =
            RuntimeRegistrationDirectory::new(&ConsumerManifest::installed(FULL_SET));
        let old = session("runtime-a", 1);
        let new = session("runtime-a", 2);
        register_and_ack(&mut directory, &old);

        let pending = directory
            .publish_pending(&new, &FULL_SET)
            .expect("replacement registration");
        assert_eq!(pending.cancelled_old, Some(old.clone()));
        assert!(
            directory.mark_registered(&new),
            "replacement becomes routable"
        );

        let capabilities = HashMap::from([
            (old.clone(), full_facts()),
            (new.clone(), full_facts()),
        ]);
        let leases = project(&directory, &capabilities);
        assert_eq!(
            leases
                .iter()
                .map(|lease| lease.session_epoch.connection_generation)
                .collect::<Vec<_>>(),
            vec![2],
            "only the current session of the replica may project"
        );
        assert_eq!(leases[0].session_epoch, new);
    }

    #[test]
    fn routing_query_directory_seam_capabilities_are_injected_by_the_caller() {
        let mut directory =
            RuntimeRegistrationDirectory::new(&ConsumerManifest::installed(FULL_SET));
        let unary_only = session("runtime-a", 1);
        let both = session("runtime-b", 1);
        let unbound = session("runtime-c", 1);
        for s in [&unary_only, &both, &unbound] {
            register_and_ack(&mut directory, s);
        }

        let mut unary_only_facts = full_facts();
        unary_only_facts.dispatch = DispatchCapabilities {
            unary: true,
            server_stream: false,
        };
        let capabilities = HashMap::from([
            (unary_only.clone(), unary_only_facts),
            (both.clone(), full_facts()),
            // `unbound` is deliberately missing: fail closed with empty
            // capabilities (the W-session directory does not retain them).
        ]);

        let unary_leases = project(&directory, &capabilities);
        assert_eq!(
            unary_leases
                .iter()
                .map(|lease| lease.session_epoch.replica_id.as_str())
                .collect::<Vec<_>>(),
            vec!["runtime-a", "runtime-b"]
        );

        let mut stream_query = query();
        stream_query.mode = DispatchMode::ServerStream;
        let view = RuntimeCandidateQuery::snapshot_directory_view(&directory, &capabilities, None);
        let stream_leases = RuntimeCandidateQuery.query(&view, &stream_query);
        assert_eq!(
            stream_leases
                .iter()
                .map(|lease| lease.session_epoch.replica_id.as_str())
                .collect::<Vec<_>>(),
            vec!["runtime-b"]
        );
    }

    #[test]
    fn routing_query_directory_lazy_load_holder_projects_for_any_build_id() {
        let mut directory =
            RuntimeRegistrationDirectory::new(&ConsumerManifest::installed(FULL_SET));
        let lazy = session("runtime-a", 1);
        register_and_ack(&mut directory, &lazy);

        // Lazy-load capability holder sharing the router artifact root is a
        // candidate for an arbitrary (not-yet-loaded) build id (contract v2
        // §1 rule 3).
        let mut lazy_facts = full_facts();
        lazy_facts.registration = RegistrationFacts {
            registered_build_ids: Vec::new(),
            lazy_load: true,
            artifact_root: Some("/shared/artifacts".to_string()),
        };
        let capabilities = HashMap::from([(lazy.clone(), lazy_facts)]);
        let view = RuntimeCandidateQuery::snapshot_directory_view(
            &directory,
            &capabilities,
            Some("/shared/artifacts".to_string()),
        );
        let leases = RuntimeCandidateQuery.query(
            &view,
            &CandidateQuery {
                mode: DispatchMode::Unary,
                build_id: "skiff-deployment-artifact-v4:sha256:9999999999999999999999999999999999999999999999999999999999999999"
                    .to_string(),
            },
        );
        assert_eq!(leases.len(), 1);
        assert!(leases[0].lazy_load);
        assert_eq!(leases[0].artifact_root.as_deref(), Some("/shared/artifacts"));
        assert!(leases[0].registered_build_ids.is_empty());
    }
}
