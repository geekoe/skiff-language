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
        self.lock().entries.get(key).map(|entry| ActorRegistryEntry {
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
