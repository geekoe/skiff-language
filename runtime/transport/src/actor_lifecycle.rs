//! Exact-build Actor lifecycle contracts shared by Router and Runtime.
//!
//! These DTOs are additive to the legacy `actor.owner.control` family. They
//! deliberately keep the logical Actor key separate from the exact execution
//! identity and carry no assembly generation, release pointer, executable
//! address, declaration file index, or provider program.

mod discard;
mod error;
mod identity;
mod snapshot;
mod validation;

pub use discard::{
    decode_actor_idle_discard_ack_frame, decode_actor_idle_discard_request_frame,
    encode_actor_idle_discard_ack_frame, encode_actor_idle_discard_request_frame,
    ActorIdleDiscardAckFrameHeader, ActorIdleDiscardAckOutcome, ActorIdleDiscardRequestFrameHeader,
    ACTOR_IDLE_DISCARD_ACK_FRAME_TYPE, ACTOR_IDLE_DISCARD_REQUEST_FRAME_TYPE,
};
pub use error::ActorLifecycleContractError;
pub use identity::{
    ActorArenaEpoch, ActorIncarnation, ExactActorExecutionIdentityFrameMetadata,
    ExactActorOwnerFenceFrameMetadata, ExactDeploymentOwnerFrameMetadata,
};
pub use snapshot::DurableActorActivationSnapshotFrameMetadata;

#[cfg(test)]
mod tests;
