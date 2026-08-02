//! `ActorLeaseExpiryScheduler` sequence tests: the three frozen lease corpus
//! scenarios (`20`-`22`) plus lease-expiry release and shutdown cleanup.

mod actor_support;

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::Value;
use skiff_router::actor::{
    ActorLeaseExpiryScheduler, ActorLogicalKey, ActorOwnershipRegistry, IdleEvictControlPort,
    LeaseError, LeaseSchedulerOptions,
};

use actor_support::{
    abi, actor_implementation_identity, actor_key, declaration_owner, fence_facts, route_authority,
};

const REQUIRED_SCENARIOS: [&str; 3] = [
    "lease-sweep-expire-and-idle-evict",
    "lease-eviction-ack-clears-request",
    "lease-eviction-retry-bounded",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    domain: String,
    initial_owner: Option<InitialOwner>,
    #[serde(rename = "idleTtlMs")]
    idle_ttl_ms: Option<u64>,
    events: Vec<Value>,
    expect: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitialOwner {
    runtime_id: String,
    epoch: u64,
    lease_expires_at: u64,
    #[serde(default)]
    last_idle_at: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeaseEvent {
    op: String,
    #[serde(default)]
    now: Option<u64>,
    #[serde(default)]
    eviction_request_id: Option<String>,
    #[serde(default)]
    reject: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LeaseExpect {
    expired: u64,
    #[serde(rename = "evictionRequests")]
    eviction_requests: u64,
    #[serde(rename = "evictionAcked")]
    eviction_acked: u64,
    #[serde(rename = "evictionRetries")]
    eviction_retries: u64,
    #[serde(rename = "evictionExhausted")]
    eviction_exhausted: u64,
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
struct FakeEvictPort {
    sent: Mutex<Vec<(ActorLogicalKey, String)>>,
}

impl IdleEvictControlPort for FakeEvictPort {
    fn send_idle_evict(
        &self,
        key: &ActorLogicalKey,
        _fence: &skiff_router::actor::ActorOwnerFence,
        eviction_request_id: &str,
        _connection: &str,
    ) -> Result<(), String> {
        self.sent
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((key.clone(), eviction_request_id.to_string()));
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

fn run_lease_scenario(raw: &str) {
    let scenario: Scenario = serde_json::from_str(raw).expect("scenario must decode");
    assert_eq!(scenario.schema_version, 1);
    assert_eq!(scenario.domain, "lease");
    assert!(REQUIRED_SCENARIOS.contains(&scenario.scenario.as_str()));
    let events: Vec<LeaseEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("lease event"))
        .collect();
    let expect: LeaseExpect =
        serde_json::from_value(scenario.expect.clone()).expect("lease expect");
    let initial = scenario.initial_owner.clone().expect("lease initialOwner");
    let registry = Arc::new(ActorOwnershipRegistry::new());
    let key = actor_key();
    registry.ensure_present(
        &key,
        abi(),
        actor_implementation_identity(),
        declaration_owner(),
    );
    let token = registry
        .reserve(
            &key,
            initial.epoch,
            &initial.runtime_id,
            &route_authority(),
            0,
        )
        .expect("initial reserve");
    registry
        .commit(&token, &fence_facts(), 0, initial.lease_expires_at)
        .expect("initial commit");
    let control = Arc::new(FakeEvictPort::default());
    let scheduler = ActorLeaseExpiryScheduler::new(
        Arc::clone(&registry),
        Arc::clone(&control) as Arc<dyn IdleEvictControlPort>,
        LeaseSchedulerOptions {
            idle_ttl_ms: scenario.idle_ttl_ms.expect("lease idleTtlMs"),
            max_eviction_retries: 3,
        },
    );
    scheduler.mark_live(&key, initial.last_idle_at, "conn-b");
    for event in &events {
        let result = match event.op.as_str() {
            "sweep" => {
                scheduler.sweep(event.now.expect("sweep now"));
                Ok(())
            }
            "evictAck" => scheduler
                .on_eviction_ack(
                    &key,
                    event
                        .eviction_request_id
                        .as_deref()
                        .expect("evictAck evictionRequestId"),
                )
                .map_err(|error| error.to_string()),
            other => panic!("unknown lease op {other}"),
        };
        assert_lease_reject(result, event.reject, event.reason.as_deref());
    }
    let health = scheduler.health();
    assert_eq!(health.expired, expect.expired, "expired");
    assert_eq!(
        health.idle_candidates, expect.eviction_requests,
        "evictionRequests"
    );
    assert_eq!(
        health.eviction_acked, expect.eviction_acked,
        "evictionAcked"
    );
    assert_eq!(
        health.eviction_retries, expect.eviction_retries,
        "evictionRetries"
    );
    assert_eq!(
        health.eviction_exhausted, expect.eviction_exhausted,
        "evictionExhausted"
    );
    assert_eq!(owner_json(&registry, &key), expect.owner, "owner");
}

fn assert_lease_reject(result: Result<(), String>, reject: bool, reason: Option<&str>) {
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
    fn lease_scenarios_drive_the_real_scheduler() {
        let dir = actor_support::actor_wire_dir();
        for (prefix, name) in [
            ("20", "lease-sweep-expire-and-idle-evict"),
            ("21", "lease-eviction-ack-clears-request"),
            ("22", "lease-eviction-retry-bounded"),
        ] {
            let raw = std::fs::read_to_string(
                dir.join("scenarios").join(format!("{prefix}-{name}.json")),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            run_lease_scenario(&raw);
        }
    }

    #[test]
    fn sweep_expires_leases_and_releases_the_fence() {
        let registry = Arc::new(ActorOwnershipRegistry::new());
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
        );
        let token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        registry
            .commit(&token, &fence_facts(), 0, 30_000)
            .expect("commit");
        let control = Arc::new(FakeEvictPort::default());
        let scheduler = ActorLeaseExpiryScheduler::new(
            Arc::clone(&registry),
            control,
            LeaseSchedulerOptions::default(),
        );
        scheduler.mark_live(&key, 0, "conn-b");
        scheduler.sweep(30_000);
        assert!(registry.current_owner(&key).is_none());
        assert_eq!(scheduler.health().expired, 1);
        assert_eq!(scheduler.health().eviction_pending, 0);
        assert_eq!(registry.health().expired, 1);
    }

    #[test]
    fn eviction_ack_with_wrong_id_is_rejected_and_owner_survives() {
        let registry = Arc::new(ActorOwnershipRegistry::new());
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
        );
        let token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        registry
            .commit(&token, &fence_facts(), 0, 40_000)
            .expect("commit");
        let scheduler = ActorLeaseExpiryScheduler::new(
            Arc::clone(&registry),
            Arc::new(FakeEvictPort::default()),
            LeaseSchedulerOptions::default(),
        );
        scheduler.mark_live(&key, 0, "conn-b");
        scheduler.sweep(30_000);
        let error = scheduler
            .on_eviction_ack(&key, "evict:other")
            .expect_err("wrong eviction id");
        assert!(matches!(error, LeaseError::EvictionMismatch));
        assert!(registry.current_owner(&key).is_some());
        scheduler
            .on_eviction_ack(&key, "evict:1")
            .expect("exact eviction ack");
        assert!(registry.current_owner(&key).is_none());
    }

    #[test]
    fn shutdown_clears_all_scheduler_timers() {
        let registry = Arc::new(ActorOwnershipRegistry::new());
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
        );
        let token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        registry
            .commit(&token, &fence_facts(), 0, 40_000)
            .expect("commit");
        let scheduler = ActorLeaseExpiryScheduler::new(
            Arc::clone(&registry),
            Arc::new(FakeEvictPort::default()),
            LeaseSchedulerOptions::default(),
        );
        scheduler.mark_live(&key, 0, "conn-b");
        scheduler.sweep(30_000);
        assert_eq!(scheduler.health().eviction_pending, 1);
        scheduler.shutdown();
        let health = scheduler.health();
        assert_eq!(health.eviction_pending, 0, "eviction pending must be zero");
        assert_eq!(health.sweep_count, 1);
    }
}
