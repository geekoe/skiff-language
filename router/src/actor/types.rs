//! Shared typed identities, fences and constants for the W-actor lane
//! (authority design §3.2/§3.4, C-actor/C-model-actor contracts).
//!
//! Identity/fence types are never interchangeable with generic strings:
//! `ActorIncarnationFence`, `ActorClaimId` and `ActorLogicalKey` are
//! distinct newtypes used only by their owning correlation paths.

use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity};
use skiff_runtime_transport::actor_method::{
    ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader,
};
use skiff_runtime_transport::actor_owner::ActorOwnerRouteAuthorityFrameHeader;
use skiff_runtime_transport::protocol::{ActorKeyFrameMetadata, ActorRefFrameMetadata};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Default activation wait deadline (C-actor §6): 30s.
pub const DEFAULT_ACTIVATION_DEADLINE_MS: u64 = 30_000;
/// Default owner-control ACK deadline (C-actor §6): 10s.
pub const DEFAULT_CONTROL_ACK_DEADLINE_MS: u64 = 10_000;
/// Default owner lease TTL (C-actor §4): 30s.
pub const DEFAULT_OWNER_LEASE_TTL_MS: u64 = 30_000;
/// Default idle TTL (C-actor §8): 30s.
pub const DEFAULT_IDLE_TTL_MS: u64 = 30_000;
/// Default activation claim / control pending budget (C-actor §4).
pub const DEFAULT_ACTOR_PENDING_BUDGET: usize = 4096;
/// Default late/settled tombstone budget (C-actor §4).
pub const DEFAULT_ACTOR_TOMBSTONE_BUDGET: usize = 1024;
/// Default idle-eviction retry bound (C-actor §8).
pub const DEFAULT_EVICTION_RETRY_BOUND: usize = 3;

/// Actor incarnation fence (authority design §3.4).
///
/// The incarnation epoch belongs to one actor logical key and advances only
/// through replace/remove/upgrade transitions owned by
/// [`super::ownership::ActorOwnershipRegistry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorIncarnationFence(pub u64);

impl ActorIncarnationFence {
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

/// Stable actor logical key (wire shape, epoch-free).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorLogicalKey {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    pub actor_id_hash: String,
}

impl ActorLogicalKey {
    pub fn from_wire(key: &ActorKeyFrameMetadata) -> Self {
        Self {
            service_id: key.service_id.clone(),
            actor_type_identity: key.actor_type_identity.clone(),
            actor_id_type_identity: key.actor_id_type_identity.clone(),
            actor_id_encoding_version: key.actor_id_encoding_version.clone(),
            canonical_actor_id_key_bytes_base64: key.canonical_actor_id_key_bytes_base64.clone(),
            actor_id_hash: key.actor_id_hash.clone().unwrap_or_default(),
        }
    }

    pub fn from_actor_ref(actor_ref: &ActorLogicalRefFrameHeader) -> Self {
        Self {
            service_id: actor_ref.service_id.clone(),
            actor_type_identity: actor_ref.actor_type_identity.clone(),
            actor_id_type_identity: actor_ref.actor_id_type_identity.clone(),
            actor_id_encoding_version: actor_ref.actor_id_encoding_version.clone(),
            canonical_actor_id_key_bytes_base64: actor_ref
                .canonical_actor_id_key_bytes_base64
                .clone(),
            actor_id_hash: actor_ref.actor_id_hash.clone(),
        }
    }

    pub fn to_actor_ref(&self, epoch: u64) -> ActorRef {
        ActorRef {
            service_id: self.service_id.clone(),
            actor_type_identity: self.actor_type_identity.clone(),
            actor_id_type_identity: self.actor_id_type_identity.clone(),
            actor_id_encoding_version: self.actor_id_encoding_version.clone(),
            canonical_actor_id_key_bytes_base64: self.canonical_actor_id_key_bytes_base64.clone(),
            actor_id_hash: self.actor_id_hash.clone(),
            epoch,
        }
    }
}

/// Resolved actor reference (logical key + incarnation epoch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorRef {
    pub service_id: String,
    pub actor_type_identity: String,
    pub actor_id_type_identity: String,
    pub actor_id_encoding_version: String,
    pub canonical_actor_id_key_bytes_base64: String,
    pub actor_id_hash: String,
    pub epoch: u64,
}

impl ActorRef {
    pub fn to_wire(&self) -> ActorRefFrameMetadata {
        ActorRefFrameMetadata {
            service_id: self.service_id.clone(),
            actor_type_identity: self.actor_type_identity.clone(),
            actor_id_type_identity: self.actor_id_type_identity.clone(),
            actor_id_encoding_version: self.actor_id_encoding_version.clone(),
            canonical_actor_id_key_bytes_base64: self.canonical_actor_id_key_bytes_base64.clone(),
            actor_id_hash: self.actor_id_hash.clone(),
            epoch: Some(self.epoch),
        }
    }
}

/// Captured immutable routing authority for one actor operation
/// (assembly identity + generation).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ActorOwnerRouteAuthority {
    pub assembly_identity: String,
    pub assembly_generation: u64,
}

impl ActorOwnerRouteAuthority {
    pub fn from_wire(authority: &ActorOwnerRouteAuthorityFrameHeader) -> Self {
        Self {
            assembly_identity: authority.assembly_identity.clone(),
            assembly_generation: authority.assembly_generation,
        }
    }

    pub fn to_wire(&self) -> ActorOwnerRouteAuthorityFrameHeader {
        ActorOwnerRouteAuthorityFrameHeader {
            assembly_identity: self.assembly_identity.clone(),
            assembly_generation: self.assembly_generation,
        }
    }
}

/// Router-local claim id (`actor-claim-<seq>`; canonical token, not on wire).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorClaimId(String);

impl ActorClaimId {
    pub fn mint(seq: u64) -> Self {
        Self(format!("actor-claim-{seq}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Router-local owner lease id mint (E-actor-parity reconciliation).
///
/// The canonical corpus mints `owner-lease-<n>` exactly once per activation
/// admission. The same minted id is carried on the `activateInitial` wire
/// fence and into the committed registry fence (and every later
/// renew/mark-live/release), matching the TS coordinator single-mint
/// semantics. The broker owns the mint; the registry never mints a second,
/// independent lease id at commit.
#[derive(Debug, Clone, Default)]
pub struct LeaseIdMint {
    next: Arc<AtomicU64>,
}

impl LeaseIdMint {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mint(&self) -> String {
        format!(
            "owner-lease-{}",
            self.next.fetch_add(1, Ordering::Relaxed) + 1
        )
    }
}

/// Authoritative claim token issued by `ActorOwnershipRegistry` (C-actor
/// §4.1). Reserve/commit/abort is the only transition channel for first-owner
/// activation; brokers hold the token but never a second claim truth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorClaimToken {
    pub claim_id: ActorClaimId,
    pub actor_key: ActorLogicalKey,
    pub expected_epoch: u64,
    pub owner_runtime_id: String,
    pub route_authority: ActorOwnerRouteAuthority,
}

/// Fence facts supplied at claim commit (C-actor §3.2 `CommitClaim`).
///
/// `owner_lease_id` is minted once by the activation broker when the claim
/// starts and is the single lease identity for the wire `activateInitial`
/// fence and the committed registry fence (E-actor-parity).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFenceFacts {
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
    pub owner_lease_id: String,
}

/// Current owner fence (C-actor §3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorOwnerFence {
    pub epoch: u64,
    pub owner_runtime_id: String,
    pub owner_lease_id: String,
    pub lease_expires_at: u64,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
}

/// One expired owner result of `ExpireLeases { now }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpiredOwner {
    pub actor_key: ActorLogicalKey,
    pub fence: ActorOwnerFence,
}

/// Owner lease release reason (health/audit vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerReleaseReason {
    Disconnected,
    Evicted,
    Upgraded,
    Shutdown,
}

impl OwnerReleaseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disconnected => "disconnected",
            Self::Evicted => "evicted",
            Self::Upgraded => "upgraded",
            Self::Shutdown => "shutdown",
        }
    }
}

/// get-or-create lineage namespace: ordinary vs test capability lineage.
/// Ordinary and test lineages never share a claim (C-actor §5).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ActorLineage {
    Ordinary,
    Test(String),
}

impl ActorLineage {
    pub fn from_test_case(capability: Option<&str>) -> Self {
        match capability {
            Some(capability) => Self::Test(capability.to_string()),
            None => Self::Ordinary,
        }
    }
}

/// Entry facts returned by the registry when a key is made present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorEntryFacts {
    pub epoch: u64,
}
