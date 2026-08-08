//! `ActorOwnershipRegistry`: actor identity / incarnation / current owner
//! fence and the authoritative claim reserve/commit/abort surface
//! (authority design §3.2, C-actor §3.2/§4.2, C-model-actor §4).
//!
//! Invariants:
//! - one actor key has at most one current owner fence and at most one
//!   in-flight reservation;
//! - a reservation is not an owner; `ActorClaimToken` is the only
//!   authoritative commit/abort channel and is invalid after one use;
//! - brokers never write owner fields; all lease/incarnation transitions
//!   (renew/release/expire/eviction-ack/replace) mutate truth here.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity};
use skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader;

use super::health::OwnershipHealth;
use super::types::{
    ActorClaimToken, ActorEntryFacts, ActorIncarnationFence, ActorLogicalKey, ActorOwnerFence,
    ActorOwnerRouteAuthority, CommitFenceFacts, ExpiredOwner, OwnerReleaseReason,
};

/// Fail-closed ownership transition errors (C-actor §4.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnershipError {
    NotPresent,
    EpochMismatch {
        current_epoch: u64,
    },
    ClaimConflict {
        current_fence: Option<ActorOwnerFence>,
    },
    ReservationInFlight,
    NoReservation,
    FenceMismatch,
    LeaseExpired,
    EvictionMismatch,
    FenceFactsMismatch,
}

impl fmt::Display for OwnershipError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPresent => write!(formatter, "actor key is not present"),
            Self::EpochMismatch { current_epoch } => {
                write!(formatter, "epoch mismatch (current epoch {current_epoch})")
            }
            Self::ClaimConflict { current_fence } => {
                write!(formatter, "claim conflict")?;
                if let Some(fence) = current_fence {
                    write!(
                        formatter,
                        ": a current owner fence is held by {}",
                        fence.owner_runtime_id
                    )?;
                }
                Ok(())
            }
            Self::ReservationInFlight => write!(formatter, "a reservation is already in flight"),
            Self::NoReservation => write!(formatter, "no reservation for caller"),
            Self::FenceMismatch => write!(formatter, "fence mismatch"),
            Self::LeaseExpired => write!(formatter, "owner lease is expired"),
            Self::EvictionMismatch => write!(formatter, "eviction request id mismatch"),
            Self::FenceFactsMismatch => write!(formatter, "commit fence facts mismatch"),
        }
    }
}

impl std::error::Error for OwnershipError {}

#[derive(Debug, Clone)]
struct ActorEntry {
    epoch: u64,
    actor_abi_identity: ActorAbiIdentity,
    actor_implementation_identity: ActorImplementationIdentity,
    declaration_owner: ActorDeclarationOwnerFrameHeader,
    /// Frozen create input (canonical JSON array bytes) saved at entry
    /// creation (put-if-absent). Used by get-or-activate to cold-activate the
    /// entry's implementation when no live incarnation exists; the task
    /// `ActorActivationSnapshot` is only used when this entry is missing.
    create_input: Vec<u8>,
    owner: Option<ActorOwnerFence>,
    eviction_request_id: Option<String>,
    reservation: Option<ActorClaimToken>,
}

/// Registry entry identity / create-input facts exposed to the task
/// actor-method admission lane (read-only view of the entry truth).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorRegistryEntry {
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
    pub create_input: Vec<u8>,
}

#[derive(Debug, Default)]
struct OwnershipCounters {
    commits: u64,
    aborts: u64,
    conflicts: u64,
    renewals: u64,
    releases: u64,
    expired: u64,
    epoch_mismatches: u64,
    rejected_commits: u64,
    rejected_aborts: u64,
}

#[derive(Debug, Default)]
struct OwnershipInner {
    entries: BTreeMap<ActorLogicalKey, ActorEntry>,
    next_claim_seq: u64,
    counters: OwnershipCounters,
}

/// Single owner of actor ownership truth (C-actor §2).
#[derive(Debug, Clone)]
pub struct ActorOwnershipRegistry {
    inner: Arc<Mutex<OwnershipInner>>,
}

impl Default for ActorOwnershipRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ActorOwnershipRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(OwnershipInner::default())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, OwnershipInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Makes an actor key present (epoch 1 when absent) and returns its
    /// incarnation facts. Identity fields are fixed at first creation.
    pub fn ensure_present(
        &self,
        key: &ActorLogicalKey,
        actor_abi_identity: ActorAbiIdentity,
        actor_implementation_identity: ActorImplementationIdentity,
        declaration_owner: ActorDeclarationOwnerFrameHeader,
        create_input: &[u8],
    ) -> ActorEntryFacts {
        let mut inner = self.lock();
        let epoch = inner
            .entries
            .entry(key.clone())
            .or_insert_with(|| ActorEntry {
                epoch: 1,
                actor_abi_identity: actor_abi_identity.clone(),
                actor_implementation_identity: actor_implementation_identity.clone(),
                declaration_owner: declaration_owner.clone(),
                create_input: create_input.to_vec(),
                owner: None,
                eviction_request_id: None,
                reservation: None,
            })
            .epoch;
        ActorEntryFacts { epoch }
    }

    /// Reserves the actor key for a concurrent first owner and issues the
    /// authoritative `ActorClaimToken`. Refuses when a reservation is in
    /// flight or a valid owner fence is held (C-actor §4.2).
    pub fn reserve(
        &self,
        key: &ActorLogicalKey,
        expected_epoch: u64,
        owner_runtime_id: &str,
        route_authority: &ActorOwnerRouteAuthority,
        now: u64,
    ) -> Result<ActorClaimToken, OwnershipError> {
        let mut inner = self.lock();
        if inner
            .entries
            .get(key)
            .is_some_and(|entry| entry.reservation.is_some())
        {
            inner.counters.conflicts += 1;
            return Err(OwnershipError::ReservationInFlight);
        }
        let valid_owner = inner
            .entries
            .get(key)
            .and_then(|entry| entry.owner.as_ref())
            .filter(|fence| fence.lease_expires_at > now)
            .cloned();
        if let Some(fence) = valid_owner {
            inner.counters.conflicts += 1;
            return Err(OwnershipError::ClaimConflict {
                current_fence: Some(fence),
            });
        }
        let current_epoch = inner
            .entries
            .get(key)
            .map(|entry| entry.epoch)
            .ok_or(OwnershipError::NotPresent)?;
        if expected_epoch != current_epoch {
            inner.counters.epoch_mismatches += 1;
            return Err(OwnershipError::EpochMismatch { current_epoch });
        }
        let seq = inner.next_claim_seq;
        inner.next_claim_seq += 1;
        let token = ActorClaimToken {
            claim_id: super::types::ActorClaimId::mint(seq),
            actor_key: key.clone(),
            expected_epoch,
            owner_runtime_id: owner_runtime_id.to_string(),
            route_authority: route_authority.clone(),
        };
        inner
            .entries
            .get_mut(key)
            .expect("actor key present")
            .reservation = Some(token.clone());
        Ok(token)
    }

    /// Commits a reservation into the current owner fence (epoch monotonic
    /// per incarnation; the lease id is the broker-minted
    /// `facts.owner_lease_id`, identical to the `activateInitial` wire
    /// fence — E-actor-parity single-mint reconciliation). The token is
    /// invalidated.
    pub fn commit(
        &self,
        token: &ActorClaimToken,
        facts: &CommitFenceFacts,
        now: u64,
        lease_ttl_ms: u64,
    ) -> Result<ActorOwnerFence, OwnershipError> {
        let mut inner = self.lock();
        let entry = inner
            .entries
            .get_mut(&token.actor_key)
            .ok_or(OwnershipError::NotPresent)?;
        let Some(reserved) = &entry.reservation else {
            inner.counters.rejected_commits += 1;
            return Err(OwnershipError::NoReservation);
        };
        if reserved != token {
            inner.counters.rejected_commits += 1;
            return Err(OwnershipError::NoReservation);
        }
        if facts.actor_abi_identity != entry.actor_abi_identity
            || facts.actor_implementation_identity != entry.actor_implementation_identity
            || facts.declaration_owner != entry.declaration_owner
        {
            inner.counters.rejected_commits += 1;
            return Err(OwnershipError::FenceFactsMismatch);
        }
        let fence = ActorOwnerFence {
            epoch: token.expected_epoch,
            owner_runtime_id: token.owner_runtime_id.clone(),
            owner_lease_id: facts.owner_lease_id.clone(),
            lease_expires_at: now.saturating_add(lease_ttl_ms),
            actor_abi_identity: facts.actor_abi_identity.clone(),
            actor_implementation_identity: facts.actor_implementation_identity.clone(),
            declaration_owner: facts.declaration_owner.clone(),
        };
        entry.reservation = None;
        entry.owner = Some(fence.clone());
        inner.counters.commits += 1;
        Ok(fence)
    }

    /// Aborts a reservation with no owner effect. The token is invalidated.
    pub fn abort(&self, token: &ActorClaimToken) -> Result<(), OwnershipError> {
        let mut inner = self.lock();
        let entry = inner
            .entries
            .get_mut(&token.actor_key)
            .ok_or(OwnershipError::NotPresent)?;
        match &entry.reservation {
            Some(reserved) if reserved == token => {
                entry.reservation = None;
                inner.counters.aborts += 1;
                Ok(())
            }
            _ => {
                inner.counters.rejected_aborts += 1;
                Err(OwnershipError::NoReservation)
            }
        }
    }

    /// Renews an exact owner fence lease (identity-exact; lease not expired).
    pub fn renew(
        &self,
        key: &ActorLogicalKey,
        fence: &ActorOwnerFence,
        ttl_ms: u64,
        now: u64,
    ) -> Result<ActorOwnerFence, OwnershipError> {
        let mut inner = self.lock();
        let entry = inner
            .entries
            .get_mut(key)
            .ok_or(OwnershipError::NotPresent)?;
        let Some(current) = &entry.owner else {
            return Err(OwnershipError::FenceMismatch);
        };
        if !fence_identity_matches(current, fence) {
            return Err(OwnershipError::FenceMismatch);
        }
        if current.lease_expires_at <= now {
            return Err(OwnershipError::LeaseExpired);
        }
        let renewed = ActorOwnerFence {
            lease_expires_at: now.saturating_add(ttl_ms),
            ..current.clone()
        };
        entry.owner = Some(renewed.clone());
        inner.counters.renewals += 1;
        Ok(renewed)
    }

    /// Releases an exact owner fence (disconnect/upgrade/shutdown path).
    pub fn release(
        &self,
        key: &ActorLogicalKey,
        fence: &ActorOwnerFence,
        _reason: OwnerReleaseReason,
    ) -> Result<(), OwnershipError> {
        let mut inner = self.lock();
        let entry = inner
            .entries
            .get_mut(key)
            .ok_or(OwnershipError::NotPresent)?;
        let Some(current) = &entry.owner else {
            return Err(OwnershipError::FenceMismatch);
        };
        if !fence_identity_matches(current, fence) {
            return Err(OwnershipError::FenceMismatch);
        }
        entry.owner = None;
        entry.eviction_request_id = None;
        inner.counters.releases += 1;
        Ok(())
    }

    /// Expires every owner fence whose lease has elapsed. Returns the exact
    /// expired fences for scheduler/control cleanup.
    pub fn expire(&self, now: u64) -> Vec<ExpiredOwner> {
        let mut inner = self.lock();
        let keys = inner.entries.keys().cloned().collect::<Vec<_>>();
        let mut expired = Vec::new();
        for key in keys {
            let Some(entry) = inner.entries.get_mut(&key) else {
                continue;
            };
            if let Some(fence) = &entry.owner {
                if fence.lease_expires_at <= now {
                    let fence = fence.clone();
                    entry.owner = None;
                    entry.eviction_request_id = None;
                    inner.counters.expired += 1;
                    expired.push(ExpiredOwner {
                        actor_key: key,
                        fence,
                    });
                }
            }
        }
        expired
    }

    /// Current owner fence (identity-exact snapshot), if any.
    pub fn current_owner(&self, key: &ActorLogicalKey) -> Option<ActorOwnerFence> {
        self.lock()
            .entries
            .get(key)
            .and_then(|entry| entry.owner.clone())
    }

    /// Keys that currently hold an owner fence.
    pub fn owned_keys(&self) -> Vec<ActorLogicalKey> {
        self.lock()
            .entries
            .iter()
            .filter(|(_, entry)| entry.owner.is_some())
            .map(|(key, _)| key.clone())
            .collect()
    }

    /// Current incarnation epoch of one key.
    pub fn entry_epoch(&self, key: &ActorLogicalKey) -> Option<u64> {
        self.lock().entries.get(key).map(|entry| entry.epoch)
    }

    /// Read-only registry entry facts (identity + frozen create input).
    /// `None` when the key is not present (snapshot restoration path).
    pub fn entry(&self, key: &ActorLogicalKey) -> Option<ActorRegistryEntry> {
        self.lock()
            .entries
            .get(key)
            .map(|entry| ActorRegistryEntry {
                actor_abi_identity: entry.actor_abi_identity.clone(),
                actor_implementation_identity: entry.actor_implementation_identity.clone(),
                declaration_owner: entry.declaration_owner.clone(),
                create_input: entry.create_input.clone(),
            })
    }

    /// Marks an owner fence as eviction-requested (scheduler trigger; truth
    /// still lives here).
    pub fn request_eviction(
        &self,
        key: &ActorLogicalKey,
        eviction_request_id: &str,
    ) -> Result<(), OwnershipError> {
        let mut inner = self.lock();
        let entry = inner
            .entries
            .get_mut(key)
            .ok_or(OwnershipError::NotPresent)?;
        if entry.owner.is_none() {
            return Err(OwnershipError::FenceMismatch);
        }
        entry.eviction_request_id = Some(eviction_request_id.to_string());
        Ok(())
    }

    /// Acknowledges an eviction: removes the owner fence only when the exact
    /// eviction request id is recorded.
    pub fn acknowledge_eviction(
        &self,
        key: &ActorLogicalKey,
        eviction_request_id: &str,
    ) -> Result<(), OwnershipError> {
        let mut inner = self.lock();
        let entry = inner
            .entries
            .get_mut(key)
            .ok_or(OwnershipError::NotPresent)?;
        if entry.owner.is_none()
            || entry
                .eviction_request_id
                .as_deref()
                .is_none_or(|recorded| recorded != eviction_request_id)
        {
            return Err(OwnershipError::EvictionMismatch);
        }
        entry.owner = None;
        entry.eviction_request_id = None;
        Ok(())
    }

    /// Advances the incarnation (replace/remove seam): epoch +1 and all
    /// owner/reservation/eviction state is cleared.
    pub fn advance_incarnation(&self, key: &ActorLogicalKey) -> Result<u64, OwnershipError> {
        let mut inner = self.lock();
        let entry = inner
            .entries
            .get_mut(key)
            .ok_or(OwnershipError::NotPresent)?;
        entry.epoch += 1;
        entry.owner = None;
        entry.eviction_request_id = None;
        entry.reservation = None;
        Ok(entry.epoch)
    }

    pub fn health(&self) -> OwnershipHealth {
        let inner = self.lock();
        OwnershipHealth {
            current_fences: inner
                .entries
                .values()
                .filter(|entry| entry.owner.is_some())
                .count(),
            in_flight_reservations: inner
                .entries
                .values()
                .filter(|entry| entry.reservation.is_some())
                .count(),
            commits: inner.counters.commits,
            aborts: inner.counters.aborts,
            conflicts: inner.counters.conflicts,
            renewals: inner.counters.renewals,
            releases: inner.counters.releases,
            expired: inner.counters.expired,
            epoch_mismatches: inner.counters.epoch_mismatches,
            rejected_commits: inner.counters.rejected_commits,
            rejected_aborts: inner.counters.rejected_aborts,
        }
    }

    /// For tests: current incarnation as a fence type (plan §3.4).
    pub fn incarnation(&self, key: &ActorLogicalKey) -> Option<ActorIncarnationFence> {
        self.entry_epoch(key).map(ActorIncarnationFence)
    }
}

fn fence_identity_matches(current: &ActorOwnerFence, candidate: &ActorOwnerFence) -> bool {
    current.epoch == candidate.epoch
        && current.owner_runtime_id == candidate.owner_runtime_id
        && current.owner_lease_id == candidate.owner_lease_id
        && current.actor_abi_identity == candidate.actor_abi_identity
        && current.actor_implementation_identity == candidate.actor_implementation_identity
        && current.declaration_owner == candidate.declaration_owner
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity};
    use skiff_runtime_transport::actor_method::{
        ActorDeclarationOwnerFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
    };

    use super::{ActorOwnershipRegistry, OwnershipError};
    use crate::actor::lease::{
        ActorLeaseExpiryScheduler, IdleEvictControlPort, LeaseSchedulerOptions,
    };
    use crate::actor::types::{
        ActorLogicalKey, ActorOwnerFence, ActorOwnerRouteAuthority, CommitFenceFacts, LeaseIdMint,
        DEFAULT_IDLE_TTL_MS, DEFAULT_OWNER_LEASE_TTL_MS,
    };

    fn test_key(service_id: &str, actor_symbol: &str, suffix: &str) -> ActorLogicalKey {
        ActorLogicalKey {
            service_id: service_id.to_string(),
            actor_type_identity: actor_symbol.to_string(),
            actor_id_type_identity: "string".to_string(),
            actor_id_encoding_version: "v1".to_string(),
            canonical_actor_id_key_bytes_base64: format!("base64:{suffix}"),
            actor_id_hash: format!("sha256:{suffix}"),
        }
    }

    fn test_owner_frame(actor_symbol: &str) -> ActorDeclarationOwnerFrameHeader {
        ActorDeclarationOwnerFrameHeader {
            unit: ActorOwnerUnitFrameHeader::Service,
            file: ActorOwnerFileFrameHeader::FileIrIdentity("file-ir-v1".to_string()),
            actor_symbol: actor_symbol.to_string(),
        }
    }

    fn test_abi() -> ActorAbiIdentity {
        ActorAbiIdentity::new("skiff-actor-runtime-abi-v1")
    }

    fn test_implementation() -> ActorImplementationIdentity {
        ActorImplementationIdentity::new("impl-v1")
    }

    #[derive(Debug, Default)]
    struct RecordingIdleEvictPort {
        sent: Arc<Mutex<Vec<String>>>,
    }

    impl IdleEvictControlPort for RecordingIdleEvictPort {
        fn send_idle_evict(
            &self,
            key: &ActorLogicalKey,
            _fence: &ActorOwnerFence,
            eviction_request_id: &str,
            _connection: &str,
        ) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(format!("{}:{eviction_request_id}", key.actor_id_hash));
            Ok(())
        }
    }

    // README §5.8 known issue (1), target phase Phase 7
    // (phase-7-actor-router.md §5: "idle request发出后Router在ack前不清fence；
    // ... owner lease expiry不会形成双owner").
    //
    // "Fix the current default lifecycle hazard in which owner lease TTL and
    // idle TTL are both 30 seconds and the sweep can expire the Router lease
    // before completing/acknowledging `IdleEvict`. A new owner cannot be
    // opened while the old Runtime instance may still exist."
    //
    // `LeaseScheduler::sweep` runs `registry.expire(now)` (ownership.rs) ahead
    // of the idle-eviction branch, so with both clocks at 30s the fence is
    // silently dropped at the first due tick and `IdleEvict` is never sent or
    // acknowledged; the registry then admits a new owner although the old
    // Runtime incarnation may still exist. This test FAILS today.
    #[test]
    fn sweep_must_not_open_new_owner_while_idle_evict_is_unacked() {
        // Precondition of the hazard: both clocks are 30s (C-actor §4/§8).
        assert_eq!(DEFAULT_OWNER_LEASE_TTL_MS, DEFAULT_IDLE_TTL_MS);
        let registry = Arc::new(ActorOwnershipRegistry::new());
        let control = Arc::new(RecordingIdleEvictPort::default());
        let scheduler = ActorLeaseExpiryScheduler::new(
            Arc::clone(&registry),
            control.clone(),
            LeaseSchedulerOptions {
                idle_ttl_ms: DEFAULT_IDLE_TTL_MS,
                max_eviction_retries: 3,
            },
        );
        let key = test_key("svc.idle-order", "SessionActor", "user-1");
        let authority = ActorOwnerRouteAuthority {
            build_id: "sha256:build-v1".to_string(),
        };
        let owner_frame = test_owner_frame("SessionActor");
        let abi = test_abi();
        let implementation = test_implementation();
        registry.ensure_present(
            &key,
            abi.clone(),
            implementation.clone(),
            owner_frame.clone(),
            b"[]",
        );
        let token = registry
            .reserve(&key, 1, "runtime-a", &authority, 0)
            .expect("first owner reserves");
        let facts = CommitFenceFacts {
            actor_abi_identity: abi,
            actor_implementation_identity: implementation,
            declaration_owner: owner_frame,
            owner_lease_id: LeaseIdMint::new().mint(),
        };
        let _fence = registry
            .commit(&token, &facts, 0, DEFAULT_OWNER_LEASE_TTL_MS)
            .expect("first owner commits");
        scheduler.mark_live(&key, 0, "runtime-a-connection");

        // One sweep at t=30s: both the owner lease (30s) and the idle TTL
        // (30s) are due in the same tick. Today the sweep expires the lease
        // first and never sends/acknowledges `IdleEvict`.
        scheduler.sweep(DEFAULT_OWNER_LEASE_TTL_MS);

        // Designed invariant (README §5.8): lease expiry is not proof that
        // the Runtime state was destroyed while the `IdleEvict` is not
        // completed/acknowledged; a new owner must not be openable.
        let fence_survived = registry.current_owner(&key).is_some();
        let new_owner_admitted = registry
            .reserve(&key, 1, "runtime-b", &authority, DEFAULT_OWNER_LEASE_TTL_MS)
            .is_ok();
        let idle_evict_frames_sent = control.sent.lock().unwrap_or_else(|p| p.into_inner()).len();
        assert!(
            !new_owner_admitted,
            "lease expiry opened a new owner while the old incarnation may \
             still exist (IdleEvict frames sent: {idle_evict_frames_sent}, \
             owner fence survived: {fence_survived})",
        );
    }

    // README §5.8 known issue (2), target phase Phase 3A/7
    // (phase-3a-deployment-owner.md exact-build owner cut;
    // phase-7-actor-router.md §5: "stale build/fence/epoch/cancel
    // continuation拒绝且不重装lease").
    //
    // "Add exact build to the owner fence and continuation validation."
    //
    // `ActorOwnerFence` carries no build identity: `commit` drops the claim's
    // `ActorOwnerRouteAuthority::build_id` and `fence_identity_matches`
    // compares no build, so a continuation minted by an older build is
    // byte-identical to the live fence in every compared field and passes
    // validation. This test FAILS today.
    #[test]
    fn continuation_without_exact_build_proof_must_be_rejected() {
        let registry = Arc::new(ActorOwnershipRegistry::new());
        let key = test_key("svc.build-pin", "SessionActor", "user-2");
        let authority_v1 = ActorOwnerRouteAuthority {
            build_id: "sha256:build-v1".to_string(),
        };
        let owner_frame = test_owner_frame("SessionActor");
        let abi = test_abi();
        let implementation = test_implementation();
        registry.ensure_present(
            &key,
            abi.clone(),
            implementation.clone(),
            owner_frame.clone(),
            b"[]",
        );
        let token = registry
            .reserve(&key, 1, "runtime-a", &authority_v1, 0)
            .expect("first owner reserves");
        let facts = CommitFenceFacts {
            actor_abi_identity: abi,
            actor_implementation_identity: implementation,
            declaration_owner: owner_frame,
            owner_lease_id: LeaseIdMint::new().mint(),
        };
        let fence = registry
            .commit(&token, &facts, 0, DEFAULT_OWNER_LEASE_TTL_MS)
            .expect("first owner commits");

        // A stale continuation for the same lease, re-issued by an older
        // build within the lease window, differs from the live fence in no
        // field the registry compares (no build identity exists today). It
        // must be rejected as FenceMismatch.
        let stale_continuation = ActorOwnerFence {
            lease_expires_at: 15_000,
            ..fence.clone()
        };
        let outcome =
            registry.renew(&key, &stale_continuation, DEFAULT_OWNER_LEASE_TTL_MS, 10_000);
        assert!(
            matches!(outcome, Err(OwnershipError::FenceMismatch)),
            "a continuation that cannot prove the exact build passed owner \
             fence validation (renew result: {outcome:?}); the fence must pin \
             the claim's exact build (README §5.8)",
        );
    }
}
