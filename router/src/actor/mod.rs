//! W-actor: Router Rust actor lane (authority design §3.2/§3.3/§5.4/§5.5,
//! C-actor/C-model-actor/C-task/C-model-task frozen contracts).
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
    LeaseHealth, OwnershipHealth,
};
pub use invocation::{
    ActorInvocationRelay, ActorInvocationRelayOptions, ActorInvokeInput, InvocationError,
    InvocationSettled, InvocationTerminal, InvocationTerminalKind, OwnerCancel, OwnerSettleKind,
};
pub use lease::{
    ActorLeaseExpiryScheduler, IdleEvictControlPort, LeaseError, LeaseSchedulerOptions,
};
pub use ownership::{ActorOwnershipRegistry, OwnershipError};
pub use types::{
    ActorClaimId, ActorClaimToken, ActorEntryFacts, ActorIncarnationFence, ActorLineage,
    ActorLogicalKey, ActorOwnerFence, ActorOwnerRouteAuthority, ActorRef, CommitFenceFacts,
    ExpiredOwner, LeaseIdMint, OwnerReleaseReason, DEFAULT_ACTIVATION_DEADLINE_MS,
    DEFAULT_ACTOR_PENDING_BUDGET, DEFAULT_ACTOR_TOMBSTONE_BUDGET, DEFAULT_CONTROL_ACK_DEADLINE_MS,
    DEFAULT_EVICTION_RETRY_BOUND, DEFAULT_IDLE_TTL_MS, DEFAULT_OWNER_LEASE_TTL_MS,
};
