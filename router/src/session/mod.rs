//! W-session: connection/session tasks, handshake, `RuntimeRegistrationDirectory`,
//! pre-auth limits/timeouts, cancellation + reserved terminal + consumer
//! manifest + ACK barrier + fail-stop, and the closed-family frame demux.
//!
//! Frozen contracts consumed here:
//! `doc/implementation/router-rust-migration/contracts/router-rust-migration-c-session-contract.md`,
//! `...-c-model-registration-contract.md`, `...-c-process-lifecycle-contract.md`
//! (authority design plan §3.2/§3.4/§3.5/§3.6/§5.4/§5.5/§6.1).

pub mod bootstrap;
pub mod budget;
pub mod consumer;
pub mod demux;
pub mod directory;
pub mod handshake;
pub mod health;
pub mod identity;
pub mod layer;
pub mod observer;
pub mod pre_auth;
pub mod task;

pub use consumer::{
    ConsumerKind, ConsumerManifest, FailStop, RuntimeSessionClosed, SessionConsumer,
};
pub use demux::{InboundFrameSink, InboundSinkSet};
pub use directory::RuntimeRegistrationDirectory;
pub use handshake::{HandshakePhase, HandshakeState, TerminalKind};
pub use identity::{RuntimeConnectionEpoch, RuntimeSessionEpoch};
pub use layer::{
    SessionFrameWriter, SessionHealthSnapshot, SessionLayer, SessionLayerError,
    SessionLayerOptions, SessionTiming,
};
pub use observer::RegistrationObserver;
