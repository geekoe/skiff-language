//! W-routing-query corpus consumer: the production `RuntimeCandidateQuery`
//! must project the same candidate sets as the frozen reference model
//! (`runtime/transport/tests/routing_query_corpus.rs`) over the same nine
//! canonical fixtures.

use std::collections::HashSet;

use skiff_router::routing::{DispatchMode, RegisteredSessionLease, RuntimeCandidateQuery};

mod routing_query_support;
use routing_query_support::*;

fn assert_lease_matches_fixture(
    fixture: &ScenarioFixture,
    lease: &RegisteredSessionLease,
    expected_tuple: &skiff_router::session::identity::RegisteredAssemblyTuple,
) {
    let session = fixture
        .sessions
        .iter()
        .find(|session| {
            session.session_epoch.replica_id == lease.session_epoch.replica_id
                && session.session_epoch.connection_generation
                    == lease.session_epoch.connection_generation
        })
        .expect("lease session must come from the fixture");
    assert_eq!(lease.registration_revision, session.revision, "revision");
    assert_eq!(
        lease.exact_registered_tuple, *expected_tuple,
        "exact tuple must equal the captured epoch tuple"
    );
    assert!(
        !lease.cancellation.cancelled,
        "projected lease must never carry a cancelled marker"
    );
    assert!(
        lease.capabilities.supports(fixture.query.mode),
        "projected lease must support the queried mode"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routing_query_production_projection_matches_frozen_corpus_scenarios() {
        for (name, json) in scenario_files() {
            let fixture: ScenarioFixture = serde_json::from_str(json)
                .unwrap_or_else(|error| panic!("{name} must decode: {error}"));
            assert_eq!(fixture.schema_version, 1, "{name}");
            assert_eq!(fixture.scenario, name, "{name}");
            assert!(
                !fixture.epoch.deployment.service_id.is_empty()
                    && !fixture.epoch.deployment.contract_version.is_empty()
                    && !fixture.epoch.deployment.deployment_revision.is_empty()
                    && !fixture
                        .epoch
                        .deployment
                        .deployment_artifact_identity
                        .is_empty(),
                "{name} deployment coordinates"
            );
            assert!(!fixture.expect.note.is_empty(), "{name} note");
            if let Some(directory_generation) = fixture.directory_current_epoch_generation {
                assert_ne!(
                    directory_generation, fixture.epoch.generation,
                    "{name} captured epoch must differ from directory current"
                );
            }
            assert!(
                REQUIRED_SCENARIOS.contains(&name),
                "{name} must be a required scenario"
            );
            let mut ids = HashSet::new();
            for session in &fixture.sessions {
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

            let epoch = build_epoch(&fixture.epoch);
            let view = build_view(&fixture);
            let query = build_query(&fixture);
            let query_port = RuntimeCandidateQuery;
            let (leases, counters) = query_port
                .query_with_counters(&epoch, &view, &query)
                .unwrap_or_else(|error| panic!("{name} must project: {error}"));

            assert_eq!(
                candidate_ids(&fixture, &leases),
                fixture.expect.candidates,
                "{name} candidates"
            );
            let expected_tuple = epoch.registered_tuple();
            for lease in &leases {
                assert_lease_matches_fixture(&fixture, lease, &expected_tuple);
            }

            let mut expected = expected_counters(&fixture);
            expected.queries = 1;
            assert_eq!(counters, expected, "{name} counters");
        }
    }

    #[test]
    fn routing_query_corpus_covers_every_required_projection_rule() {
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
    fn routing_query_heartbeat_freshness_never_enters_the_production_projection() {
        // Scenario 08 freezes the rule: the fixture sessions are heartbeat-stale
        // yet remain candidates. The production view type has no heartbeat field;
        // parsing the documentation field must not change the projection.
        let json =
        include_str!("../../runtime/transport/testdata/routing-query/scenarios/08-heartbeat-freshness-ignored.json");
        let fixture: ScenarioFixture = serde_json::from_str(json).expect("heartbeat fixture");
        assert!(
            fixture
                .sessions
                .iter()
                .all(|session| !session.heartbeat_fresh),
            "fixture session must be heartbeat-stale"
        );
        let epoch = build_epoch(&fixture.epoch);
        let view = build_view(&fixture);
        let query = build_query(&fixture);
        let leases = RuntimeCandidateQuery
            .query(&epoch, &view, &query)
            .expect("heartbeat-stale session must still project");
        assert_eq!(leases.len(), 1);
    }

    #[test]
    fn routing_query_captured_epoch_is_a_whole_lease() {
        // Scenario 09: the query must only use the captured epoch (generation 42)
        // even when the directory's current generation (43) differs; old captured
        // epochs continue to project their own exact tuple without a global pin
        // map.
        let json = include_str!(
        "../../runtime/transport/testdata/routing-query/scenarios/09-epoch-capture-is-whole-lease.json"
    );
        let fixture: ScenarioFixture = serde_json::from_str(json).expect("whole-lease fixture");
        let epoch = build_epoch(&fixture.epoch);
        let view = build_view(&fixture);
        let query = build_query(&fixture);
        let leases = RuntimeCandidateQuery
            .query(&epoch, &view, &query)
            .expect("captured epoch must project");
        assert_eq!(candidate_ids(&fixture, &leases), vec!["s1"]);
        assert_eq!(leases[0].exact_registered_tuple, epoch.registered_tuple());
    }

    #[test]
    fn routing_query_both_dispatch_modes_are_supported_by_the_typed_query() {
        let json = include_str!(
        "../../runtime/transport/testdata/routing-query/scenarios/01-exact-single-candidate.json"
    );
        let fixture: ScenarioFixture = serde_json::from_str(json).expect("unary fixture");
        let epoch = build_epoch(&fixture.epoch);
        let view = build_view(&fixture);
        let mut query = build_query(&fixture);
        assert_eq!(query.mode, DispatchMode::Unary);

        query.mode = DispatchMode::ServerStream;
        let leases = RuntimeCandidateQuery
            .query(&epoch, &view, &query)
            .expect("serverStream query must project");
        assert_eq!(leases.len(), 1);
        assert!(
            leases[0].capabilities.supports(DispatchMode::ServerStream),
            "capability metadata must be carried on the lease"
        );
    }
}
