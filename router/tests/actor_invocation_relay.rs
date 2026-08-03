//! `ActorInvocationRelay` sequence tests: the five frozen invocation corpus
//! scenarios (`11`-`15`) plus exact-fence / saturation / shutdown edges.

mod actor_support;

use serde::Deserialize;
use serde_json::Value;
use skiff_router::actor::{
    ActorInvocationRelay, ActorInvocationRelayOptions, ActorInvokeInput, InvocationError,
    OwnerSettleKind,
};

use actor_support::{fence, route_authority};

const REQUIRED_SCENARIOS: [&str; 5] = [
    "invoke-return-exact-owner",
    "invoke-error-caller-forward",
    "invoke-cancel-correlation",
    "invoke-duplicate-settle-rejected",
    "invoke-owner-disconnect-terminals-pending",
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
struct InvocationEvent {
    op: String,
    #[serde(default)]
    caller: Option<String>,
    #[serde(default)]
    invocation_id: Option<String>,
    #[serde(default)]
    owner_runtime_id: Option<String>,
    #[serde(default)]
    epoch: Option<u64>,
    #[serde(default)]
    correlation: Option<String>,
    #[serde(default)]
    runtime_id: Option<String>,
    #[serde(default)]
    reject: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InvocationExpect {
    pending: usize,
    settled: u64,
    rejected: u64,
    terminals: u64,
    tombstones: usize,
}

fn invoke_input(
    caller: &str,
    invocation_id: &str,
    owner_runtime_id: &str,
    epoch: u64,
    correlation: &str,
) -> ActorInvokeInput {
    ActorInvokeInput {
        invocation_id: invocation_id.to_string(),
        caller_connection: caller.to_string(),
        caller_runtime_id: caller.to_string(),
        owner_fence: fence(owner_runtime_id, epoch, 40_000),
        owner_connection: "conn-b".to_string(),
        route_authority: route_authority(),
        correlation: correlation.to_string(),
        deadline: None,
        test_case_capability: None,
        now: 0,
    }
}

fn run_invocation_scenario(raw: &str) {
    let scenario: Scenario = serde_json::from_str(raw).expect("scenario must decode");
    assert_eq!(scenario.schema_version, 1);
    assert_eq!(scenario.domain, "invocation");
    assert!(REQUIRED_SCENARIOS.contains(&scenario.scenario.as_str()));
    let events: Vec<InvocationEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("invocation event"))
        .collect();
    let expect: InvocationExpect =
        serde_json::from_value(scenario.expect.clone()).expect("invocation expect");
    let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
    for event in &events {
        let result = match event.op.as_str() {
            "invoke" => relay
                .invoke(&invoke_input(
                    event.caller.as_deref().expect("invoke caller"),
                    event.invocation_id.as_deref().expect("invoke invocationId"),
                    event
                        .owner_runtime_id
                        .as_deref()
                        .expect("invoke ownerRuntimeId"),
                    event.epoch.expect("invoke epoch"),
                    event.correlation.as_deref().expect("invoke correlation"),
                ))
                .map_err(|error| error.to_string()),
            "ownerReturn" | "ownerError" => {
                let kind = if event.op == "ownerReturn" {
                    OwnerSettleKind::Return
                } else {
                    OwnerSettleKind::Error
                };
                relay
                    .on_owner_settle(
                        event.invocation_id.as_deref().expect("settle invocationId"),
                        &fence(
                            event.runtime_id.as_deref().expect("settle runtimeId"),
                            event.epoch.expect("settle epoch"),
                            40_000,
                        ),
                        "conn-b",
                        kind,
                    )
                    .map(|_| ())
            }
            "callerCancel" => relay
                .on_caller_cancel(
                    event.caller.as_deref().expect("cancel caller"),
                    event.invocation_id.as_deref().expect("cancel invocationId"),
                    event.correlation.as_deref().expect("cancel correlation"),
                )
                .map(|_| ()),
            "ownerDisconnect" => {
                relay.on_owner_disconnect(
                    event.runtime_id.as_deref().expect("disconnect runtimeId"),
                    "conn-b",
                );
                Ok(())
            }
            other => panic!("unknown invocation op {other}"),
        };
        assert_invocation_reject(result, event.reject, event.reason.as_deref());
    }
    let health = relay.health();
    assert_eq!(health.pending, expect.pending, "pending");
    assert_eq!(health.settled, expect.settled, "settled");
    assert_eq!(health.rejected, expect.rejected, "rejected");
    assert_eq!(health.terminals, expect.terminals, "terminals");
    assert_eq!(health.tombstones, expect.tombstones, "tombstones");
}

fn assert_invocation_reject(result: Result<(), String>, reject: bool, reason: Option<&str>) {
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
    fn invocation_scenarios_drive_the_real_relay() {
        let dir = actor_support::actor_wire_dir();
        for (prefix, name) in [
            ("11", "invoke-return-exact-owner"),
            ("12", "invoke-error-caller-forward"),
            ("13", "invoke-cancel-correlation"),
            ("14", "invoke-duplicate-settle-rejected"),
            ("15", "invoke-owner-disconnect-terminals-pending"),
        ] {
            let raw = std::fs::read_to_string(
                dir.join("scenarios").join(format!("{prefix}-{name}.json")),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            run_invocation_scenario(&raw);
        }
    }

    #[test]
    fn settle_from_the_wrong_owner_or_connection_is_rejected() {
        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
        relay
            .invoke(&invoke_input("conn-c", "inv:1", "runtime-b", 7, "cancel:1"))
            .expect("invoke");
        let wrong_fence = fence("runtime-x", 7, 40_000);
        assert!(
            relay
                .on_owner_settle("inv:1", &wrong_fence, "conn-b", OwnerSettleKind::Return)
                .is_err(),
            "wrong owner must be rejected"
        );
        let right_fence = fence("runtime-b", 7, 40_000);
        assert!(
            relay
                .on_owner_settle("inv:1", &right_fence, "conn-other", OwnerSettleKind::Return)
                .is_err(),
            "wrong connection must be rejected"
        );
        assert_eq!(relay.health().rejected, 2);
        assert_eq!(relay.health().pending, 1);
        relay
            .on_owner_settle("inv:1", &right_fence, "conn-b", OwnerSettleKind::Return)
            .expect("exact owner settle");
        assert_eq!(relay.health().pending, 0);
        assert_eq!(relay.health().settled, 1);
    }

    #[test]
    fn caller_disconnect_cancels_the_owner_and_terminals_pending() {
        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
        relay
            .invoke(&invoke_input("conn-c", "inv:1", "runtime-b", 7, "cancel:1"))
            .expect("invoke");
        let (cancels, terminals) = relay.on_caller_disconnect("conn-c");
        assert_eq!(cancels.len(), 1);
        assert_eq!(cancels[0].invocation_id, "inv:1");
        assert_eq!(terminals.len(), 1);
        assert_eq!(relay.health().pending, 0);
        assert_eq!(relay.health().tombstones, 1);
    }

    #[test]
    fn deadline_cancels_owner_and_terminals_caller() {
        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
        let mut input = invoke_input("conn-c", "inv:1", "runtime-b", 7, "cancel:1");
        input.now = 0;
        input.deadline = Some(
            skiff_runtime_transport::actor_method::ActorMethodDeadlineFrameHeader {
                timeout_ms: 300_000,
                expires_at: "inv-deadline".to_string(),
            },
        );
        relay.invoke(&input).expect("invoke");
        let expired = relay.expire_deadlines(300_000);
        assert_eq!(expired.len(), 1);
        let (cancel, terminal) = &expired[0];
        assert_eq!(cancel.invocation_id, "inv:1");
        assert_eq!(terminal.invocation_id, "inv:1");
        assert_eq!(relay.health().deadline_cancels, 1);
        assert_eq!(relay.health().pending, 0);
    }

    #[test]
    fn saturation_rejects_without_leaking_pending() {
        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions {
            max_concurrency: 1,
            max_tombstones: 1024,
        });
        relay
            .invoke(&invoke_input("conn-c", "inv:1", "runtime-b", 7, "cancel:1"))
            .expect("first invoke");
        let error = relay
            .invoke(&invoke_input("conn-c", "inv:2", "runtime-b", 7, "cancel:2"))
            .expect_err("second invoke must be saturated");
        assert!(matches!(error, InvocationError::Saturated));
        assert_eq!(relay.health().saturated, 1);
        assert_eq!(relay.health().pending, 1);
        let right_fence = fence("runtime-b", 7, 40_000);
        relay
            .on_owner_settle("inv:1", &right_fence, "conn-b", OwnerSettleKind::Return)
            .expect("settle");
        relay
            .invoke(&invoke_input("conn-c", "inv:2", "runtime-b", 7, "cancel:2"))
            .expect("invoke after settle");
        assert_eq!(relay.health().pending, 1);
    }

    #[test]
    fn shutdown_terminals_all_pending_and_clears_tombstones() {
        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
        relay
            .invoke(&invoke_input("conn-c", "inv:1", "runtime-b", 7, "cancel:1"))
            .expect("invoke");
        relay
            .invoke(&invoke_input("conn-c", "inv:2", "runtime-b", 7, "cancel:2"))
            .expect("invoke");
        let right_fence = fence("runtime-b", 7, 40_000);
        relay
            .on_owner_settle("inv:1", &right_fence, "conn-b", OwnerSettleKind::Return)
            .expect("settle inv:1");
        let terminals = relay.shutdown();
        assert_eq!(terminals.len(), 1);
        let health = relay.health();
        assert_eq!(health.pending, 0, "pending must be zero after shutdown");
        assert_eq!(
            health.tombstones, 0,
            "tombstones must be zero after shutdown"
        );
        assert_eq!(health.terminals, 1);
        assert!(!relay.is_active_parent("inv:2"));
    }

    #[test]
    fn parent_snapshot_uses_owner_connection_and_inactive_after_settle() {
        // E-actor-parity (C-spawn §4.2): an ordinary actor-method invocation's
        // spawn parent authority is the runtime connection where the method
        // executes (the owner). The caller may differ when the Router pins
        // the owner to another replica, so the snapshot must resolve to the
        // owner connection, not the original caller.
        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
        relay
            .invoke(&invoke_input("conn-c", "inv:1", "runtime-b", 7, "cancel:1"))
            .expect("invoke");
        let snapshot = relay.parent_snapshot("inv:1").expect("parent snapshot");
        assert_eq!(snapshot.connection, "conn-b");
        assert_eq!(snapshot.runtime_id, "runtime-b");
        assert_eq!(snapshot.assembly_generation, 42);
        assert!(snapshot.active);
        assert!(!snapshot.replaced);
        let right_fence = fence("runtime-b", 7, 40_000);
        relay
            .on_owner_settle("inv:1", &right_fence, "conn-b", OwnerSettleKind::Return)
            .expect("settle");
        assert!(relay.parent_snapshot("inv:1").is_none());
    }

    #[test]
    fn parent_snapshot_keeps_caller_origin_for_test_capability_lineage() {
        // E-actor-parity (C-spawn §4.2 / TS dispatcher parity): test-capability
        // invocations keep the capability parent origin (the caller
        // connection), matching the leaf's dual-semantics record.
        let relay = ActorInvocationRelay::new(ActorInvocationRelayOptions::default());
        relay
            .invoke(&ActorInvokeInput {
                invocation_id: "inv:cap-1".to_string(),
                caller_connection: "conn-c".to_string(),
                caller_runtime_id: "conn-c".to_string(),
                owner_fence: fence("runtime-b", 7, 40_000),
                owner_connection: "conn-b".to_string(),
                route_authority: route_authority(),
                correlation: "cancel:cap-1".to_string(),
                deadline: None,
                test_case_capability: Some("test:cap-1".to_string()),
                now: 0,
            })
            .expect("invoke");
        let snapshot = relay.parent_snapshot("inv:cap-1").expect("parent snapshot");
        assert_eq!(snapshot.connection, "conn-c");
        assert_eq!(snapshot.runtime_id, "conn-c");
        assert_eq!(snapshot.test_case_capability.as_deref(), Some("test:cap-1"));
        assert!(snapshot.active);
        assert!(!snapshot.replaced);
    }
}
