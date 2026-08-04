//! Reference-model corpus verifier for C-routing-query
//! (`doc/implementation/router-rust-migration-c-routing-query-contract.md`):
//! captured RoutingEpoch + exact registered tuple/revision/cancellation →
//! exact RuntimeSessionEpoch candidates.
//!
//! TEST-ONLY reference model. Not production code; W-routing-query must
//! implement the frozen projection and consume the same fixtures.

use std::collections::HashSet;

use serde::Deserialize;

const REQUIRED_SCENARIOS: [&str; 9] = [
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

fn scenario_files() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "exact-single-candidate",
            include_str!("../testdata/routing-query/scenarios/01-exact-single-candidate.json"),
        ),
        (
            "multiple-replicas-exact",
            include_str!(
                "../testdata/routing-query/scenarios/02-multiple-replicas-exact.json"
            ),
        ),
        (
            "cancelled-excluded",
            include_str!("../testdata/routing-query/scenarios/03-cancelled-excluded.json"),
        ),
        (
            "stale-revision-excluded",
            include_str!("../testdata/routing-query/scenarios/04-stale-revision-excluded.json"),
        ),
        (
            "tuple-assembly-mismatch-excluded",
            include_str!(
                "../testdata/routing-query/scenarios/05-tuple-assembly-mismatch-excluded.json"
            ),
        ),
        (
            "tuple-config-snapshot-mismatch-excluded",
            include_str!(
                "../testdata/routing-query/scenarios/06-tuple-config-snapshot-mismatch-excluded.json"
            ),
        ),
        (
            "capability-server-stream-missing-excluded",
            include_str!(
                "../testdata/routing-query/scenarios/07-capability-server-stream-missing-excluded.json"
            ),
        ),
        (
            "heartbeat-freshness-ignored",
            include_str!(
                "../testdata/routing-query/scenarios/08-heartbeat-freshness-ignored.json"
            ),
        ),
        (
            "epoch-capture-is-whole-lease",
            include_str!(
                "../testdata/routing-query/scenarios/09-epoch-capture-is-whole-lease.json"
            ),
        ),
    ]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Epoch {
    profile: String,
    generation: u64,
    #[serde(rename = "assemblyIdentity")]
    assembly_identity: String,
    #[serde(rename = "configSnapshotId")]
    config_snapshot_id: String,
    deployment: Deployment,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Deployment {
    #[serde(rename = "serviceId")]
    service_id: String,
    #[serde(rename = "contractVersion")]
    contract_version: String,
    #[serde(rename = "deploymentRevision")]
    deployment_revision: String,
    #[serde(rename = "deploymentArtifactIdentity")]
    deployment_artifact_identity: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Query {
    mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Tuple {
    profile: String,
    generation: u64,
    assembly: String,
    #[serde(rename = "configSnapshot")]
    config_snapshot: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Session {
    id: String,
    #[serde(rename = "sessionEpoch")]
    session_epoch: SessionEpoch,
    revision: u64,
    registered: bool,
    tuple: Option<Tuple>,
    cancelled: bool,
    capabilities: Vec<String>,
    #[serde(rename = "heartbeatFresh", default = "default_true")]
    heartbeat_fresh: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionEpoch {
    #[serde(rename = "replicaId")]
    replica_id: String,
    #[serde(rename = "connectionGeneration")]
    connection_generation: u64,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    #[serde(default = "default_revision")]
    #[serde(rename = "directoryRevision")]
    directory_revision: u64,
    #[serde(default)]
    #[serde(rename = "directoryCurrentEpochGeneration")]
    directory_current_epoch_generation: Option<u64>,
    epoch: Epoch,
    query: Query,
    sessions: Vec<Session>,
    expect: Expect,
}

fn default_revision() -> u64 {
    1
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Expect {
    candidates: Vec<String>,
    #[serde(default)]
    note: String,
}

fn epoch_tuple(epoch: &Epoch) -> Tuple {
    Tuple {
        profile: epoch.profile.clone(),
        generation: epoch.generation,
        assembly: epoch.assembly_identity.clone(),
        config_snapshot: epoch.config_snapshot_id.clone(),
    }
}

/// Frozen projection (C-routing-query §3): whole captured epoch, exact tuple,
/// one complete revision, cancelled exclusion, capability match, heartbeat
/// freshness ignored.
fn candidate_query(
    epoch: &Epoch,
    query: &Query,
    directory_revision: u64,
    sessions: &[Session],
) -> Vec<String> {
    let expected_tuple = epoch_tuple(epoch);
    let mut candidates = Vec::new();
    for session in sessions {
        if !session.registered
            || session.cancelled
            || session.revision != directory_revision
            || session.tuple.as_ref() != Some(&expected_tuple)
            || !session
                .capabilities
                .iter()
                .any(|capability| capability == &query.mode)
        {
            continue;
        }
        candidates.push(session.id.clone());
    }
    candidates
}

#[test]
fn routing_query_scenarios_match_frozen_projection() {
    for (name, json) in scenario_files() {
        let scenario: Scenario = serde_json::from_str(json)
            .unwrap_or_else(|error| panic!("{name} must decode: {error}"));
        assert_eq!(scenario.schema_version, 1, "{name}");
        assert_eq!(scenario.scenario, name, "{name}");
        assert!(
            !scenario.epoch.deployment.service_id.is_empty()
                && !scenario.epoch.deployment.contract_version.is_empty()
                && !scenario.epoch.deployment.deployment_revision.is_empty()
                && !scenario
                    .epoch
                    .deployment
                    .deployment_artifact_identity
                    .is_empty(),
            "{name} deployment coordinates"
        );
        assert!(!scenario.expect.note.is_empty(), "{name} note");
        if let Some(directory_generation) = scenario.directory_current_epoch_generation {
            assert_ne!(
                directory_generation, scenario.epoch.generation,
                "{name} captured epoch must differ from directory current"
            );
        }
        assert!(
            REQUIRED_SCENARIOS.contains(&name),
            "{name} must be a required scenario"
        );
        let mut ids = std::collections::HashSet::new();
        for session in &scenario.sessions {
            assert!(!session.id.is_empty(), "{name} session id");
            assert!(
                ids.insert(session.id.as_str()),
                "{name} duplicate session id {}",
                session.id
            );
            assert!(
                !session.session_epoch.replica_id.is_empty(),
                "{name} session replica"
            );
        }

        let candidates = candidate_query(
            &scenario.epoch,
            &scenario.query,
            scenario.directory_revision,
            &scenario.sessions,
        );
        assert_eq!(candidates, scenario.expect.candidates, "{name}");
    }
}

#[test]
fn scenarios_cover_every_required_projection_rule() {
    let names: HashSet<&str> = scenario_files().iter().map(|(name, _)| *name).collect();
    for required in REQUIRED_SCENARIOS {
        assert!(
            names.contains(required),
            "required scenario {required} is missing"
        );
    }
    assert_eq!(names.len(), REQUIRED_SCENARIOS.len());
}

#[test]
fn heartbeat_freshness_never_enters_candidate_projection() {
    // The corpus scenario 08 freezes this rule; this test pins the reference
    // model to ignore the field entirely (it is only parsed for documentation).
    let json =
        include_str!("../testdata/routing-query/scenarios/08-heartbeat-freshness-ignored.json");
    let scenario: Scenario = serde_json::from_str(json).expect("heartbeat scenario must decode");
    assert!(
        scenario
            .sessions
            .iter()
            .all(|session| !session.heartbeat_fresh),
        "fixture session must be heartbeat-stale"
    );
    let candidates = candidate_query(
        &scenario.epoch,
        &scenario.query,
        scenario.directory_revision,
        &scenario.sessions,
    );
    assert_eq!(candidates.len(), 1);
}
