//! `ActorOwnerControlBroker` sequence tests: the four frozen control corpus
//! scenarios (`16`-`19`) plus timeout/disconnect/saturation/shutdown edges.

mod actor_support;

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use skiff_router::actor::{
    ActorOwnerControlBroker, ControlAckOutcome, ControlBrokerOptions, OwnerControlRequest,
};
use skiff_runtime_transport::actor_owner::ActorOwnerControlOperation;

use actor_support::{fence, route_authority};

const REQUIRED_SCENARIOS: [&str; 4] = [
    "control-ack-exact-correlation",
    "control-ack-timeout-rejected",
    "control-late-ack-tombstone",
    "control-ack-wrong-operation-rejected",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    domain: String,
    events: Vec<Value>,
    expect: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControlEvent {
    op: String,
    #[serde(default)]
    request_id: Option<String>,
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    runtime_id: Option<String>,
    #[serde(default)]
    accepted: Option<bool>,
    #[serde(default)]
    reject: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControlExpect {
    pending: usize,
    accepted: u64,
    rejected: u64,
    #[serde(rename = "lateAcks")]
    late_acks: u64,
    timeouts: u64,
    outcomes: BTreeMap<String, String>,
}

fn operation_from_wire(name: &str) -> ActorOwnerControlOperation {
    match name {
        "markUpgrading" => ActorOwnerControlOperation::MarkUpgrading,
        "discard" => ActorOwnerControlOperation::Discard,
        "activate" => ActorOwnerControlOperation::Activate,
        "activateInitial" => ActorOwnerControlOperation::ActivateInitial,
        "idleEvict" => ActorOwnerControlOperation::IdleEvict,
        other => panic!("unknown control operation {other}"),
    }
}

fn send_request(
    broker: &ActorOwnerControlBroker,
    request_id: &str,
    operation: &str,
    runtime_id: &str,
) -> Result<(), String> {
    broker
        .send_control(&OwnerControlRequest {
            request_id: request_id.to_string(),
            operation: operation_from_wire(operation),
            runtime_id: runtime_id.to_string(),
            connection: "conn-b".to_string(),
            fence: fence(runtime_id, 7, 40_000),
            route_authority: route_authority(),
            deadline_at: 10_000,
        })
        .map_err(|error| error.to_string())
}

fn run_control_scenario(raw: &str) {
    let scenario: Scenario = serde_json::from_str(raw).expect("scenario must decode");
    assert_eq!(scenario.schema_version, 1);
    assert_eq!(scenario.domain, "control");
    assert!(REQUIRED_SCENARIOS.contains(&scenario.scenario.as_str()));
    let events: Vec<ControlEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("control event"))
        .collect();
    let expect: ControlExpect =
        serde_json::from_value(scenario.expect.clone()).expect("control expect");
    let broker = ActorOwnerControlBroker::new(ControlBrokerOptions::default());
    for event in &events {
        let result = match event.op.as_str() {
            "sendControl" => send_request(
                &broker,
                event.request_id.as_deref().expect("sendControl requestId"),
                event.operation.as_deref().expect("sendControl operation"),
                event.runtime_id.as_deref().expect("sendControl runtimeId"),
            ),
            "ack" => {
                let outcome = broker.on_ack(
                    event.runtime_id.as_deref().expect("ack runtimeId"),
                    event.request_id.as_deref().expect("ack requestId"),
                    operation_from_wire(event.operation.as_deref().expect("ack operation")),
                    "conn-b",
                    event.accepted.unwrap_or(false),
                );
                match outcome {
                    ControlAckOutcome::Accepted | ControlAckOutcome::Rejected => Ok(()),
                    ControlAckOutcome::LateAck => Err("late ACK".to_string()),
                    ControlAckOutcome::WrongCorrelation | ControlAckOutcome::Unknown => {
                        Err("wrong operation".to_string())
                    }
                }
            }
            "timeout" => {
                let outcome =
                    broker.timeout(event.request_id.as_deref().expect("timeout requestId"));
                assert!(matches!(
                    outcome,
                    skiff_router::actor::ControlTimeoutOutcome::TimedOut { .. }
                ));
                Ok(())
            }
            other => panic!("unknown control op {other}"),
        };
        assert_control_reject(result, event.reject, event.reason.as_deref());
    }
    let health = broker.health();
    assert_eq!(health.pending, expect.pending, "pending");
    assert_eq!(health.accepted, expect.accepted, "accepted");
    assert_eq!(health.rejected, expect.rejected, "rejected");
    assert_eq!(health.late_acks, expect.late_acks, "lateAcks");
    assert_eq!(health.timeouts, expect.timeouts, "timeouts");
    let mut outcomes = BTreeMap::new();
    for request_id in ["req:1".to_string()] {
        if let Some(accepted) = broker.outcome_for(&request_id) {
            outcomes.insert(
                request_id,
                if accepted { "accepted" } else { "rejected" }.to_string(),
            );
        }
    }
    assert_eq!(outcomes, expect.outcomes, "outcomes");
}

fn assert_control_reject(result: Result<(), String>, reject: bool, reason: Option<&str>) {
    match (result, reject) {
        (Ok(()), true) => panic!("event must be rejected but succeeded"),
        (Err(error), false) => panic!("event must succeed but was rejected: {error}"),
        (Err(error), true) => {
            if let Some(reason) = reason {
                assert!(
                    error.contains(reason),
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
    fn control_scenarios_drive_the_real_broker() {
        let dir = actor_support::actor_wire_dir();
        for (prefix, name) in [
            ("16", "control-ack-exact-correlation"),
            ("17", "control-ack-timeout-rejected"),
            ("18", "control-late-ack-tombstone"),
            ("19", "control-ack-wrong-operation-rejected"),
        ] {
            let raw = std::fs::read_to_string(
                dir.join("scenarios").join(format!("{prefix}-{name}.json")),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            run_control_scenario(&raw);
        }
    }

    #[test]
    fn pending_snapshot_keeps_the_exact_fence_and_authority() {
        let broker = ActorOwnerControlBroker::new(ControlBrokerOptions::default());
        send_request(&broker, "req:1", "activateInitial", "runtime-b").expect("send");
        let (fence, authority) = broker.pending_snapshot("req:1").expect("snapshot");
        assert_eq!(fence.owner_runtime_id, "runtime-b");
        assert_eq!(fence.epoch, 7);
        assert_eq!(authority.assembly_generation, 42);
    }

    #[test]
    fn duplicate_and_saturated_sends_are_rejected() {
        let broker = ActorOwnerControlBroker::new(ControlBrokerOptions {
            max_pending: 1,
            ..ControlBrokerOptions::default()
        });
        send_request(&broker, "req:1", "activateInitial", "runtime-b").expect("send");
        let duplicate = send_request(&broker, "req:1", "activateInitial", "runtime-b")
            .expect_err("duplicate request id");
        assert!(duplicate.contains("already pending"));
        let saturated =
            send_request(&broker, "req:2", "idleEvict", "runtime-b").expect_err("saturated");
        assert!(saturated.contains("saturated"));
        assert_eq!(broker.health().saturated, 1);
    }

    #[test]
    fn disconnect_resolves_pending_false_and_shutdown_clears_tombstones() {
        let broker = ActorOwnerControlBroker::new(ControlBrokerOptions::default());
        send_request(&broker, "req:1", "activateInitial", "runtime-b").expect("send");
        send_request(&broker, "req:2", "idleEvict", "runtime-b").expect("send");
        let outcomes = broker.on_owner_disconnect("runtime-b", "conn-b");
        assert_eq!(outcomes.len(), 2);
        assert_eq!(broker.health().pending, 0);
        assert_eq!(broker.health().tombstones, 2);
        let late = broker.on_ack(
            "runtime-b",
            "req:1",
            ActorOwnerControlOperation::ActivateInitial,
            "conn-b",
            true,
        );
        assert_eq!(late, ControlAckOutcome::LateAck);
        let shutdown = broker.shutdown();
        assert!(shutdown.is_empty());
        assert_eq!(broker.health().pending, 0);
        assert_eq!(
            broker.health().tombstones,
            0,
            "tombstones must be zero after shutdown"
        );
    }

    #[test]
    fn deadline_sweep_times_out_all_pending() {
        let broker = ActorOwnerControlBroker::new(ControlBrokerOptions {
            ack_deadline_ms: 10_000,
            ..ControlBrokerOptions::default()
        });
        send_request(&broker, "req:1", "activateInitial", "runtime-b").expect("send");
        let outcomes = broker.expire_deadlines(10_000);
        assert_eq!(outcomes.len(), 1);
        assert_eq!(broker.health().timeouts, 1);
        assert_eq!(broker.outcome_for("req:1"), Some(false));
        assert_eq!(broker.health().pending, 0);
    }
}
