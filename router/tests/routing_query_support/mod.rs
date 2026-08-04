//! Shared fixtures for W-routing-query integration tests. Consumes the same
//! canonical corpus as the transport reference projection
//! (`runtime/transport/tests/routing_query_corpus.rs`): the production
//! `RuntimeCandidateQuery` must project the same candidate sets.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
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
    CandidateDirectoryView, CandidateQuery, CandidateSession, DispatchCapabilities, DispatchMode,
    RegisteredSessionLease, RoutingQueryCounters,
};
use skiff_router::session::identity::{RegisteredAssemblyTuple, RuntimeSessionEpoch};
use skiff_runtime_config_snapshot::RuntimeConfigSnapshot;

pub const REQUIRED_SCENARIOS: [&str; 9] = [
    "exact-single-candidate",
    "multiple-replicas-exact",
    "cancelled-excluded",
    "stale-revision-excluded",
    "tuple-assembly-mismatch-excluded",
    "tuple-config-snapshot-mismatch-excluded",
    "capability-server-stream-missing-excluded",
    "heartbeat-freshness-ignored",
    "epoch-capture-is-whole-lease",
];

pub fn scenario_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "exact-single-candidate",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/01-exact-single-candidate.json"),
        ),
        (
            "multiple-replicas-exact",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/02-multiple-replicas-exact.json"),
        ),
        (
            "cancelled-excluded",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/03-cancelled-excluded.json"),
        ),
        (
            "stale-revision-excluded",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/04-stale-revision-excluded.json"),
        ),
        (
            "tuple-assembly-mismatch-excluded",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/05-tuple-assembly-mismatch-excluded.json"),
        ),
        (
            "tuple-config-snapshot-mismatch-excluded",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/06-tuple-config-snapshot-mismatch-excluded.json"),
        ),
        (
            "capability-server-stream-missing-excluded",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/07-capability-server-stream-missing-excluded.json"),
        ),
        (
            "heartbeat-freshness-ignored",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/08-heartbeat-freshness-ignored.json"),
        ),
        (
            "epoch-capture-is-whole-lease",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/09-epoch-capture-is-whole-lease.json"),
        ),
    ]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EpochFixture {
    pub profile: String,
    pub generation: u64,
    pub assembly_identity: String,
    pub config_snapshot_id: String,
    pub deployment: DeploymentFixture,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentFixture {
    pub service_id: String,
    pub contract_version: String,
    pub deployment_revision: String,
    pub deployment_artifact_identity: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryFixture {
    pub mode: DispatchMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TupleFixture {
    pub profile: String,
    pub generation: u64,
    pub assembly: String,
    pub config_snapshot: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionFixture {
    pub id: String,
    pub session_epoch: SessionEpochFixture,
    pub revision: u64,
    pub registered: bool,
    pub tuple: Option<TupleFixture>,
    pub cancelled: bool,
    pub capabilities: Vec<String>,
    #[serde(default = "default_true")]
    pub heartbeat_fresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionEpochFixture {
    pub replica_id: String,
    pub connection_generation: u64,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScenarioFixture {
    pub schema_version: u32,
    pub scenario: String,
    #[serde(default = "default_revision")]
    pub directory_revision: u64,
    #[serde(default)]
    pub directory_current_epoch_generation: Option<u64>,
    pub epoch: EpochFixture,
    pub query: QueryFixture,
    pub sessions: Vec<SessionFixture>,
    pub expect: ExpectFixture,
}

fn default_revision() -> u64 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpectFixture {
    pub candidates: Vec<String>,
    #[serde(default)]
    pub note: String,
}

pub fn snapshot_ref(id: &str) -> RuntimeConfigSnapshotRef {
    RuntimeConfigSnapshotRef {
        snapshot_id: RuntimeConfigSnapshotId::parse(id).expect("fixture snapshot id must parse"),
    }
}

pub fn deployment_ref(deployment: &DeploymentFixture) -> ServiceDeploymentRef {
    ServiceDeploymentRef {
        service_id: deployment.service_id.clone(),
        contract_version: deployment.contract_version.clone(),
        deployment_revision: DeploymentRevision::new(deployment.deployment_revision.clone()),
        deployment_artifact_identity: DeploymentArtifactIdentity::new(
            deployment.deployment_artifact_identity.clone(),
        ),
    }
}

/// Builds a real immutable `RoutingEpoch` from the corpus epoch block. The
/// deployment block becomes the epoch's exact deployment projection.
pub fn build_epoch(fixture: &EpochFixture) -> Arc<RoutingEpoch> {
    let assembly = RuntimeAssembly {
        schema_version: RUNTIME_ASSEMBLY_SCHEMA_VERSION.to_string(),
        assembly_identity: AssemblyIdentity::new(fixture.assembly_identity.clone()),
        roots: Vec::new(),
        resolved_deployments: vec![deployment_ref(&fixture.deployment)],
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
    let snapshot = RuntimeConfigSnapshot::new(
        fixture.profile.clone(),
        snapshot_ref(&fixture.config_snapshot_id),
        Vec::new(),
    )
    .expect("fixture snapshot must be valid");
    let projection = ActorRoutingProjection::new(
        ACTOR_ROUTING_PROJECTION_SCHEMA_VERSION.to_string(),
        Vec::new(),
    )
    .expect("empty actor projection");
    let catalog = Arc::new(ActorRoutingCatalog::from_projection(Arc::new(projection)));
    Arc::new(
        RoutingEpoch::new(
            fixture.profile.clone(),
            fixture.generation,
            Arc::new(assembly),
            Arc::new(snapshot),
            catalog,
        )
        .expect("fixture epoch must be valid"),
    )
}

pub fn tuple_from_fixture(tuple: &TupleFixture) -> RegisteredAssemblyTuple {
    RegisteredAssemblyTuple {
        profile: tuple.profile.clone(),
        generation: tuple.generation,
        assembly: RuntimeAssemblyRef {
            assembly_identity: AssemblyIdentity::new(tuple.assembly.clone()),
        },
        config_snapshot: snapshot_ref(&tuple.config_snapshot),
    }
}

pub fn build_view(fixture: &ScenarioFixture) -> CandidateDirectoryView {
    CandidateDirectoryView {
        revision: Some(fixture.directory_revision),
        sessions: fixture
            .sessions
            .iter()
            .map(|session| CandidateSession {
                session_epoch: RuntimeSessionEpoch {
                    replica_id: session.session_epoch.replica_id.clone(),
                    connection_generation: session.session_epoch.connection_generation,
                },
                registered: session.registered,
                registered_tuple: session.tuple.as_ref().map(tuple_from_fixture),
                registration_revision: session.revision,
                cancelled: session.cancelled,
                capabilities: DispatchCapabilities {
                    unary: session.capabilities.iter().any(|mode| mode == "unary"),
                    server_stream: session
                        .capabilities
                        .iter()
                        .any(|mode| mode == "serverStream"),
                },
            })
            .collect(),
    }
}

pub fn build_query(fixture: &ScenarioFixture) -> CandidateQuery {
    CandidateQuery {
        mode: fixture.query.mode,
        deployment: deployment_ref(&fixture.epoch.deployment),
    }
}

/// Maps projected leases back to corpus session ids (fixture order).
pub fn candidate_ids(fixture: &ScenarioFixture, leases: &[RegisteredSessionLease]) -> Vec<String> {
    let by_epoch = fixture
        .sessions
        .iter()
        .map(|session| {
            (
                RuntimeSessionEpoch {
                    replica_id: session.session_epoch.replica_id.clone(),
                    connection_generation: session.session_epoch.connection_generation,
                },
                session.id.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    leases
        .iter()
        .map(|lease| {
            by_epoch
                .get(&lease.session_epoch)
                .cloned()
                .unwrap_or_else(|| format!("unknown:{}", lease.session_epoch.replica_id))
        })
        .collect()
}

/// Frozen per-scenario counter expectations (C-routing-query §5.6). `queries`
/// is filled by the caller because each test runs exactly one query.
pub fn expected_counters(fixture: &ScenarioFixture) -> RoutingQueryCounters {
    let mut counters = RoutingQueryCounters {
        candidates_returned: fixture.expect.candidates.len() as u64,
        ..RoutingQueryCounters::default()
    };
    for session in &fixture.sessions {
        let tuple_matches = session.tuple.as_ref().is_some_and(|tuple| {
            tuple.profile == fixture.epoch.profile
                && tuple.generation == fixture.epoch.generation
                && tuple.assembly == fixture.epoch.assembly_identity
                && tuple.config_snapshot == fixture.epoch.config_snapshot_id
        });
        let capability_matches = session.capabilities.iter().any(|capability| {
            capability
                == match fixture.query.mode {
                    DispatchMode::Unary => "unary",
                    DispatchMode::ServerStream => "serverStream",
                }
        });
        if !session.registered {
            continue;
        }
        if !tuple_matches {
            counters.excluded_tuple_mismatch += 1;
        } else if session.revision != fixture.directory_revision {
            counters.excluded_stale_revision += 1;
        } else if session.cancelled {
            counters.excluded_cancelled += 1;
        } else if !capability_matches {
            counters.excluded_capability += 1;
        }
    }
    counters
}
