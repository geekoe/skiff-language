//! `ActorActivationRequestBroker`: get-or-create operation dedup and
//! activation request/ACK correlation (plan §3.2, C-actor §3.3/§5,
//! C-model-actor §5).
//!
//! The broker never owns claim truth: it holds the `ActorClaimToken` issued
//! by `ActorOwnershipRegistry`, executes the `activateInitial` control
//! through [`ActivationControlPort`] and returns commit/abort to the
//! registry. ACK correlation is exact (request id, owner runtime connection
//! and operation); waiters of the same lineage share one claim, and
//! different test lineages fail closed.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, Mutex};

use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity};
use skiff_runtime_transport::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorMethodDeadlineFrameHeader,
};

use super::health::ActivationHealth;
use super::ownership::{ActorOwnershipRegistry, OwnershipError};
use super::types::{
    ActorClaimToken, ActorLineage, ActorLogicalKey, ActorOwnerRouteAuthority, ActorRef,
    CommitFenceFacts, LeaseIdMint, DEFAULT_ACTIVATION_DEADLINE_MS, DEFAULT_ACTOR_PENDING_BUDGET,
    DEFAULT_ACTOR_TOMBSTONE_BUDGET, DEFAULT_OWNER_LEASE_TTL_MS,
};

/// Broker construction options (C-actor §4 capacity constants).
#[derive(Debug, Clone)]
pub struct ActorActivationBrokerOptions {
    pub activation_deadline_ms: u64,
    pub lease_ttl_ms: u64,
    pub max_claims: usize,
    pub max_tombstones: usize,
}

impl Default for ActorActivationBrokerOptions {
    fn default() -> Self {
        Self {
            activation_deadline_ms: DEFAULT_ACTIVATION_DEADLINE_MS,
            lease_ttl_ms: DEFAULT_OWNER_LEASE_TTL_MS,
            max_claims: DEFAULT_ACTOR_PENDING_BUDGET,
            max_tombstones: DEFAULT_ACTOR_TOMBSTONE_BUDGET,
        }
    }
}

/// Non-blocking `activateInitial` send port.
///
/// The production composition wires this to `ActorOwnerControlBroker` +
/// the exact owner runtime writer; the broker only correlates the ACK by the
/// minted request id.
pub trait ActivationControlPort: Send + Sync + fmt::Debug {
    fn send_activate_initial(&self, request: &ActivateInitialControlRequest) -> Result<(), String>;
}

/// Typed `activateInitial` control payload handed to the control/writer seam.
#[derive(Debug, Clone)]
pub struct ActivateInitialControlRequest {
    pub request_id: String,
    pub actor_key: ActorLogicalKey,
    pub facts: CommitFenceFacts,
    pub owner_runtime_id: String,
    pub owner_connection: String,
    pub route_authority: ActorOwnerRouteAuthority,
    pub bootstrap_bytes: Vec<u8>,
    pub deadline: ActorMethodDeadlineFrameHeader,
    pub test_case_capability: Option<String>,
    pub test_case_parent_request_id: Option<String>,
}

/// One typed get-or-create admission (C-actor §3.3).
#[derive(Debug, Clone)]
pub struct ActorGetOrCreateRequest {
    pub rpc_id: String,
    pub actor_key: ActorLogicalKey,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
    pub bootstrap_bytes: Vec<u8>,
    pub owner_runtime_id: String,
    pub owner_connection: String,
    pub route_authority: ActorOwnerRouteAuthority,
    pub deadline: Option<ActorMethodDeadlineFrameHeader>,
    pub test_case_capability: Option<String>,
    pub test_case_parent_request_id: Option<String>,
    pub now: u64,
}

/// Immediate result of a get-or-create admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetOrCreateOutcome {
    /// Existing owner fence resolved without a reservation.
    Resolved(ActorRef),
    /// Joined an in-flight claim of the same lineage.
    Joined,
    /// First caller: claim reserved and `activateInitial` dispatched.
    StartedActivation {
        request_id: String,
    },
    /// Different test lineage while a claim is in flight; both fail closed.
    LineageConflict,
    Saturated,
    Failed {
        code: String,
    },
}

/// Result of correlating an activation ACK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationAckOutcome {
    Committed { epoch: u64, waiters: Vec<String> },
    Aborted { waiters: Vec<String> },
    CommitRejected { waiters: Vec<String> },
    LateAck,
    WrongCorrelation,
}

/// Result of an activation deadline expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationTimeoutOutcome {
    pub waiters: Vec<String>,
}

/// Final per-waiter outcome (corpus vocabulary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivationWaiterOutcome {
    Resolved { epoch: u64 },
    Failed { code: String },
}

impl ActivationWaiterOutcome {
    pub fn as_corpus_string(&self) -> String {
        match self {
            Self::Resolved { epoch } => format!("resolved:{epoch}"),
            Self::Failed { code } => format!("failed:{code}"),
        }
    }
}

#[derive(Debug, Clone)]
struct ActivationClaim {
    token: ActorClaimToken,
    facts: CommitFenceFacts,
    lineage: ActorLineage,
    owner_connection: String,
    waiters: Vec<String>,
    deadline_at: u64,
}

#[derive(Debug, Default)]
struct ActivationCounters {
    dedup_joins: u64,
    lineage_conflicts: u64,
    commits: u64,
    aborts: u64,
    timeouts: u64,
    late_acks: u64,
    wrong_correlation: u64,
    saturated: u64,
}

#[derive(Debug, Default)]
struct ActivationInner {
    claims: BTreeMap<ActorLogicalKey, ActivationClaim>,
    by_request_id: BTreeMap<String, ActorLogicalKey>,
    tombstones: BTreeSet<String>,
    outcomes: BTreeMap<String, ActivationWaiterOutcome>,
    counters: ActivationCounters,
}

/// get-or-create dedup/ACK owner (C-actor §2).
#[derive(Debug)]
pub struct ActorActivationRequestBroker {
    registry: Arc<ActorOwnershipRegistry>,
    control: Arc<dyn ActivationControlPort>,
    options: ActorActivationBrokerOptions,
    inner: Arc<Mutex<ActivationInner>>,
    lease_mint: LeaseIdMint,
}

impl ActorActivationRequestBroker {
    pub fn new(
        registry: Arc<ActorOwnershipRegistry>,
        control: Arc<dyn ActivationControlPort>,
        options: ActorActivationBrokerOptions,
    ) -> Self {
        Self {
            registry,
            control,
            options,
            inner: Arc::new(Mutex::new(ActivationInner::default())),
            lease_mint: LeaseIdMint::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ActivationInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// get-or-create admission (C-actor §5): dedup by actor logical key,
    /// join same lineage, reject different lineage, resolve existing owner.
    pub fn get_or_create(&self, request: &ActorGetOrCreateRequest) -> GetOrCreateOutcome {
        let mut inner = self.lock();
        if inner.claims.len() >= self.options.max_claims {
            inner.counters.saturated += 1;
            return GetOrCreateOutcome::Saturated;
        }
        let lineage = ActorLineage::from_test_case(request.test_case_capability.as_deref());
        if let Some(claim) = inner.claims.get(&request.actor_key) {
            if claim.lineage != lineage {
                inner.counters.lineage_conflicts += 1;
                inner.outcomes.insert(
                    request.rpc_id.clone(),
                    ActivationWaiterOutcome::Failed {
                        code: "ActorCreateLineageConflict".to_string(),
                    },
                );
                return GetOrCreateOutcome::LineageConflict;
            }
            inner
                .claims
                .get_mut(&request.actor_key)
                .expect("claim exists")
                .waiters
                .push(request.rpc_id.clone());
            inner.counters.dedup_joins += 1;
            return GetOrCreateOutcome::Joined;
        }
        if let Some(fence) = self.registry.current_owner(&request.actor_key) {
            let actor_ref = request.actor_key.to_actor_ref(fence.epoch);
            inner.outcomes.insert(
                request.rpc_id.clone(),
                ActivationWaiterOutcome::Resolved { epoch: fence.epoch },
            );
            return GetOrCreateOutcome::Resolved(actor_ref);
        }

        let facts = self.registry.ensure_present(
            &request.actor_key,
            request.actor_abi_identity.clone(),
            request.actor_implementation_identity.clone(),
            request.declaration_owner.clone(),
        );
        let token = match self.registry.reserve(
            &request.actor_key,
            facts.epoch,
            &request.owner_runtime_id,
            &request.route_authority,
            request.now,
        ) {
            Ok(token) => token,
            Err(error) => {
                inner.outcomes.insert(
                    request.rpc_id.clone(),
                    ActivationWaiterOutcome::Failed {
                        code: ownership_failure_code(&error),
                    },
                );
                return GetOrCreateOutcome::Failed {
                    code: ownership_failure_code(&error),
                };
            }
        };

        let request_id = token.claim_id.as_str().to_string();
        let owner_lease_id = self.lease_mint.mint();
        let deadline_at = request
            .now
            .saturating_add(self.options.activation_deadline_ms);
        let claim = ActivationClaim {
            token: token.clone(),
            facts: CommitFenceFacts {
                actor_abi_identity: request.actor_abi_identity.clone(),
                actor_implementation_identity: request.actor_implementation_identity.clone(),
                declaration_owner: request.declaration_owner.clone(),
                owner_lease_id: owner_lease_id.clone(),
            },
            lineage,
            owner_connection: request.owner_connection.clone(),
            waiters: vec![request.rpc_id.clone()],
            deadline_at,
        };
        inner.claims.insert(request.actor_key.clone(), claim);
        inner
            .by_request_id
            .insert(request_id.clone(), request.actor_key.clone());

        let control_request = ActivateInitialControlRequest {
            request_id: request_id.clone(),
            actor_key: request.actor_key.clone(),
            facts: CommitFenceFacts {
                actor_abi_identity: request.actor_abi_identity.clone(),
                actor_implementation_identity: request.actor_implementation_identity.clone(),
                declaration_owner: request.declaration_owner.clone(),
                owner_lease_id: owner_lease_id.clone(),
            },
            owner_runtime_id: request.owner_runtime_id.clone(),
            owner_connection: request.owner_connection.clone(),
            route_authority: request.route_authority.clone(),
            bootstrap_bytes: request.bootstrap_bytes.clone(),
            deadline: request
                .deadline
                .clone()
                .unwrap_or(ActorMethodDeadlineFrameHeader {
                    timeout_ms: self.options.activation_deadline_ms,
                    expires_at: "activation-deadline".to_string(),
                }),
            test_case_capability: request.test_case_capability.clone(),
            test_case_parent_request_id: request.test_case_parent_request_id.clone(),
        };
        if let Err(write_error) = self.control.send_activate_initial(&control_request) {
            let _ = write_error;
            let waiters = inner
                .claims
                .remove(&request.actor_key)
                .expect("claim exists")
                .waiters;
            inner.by_request_id.remove(&request_id);
            inner.tombstones.insert(request_id);
            let _ = self.registry.abort(&token);
            inner.counters.aborts += 1;
            for waiter in &waiters {
                inner.outcomes.insert(
                    waiter.clone(),
                    ActivationWaiterOutcome::Failed {
                        code: "AckRejected".to_string(),
                    },
                );
            }
            return GetOrCreateOutcome::Failed {
                code: "AckRejected".to_string(),
            };
        }
        GetOrCreateOutcome::StartedActivation { request_id }
    }

    /// Correlates the `activateInitial` ACK (exact request id + owner
    /// runtime connection) and commits/aborts the token back to the registry.
    pub fn on_activation_ack(
        &self,
        request_id: &str,
        runtime_id: &str,
        connection: &str,
        accepted: bool,
        now: u64,
    ) -> ActivationAckOutcome {
        let mut inner = self.lock();
        let Some(key) = inner.by_request_id.get(request_id).cloned() else {
            if inner.tombstones.contains(request_id) {
                inner.counters.late_acks += 1;
                return ActivationAckOutcome::LateAck;
            }
            inner.counters.wrong_correlation += 1;
            return ActivationAckOutcome::WrongCorrelation;
        };
        let Some(claim) = inner.claims.get(&key).cloned() else {
            inner.counters.wrong_correlation += 1;
            return ActivationAckOutcome::WrongCorrelation;
        };
        if claim.token.owner_runtime_id != runtime_id || claim.owner_connection != connection {
            inner.counters.wrong_correlation += 1;
            return ActivationAckOutcome::WrongCorrelation;
        }
        let waiters = claim.waiters.clone();
        let token = claim.token.clone();
        let facts = claim.facts.clone();
        inner.claims.remove(&key);
        inner.by_request_id.remove(request_id);
        inner.tombstones.insert(request_id.to_string());
        if accepted {
            match self
                .registry
                .commit(&token, &facts, now, self.options.lease_ttl_ms)
            {
                Ok(fence) => {
                    inner.counters.commits += 1;
                    for waiter in &waiters {
                        inner.outcomes.insert(
                            waiter.clone(),
                            ActivationWaiterOutcome::Resolved { epoch: fence.epoch },
                        );
                    }
                    ActivationAckOutcome::Committed {
                        epoch: fence.epoch,
                        waiters,
                    }
                }
                Err(_) => {
                    for waiter in &waiters {
                        inner.outcomes.insert(
                            waiter.clone(),
                            ActivationWaiterOutcome::Failed {
                                code: "CommitRejected".to_string(),
                            },
                        );
                    }
                    ActivationAckOutcome::CommitRejected { waiters }
                }
            }
        } else {
            let _ = self.registry.abort(&token);
            inner.counters.aborts += 1;
            for waiter in &waiters {
                inner.outcomes.insert(
                    waiter.clone(),
                    ActivationWaiterOutcome::Failed {
                        code: "AckRejected".to_string(),
                    },
                );
            }
            ActivationAckOutcome::Aborted { waiters }
        }
    }

    /// Expires every activation claim whose deadline elapsed (timer sweep).
    pub fn expire_deadlines(&self, now: u64) -> Vec<ActivationTimeoutOutcome> {
        let request_ids = {
            let inner = self.lock();
            inner
                .by_request_id
                .iter()
                .filter_map(|(request_id, key)| {
                    let claim = inner.claims.get(key)?;
                    (claim.deadline_at <= now).then(|| request_id.clone())
                })
                .collect::<Vec<_>>()
        };
        request_ids
            .into_iter()
            .filter_map(|request_id| self.on_activation_timeout(&request_id, now))
            .collect()
    }

    /// Activation deadline terminal: abort the token and fail all waiters.
    pub fn on_activation_timeout(
        &self,
        request_id: &str,
        now: u64,
    ) -> Option<ActivationTimeoutOutcome> {
        let mut inner = self.lock();
        let key = inner.by_request_id.get(request_id)?.clone();
        let claim = inner.claims.remove(&key)?;
        inner.by_request_id.remove(request_id);
        inner.tombstones.insert(request_id.to_string());
        inner.counters.timeouts += 1;
        let _ = self.registry.abort(&claim.token);
        inner.counters.aborts += 1;
        let waiters = claim.waiters.clone();
        for waiter in &waiters {
            inner.outcomes.insert(
                waiter.clone(),
                ActivationWaiterOutcome::Failed {
                    code: "ActivationTimeout".to_string(),
                },
            );
        }
        let _ = now;
        Some(ActivationTimeoutOutcome { waiters })
    }

    /// Owner disconnect: frozen semantics keep waiters suspended until the
    /// activation deadline; stale ACKs are rejected by exact connection.
    pub fn on_owner_disconnect(&self, _runtime_id: &str, _connection: &str) {}

    /// Shutdown: fail all waiters, abort tokens, clear claims/tombstones.
    pub fn shutdown(&self) -> Vec<ActivationTimeoutOutcome> {
        let mut inner = self.lock();
        let keys = inner.claims.keys().cloned().collect::<Vec<_>>();
        let mut outcomes = Vec::new();
        for key in keys {
            let Some(claim) = inner.claims.remove(&key) else {
                continue;
            };
            let request_id = claim.token.claim_id.as_str().to_string();
            inner.by_request_id.remove(&request_id);
            let _ = self.registry.abort(&claim.token);
            inner.counters.aborts += 1;
            let waiters = claim.waiters.clone();
            for waiter in &waiters {
                inner.outcomes.insert(
                    waiter.clone(),
                    ActivationWaiterOutcome::Failed {
                        code: "RouterShutdown".to_string(),
                    },
                );
            }
            outcomes.push(ActivationTimeoutOutcome { waiters });
        }
        inner.tombstones.clear();
        outcomes
    }

    /// Final outcome of one waiter rpc id (corpus assertions).
    pub fn outcome_for(&self, rpc_id: &str) -> Option<String> {
        self.lock()
            .outcomes
            .get(rpc_id)
            .map(ActivationWaiterOutcome::as_corpus_string)
    }

    pub fn health(&self) -> ActivationHealth {
        let inner = self.lock();
        ActivationHealth {
            pending_claims: inner.claims.len(),
            pending_waiters: inner.claims.values().map(|claim| claim.waiters.len()).sum(),
            dedup_joins: inner.counters.dedup_joins,
            lineage_conflicts: inner.counters.lineage_conflicts,
            commits: inner.counters.commits,
            aborts: inner.counters.aborts,
            timeouts: inner.counters.timeouts,
            late_acks: inner.counters.late_acks,
            wrong_correlation: inner.counters.wrong_correlation,
            saturated: inner.counters.saturated,
            tombstones: inner.tombstones.len(),
        }
    }
}

fn ownership_failure_code(error: &OwnershipError) -> String {
    match error {
        OwnershipError::ClaimConflict { .. } => "ActorCreateConflict".to_string(),
        OwnershipError::EpochMismatch { .. } => "IncarnationReplaced".to_string(),
        OwnershipError::NotPresent => "NotPresent".to_string(),
        OwnershipError::ReservationInFlight => "ActorCreateConflict".to_string(),
        _ => "ActorCreateConflict".to_string(),
    }
}
