//! `ActorOwnershipRegistry` sequence tests: the six frozen ownership corpus
//! scenarios (`runtime/transport/testdata/actor-wire/scenarios/01-06`) plus
//! renew/release/incarnation edge sequences.

mod actor_support;

use serde::Deserialize;
use serde_json::Value;
use skiff_router::actor::{
    ActorClaimId, ActorClaimToken, ActorIncarnationFence, ActorOwnerFence, ActorOwnershipRegistry,
    OwnerReleaseReason, OwnershipError,
};
use std::collections::BTreeMap;

use actor_support::{
    abi, actor_implementation_identity, actor_key, declaration_owner, fence_facts, route_authority,
};

const REQUIRED_SCENARIOS: [&str; 6] = [
    "claim-reserve-commit-single-owner",
    "claim-reserve-conflict-while-owner-held",
    "claim-abort-no-effect",
    "claim-commit-twice-rejected",
    "claim-reservation-not-owner",
    "lease-expire-releases-fence",
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
    events: Vec<Value>,
    expect: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct OwnEvent {
    op: String,
    #[serde(default)]
    caller: Option<String>,
    #[serde(default)]
    expected_epoch: Option<u64>,
    #[serde(default)]
    owner_runtime_id: Option<String>,
    #[serde(default)]
    lease_ttl_ms: Option<u64>,
    #[serde(default)]
    now: Option<u64>,
    #[serde(default)]
    reject: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnExpect {
    reservations: usize,
    commits: u64,
    aborts: u64,
    conflicts: u64,
    renewals: u64,
    releases: u64,
    expired: u64,
    #[serde(rename = "epochMismatches")]
    epoch_mismatches: u64,
    #[serde(rename = "rejectedCommits")]
    rejected_commits: u64,
    #[serde(rename = "rejectedAborts")]
    rejected_aborts: u64,
    owner: Option<OwnerJson>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnerJson {
    runtime_id: String,
    epoch: u64,
    lease_expires_at: u64,
}

fn owner_json(
    registry: &ActorOwnershipRegistry,
    key: &skiff_router::actor::ActorLogicalKey,
) -> Option<OwnerJson> {
    registry.current_owner(key).map(|fence| OwnerJson {
        runtime_id: fence.owner_runtime_id,
        epoch: fence.epoch,
        lease_expires_at: fence.lease_expires_at,
    })
}

fn forged_token(caller: &str) -> ActorClaimToken {
    ActorClaimToken {
        claim_id: ActorClaimId::mint(9_999),
        actor_key: actor_key(),
        expected_epoch: 1,
        owner_runtime_id: caller.to_string(),
        route_authority: route_authority(),
    }
}

fn run_ownership_scenario(raw: &str) {
    let scenario: Scenario = serde_json::from_str(raw).expect("scenario must decode");
    assert_eq!(scenario.schema_version, 1);
    assert_eq!(scenario.domain, "ownership");
    assert!(REQUIRED_SCENARIOS.contains(&scenario.scenario.as_str()));
    let events: Vec<OwnEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("ownership event"))
        .collect();
    let expect: OwnExpect =
        serde_json::from_value(scenario.expect.clone()).expect("ownership expect");
    let registry = ActorOwnershipRegistry::new();
    let key = actor_key();
    registry.ensure_present(
        &key,
        abi(),
        actor_implementation_identity(),
        declaration_owner(),
        &[],
    );
    let mut tokens: BTreeMap<String, ActorClaimToken> = BTreeMap::new();
    for event in &events {
        let result = match event.op.as_str() {
            "reserve" => registry
                .reserve(
                    &key,
                    event.expected_epoch.expect("reserve expectedEpoch"),
                    "runtime-b",
                    &route_authority(),
                    event.now.unwrap_or(0),
                )
                .inspect(|token| {
                    tokens.insert(event.caller.clone().expect("reserve caller"), token.clone());
                })
                .map(|_| ()),
            "commit" => {
                let token = tokens
                    .get(event.caller.as_deref().expect("commit caller"))
                    .cloned()
                    .unwrap_or_else(|| {
                        forged_token(event.caller.as_deref().expect("commit caller"))
                    });
                registry
                    .commit(
                        &token,
                        &fence_facts(),
                        event.now.unwrap_or(0),
                        event.lease_ttl_ms.expect("commit leaseTtlMs"),
                    )
                    .map(|_| ())
            }
            "abort" => {
                let token = tokens
                    .get(event.caller.as_deref().expect("abort caller"))
                    .cloned()
                    .unwrap_or_else(|| {
                        forged_token(event.caller.as_deref().expect("abort caller"))
                    });
                registry.abort(&token)
            }
            "expire" => {
                registry.expire(event.now.expect("expire now"));
                Ok(())
            }
            other => panic!("unknown ownership op {other}"),
        };
        assert_reject(result, event.reject, event.reason.as_deref());
    }
    let health = registry.health();
    assert_eq!(
        health.in_flight_reservations, expect.reservations,
        "reservations"
    );
    assert_eq!(health.commits, expect.commits, "commits");
    assert_eq!(health.aborts, expect.aborts, "aborts");
    assert_eq!(health.conflicts, expect.conflicts, "conflicts");
    assert_eq!(health.renewals, expect.renewals, "renewals");
    assert_eq!(health.releases, expect.releases, "releases");
    assert_eq!(health.expired, expect.expired, "expired");
    assert_eq!(
        health.epoch_mismatches, expect.epoch_mismatches,
        "epochMismatches"
    );
    assert_eq!(
        health.rejected_commits, expect.rejected_commits,
        "rejectedCommits"
    );
    assert_eq!(
        health.rejected_aborts, expect.rejected_aborts,
        "rejectedAborts"
    );
    assert_eq!(owner_json(&registry, &key), expect.owner, "owner");
}

fn assert_reject(result: Result<(), OwnershipError>, reject: bool, reason: Option<&str>) {
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
    fn ownership_scenarios_drive_the_real_registry() {
        let dir = actor_support::actor_wire_dir();
        for name in REQUIRED_SCENARIOS {
            let raw = std::fs::read_to_string(
                dir.join("scenarios")
                    .join(format!("{}-{name}.json", scenario_prefix(name))),
            )
            .unwrap_or_else(|error| panic!("{name}: {error}"));
            run_ownership_scenario(&raw);
        }
    }

    #[test]
    fn renew_extends_an_exact_fence_once() {
        let registry = ActorOwnershipRegistry::new();
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        let fence = registry
            .commit(&token, &fence_facts(), 1000, 30_000)
            .expect("commit");
        assert_eq!(fence.lease_expires_at, 31_000);
        let renewed = registry.renew(&key, &fence, 30_000, 20_000).expect("renew");
        assert_eq!(renewed.lease_expires_at, 50_000);
        assert_eq!(registry.health().renewals, 1);

        // Lease expiry is evaluated against the registry's current lease, so
        // a stale snapshot of the same fence identity renews successfully.
        let stale_renewed = registry
            .renew(&key, &fence, 30_000, 20_000)
            .expect("stale lease snapshot renews by identity");
        assert_eq!(stale_renewed.lease_expires_at, 50_000);

        let mut wrong_lease = renewed.clone();
        wrong_lease.owner_lease_id = "owner-lease-other".to_string();
        let mismatch = registry
            .renew(&key, &wrong_lease, 30_000, 20_000)
            .expect_err("different lease id must be rejected");
        assert!(matches!(mismatch, OwnershipError::FenceMismatch));
    }

    #[test]
    fn renew_after_expiry_is_rejected() {
        let registry = ActorOwnershipRegistry::new();
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        let fence = registry
            .commit(&token, &fence_facts(), 0, 30_000)
            .expect("commit");
        let error = registry
            .renew(&key, &fence, 30_000, 30_000)
            .expect_err("expired lease must reject renew");
        assert!(matches!(error, OwnershipError::LeaseExpired));
    }

    #[test]
    fn committed_fence_pins_the_exact_route_build() {
        let registry = ActorOwnershipRegistry::new();
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        let fence = registry
            .commit(&token, &fence_facts(), 0, 30_000)
            .expect("commit");
        assert_eq!(fence.build_id, route_authority().build_id);
        assert_eq!(
            registry
                .current_owner(&key)
                .expect("current owner")
                .build_id,
            route_authority().build_id
        );
    }

    #[test]
    fn actor_reacquire_uses_the_same_exact_route_build() {
        let registry = ActorOwnershipRegistry::new();
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let first_token = registry
            .reserve(&key, 1, "runtime-a", &route_authority(), 0)
            .expect("first reserve");
        let first = registry
            .commit(&first_token, &fence_facts(), 0, 30_000)
            .expect("first commit");
        registry
            .release(&key, &first, OwnerReleaseReason::Disconnected)
            .expect("release");

        let second_token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 1_000)
            .expect("reacquire reserve");
        let second = registry
            .commit(&second_token, &fence_facts(), 1_000, 30_000)
            .expect("reacquire commit");
        assert_eq!(second.build_id, route_authority().build_id);
        assert_eq!(second.epoch, 1);
        assert_eq!(registry.health().releases, 1);
        assert_eq!(registry.health().commits, 2);
    }

    #[test]
    fn stale_route_build_cannot_renew_or_release_the_new_incarnation() {
        let registry = ActorOwnershipRegistry::new();
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let first_token = registry
            .reserve(&key, 1, "runtime-a", &route_authority(), 0)
            .expect("first reserve");
        let first = registry
            .commit(&first_token, &fence_facts(), 0, 30_000)
            .expect("first commit");
        registry
            .release(&key, &first, OwnerReleaseReason::Disconnected)
            .expect("release");

        let replacement_authority = skiff_router::actor::ActorOwnerRouteAuthority {
            build_id: format!("skiff-deployment-artifact-v4:sha256:{}", "d".repeat(64)),
        };
        let replacement_token = registry
            .reserve(&key, 1, "runtime-b", &replacement_authority, 1_000)
            .expect("replacement reserve");
        let replacement = registry
            .commit(&replacement_token, &fence_facts(), 1_000, 30_000)
            .expect("replacement commit");

        let stale = ActorOwnerFence {
            build_id: route_authority().build_id,
            ..replacement.clone()
        };
        assert!(matches!(
            registry
                .renew(&key, &stale, 30_000, 2_000)
                .expect_err("stale build renew"),
            OwnershipError::FenceMismatch
        ));
        assert!(matches!(
            registry
                .release(&key, &stale, OwnerReleaseReason::Upgraded)
                .expect_err("stale build release"),
            OwnershipError::FenceMismatch
        ));
        assert!(registry.current_owner(&key).is_some());
    }

    #[test]
    fn release_clears_only_the_exact_fence() {
        let registry = ActorOwnershipRegistry::new();
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        let fence = registry
            .commit(&token, &fence_facts(), 0, 30_000)
            .expect("commit");
        let wrong = {
            let mut wrong = fence.clone();
            wrong.owner_runtime_id = "runtime-other".to_string();
            wrong
        };
        assert!(matches!(
            registry
                .release(&key, &wrong, OwnerReleaseReason::Disconnected)
                .expect_err("wrong fence must be rejected"),
            OwnershipError::FenceMismatch
        ));
        registry
            .release(&key, &fence, OwnerReleaseReason::Disconnected)
            .expect("exact release");
        assert_eq!(registry.health().releases, 1);
        assert!(registry.current_owner(&key).is_none());
    }

    #[test]
    fn incarnation_advances_and_clears_owner_and_reservation() {
        let registry = ActorOwnershipRegistry::new();
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        registry
            .commit(&token, &fence_facts(), 0, 30_000)
            .expect("commit");
        assert_eq!(registry.incarnation(&key), Some(ActorIncarnationFence(1)));
        let epoch = registry.advance_incarnation(&key).expect("advance");
        assert_eq!(epoch, 2);
        assert_eq!(registry.incarnation(&key), Some(ActorIncarnationFence(2)));
        assert!(registry.current_owner(&key).is_none());
        assert_eq!(registry.health().in_flight_reservations, 0);
    }

    #[test]
    fn double_abort_and_commit_after_abort_are_rejected() {
        let registry = ActorOwnershipRegistry::new();
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let token = registry
            .reserve(&key, 1, "runtime-b", &route_authority(), 0)
            .expect("reserve");
        registry.abort(&token).expect("first abort");
        assert!(matches!(
            registry.abort(&token).expect_err("double abort"),
            OwnershipError::NoReservation
        ));
        assert!(matches!(
            registry
                .commit(&token, &fence_facts(), 0, 30_000)
                .expect_err("commit after abort"),
            OwnershipError::NoReservation
        ));
        assert_eq!(registry.health().rejected_aborts, 1);
        assert_eq!(registry.health().rejected_commits, 1);
    }

    #[test]
    fn reserve_with_wrong_epoch_fails_closed() {
        let registry = ActorOwnershipRegistry::new();
        let key = actor_key();
        registry.ensure_present(
            &key,
            abi(),
            actor_implementation_identity(),
            declaration_owner(),
            &[],
        );
        let error = registry
            .reserve(&key, 7, "runtime-b", &route_authority(), 0)
            .expect_err("wrong epoch");
        assert!(matches!(
            error,
            OwnershipError::EpochMismatch { current_epoch: 1 }
        ));
        assert_eq!(registry.health().epoch_mismatches, 1);
    }

    fn scenario_prefix(name: &str) -> &'static str {
        match name {
            "claim-reserve-commit-single-owner" => "01",
            "claim-reserve-conflict-while-owner-held" => "02",
            "claim-abort-no-effect" => "03",
            "claim-commit-twice-rejected" => "04",
            "claim-reservation-not-owner" => "05",
            "lease-expire-releases-fence" => "06",
            _ => panic!("unknown ownership scenario {name}"),
        }
    }
}
