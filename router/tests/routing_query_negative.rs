//! W-routing-query negative examples: fail-closed projection semantics
//! (C-routing-query §3/§4/§5.4). Together with `routing_query_corpus.rs`
//! these form the shared sequence corpus consumed later by W-dispatch and
//! W-activation.

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
    CandidateDirectoryView, CandidateQuery, CandidateSession,
    DispatchCapabilities, DispatchMode, RuntimeCandidateQuery,
};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

const ASSEMBLY: &str = "skiff-runtime-assembly-v3:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SNAPSHOT: &str = "skiff-runtime-config-snapshot-v1:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn snapshot_ref(id: &str) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(id).expect("snapshot id"),
    }
}

fn deployment(service_id: &str) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: service_id.to_string(),
        contract_version: "1.0.0".to_string(),
        deployment_revision: DeploymentRevision::new("deployment-1"),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(
            "skiff-deployment-artifact-v4:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        ),
    }
}

fn epoch(deployments: Vec<ServiceDeploymentRef>) -> Arc<RoutingEpoch> {
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new(ASSEMBLY),
        roots: Vec::new(),
        resolved_deployments: deployments,
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
        RoutingEpoch::new("prod", 42, Arc::new(assembly), Arc::new(snapshot), catalog)
            .expect("epoch"),
    )
}

fn tuple(generation: u64, assembly: &str, snapshot: &str) -> RegisteredAssemblyTuple {
    RegisteredAssemblyTuple {
        profile: "prod".to_string(),
        generation,
        assembly: RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(assembly),
        },
        config_snapshot: snapshot_ref(snapshot),
    }
}

fn session(replica: &str, connection_generation: u64) -> RuntimeSessionEpoch {
    RuntimeSessionEpoch {
        replica_id: replica.to_string(),
        connection_generation,
    }
}

fn exact_session(replica: &str) -> CandidateSession {
    CandidateSession {
        session_epoch: session(replica, 1),
        registered: true,
        registered_tuple: Some(tuple(42, ASSEMBLY, SNAPSHOT)),
        registration_revision: 1,
        cancelled: false,
        capabilities: DispatchCapabilities {
            unary: true,
            server_stream: true,
        },
        registered_build_ids: vec![deployment("example.com/service-1")
            .deployment_artifact_identity
            .to_string()],
        lazy_load: false,
        artifact_root: None,
    }
}

fn view(revision: Option<u64>, sessions: Vec<CandidateSession>) -> CandidateDirectoryView {
    CandidateDirectoryView {
        revision,
        router_artifact_root: None,
        sessions,
    }
}

fn query() -> CandidateQuery {
    CandidateQuery {
        mode: DispatchMode::Unary,
        build_id: deployment("example.com/service-1")
            .deployment_artifact_identity
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_query_unknown_build_id_has_no_candidate() {
        let epoch = epoch(vec![deployment("example.com/service-1")]);
        let view = view(Some(1), vec![exact_session("runtime-a")]);
        let mut query = query();
        query.build_id = deployment("example.com/other-service")
            .deployment_artifact_identity
            .to_string();

        let leases = RuntimeCandidateQuery.query(&view, &query);
        assert!(
            leases.is_empty(),
            "an unknown build id yields no candidate (fail closed)"
        );
    }

    #[test]
    fn routing_query_no_candidates_is_an_empty_fail_closed_signal() {
        let epoch = epoch(vec![deployment("example.com/service-1")]);
        let mut session = exact_session("runtime-a");
        session.cancelled = true;
        let view = view(Some(1), vec![session]);

        let (leases, counters) = RuntimeCandidateQuery.query_with_counters(&view, &query());
        assert!(leases.is_empty());
        assert_eq!(counters.excluded_cancelled, 1);
        assert_eq!(counters.candidates_returned, 0);
    }

    #[test]
    fn routing_query_unregistered_session_is_never_a_candidate() {
        let epoch = epoch(vec![deployment("example.com/service-1")]);
        let mut session = exact_session("runtime-a");
        session.registered = false;
        let view = view(Some(1), vec![session]);

        let (leases, counters) = RuntimeCandidateQuery.query_with_counters(&view, &query());
        assert!(leases.is_empty());
        // §5.6 has no unregistered counter; exclusion is silent by design.
        assert_eq!(counters.queries, 1);
        assert_eq!(counters.candidates_returned, 0);
        assert_eq!(counters.excluded_stale_revision, 0);
        assert_eq!(counters.excluded_cancelled, 0);
        assert_eq!(counters.excluded_build_id, 0);
        assert_eq!(counters.excluded_capability, 0);
    }

    #[test]
    fn routing_query_torn_view_excludes_stale_revision_without_partial_projection() {
        let epoch = epoch(vec![deployment("example.com/service-1")]);
        let mut stale = exact_session("runtime-a");
        stale.registration_revision = 1;
        let mut current = exact_session("runtime-b");
        current.registration_revision = 2;
        let view = view(Some(2), vec![stale, current]);

        let (leases, counters) = RuntimeCandidateQuery.query_with_counters(&view, &query());
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].session_epoch.replica_id, "runtime-b");
        assert_eq!(counters.excluded_stale_revision, 1);
        assert_eq!(counters.candidates_returned, 1);
    }

    #[test]
    fn routing_query_duplicate_session_epoch_projects_once() {
        let epoch = epoch(vec![deployment("example.com/service-1")]);
        let first = exact_session("runtime-a");
        let mut duplicate = exact_session("runtime-a");
        duplicate.cancelled = true;
        // First occurrence wins; a directory view is a set (one current per
        // replica), so duplicates are a caller bug handled defensively.
        let view = view(Some(1), vec![first, duplicate]);

        let leases = RuntimeCandidateQuery.query(&view, &query());
        assert_eq!(leases.len(), 1);
    }

    #[test]
    fn routing_query_session_failing_multiple_rules_counts_the_first_rule_once() {
        let epoch = epoch(vec![deployment("example.com/service-1")]);
        let mut session = exact_session("runtime-a");
        session.cancelled = true;
        session.capabilities = DispatchCapabilities::default();
        let view = view(Some(1), vec![session]);

        let (leases, counters) = RuntimeCandidateQuery.query_with_counters(&view, &query());
        assert!(leases.is_empty());
        assert_eq!(counters.excluded_cancelled, 1);
        assert_eq!(counters.excluded_build_id, 0);
        assert_eq!(counters.excluded_capability, 0);
    }

    #[test]
    fn routing_query_session_without_capability_binding_is_capability_excluded() {
        let epoch = epoch(vec![deployment("example.com/service-1")]);
        let mut session = exact_session("runtime-a");
        session.capabilities = DispatchCapabilities::default();
        let view = view(Some(1), vec![session]);

        let (leases, counters) = RuntimeCandidateQuery.query_with_counters(&view, &query());
        assert!(leases.is_empty());
        assert_eq!(counters.excluded_capability, 1);

        let mut stream_query = query();
        stream_query.mode = DispatchMode::ServerStream;
        let (stream_leases, _) = RuntimeCandidateQuery.query_with_counters(&view, &stream_query);
        assert!(stream_leases.is_empty());
    }
}
