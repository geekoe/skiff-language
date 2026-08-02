//! W-actor: Router Rust actor lane (authority design §3.2/§3.3/§5.4/§5.5,
//! C-actor/C-model-actor/C-spawn/C-model-spawn frozen contracts).
//!
//! Owners and their unique invariants:
//!
//! - [`catalog::ActorMethodCatalogView`]: read-only typed query over the A3
//!   actor index inside an explicitly captured `Arc<RoutingEpoch>`; no
//!   independent index, mailbox, refresh or File IR reading.
//! - [`ownership::ActorOwnershipRegistry`]: actor identity, incarnation,
//!   current owner fence and the authoritative `ActorClaimToken`
//!   reserve/commit/abort channel; brokers never hold a second claim truth.
//! - [`activation::ActorActivationRequestBroker`]: get-or-create dedup and
//!   activation request/ACK correlation (executes while holding the token).
//! - [`invocation::ActorInvocationRelay`]: method invocation/return/error/
//!   cancel correlation with exact-fence settle.
//! - [`control::ActorOwnerControlBroker`]: claim/renew/evict owner-control
//!   correlation.
//! - [`lease::ActorLeaseExpiryScheduler`]: lease/idle deadline scheduling and
//!   bounded eviction trigger.
//! - [`spawn::SpawnSubmitRouter`]: stateless exact parent-kind selection
//!   (`request | actorInvocation`); sink stores no pending and accepted
//!   spawns are separated from parent lifecycle.
//!
//! All owners are synchronous reducers (never cross `.await` while holding
//! state) and publish only counter/occupancy health projections.

mod activation;
mod catalog;
mod control;
mod health;
mod invocation;
mod lease;
mod ownership;
mod spawn;
mod spawn_sink;
mod types;

pub use activation::{
    ActivateInitialControlRequest, ActivationAckOutcome, ActivationControlPort,
    ActivationTimeoutOutcome, ActivationWaiterOutcome, ActorActivationBrokerOptions,
    ActorActivationRequestBroker, ActorGetOrCreateRequest, GetOrCreateOutcome,
};
pub use catalog::{ActorMethodCatalogView, CatalogQuery};
pub use control::{
    ActorOwnerControlBroker, ControlAckOutcome, ControlBrokerOptions, ControlError,
    ControlTimeoutOutcome, OwnerControlRequest,
};
pub use health::{
    ActivationHealth, ActorHealthSnapshot, CatalogHealth, ControlHealth, InvocationHealth,
    LeaseHealth, OwnershipHealth, SpawnHealth,
};
pub use invocation::{
    ActorInvocationRelay, ActorInvocationRelayOptions, ActorInvokeInput, InvocationError,
    InvocationSettled, InvocationTerminal, InvocationTerminalKind, OwnerCancel, OwnerSettleKind,
};
pub use lease::{
    ActorLeaseExpiryScheduler, IdleEvictControlPort, LeaseError, LeaseSchedulerOptions,
};
pub use ownership::{ActorOwnershipRegistry, OwnershipError};
pub use spawn::{
    ActorLaneSpawnControl, ActorMethodSpawnExecutionSink, ActorSpawnParentResolver,
    FunctionSpawnParentResolver, ParentQuery, RelaySpawnParentLookup, SpawnAuthorityProbe,
    SpawnErrorCode, SpawnParentAuthority, SpawnParentLookup, SpawnParentResolution,
    SpawnParentSnapshot, SpawnSubmitAcceptance, SpawnSubmitError, SpawnSubmitRouter,
};
pub use spawn_sink::{spawn_error_code, PendingSpawnWire, SpawnWireHealth, SpawnWireStore};
pub use types::{
    ActorClaimId, ActorClaimToken, ActorEntryFacts, ActorIncarnationFence, ActorLineage,
    ActorLogicalKey, ActorOwnerFence, ActorOwnerRouteAuthority, ActorRef, CommitFenceFacts,
    ExpiredOwner, OwnerReleaseReason, DEFAULT_ACTIVATION_DEADLINE_MS, DEFAULT_ACTOR_PENDING_BUDGET,
    DEFAULT_ACTOR_TOMBSTONE_BUDGET, DEFAULT_CONTROL_ACK_DEADLINE_MS, DEFAULT_EVICTION_RETRY_BOUND,
    DEFAULT_IDLE_TTL_MS, DEFAULT_OWNER_LEASE_TTL_MS, SPAWNED_ACTOR_METHOD_DEADLINE_MS,
    SPAWNED_ACTOR_METHOD_LEASE_MS,
};
