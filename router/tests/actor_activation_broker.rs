//! `ActorActivationRequestBroker` sequence tests: the four frozen activation
//! corpus scenarios (`07`-`10`) driven through the real registry + broker +
//! a fake `activateInitial` control port.

mod actor_support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::Value;
use skiff_router::actor::{
    ActivateInitialControlRequest, ActivationAckOutcome, ActivationControlPort,
    ActorActivationBrokerOptions, ActorActivationRequestBroker, ActorGetOrCreateRequest,
    ActorLogicalKey, ActorOwnershipRegistry, GetOrCreateOutcome,
};

use actor_support::{
    abi, actor_implementation_identity, actor_key, declaration_owner, fence_facts, route_authority,
};

const REQUIRED_SCENARIOS: [&str; 4] = [
    "get-or-create-first-joins-same-outcome",
    "get-or-create-lineage-conflict",
    "get-or-create-existing-no-reserve",
    "get-or-create-ack-timeout-aborts-token",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct Scenario {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    domain: String,
    epoch: Option<u64>,
    initial_owner: Option<InitialOwner>,
    events: Vec<Value>,
    expect: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct InitialOwner {
    runtime_id: String,
    epoch: u64,
    lease_expires_at: u64,
    #[serde(default)]
    now: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivationEvent {
    op: String,
    #[serde(default)]
    caller: Option<String>,
    #[serde(default)]
    rpc_id: Option<String>,
    #[serde(default)]
    lineage: Option<String>,
    #[serde(default)]
    runtime_id: Option<String>,
    #[serde(default)]
    accepted: Option<bool>,
    #[serde(default)]
    now: Option<u64>,
    #[serde(default)]
    reject: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivationExpect {
    outcomes: BTreeMap<String, String>,
    claims: usize,
    reservations: usize,
    commits: u64,
    aborts: u64,
    #[serde(rename = "lineageConflicts")]
    lineage_conflicts: u64,
    owner: Option<OwnerJson>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnerJson {
    runtime_id: String,
    epoch: u64,
    lease_expires_at: u64,
}

#[derive(Debug, Default)]
struct FakeControlPort {
    sent: Mutex<Vec<ActivateInitialControlRequest>>,
}

impl ActivationControlPort for FakeControlPort {
    fn send_activate_initial(&self, request: &ActivateInitialControlRequest) -> Result<(), String> {
        self.sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request.clone());
        Ok(())
    }
}

fn owner_json(registry: &ActorOwnershipRegistry, key: &ActorLogicalKey) -> Option<OwnerJson> {
    registry.current_owner(key).map(|fence| OwnerJson {
        runtime_id: fence.owner_runtime_id,
        epoch: fence.epoch,
        lease_expires_at: fence.lease_expires_at,
    })
}

fn broker_pair(
    initial_owner: Option<InitialOwner>,
) -> (
    Arc<ActorOwnershipRegistry>,
    ActorActivationRequestBroker,
    Arc<FakeControlPort>,
) {
    let registry = Arc::new(ActorOwnershipRegistry::new());
    if let Some(owner) = initial_owner {
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let token = registry
            .reserve(
                &key,
                owner.epoch,
                &owner.runtime_id,
                &route_authority(),
                owner.now.unwrap_or(0),
            )
            .expect("initial owner reserve");
        registry
            .commit(&token, &fence_facts(), owner.now.unwrap_or(0), 30_000)
            .expect("initial owner commit");
    }
    let control = Arc::new(FakeControlPort::default());
    let broker = ActorActivationRequestBroker::new(
        Arc::clone(&registry),
        Arc::clone(&control) as Arc<dyn ActivationControlPort>,
        ActorActivationBrokerOptions {
            activation_deadline_ms: 30_000,
            lease_ttl_ms: 30_000,
            max_claims: 4096,
            max_tombstones: 1024,
        },
    );
    (registry, broker, control)
}

fn get_or_create_request(
    _caller: &str,
    rpc_id: &str,
    lineage: &str,
    now: u64,
) -> ActorGetOrCreateRequest {
    ActorGetOrCreateRequest {
        rpc_id: rpc_id.to_string(),
        actor_key: actor_key(),
        actor_abi_identity: abi(),
        actor_implementation_identity: actor_implementation_identity(),
        declaration_owner: declaration_owner(),
        bootstrap_bytes: vec![0x0a, 0x0b],
        owner_runtime_id: "runtime-b".to_string(),
        owner_connection: "conn-b".to_string(),
        route_authority: route_authority(),
        deadline: None,
        test_case_capability: (lineage != "ordinary").then(|| "test:cap-1".to_string()),
        test_case_parent_request_id: (lineage != "ordinary").then(|| "parent-1".to_string()),
        now,
    }
}

fn run_activation_scenario(raw: &str) {
    let scenario: Scenario = serde_json::from_str(raw).expect("scenario must decode");
    assert_eq!(scenario.schema_version, 1);
    assert_eq!(scenario.domain, "activation");
    assert!(REQUIRED_SCENARIOS.contains(&scenario.scenario.as_str()));
    let events: Vec<ActivationEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("activation event"))
        .collect();
    let expect: ActivationExpect =
        serde_json::from_value(scenario.expect.clone()).expect("activation expect");
    let (registry, broker, _control) = broker_pair(scenario.initial_owner.clone());
    let baseline = registry.health();
    let mut request_id: Option<String> = None;
    for event in &events {
        let result = match event.op.as_str() {
            "getOrCreate" => {
                let caller = event.caller.as_deref().expect("getOrCreate caller");
                let rpc_id = event.rpc_id.as_deref().expect("getOrCreate rpcId");
                let lineage = event.lineage.as_deref().expect("getOrCreate lineage");
                let outcome = broker.get_or_create(&get_or_create_request(
                    caller,
                    rpc_id,
                    lineage,
                    event.now.unwrap_or(0),
                ));
                match outcome {
                    GetOrCreateOutcome::StartedActivation {
                        request_id: started,
                    } => {
                        assert!(request_id.is_none(), "single claim per scenario");
                        request_id = Some(started);
                        Ok(())
                    }
                    GetOrCreateOutcome::Resolved(_) | GetOrCreateOutcome::Joined => Ok(()),
                    GetOrCreateOutcome::LineageConflict => {
                        Err("ActorCreateLineageConflict".to_string())
                    }
                    GetOrCreateOutcome::Saturated => {
                        panic!("unexpected saturation in corpus scenario")
                    }
                    GetOrCreateOutcome::Failed { code } => Err(code),
                }
            }
            "ack" => {
                let outcome = broker.on_activation_ack(
                    request_id.as_deref().expect("ack requires started claim"),
                    event.runtime_id.as_deref().expect("ack runtimeId"),
                    "conn-b",
                    event.accepted.unwrap_or(false),
                    event.now.unwrap_or(0),
                );
                match outcome {
                    ActivationAckOutcome::Committed { .. }
                    | ActivationAckOutcome::Aborted { .. }
                    | ActivationAckOutcome::CommitRejected { .. } => Ok(()),
                    ActivationAckOutcome::LateAck | ActivationAckOutcome::WrongCorrelation => {
                        Err("late or wrong-correlation activation ack".to_string())
                    }
                }
            }
            "timeout" => {
                broker.on_activation_timeout(
                    request_id
                        .as_deref()
                        .expect("timeout requires started claim"),
                    event.now.unwrap_or(0),
                );
                Ok(())
            }
            "disconnect" => {
                broker.on_owner_disconnect(
                    event.runtime_id.as_deref().expect("disconnect runtimeId"),
                    "conn-b",
                );
                Ok(())
            }
            other => panic!("unknown activation op {other}"),
        };
        assert_activation_reject(result, event.reject, event.reason.as_deref());
    }
    let key = actor_key();
    let mut outcomes = BTreeMap::new();
    for rpc in [
        "rpc:1".to_string(),
        "rpc:2".to_string(),
        "rpc:3".to_string(),
    ] {
        if let Some(outcome) = broker.outcome_for(&rpc) {
            outcomes.insert(rpc, outcome);
        }
    }
    assert_eq!(outcomes, expect.outcomes, "outcomes");
    let health = broker.health();
    assert_eq!(health.pending_claims, expect.claims, "claims");
    assert_eq!(
        registry
            .health()
            .in_flight_reservations
            .saturating_sub(baseline.in_flight_reservations),
        expect.reservations,
        "reservations"
    );
    assert_eq!(
        registry.health().commits.saturating_sub(baseline.commits),
        expect.commits,
        "commits"
    );
    assert_eq!(
        registry.health().aborts.saturating_sub(baseline.aborts),
        expect.aborts,
        "aborts"
    );
    assert_eq!(
        health.lineage_conflicts, expect.lineage_conflicts,
        "lineageConflicts"
    );
    assert_eq!(owner_json(&registry, &key), expect.owner, "owner");
}

fn assert_activation_reject(result: Result<(), String>, reject: bool, reason: Option<&str>) {
    match (result, reject) {
        (Ok(()), true) => panic!("event must be rejected but succeeded"),
        (Err(error), false) => panic!("event must succeed but was rejected: {error}"),
        (Err(error), true) => {
            if let Some(reason) = reason {
                assert!(
                    error.to_string().contains(reason),
                    "rejection reason {error} does not mention {reason}"
                );
            }
        }
        (Ok(()), false) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activation_scenarios_drive_the_real_broker() {
        let dir = actor_support::actor_wire_dir();
        for (prefix, name) in [
            ("07", "get-or-create-first-joins-same-outcome"),
            ("08", "get-or-create-lineage-conflict"),
            ("09", "get-or-create-existing-no-reserve"),
            ("10", "get-or-create-ack-timeout-aborts-token"),
        ] {
            let raw = std::fs::read_to_string(
                dir.join("scenarios").join(format!("{prefix}-{name}.json")),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            run_activation_scenario(&raw);
        }
    }

    #[test]
    fn ack_from_the_wrong_connection_is_rejected_and_waiters_survive() {
        let (registry, broker, _) = broker_pair(None);
        let outcome = broker.get_or_create(&get_or_create_request("c1", "rpc:1", "ordinary", 0));
        let GetOrCreateOutcome::StartedActivation { request_id } = outcome else {
            panic!("expected started activation");
        };
        let ack = broker.on_activation_ack(&request_id, "runtime-b", "conn-other", true, 1000);
        assert_eq!(ack, ActivationAckOutcome::WrongCorrelation);
        assert_eq!(broker.health().wrong_correlation, 1);
        assert_eq!(broker.health().pending_claims, 1);
        let ack = broker.on_activation_ack(&request_id, "runtime-b", "conn-b", true, 1000);
        assert!(matches!(
            ack,
            ActivationAckOutcome::Committed { epoch: 1, .. }
        ));
        assert_eq!(
            registry.current_owner(&actor_key()).expect("owner").epoch,
            1
        );
        assert_eq!(broker.outcome_for("rpc:1").as_deref(), Some("resolved:1"));
    }

    #[test]
    fn late_ack_after_timeout_is_a_tombstone() {
        let (_, broker, _) = broker_pair(None);
        let outcome = broker.get_or_create(&get_or_create_request("c1", "rpc:1", "ordinary", 0));
        let GetOrCreateOutcome::StartedActivation { request_id } = outcome else {
            panic!("expected started activation");
        };
        broker.on_activation_timeout(&request_id, 31_000);
        let ack = broker.on_activation_ack(&request_id, "runtime-b", "conn-b", true, 32_000);
        assert_eq!(ack, ActivationAckOutcome::LateAck);
        assert_eq!(broker.health().late_acks, 1);
        assert_eq!(
            broker.outcome_for("rpc:1").as_deref(),
            Some("failed:ActivationTimeout")
        );
    }

    #[test]
    fn expired_activation_deadlines_fail_all_waiters() {
        let (registry, broker, _) = broker_pair(None);
        broker.get_or_create(&get_or_create_request("c1", "rpc:1", "ordinary", 0));
        broker.get_or_create(&get_or_create_request("c2", "rpc:2", "ordinary", 1));
        let outcomes = broker.expire_deadlines(30_000);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].waiters.len(), 2);
        assert_eq!(
            broker.outcome_for("rpc:1").as_deref(),
            Some("failed:ActivationTimeout")
        );
        assert_eq!(
            broker.outcome_for("rpc:2").as_deref(),
            Some("failed:ActivationTimeout")
        );
        assert_eq!(registry.health().aborts, 1);
        assert_eq!(broker.health().pending_claims, 0);
        assert_eq!(broker.health().tombstones, 1);
    }

    #[test]
    fn owner_lease_id_is_minted_once_and_reused_at_commit() {
        // E-actor-parity reconciliation: the broker mints one lease id per
        // activation admission; the same id reaches the activateInitial
        // control request (whose facts the production control port writes
        // into the wire fence) and the committed registry fence.
        let (registry, broker, control) = broker_pair(None);
        let outcome = broker.get_or_create(&get_or_create_request("c1", "rpc:1", "ordinary", 0));
        let GetOrCreateOutcome::StartedActivation { request_id } = outcome else {
            panic!("expected started activation");
        };
        let sent = control
            .sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(sent.len(), 1);
        let wire_lease_id = sent[0].facts.owner_lease_id.clone();
        assert!(
            wire_lease_id.starts_with("owner-lease-"),
            "broker mint must use the canonical owner-lease-<n> shape: {wire_lease_id}"
        );
        drop(sent);

        assert!(matches!(
            broker.on_activation_ack(&request_id, "runtime-b", "conn-b", true, 1000),
            ActivationAckOutcome::Committed { epoch: 1, .. }
        ));
        let committed = registry
            .current_owner(&actor_key())
            .expect("committed owner");
        assert_eq!(
            committed.owner_lease_id, wire_lease_id,
            "wire activateInitial lease id must equal the committed registry fence lease id"
        );

        // A second activation (after release) mints a distinct lease id so
        // the old fence identity never aliases the new owner.
        registry
            .release(
                &actor_key(),
                &committed,
                skiff_router::actor::OwnerReleaseReason::Evicted,
            )
            .expect("release for second activation");
        let outcome = broker.get_or_create(&get_or_create_request("c2", "rpc:2", "ordinary", 0));
        let GetOrCreateOutcome::StartedActivation { request_id } = outcome else {
            panic!("expected second started activation");
        };
        let sent = control
            .sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let second_lease_id = sent
            .last()
            .expect("second request")
            .facts
            .owner_lease_id
            .clone();
        drop(sent);
        assert_ne!(second_lease_id, wire_lease_id);
        assert!(matches!(
            broker.on_activation_ack(&request_id, "runtime-b", "conn-b", true, 2000),
            ActivationAckOutcome::Committed { epoch: 1, .. }
        ));
        assert_eq!(
            registry
                .current_owner(&actor_key())
                .expect("second owner")
                .owner_lease_id,
            second_lease_id
        );
    }
}
