//! Client-side Runtime handshake driver (H-registration-cut).
//!
//! The Runtime is the WebSocket client of the frozen §3.5 handshake:
//!
//! ```text
//! connect
//! <- Router sends router.bootstrap
//! -> Runtime sends runtime.capabilities
//! <- Router sends runtime.registered ACK
//! -> Runtime starts runtime.health
//! ```
//!
//! This struct is the single phase authority for one Router session. It is
//! deliberately free of sockets and time; `router_session.rs` drives it.
//! Wrong order, identity change, duplicate bootstrap, direction violations,
//! ACK-before-registration and business-frames-before-ACK are all strict
//! terminals (C-model-registration §2.3).

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientHandshakePhase {
    WaitingBootstrap,
    BootstrapReceived,
    RegistrationSent,
    Registered,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ClientTerminalKind {
    WrongOrder,
    IdentityChange,
    DirectionViolation,
    BootstrapTimeout,
    RegisteredTimeout,
    WriteFailed,
    Disconnect,
}

impl ClientTerminalKind {
    pub const fn description(self) -> &'static str {
        match self {
            Self::WrongOrder => "frame arrived outside its handshake phase",
            Self::IdentityChange => "registered ACK identity does not match this Runtime replica",
            Self::DirectionViolation => "runtime-to-router frame arrived from the Router",
            Self::BootstrapTimeout => "router.bootstrap deadline expired",
            Self::RegisteredTimeout => "runtime.registered ACK deadline expired",
            Self::WriteFailed => "handshake outbound frame write failed",
            Self::Disconnect => "physical connection closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientTimeoutKind {
    Bootstrap,
    Registered,
}

/// Handshake deadlines (C-session §4 defaults, process-level constants).
#[derive(Debug, Clone, Copy)]
pub struct HandshakeDeadlines {
    pub bootstrap: Duration,
    pub registered: Duration,
}

impl Default for HandshakeDeadlines {
    fn default() -> Self {
        Self {
            bootstrap: Duration::from_secs(10),
            registered: Duration::from_secs(30),
        }
    }
}

#[derive(Debug)]
pub struct ClientHandshake {
    phase: ClientHandshakePhase,
    terminal: Option<ClientTerminalKind>,
    /// Capabilities are queued right after bootstrap; the writer must flush
    /// the frame before an inbound ACK can be accepted.
    registration_writes_pending: u8,
}

impl ClientHandshake {
    pub fn new() -> Self {
        Self {
            phase: ClientHandshakePhase::WaitingBootstrap,
            terminal: None,
            registration_writes_pending: 0,
        }
    }

    /// Test shortcut: the connection starts as if the whole handshake already
    /// completed (used by `run_connected_session_with_bootstrap` and the
    /// frame-level test helpers).
    pub fn registered() -> Self {
        Self {
            phase: ClientHandshakePhase::Registered,
            terminal: None,
            registration_writes_pending: 0,
        }
    }

    /// Test shortcut: bootstrap received and the capabilities frame queued,
    /// waiting for the ACK (used by frame-level ACK tests).
    #[allow(dead_code)]
    pub fn register_sent() -> Self {
        Self {
            phase: ClientHandshakePhase::RegistrationSent,
            terminal: None,
            registration_writes_pending: 0,
        }
    }

    pub fn phase(&self) -> ClientHandshakePhase {
        self.phase
    }

    #[allow(dead_code)]
    pub fn terminal(&self) -> Option<ClientTerminalKind> {
        self.terminal
    }

    #[allow(dead_code)]
    pub fn is_closed(&self) -> bool {
        self.phase == ClientHandshakePhase::Closed
    }

    fn set_terminal(&mut self, kind: ClientTerminalKind) {
        debug_assert!(self.terminal.is_none(), "terminal already set");
        self.terminal = Some(kind);
        self.phase = ClientHandshakePhase::Closed;
    }

    /// `router.bootstrap` arrived while waiting for it.
    pub fn on_bootstrap(&mut self) -> Result<(), ClientTerminalKind> {
        if self.phase != ClientHandshakePhase::WaitingBootstrap {
            let terminal = ClientTerminalKind::WrongOrder;
            self.set_terminal(terminal);
            return Err(terminal);
        }
        self.phase = ClientHandshakePhase::BootstrapReceived;
        Ok(())
    }

    /// Capabilities were queued after bootstrap.
    pub fn mark_registration_queued(&mut self) {
        self.registration_writes_pending = 1;
    }

    /// The queued handshake outbound frame was flushed to the socket. The
    /// phase becomes `RegistrationSent` only after the frame is on the wire,
    /// so an ACK can never be accepted before the capabilities actually
    /// reached the Router.
    pub fn on_registration_write_flushed(&mut self) {
        if self.registration_writes_pending == 0 {
            return;
        }
        self.registration_writes_pending -= 1;
        if self.registration_writes_pending == 0 {
            debug_assert_eq!(
                self.phase,
                ClientHandshakePhase::BootstrapReceived,
                "registration writes must flush before the phase moves on"
            );
            self.phase = ClientHandshakePhase::RegistrationSent;
        }
    }

    /// `runtime.registered` ACK: only valid after the capabilities were
    /// flushed, and the ACK runtime_id must equal this Runtime's replica
    /// identity.
    pub fn on_registered(
        &mut self,
        ack_runtime_id: &str,
        expected_runtime_id: &str,
    ) -> Result<(), ClientTerminalKind> {
        match self.phase {
            ClientHandshakePhase::RegistrationSent => {
                if ack_runtime_id != expected_runtime_id {
                    let terminal = ClientTerminalKind::IdentityChange;
                    self.set_terminal(terminal);
                    return Err(terminal);
                }
                self.phase = ClientHandshakePhase::Registered;
                Ok(())
            }
            // A repeated `runtime.registered` ACK for the same replica
            // identity is idempotent; a mismatched identity stays a strict
            // terminal.
            ClientHandshakePhase::Registered => {
                if ack_runtime_id != expected_runtime_id {
                    let terminal = ClientTerminalKind::IdentityChange;
                    self.set_terminal(terminal);
                    return Err(terminal);
                }
                Ok(())
            }
            _ => {
                let terminal = ClientTerminalKind::WrongOrder;
                self.set_terminal(terminal);
                Err(terminal)
            }
        }
    }

    /// Any business/activation/control frame requires Registered.
    pub fn on_business_frame(&mut self) -> Result<(), ClientTerminalKind> {
        if self.phase != ClientHandshakePhase::Registered {
            let terminal = ClientTerminalKind::WrongOrder;
            self.set_terminal(terminal);
            return Err(terminal);
        }
        Ok(())
    }

    /// Frames that only flow Runtime->Router cannot arrive from the Router.
    pub fn on_direction_violation(&mut self, frame_type: &str) -> ClientTerminalKind {
        let _ = frame_type;
        let terminal = ClientTerminalKind::DirectionViolation;
        self.set_terminal(terminal);
        terminal
    }

    pub fn on_timeout(&mut self, kind: ClientTimeoutKind) -> ClientTerminalKind {
        let terminal = match (kind, self.phase) {
            (ClientTimeoutKind::Bootstrap, ClientHandshakePhase::WaitingBootstrap) => {
                ClientTerminalKind::BootstrapTimeout
            }
            (
                ClientTimeoutKind::Registered,
                ClientHandshakePhase::BootstrapReceived | ClientHandshakePhase::RegistrationSent,
            ) => ClientTerminalKind::RegisteredTimeout,
            _ => ClientTerminalKind::WrongOrder,
        };
        self.set_terminal(terminal);
        terminal
    }

    #[allow(dead_code)]
    pub fn on_write_failed(&mut self) -> ClientTerminalKind {
        let terminal = ClientTerminalKind::WriteFailed;
        self.set_terminal(terminal);
        terminal
    }

    #[allow(dead_code)]
    pub fn on_disconnect(&mut self) -> ClientTerminalKind {
        let terminal = ClientTerminalKind::Disconnect;
        self.set_terminal(terminal);
        terminal
    }
}

impl Default for ClientHandshake {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_phase_repeat_ack_same_runtime_is_idempotent() {
        let mut handshake = ClientHandshake::registered();
        handshake
            .on_registered("runtime-base", "runtime-base")
            .expect("post-commit same-session re-register ACK must be accepted");
        assert_eq!(handshake.phase, ClientHandshakePhase::Registered);
        assert!(handshake.terminal.is_none());
    }

    #[test]
    fn registered_phase_repeat_ack_wrong_runtime_is_terminal() {
        let mut handshake = ClientHandshake::registered();
        let error = handshake
            .on_registered("runtime-other", "runtime-base")
            .expect_err("mismatched re-register ACK identity must fail");
        assert!(matches!(error, ClientTerminalKind::IdentityChange));
        assert_eq!(handshake.phase, ClientHandshakePhase::Closed);
    }

    #[test]
    fn waiting_bootstrap_ack_remains_wrong_order() {
        let mut handshake = ClientHandshake::new();
        let error = handshake
            .on_registered("runtime-base", "runtime-base")
            .expect_err("ACK before register must remain wrong order");
        assert!(matches!(error, ClientTerminalKind::WrongOrder));
    }
}
