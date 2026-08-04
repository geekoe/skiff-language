//! Reference-model sequence tests for the C-model-actor owners
//! (`doc/implementation/router-rust-migration-c-model-actor-contract.md`):
//! `ActorOwnershipRegistry` (ActorClaimToken reserve/commit/abort),
//! `ActorActivationRequestBroker` (get-or-create dedup),
//! `ActorInvocationRelay`, `ActorOwnerControlBroker` and
//! `ActorLeaseExpiryScheduler`.
//!
//! This is a TEST-ONLY reference model driven by the frozen scenario corpus
//! under `runtime/transport/testdata/actor-wire/scenarios/`. It is not
//! production code and must not be treated as the W-actor implementation.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Deserialize;
use serde_json::Value;

const REQUIRED_SCENARIOS: [&str; 22] = [
    "claim-reserve-commit-single-owner",
    "claim-reserve-conflict-while-owner-held",
    "claim-abort-no-effect",
    "claim-commit-twice-rejected",
    "claim-reservation-not-owner",
    "lease-expire-releases-fence",
    "get-or-create-first-joins-same-outcome",
    "get-or-create-lineage-conflict",
    "get-or-create-existing-no-reserve",
    "get-or-create-ack-timeout-aborts-token",
    "invoke-return-exact-owner",
    "invoke-error-caller-forward",
    "invoke-cancel-correlation",
    "invoke-duplicate-settle-rejected",
    "invoke-owner-disconnect-terminals-pending",
    "control-ack-exact-correlation",
    "control-ack-timeout-rejected",
    "control-late-ack-tombstone",
    "control-ack-wrong-operation-rejected",
    "lease-sweep-expire-and-idle-evict",
    "lease-eviction-ack-clears-request",
    "lease-eviction-retry-bounded",
];

const SCENARIOS: [(&str, &str); 22] = [
    (
        "claim-reserve-commit-single-owner",
        include_str!("../testdata/actor-wire/scenarios/01-claim-reserve-commit-single-owner.json"),
    ),
    (
        "claim-reserve-conflict-while-owner-held",
        include_str!(
            "../testdata/actor-wire/scenarios/02-claim-reserve-conflict-while-owner-held.json"
        ),
    ),
    (
        "claim-abort-no-effect",
        include_str!("../testdata/actor-wire/scenarios/03-claim-abort-no-effect.json"),
    ),
    (
        "claim-commit-twice-rejected",
        include_str!("../testdata/actor-wire/scenarios/04-claim-commit-twice-rejected.json"),
    ),
    (
        "claim-reservation-not-owner",
        include_str!("../testdata/actor-wire/scenarios/05-claim-reservation-not-owner.json"),
    ),
    (
        "lease-expire-releases-fence",
        include_str!("../testdata/actor-wire/scenarios/06-lease-expire-releases-fence.json"),
    ),
    (
        "get-or-create-first-joins-same-outcome",
        include_str!(
            "../testdata/actor-wire/scenarios/07-get-or-create-first-joins-same-outcome.json"
        ),
    ),
    (
        "get-or-create-lineage-conflict",
        include_str!("../testdata/actor-wire/scenarios/08-get-or-create-lineage-conflict.json"),
    ),
    (
        "get-or-create-existing-no-reserve",
        include_str!("../testdata/actor-wire/scenarios/09-get-or-create-existing-no-reserve.json"),
    ),
    (
        "get-or-create-ack-timeout-aborts-token",
        include_str!(
            "../testdata/actor-wire/scenarios/10-get-or-create-ack-timeout-aborts-token.json"
        ),
    ),
    (
        "invoke-return-exact-owner",
        include_str!("../testdata/actor-wire/scenarios/11-invoke-return-exact-owner.json"),
    ),
    (
        "invoke-error-caller-forward",
        include_str!("../testdata/actor-wire/scenarios/12-invoke-error-caller-forward.json"),
    ),
    (
        "invoke-cancel-correlation",
        include_str!("../testdata/actor-wire/scenarios/13-invoke-cancel-correlation.json"),
    ),
    (
        "invoke-duplicate-settle-rejected",
        include_str!("../testdata/actor-wire/scenarios/14-invoke-duplicate-settle-rejected.json"),
    ),
    (
        "invoke-owner-disconnect-terminals-pending",
        include_str!(
            "../testdata/actor-wire/scenarios/15-invoke-owner-disconnect-terminals-pending.json"
        ),
    ),
    (
        "control-ack-exact-correlation",
        include_str!("../testdata/actor-wire/scenarios/16-control-ack-exact-correlation.json"),
    ),
    (
        "control-ack-timeout-rejected",
        include_str!("../testdata/actor-wire/scenarios/17-control-ack-timeout-rejected.json"),
    ),
    (
        "control-late-ack-tombstone",
        include_str!("../testdata/actor-wire/scenarios/18-control-late-ack-tombstone.json"),
    ),
    (
        "control-ack-wrong-operation-rejected",
        include_str!(
            "../testdata/actor-wire/scenarios/19-control-ack-wrong-operation-rejected.json"
        ),
    ),
    (
        "lease-sweep-expire-and-idle-evict",
        include_str!("../testdata/actor-wire/scenarios/20-lease-sweep-expire-and-idle-evict.json"),
    ),
    (
        "lease-eviction-ack-clears-request",
        include_str!("../testdata/actor-wire/scenarios/21-lease-eviction-ack-clears-request.json"),
    ),
    (
        "lease-eviction-retry-bounded",
        include_str!("../testdata/actor-wire/scenarios/22-lease-eviction-retry-bounded.json"),
    ),
];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    scenario: String,
    domain: String,
    #[serde(default)]
    epoch: Option<u64>,
    #[serde(default)]
    initial_owner: Option<InitialOwner>,
    #[serde(default)]
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OwnerJson {
    runtime_id: String,
    epoch: u64,
    lease_expires_at: u64,
}

// ---------------------------------------------------------------------------
// ActorOwnershipRegistry reference model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OwnershipCounters {
    reservations: usize,
    commits: usize,
    aborts: usize,
    conflicts: usize,
    renewals: usize,
    releases: usize,
    expired: usize,
    epoch_mismatches: usize,
    rejected_commits: usize,
    rejected_aborts: usize,
}

impl Default for OwnershipCounters {
    fn default() -> Self {
        Self {
            reservations: 0,
            commits: 0,
            aborts: 0,
            conflicts: 0,
            renewals: 0,
            releases: 0,
            expired: 0,
            epoch_mismatches: 0,
            rejected_commits: 0,
            rejected_aborts: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnerFence {
    runtime_id: String,
    epoch: u64,
    lease_expires_at: u64,
}

#[derive(Debug, Clone)]
struct Reservation {
    caller: String,
    expected_epoch: u64,
}

#[derive(Debug, Clone)]
struct OwnershipRegistry {
    epoch: u64,
    owner: Option<OwnerFence>,
    reservation: Option<Reservation>,
    counters: OwnershipCounters,
}

impl OwnershipRegistry {
    fn new(epoch: u64) -> Self {
        Self {
            epoch,
            owner: None,
            reservation: None,
            counters: OwnershipCounters::default(),
        }
    }

    fn reserve(&mut self, caller: &str, expected_epoch: u64, now: u64) -> Result<(), &'static str> {
        if self.reservation.is_some() {
            self.counters.conflicts += 1;
            return Err("a reservation is already in flight");
        }
        if let Some(owner) = &self.owner {
            if owner.lease_expires_at > now {
                self.counters.conflicts += 1;
                return Err("conflict: a current owner fence is held");
            }
        }
        if expected_epoch != self.epoch {
            self.counters.epoch_mismatches += 1;
            return Err("epoch mismatch");
        }
        self.reservation = Some(Reservation {
            caller: caller.to_string(),
            expected_epoch,
        });
        self.counters.reservations += 1;
        Ok(())
    }

    fn commit(
        &mut self,
        caller: &str,
        runtime_id: &str,
        lease_ttl_ms: u64,
        now: u64,
    ) -> Result<(), &'static str> {
        let Some(reservation) = self.reservation.clone() else {
            self.counters.rejected_commits += 1;
            return Err("no reservation for caller");
        };
        if reservation.caller != caller {
            self.counters.rejected_commits += 1;
            return Err("no reservation for caller");
        }
        self.owner = Some(OwnerFence {
            runtime_id: runtime_id.to_string(),
            epoch: reservation.expected_epoch,
            lease_expires_at: now.saturating_add(lease_ttl_ms),
        });
        self.reservation = None;
        self.counters.commits += 1;
        self.counters.reservations = self.counters.reservations.saturating_sub(1);
        Ok(())
    }

    fn abort(&mut self, caller: &str) -> Result<(), &'static str> {
        let Some(reservation) = &self.reservation else {
            self.counters.rejected_aborts += 1;
            return Err("no reservation to abort");
        };
        if reservation.caller != caller {
            self.counters.rejected_aborts += 1;
            return Err("reservation belongs to a different caller");
        }
        self.reservation = None;
        self.counters.aborts += 1;
        self.counters.reservations = self.counters.reservations.saturating_sub(1);
        Ok(())
    }

    fn expire(&mut self, now: u64) {
        if let Some(owner) = &self.owner {
            if owner.lease_expires_at <= now {
                self.owner = None;
                self.counters.expired += 1;
            }
        }
    }

    fn set_owner(&mut self, owner: OwnerFence) {
        self.owner = Some(owner);
    }

    fn owner_json(&self) -> Option<OwnerJson> {
        self.owner.clone().map(|owner| OwnerJson {
            runtime_id: owner.runtime_id,
            epoch: owner.epoch,
            lease_expires_at: owner.lease_expires_at,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    commits: usize,
    aborts: usize,
    conflicts: usize,
    renewals: usize,
    releases: usize,
    expired: usize,
    #[serde(rename = "epochMismatches")]
    epoch_mismatches: usize,
    #[serde(rename = "rejectedCommits")]
    rejected_commits: usize,
    #[serde(rename = "rejectedAborts")]
    rejected_aborts: usize,
    owner: Option<OwnerJson>,
}

fn run_ownership(scenario: &Scenario) {
    let events: Vec<OwnEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("ownership event"))
        .collect();
    let expect: OwnExpect =
        serde_json::from_value(scenario.expect.clone()).expect("ownership expect");
    let mut registry = OwnershipRegistry::new(scenario.epoch.expect("ownership epoch"));
    for event in &events {
        let result = match event.op.as_str() {
            "reserve" => registry.reserve(
                event.caller.as_deref().expect("reserve caller"),
                event.expected_epoch.expect("reserve expectedEpoch"),
                event.now.unwrap_or(0),
            ),
            "commit" => registry.commit(
                event.caller.as_deref().expect("commit caller"),
                event.owner_runtime_id.as_deref().expect("commit runtime"),
                event.lease_ttl_ms.expect("commit leaseTtlMs"),
                event.now.unwrap_or(0),
            ),
            "abort" => registry.abort(event.caller.as_deref().expect("abort caller")),
            "expire" => {
                registry.expire(event.now.expect("expire now"));
                Ok(())
            }
            other => panic!("unknown ownership op {other}"),
        };
        assert_reject(result, event.reject, event.reason.as_deref());
    }
    let counters = &registry.counters;
    assert_eq!(counters.reservations, expect.reservations, "reservations");
    assert_eq!(counters.commits, expect.commits, "commits");
    assert_eq!(counters.aborts, expect.aborts, "aborts");
    assert_eq!(counters.conflicts, expect.conflicts, "conflicts");
    assert_eq!(counters.renewals, expect.renewals, "renewals");
    assert_eq!(counters.releases, expect.releases, "releases");
    assert_eq!(counters.expired, expect.expired, "expired");
    assert_eq!(
        counters.epoch_mismatches, expect.epoch_mismatches,
        "epochMismatches"
    );
    assert_eq!(
        counters.rejected_commits, expect.rejected_commits,
        "rejectedCommits"
    );
    assert_eq!(
        counters.rejected_aborts, expect.rejected_aborts,
        "rejectedAborts"
    );
    assert_eq!(registry.owner_json(), expect.owner, "owner");
}

// ---------------------------------------------------------------------------
// ActorActivationRequestBroker reference model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Lineage {
    Ordinary,
    Test(String),
}

#[derive(Debug, Clone)]
struct ActivationClaim {
    caller: String,
    lineage: Lineage,
    rpc_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct ActivationBroker {
    registry: OwnershipRegistry,
    claim: Option<ActivationClaim>,
    outcomes: BTreeMap<String, String>,
    lineage_conflicts: usize,
    default_lease_ttl_ms: u64,
}

impl ActivationBroker {
    fn new(epoch: u64, initial_owner: Option<InitialOwner>, lease_ttl_ms: u64) -> Self {
        let mut registry = OwnershipRegistry::new(epoch);
        if let Some(owner) = initial_owner {
            registry.set_owner(OwnerFence {
                runtime_id: owner.runtime_id,
                epoch: owner.epoch,
                lease_expires_at: owner.lease_expires_at,
            });
        }
        Self {
            registry,
            claim: None,
            outcomes: BTreeMap::new(),
            lineage_conflicts: 0,
            default_lease_ttl_ms: lease_ttl_ms,
        }
    }

    fn get_or_create(
        &mut self,
        caller: &str,
        rpc_id: &str,
        lineage: Lineage,
        now: u64,
    ) -> Result<(), &'static str> {
        if let Some(claim) = &mut self.claim {
            if claim.lineage != lineage {
                self.lineage_conflicts += 1;
                self.outcomes.insert(
                    rpc_id.to_string(),
                    "failed:ActorCreateLineageConflict".to_string(),
                );
                return Err("ActorCreateLineageConflict");
            }
            claim.rpc_ids.push(rpc_id.to_string());
            return Ok(());
        }
        if let Some(owner) = &self.registry.owner {
            self.outcomes
                .insert(rpc_id.to_string(), format!("resolved:{}", owner.epoch));
            return Ok(());
        }
        self.registry
            .reserve(caller, self.registry.epoch, now)
            .expect("first getOrCreate must reserve the actor key");
        self.claim = Some(ActivationClaim {
            caller: caller.to_string(),
            lineage,
            rpc_ids: vec![rpc_id.to_string()],
        });
        Ok(())
    }

    fn ack(&mut self, runtime_id: &str, accepted: bool, now: u64) {
        let claim = self
            .claim
            .clone()
            .expect("activation ACK without pending claim");
        if accepted {
            self.registry
                .commit(&claim.caller, runtime_id, self.default_lease_ttl_ms, now)
                .expect("activation ACK accepted must commit the claim token");
            let epoch = self.registry.owner.as_ref().expect("committed owner").epoch;
            for rpc_id in &claim.rpc_ids {
                self.outcomes
                    .insert(rpc_id.clone(), format!("resolved:{epoch}"));
            }
        } else {
            self.registry
                .abort(&claim.caller)
                .expect("ack rejected aborts token");
            for rpc_id in &claim.rpc_ids {
                self.outcomes
                    .insert(rpc_id.clone(), "failed:AckRejected".to_string());
            }
        }
        self.claim = None;
    }

    fn timeout(&mut self) {
        let claim = self
            .claim
            .clone()
            .expect("activation timeout without pending claim");
        self.registry
            .abort(&claim.caller)
            .expect("timeout aborts token");
        for rpc_id in &claim.rpc_ids {
            self.outcomes
                .insert(rpc_id.clone(), "failed:ActivationTimeout".to_string());
        }
        self.claim = None;
    }

    fn disconnect(&mut self, _runtime_id: &str) {
        // Frozen semantics: waiting get callers stay suspended until the
        // activation deadline and then fail; the timeout settles them.
    }
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
    commits: usize,
    aborts: usize,
    #[serde(rename = "lineageConflicts")]
    lineage_conflicts: usize,
    owner: Option<OwnerJson>,
}

fn run_activation(scenario: &Scenario) {
    let events: Vec<ActivationEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("activation event"))
        .collect();
    let expect: ActivationExpect =
        serde_json::from_value(scenario.expect.clone()).expect("activation expect");
    let mut broker = ActivationBroker::new(
        scenario.epoch.expect("activation epoch"),
        scenario.initial_owner.clone(),
        30_000,
    );
    for event in &events {
        let result = match event.op.as_str() {
            "getOrCreate" => broker.get_or_create(
                event.caller.as_deref().expect("getOrCreate caller"),
                event.rpc_id.as_deref().expect("getOrCreate rpcId"),
                match event.lineage.as_deref().expect("getOrCreate lineage") {
                    "ordinary" => Lineage::Ordinary,
                    other => Lineage::Test(other.to_string()),
                },
                event.now.unwrap_or(0),
            ),
            "ack" => {
                broker.ack(
                    event.runtime_id.as_deref().expect("ack runtimeId"),
                    event.accepted.expect("ack accepted"),
                    event.now.unwrap_or(0),
                );
                Ok(())
            }
            "timeout" => {
                broker.timeout();
                Ok(())
            }
            "disconnect" => {
                broker.disconnect(event.runtime_id.as_deref().expect("disconnect runtimeId"));
                Ok(())
            }
            other => panic!("unknown activation op {other}"),
        };
        assert_reject(result, event.reject, event.reason.as_deref());
    }
    assert_eq!(broker.outcomes, expect.outcomes, "outcomes");
    assert_eq!(broker.claim.is_some() as usize, expect.claims, "claims");
    assert_eq!(
        broker.registry.counters.reservations, expect.reservations,
        "reservations"
    );
    assert_eq!(broker.registry.counters.commits, expect.commits, "commits");
    assert_eq!(broker.registry.counters.aborts, expect.aborts, "aborts");
    assert_eq!(
        broker.lineage_conflicts, expect.lineage_conflicts,
        "lineageConflicts"
    );
    assert_eq!(broker.registry.owner_json(), expect.owner, "owner");
}

// ---------------------------------------------------------------------------
// ActorInvocationRelay reference model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct PendingInvocation {
    caller: String,
    owner_runtime_id: String,
    epoch: u64,
    correlation: String,
}

#[derive(Debug, Default)]
struct InvocationCounters {
    settled: usize,
    rejected: usize,
    terminals: usize,
    tombstones: usize,
}

#[derive(Debug, Default)]
struct InvocationRelay {
    pending: HashMap<String, PendingInvocation>,
    tombstones: HashSet<String>,
    counters: InvocationCounters,
}

impl InvocationRelay {
    fn invoke(
        &mut self,
        caller: &str,
        invocation_id: &str,
        owner_runtime_id: &str,
        epoch: u64,
        correlation: &str,
    ) -> Result<(), &'static str> {
        if self.pending.contains_key(invocation_id) {
            self.counters.rejected += 1;
            return Err("invocation already pending");
        }
        self.pending.insert(
            invocation_id.to_string(),
            PendingInvocation {
                caller: caller.to_string(),
                owner_runtime_id: owner_runtime_id.to_string(),
                epoch,
                correlation: correlation.to_string(),
            },
        );
        Ok(())
    }

    fn settle(
        &mut self,
        invocation_id: &str,
        owner: Option<(&str, u64)>,
        cancel: Option<(&str, &str)>,
    ) -> Result<(), &'static str> {
        let Some(pending) = self.pending.get(invocation_id) else {
            self.counters.rejected += 1;
            return Err("already settled (duplicate settle or unknown invocation)");
        };
        if let Some((owner_runtime_id, epoch)) = owner {
            if pending.owner_runtime_id != owner_runtime_id || pending.epoch != epoch {
                self.counters.rejected += 1;
                return Err("settle did not come from the exact admitted owner");
            }
        }
        if let Some((caller, correlation)) = cancel {
            if pending.caller != caller || pending.correlation != correlation {
                self.counters.rejected += 1;
                return Err("cancel did not come from the correlated caller");
            }
        }
        self.pending.remove(invocation_id);
        if self.tombstones.insert(invocation_id.to_string()) {
            self.counters.tombstones += 1;
        }
        self.counters.settled += 1;
        Ok(())
    }

    fn owner_disconnect(&mut self, runtime_id: &str) {
        let ids: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.owner_runtime_id == runtime_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.pending.remove(&id);
            if self.tombstones.insert(id.clone()) {
                self.counters.tombstones += 1;
            }
            self.counters.terminals += 1;
        }
    }
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
    settled: usize,
    rejected: usize,
    terminals: usize,
    tombstones: usize,
}

fn run_invocation(scenario: &Scenario) {
    let events: Vec<InvocationEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("invocation event"))
        .collect();
    let expect: InvocationExpect =
        serde_json::from_value(scenario.expect.clone()).expect("invocation expect");
    let mut relay = InvocationRelay::default();
    for event in &events {
        let result = match event.op.as_str() {
            "invoke" => relay.invoke(
                event.caller.as_deref().expect("invoke caller"),
                event.invocation_id.as_deref().expect("invoke invocationId"),
                event
                    .owner_runtime_id
                    .as_deref()
                    .expect("invoke ownerRuntimeId"),
                event.epoch.expect("invoke epoch"),
                event.correlation.as_deref().expect("invoke correlation"),
            ),
            "ownerReturn" | "ownerError" => relay.settle(
                event.invocation_id.as_deref().expect("settle invocationId"),
                Some((
                    event.runtime_id.as_deref().expect("settle runtimeId"),
                    event.epoch.expect("settle epoch"),
                )),
                None,
            ),
            "callerCancel" => relay.settle(
                event.invocation_id.as_deref().expect("cancel invocationId"),
                None,
                Some((
                    event.caller.as_deref().expect("cancel caller"),
                    event.correlation.as_deref().expect("cancel correlation"),
                )),
            ),
            "ownerDisconnect" => {
                relay.owner_disconnect(event.runtime_id.as_deref().expect("disconnect runtimeId"));
                Ok(())
            }
            other => panic!("unknown invocation op {other}"),
        };
        assert_reject(result, event.reject, event.reason.as_deref());
    }
    assert_eq!(relay.pending.len(), expect.pending, "pending");
    assert_eq!(relay.counters.settled, expect.settled, "settled");
    assert_eq!(relay.counters.rejected, expect.rejected, "rejected");
    assert_eq!(relay.counters.terminals, expect.terminals, "terminals");
    assert_eq!(relay.counters.tombstones, expect.tombstones, "tombstones");
}

// ---------------------------------------------------------------------------
// ActorOwnerControlBroker reference model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct PendingControl {
    operation: String,
    runtime_id: String,
}

#[derive(Debug, Default)]
struct ControlBroker {
    pending: HashMap<String, PendingControl>,
    settled: HashSet<String>,
    outcomes: BTreeMap<String, String>,
    accepted: usize,
    rejected: usize,
    late_acks: usize,
    timeouts: usize,
}

impl ControlBroker {
    fn send_control(
        &mut self,
        request_id: &str,
        operation: &str,
        runtime_id: &str,
    ) -> Result<(), &'static str> {
        if self.pending.contains_key(request_id) {
            self.rejected += 1;
            return Err("control request already pending");
        }
        self.pending.insert(
            request_id.to_string(),
            PendingControl {
                operation: operation.to_string(),
                runtime_id: runtime_id.to_string(),
            },
        );
        Ok(())
    }

    fn ack(
        &mut self,
        runtime_id: &str,
        request_id: &str,
        operation: &str,
        accepted: bool,
    ) -> Result<(), &'static str> {
        let Some(pending) = self.pending.get(request_id) else {
            self.late_acks += 1;
            self.rejected += 1;
            return Err("late ACK without pending control");
        };
        if pending.runtime_id != runtime_id || pending.operation != operation {
            self.rejected += 1;
            return Err("wrong operation");
        }
        self.pending.remove(request_id);
        self.settled.insert(request_id.to_string());
        let outcome = if accepted { "accepted" } else { "rejected" };
        self.outcomes
            .insert(request_id.to_string(), outcome.to_string());
        if accepted {
            self.accepted += 1;
        } else {
            self.rejected += 1;
        }
        Ok(())
    }

    fn timeout(&mut self, request_id: &str) {
        if self.pending.remove(request_id).is_some() {
            self.settled.insert(request_id.to_string());
            self.outcomes
                .insert(request_id.to_string(), "rejected".to_string());
            self.timeouts += 1;
        }
    }
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
    accepted: usize,
    rejected: usize,
    #[serde(rename = "lateAcks")]
    late_acks: usize,
    timeouts: usize,
    outcomes: BTreeMap<String, String>,
}

fn run_control(scenario: &Scenario) {
    let events: Vec<ControlEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("control event"))
        .collect();
    let expect: ControlExpect =
        serde_json::from_value(scenario.expect.clone()).expect("control expect");
    let mut broker = ControlBroker::default();
    for event in &events {
        let result = match event.op.as_str() {
            "sendControl" => broker.send_control(
                event.request_id.as_deref().expect("sendControl requestId"),
                event.operation.as_deref().expect("sendControl operation"),
                event.runtime_id.as_deref().expect("sendControl runtimeId"),
            ),
            "ack" => broker.ack(
                event.runtime_id.as_deref().expect("ack runtimeId"),
                event.request_id.as_deref().expect("ack requestId"),
                event.operation.as_deref().expect("ack operation"),
                event.accepted.unwrap_or(false),
            ),
            "timeout" => {
                broker.timeout(event.request_id.as_deref().expect("timeout requestId"));
                Ok(())
            }
            other => panic!("unknown control op {other}"),
        };
        assert_reject(result, event.reject, event.reason.as_deref());
    }
    assert_eq!(broker.pending.len(), expect.pending, "pending");
    assert_eq!(broker.accepted, expect.accepted, "accepted");
    assert_eq!(broker.rejected, expect.rejected, "rejected");
    assert_eq!(broker.late_acks, expect.late_acks, "lateAcks");
    assert_eq!(broker.timeouts, expect.timeouts, "timeouts");
    assert_eq!(broker.outcomes, expect.outcomes, "outcomes");
}

// ---------------------------------------------------------------------------
// ActorLeaseExpiryScheduler reference model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LeaseOwner {
    fence: OwnerFence,
    last_idle_at: u64,
}

#[derive(Debug, Clone)]
struct EvictionRequest {
    id: String,
    retries: usize,
    exhausted: bool,
}

#[derive(Debug, Default)]
struct LeaseScheduler {
    owner: Option<LeaseOwner>,
    eviction: Option<EvictionRequest>,
    eviction_seq: u64,
    expired: usize,
    eviction_requests: usize,
    eviction_acked: usize,
    eviction_retries: usize,
    eviction_exhausted: usize,
}

impl LeaseScheduler {
    fn new(owner: Option<LeaseOwner>) -> Self {
        Self {
            owner,
            ..Self::default()
        }
    }

    fn sweep(&mut self, now: u64, idle_ttl_ms: u64) {
        if let Some(owner) = &self.owner {
            if owner.fence.lease_expires_at <= now {
                self.owner = None;
                self.expired += 1;
                return;
            }
        }
        if let Some(eviction) = &mut self.eviction {
            if eviction.exhausted {
                return;
            }
            if eviction.retries >= 3 {
                eviction.exhausted = true;
                self.eviction_exhausted += 1;
            } else {
                eviction.retries += 1;
                self.eviction_retries += 1;
            }
            return;
        }
        let Some(owner) = &self.owner else {
            return;
        };
        if now.saturating_sub(owner.last_idle_at) >= idle_ttl_ms {
            self.eviction_seq += 1;
            let id = format!("evict:{}", self.eviction_seq);
            self.eviction = Some(EvictionRequest {
                id,
                retries: 0,
                exhausted: false,
            });
            self.eviction_requests += 1;
        }
    }

    fn evict_ack(&mut self, eviction_request_id: &str) -> Result<(), &'static str> {
        let Some(eviction) = &self.eviction else {
            return Err("no pending eviction");
        };
        if eviction.id != eviction_request_id {
            return Err("eviction request id mismatch");
        }
        self.owner = None;
        self.eviction = None;
        self.eviction_acked += 1;
        Ok(())
    }

    fn owner_json(&self) -> Option<OwnerJson> {
        self.owner.clone().map(|owner| OwnerJson {
            runtime_id: owner.fence.runtime_id,
            epoch: owner.fence.epoch,
            lease_expires_at: owner.fence.lease_expires_at,
        })
    }
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
    expired: usize,
    #[serde(rename = "evictionRequests")]
    eviction_requests: usize,
    #[serde(rename = "evictionAcked")]
    eviction_acked: usize,
    #[serde(rename = "evictionRetries")]
    eviction_retries: usize,
    #[serde(rename = "evictionExhausted")]
    eviction_exhausted: usize,
    owner: Option<OwnerJson>,
}

fn run_lease(scenario: &Scenario) {
    let events: Vec<LeaseEvent> = scenario
        .events
        .iter()
        .map(|value| serde_json::from_value(value.clone()).expect("lease event"))
        .collect();
    let expect: LeaseExpect =
        serde_json::from_value(scenario.expect.clone()).expect("lease expect");
    let initial = scenario
        .initial_owner
        .clone()
        .expect("lease scenario requires initialOwner");
    let scheduler = LeaseScheduler::new(Some(LeaseOwner {
        fence: OwnerFence {
            runtime_id: initial.runtime_id,
            epoch: initial.epoch,
            lease_expires_at: initial.lease_expires_at,
        },
        last_idle_at: initial.last_idle_at,
    }));
    let mut scheduler = scheduler;
    let idle_ttl_ms = scenario
        .idle_ttl_ms
        .expect("lease scenario requires idleTtlMs");
    for event in &events {
        let result = match event.op.as_str() {
            "sweep" => {
                scheduler.sweep(event.now.expect("sweep now"), idle_ttl_ms);
                Ok(())
            }
            "evictAck" => scheduler.evict_ack(
                event
                    .eviction_request_id
                    .as_deref()
                    .expect("evictAck evictionRequestId"),
            ),
            other => panic!("unknown lease op {other}"),
        };
        assert_reject(result, event.reject, event.reason.as_deref());
    }
    assert_eq!(scheduler.expired, expect.expired, "expired");
    assert_eq!(
        scheduler.eviction_requests, expect.eviction_requests,
        "evictionRequests"
    );
    assert_eq!(
        scheduler.eviction_acked, expect.eviction_acked,
        "evictionAcked"
    );
    assert_eq!(
        scheduler.eviction_retries, expect.eviction_retries,
        "evictionRetries"
    );
    assert_eq!(
        scheduler.eviction_exhausted, expect.eviction_exhausted,
        "evictionExhausted"
    );
    assert_eq!(scheduler.owner_json(), expect.owner, "owner");
}

fn assert_reject(result: Result<(), &'static str>, reject: bool, reason: Option<&str>) {
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
    fn all_required_actor_scenarios_are_present_and_frozen() {
        for (name, raw) in SCENARIOS {
            let scenario: Scenario =
                serde_json::from_str(raw).unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(scenario.schema_version, 1, "{name}: schemaVersion");
            assert_eq!(scenario.scenario, name, "{name}: scenario name");
            assert!(
                REQUIRED_SCENARIOS.contains(&scenario.scenario.as_str()),
                "scenario {name} is not in the frozen required list"
            );
        }
        for required in REQUIRED_SCENARIOS {
            assert!(
                SCENARIOS.iter().any(|(name, _)| *name == required),
                "required scenario {required} is missing"
            );
        }
    }

    #[test]
    fn ownership_scenarios_drive_the_claim_token_reference_model() {
        for (_name, raw) in SCENARIOS {
            let scenario: Scenario = serde_json::from_str(raw).unwrap();
            if scenario.domain == "ownership" {
                run_ownership(&scenario);
            }
        }
    }

    #[test]
    fn activation_scenarios_drive_the_get_or_create_dedup_reference_model() {
        for (_name, raw) in SCENARIOS {
            let scenario: Scenario = serde_json::from_str(raw).unwrap();
            if scenario.domain == "activation" {
                run_activation(&scenario);
            }
        }
    }

    #[test]
    fn invocation_scenarios_drive_the_invocation_relay_reference_model() {
        for (_name, raw) in SCENARIOS {
            let scenario: Scenario = serde_json::from_str(raw).unwrap();
            if scenario.domain == "invocation" {
                run_invocation(&scenario);
            }
        }
    }

    #[test]
    fn control_scenarios_drive_the_owner_control_broker_reference_model() {
        for (_name, raw) in SCENARIOS {
            let scenario: Scenario = serde_json::from_str(raw).unwrap();
            if scenario.domain == "control" {
                run_control(&scenario);
            }
        }
    }

    #[test]
    fn lease_scenarios_drive_the_lease_expiry_scheduler_reference_model() {
        for (_name, raw) in SCENARIOS {
            let scenario: Scenario = serde_json::from_str(raw).unwrap();
            if scenario.domain == "lease" {
                run_lease(&scenario);
            }
        }
    }
}
