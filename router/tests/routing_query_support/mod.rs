//! Shared fixtures for W-routing-query integration tests. Consumes the same
//! canonical corpus as the transport reference projection
//! (`runtime/transport/tests/routing_query_corpus.rs`): the production
//! `RuntimeCandidateQuery` must project the same candidate sets.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;
use skiff_artifact_model::{DeploymentArtifactIdentity, DeploymentRevision, ServiceDeploymentRef};
use skiff_router::routing::{
    CandidateDirectoryView, CandidateQuery, CandidateSession, DispatchCapabilities, DispatchMode,
    RegisteredSessionLease, RoutingQueryCounters,
};
use skiff_router::session::identity::RuntimeSessionEpoch;

pub const REQUIRED_SCENARIOS: [&str; 11] = [
    "exact-single-candidate",
    "multiple-replicas-exact",
    "cancelled-excluded",
    "stale-revision-excluded",
    "build-id-not-loaded-excluded",
    "build-id-registered-wins-over-root-mismatch",
    "capability-server-stream-missing-excluded",
    "heartbeat-freshness-ignored",
    "lazy-load-exact-root-candidate",
    "lazy-load-root-mismatch-excluded",
    "build-id-loaded-with-missing-capability-excluded",
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
            "build-id-not-loaded-excluded",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/05-tuple-assembly-mismatch-excluded.json"),
        ),
        (
            "build-id-registered-wins-over-root-mismatch",
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
            "lazy-load-exact-root-candidate",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/10-lazy-load-exact-root-candidate.json"),
        ),
        (
            "lazy-load-root-mismatch-excluded",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/11-lazy-load-root-mismatch-excluded.json"),
        ),
        (
            "build-id-loaded-with-missing-capability-excluded",
            include_str!("../../../runtime/transport/testdata/routing-query/scenarios/12-build-id-loaded-with-missing-capability-excluded.json"),
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryFixture {
    pub mode: DispatchMode,
    /// Queried deployment build id; defaults to the epoch deployment's
    /// artifact identity when the field is absent.
    #[serde(default)]
    pub build_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionFixture {
    pub id: String,
    pub session_epoch: SessionEpochFixture,
    pub revision: u64,
    pub registered: bool,
    pub cancelled: bool,
    pub capabilities: Vec<String>,
    #[serde(default = "default_true")]
    pub heartbeat_fresh: bool,
    #[serde(default)]
    pub loaded_build_ids: Vec<String>,
    #[serde(default)]
    pub lazy_load: bool,
    #[serde(default)]
    pub artifact_root: Option<String>,
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
    pub router_artifact_root: Option<String>,
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

pub fn build_view(fixture: &ScenarioFixture) -> CandidateDirectoryView {
    CandidateDirectoryView {
        revision: Some(fixture.directory_revision),
        router_artifact_root: fixture.router_artifact_root.clone(),
        sessions: fixture
            .sessions
            .iter()
            .map(|session| CandidateSession {
                session_epoch: RuntimeSessionEpoch {
                    replica_id: session.session_epoch.replica_id.clone(),
                    connection_generation: session.session_epoch.connection_generation,
                },
                registered: session.registered,
                registration_revision: session.revision,
                cancelled: session.cancelled,
                capabilities: DispatchCapabilities {
                    unary: session.capabilities.iter().any(|mode| mode == "unary"),
                    server_stream: session
                        .capabilities
                        .iter()
                        .any(|mode| mode == "serverStream"),
                },
                registered_build_ids: session.loaded_build_ids.clone(),
                lazy_load: session.lazy_load,
                artifact_root: session.artifact_root.clone(),
            })
            .collect(),
    }
}

pub fn build_query(fixture: &ScenarioFixture) -> CandidateQuery {
    CandidateQuery {
        mode: fixture.query.mode,
        build_id: fixture.query.build_id.clone().unwrap_or_else(|| {
            fixture
                .epoch
                .deployment
                .deployment_artifact_identity
                .clone()
        }),
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
    let build_id = fixture.query.build_id.clone().unwrap_or_else(|| {
        fixture
            .epoch
            .deployment
            .deployment_artifact_identity
            .clone()
    });
    for session in &fixture.sessions {
        let build_id_eligible = session.loaded_build_ids.iter().any(|id| id == &build_id)
            || (session.lazy_load && session.artifact_root == fixture.router_artifact_root);
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
        if session.revision != fixture.directory_revision {
            counters.excluded_stale_revision += 1;
        } else if session.cancelled {
            counters.excluded_cancelled += 1;
        } else if !build_id_eligible {
            counters.excluded_build_id += 1;
        } else if !capability_matches {
            counters.excluded_capability += 1;
        }
    }
    counters
}
