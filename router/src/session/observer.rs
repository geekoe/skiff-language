//! Session registration observer seam (plan §4.2 cold recovery).
//!
//! The session task notifies the observer after a Runtime registration has
//! been ACKed and the session is routable in the directory. The activation
//! coordinator consumes this seam so a recovery transaction can bind an
//! expected replica's new exact session and enqueue `Prepare`. The observer
//! is additive: no session state-machine or directory semantics change, and
//! an absent observer is a no-op.

use std::fmt;

use crate::session::identity::RuntimeSessionEpoch;

/// Called after a session becomes routable (`runtime.registered` ACK
/// written). Implementations must be synchronous and non-blocking.
pub trait RegistrationObserver: Send + Sync + fmt::Debug {
    fn on_session_registered(&self, session: &RuntimeSessionEpoch);
}
