//! W-routing-query × W-session seam: the stateless query runs against the
//! real `RuntimeRegistrationDirectory` (multi-replica, cancellation,
//! transition/replacement) with a real immutable `RoutingEpoch` (delivery
//! obligation 2 of the C-routing-query contract).

use std::collections::HashMap;
use std::sync::Arc;

use skiff_artifact_model::{
    AssemblyIdentity, CanonicalPackageLinkPlan, DeploymentArtifactIdentity, DeploymentRevision,
    RuntimeAssembly, RuntimeAssemblyRef, RuntimeConfigSnapshotId, RuntimeConfigSnapshotRef,
    ServiceDeploymentRef, RUNTIME_ASSEMBLY_SCHEMA_VERSION,
};
use skiff_deployment::projection::actor_routing::{
    ActorRoutingProjection, ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION,
};
use skiff_router::artifact::ActorRoutingCatalog;
use skiff_router::bootstrap::RoutingEpoch;
use skiff_router::routing::{
    CandidateQuery, DispatchCapabilities, DispatchMode, RegisteredSessionLease,
    RuntimeCandidateQuery,
};
use skiff_router::session::consumer::ConsumerManifest;
use skiff_router::session::directory::{RegistrationFacts, RuntimeRegistrationDirectory};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_router::session::layer::SessionRegistrationFacts;
use skiff_router::session::ConsumerKind;
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

const ASSEMBLY: &str = "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SNAPSHOT: &str = "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

const FULL_SET: [ConsumerKind; 7] = [
    ConsumerKind::AdmissionPool,
    ConsumerKind::HealthLedger,
    ConsumerKind::RequestDispatcher,
    ConsumerKind::RuntimeGenerationPinLedger,
    ConsumerKind::WebSocketRequestBroker,
    ConsumerKind::ActorSessionOwner,
    ConsumerKind::ActivationCoordinator,
];

fn snapshot_ref(id: &str) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(id).expect("snapshot id"),
    }
}

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

fn epoch(generation: u64) -> Arc<RoutingEpoch> {
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new(ASSEMBLY),
        roots: Vec::new(),
        resolved_deployments: vec![deployment()],
        resolved_contracts: Vec::new(),
        resolved_packages: Vec::new(),
        package_link_plan: CanonicalPackageLinkPlan {
            code_slots: Vec::new(),
            package_links: Vec::new(),
        },
        service_binding_templates: Vec::new(),
        activation_templates: Vec::new(),
        gateway_ingress: Vec::new(),
    };
    let snapshot =
        RuntimeConfigSnapshot::new("prod", snapshot_ref(SNAPSHOT), Vec::new()).expect("snapshot");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty projection");
    let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)));
    Arc::new(
        RoutingEpoch::new(
            "prod",
            generation,
            Arc::new(assembly),
            Arc::new(snapshot),
            catalog,
        )
        .expect("epoch"),
    )
}

fn tuple(generation: u64) -> RegisteredAssemblyTuple {
    RegisteredAssemblyTuple {
        profile: "prod".to_string(),
        generation,
        assembly: RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(ASSEMBLY),
        },
        config_snapshot: snapshot_ref(SNAPSHOT),
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
    tuple: &RegisteredAssemblyTuple,
) {
    directory
        .publish_pending(session, tuple.clone(), &FULL_SET)
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
        let tuple_42 = tuple(42);
        let a = session("runtime-a", 1);
        let b = session("runtime-b", 3);
        register_and_ack(&mut directory, &a, &tuple_42);
        register_and_ack(&mut directory, &b, &tuple_42);

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
            assert_eq!(lease.exact_registered_tuple, tuple_42);
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
        let tuple_42 = tuple(42);
        let a = session("runtime-a", 1);
        let b = session("runtime-b", 1);
        register_and_ack(&mut directory, &a, &tuple_42);
        register_and_ack(&mut directory, &b, &tuple_42);
        assert!(
            directory.begin_close(&a).is_some(),
            "close protocol marks cancelled"
        );

        // A pending session (published but not ACKed) is never routable.
        let pending = session("runtime-c", 1);
        directory
            .publish_pending(&pending, tuple_42.clone(), &FULL_SET)
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
    fn routing_query_directory_transition_reads_one_complete_revision() {
        let mut directory =
            RuntimeRegistrationDirectory::new(&ConsumerManifest::installed(FULL_SET));
        let a = session("runtime-a", 1);
        register_and_ack(&mut directory, &a, &tuple(42));

        let tuple_43 = tuple(43);
        let outcome = directory.transition(&a, tuple_43.clone(), &tuple_43, None);
        assert_eq!(
            outcome,
            skiff_router::session::directory::TransitionOutcome::Published { revision: 2 }
        );

        // The record holds one complete new revision: new tuple with new
        // revision, never a mixture of old tuple + new revision.
        let record = directory.record(&a).expect("session record");
        assert_eq!(record.registered_tuple, Some(tuple_43.clone()));
        assert_eq!(record.registration_revision, 2);

        // The candidate projection is build-id based (contract v2 §1): the
        // transitioned session stays eligible with its updated revision.
        let capabilities = HashMap::from([(a.clone(), full_facts())]);
        let leases = project(&directory, &capabilities);
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].registration_revision, 2);
        assert_eq!(leases[0].exact_registered_tuple, tuple_43);
    }

    #[test]
    fn routing_query_directory_replacement_never_projects_the_cancelled_old_session() {
        let mut directory =
            RuntimeRegistrationDirectory::new(&ConsumerManifest::installed(FULL_SET));
        let tuple_42 = tuple(42);
        let old = session("runtime-a", 1);
        let new = session("runtime-a", 2);
        register_and_ack(&mut directory, &old, &tuple_42);

        let pending = directory
            .publish_pending(&new, tuple_42.clone(), &FULL_SET)
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
        let tuple_42 = tuple(42);
        let unary_only = session("runtime-a", 1);
        let both = session("runtime-b", 1);
        let unbound = session("runtime-c", 1);
        for s in [&unary_only, &both, &unbound] {
            register_and_ack(&mut directory, s, &tuple_42);
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
}
