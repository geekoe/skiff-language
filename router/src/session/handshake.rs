//! Per-connection handshake phase state machine (M4: capabilities-only
//! registration).
//!
//! This is the single phase authority for one physical connection. It is
//! deliberately free of sockets, directories and time: the session task
//! drives it, and the layer performs the corresponding directory mutations.
//! Registration = bootstrap write → capabilities bind → `runtime.registered`
//! ACK; the epoch tuple, Register frame and generation terminals are retired.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakePhase {
    Accepted,
    BootstrapSent,
    CapabilitiesBound,
    Registered,
    Closed,
}

/// Strict terminal classification (M4: no tuple/register terminals).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    WrongOrder,
    IdentityChange,
    BootstrapWriteFail,
    AckLoss,
    BootstrapTimeout,
    CapabilitiesTimeout,
    Disconnect,
    PreAuthLimitRejected,
    MalformedFrame,
    UnimplementedFamily,
    IngressBudgetExceeded,
    OutboundBudgetExceeded,
    RegistrationRefused,
}

impl TerminalKind {
    pub const fn description(self) -> &'static str {
        match self {
            Self::WrongOrder => "frame arrived outside its handshake phase",
            Self::IdentityChange => "replica identity changed on one connection",
            Self::BootstrapWriteFail => "router.bootstrap write failed",
            Self::AckLoss => "runtime.registered ACK write failed or timed out",
            Self::BootstrapTimeout => "bootstrap deadline expired",
            Self::CapabilitiesTimeout => "capabilities deadline expired",
            Self::Disconnect => "physical connection closed",
            Self::PreAuthLimitRejected => "pre-auth connection limit reached",
            Self::MalformedFrame => "frame failed framing/direction/payload validation",
            Self::UnimplementedFamily => "frame family has no installed sink",
            Self::IngressBudgetExceeded => "per-session inbound budget exceeded",
            Self::OutboundBudgetExceeded => "per-session outbound budget exceeded",
            Self::RegistrationRefused => {
                "installed-consumer permit acquisition refused registration"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitiesEvent {
    Bound,
    /// The same replica re-advertises its dispatch surface after the session
    /// is bound (loaded build-id set / lazy-load advertisement refresh).
    /// Capabilities are re-recorded, never rebound.
    Refreshed,
    Terminal(TerminalKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthEvent {
    Observed,
    Terminal(TerminalKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    Bootstrap,
    Capabilities,
}

/// Pure per-connection handshake state.
#[derive(Debug, Clone)]
pub struct HandshakeState {
    phase: HandshakePhase,
    terminal: Option<TerminalKind>,
    replica: Option<String>,
}

impl HandshakeState {
    pub fn new() -> Self {
        Self {
            phase: HandshakePhase::Accepted,
            terminal: None,
            replica: None,
        }
    }

    pub fn phase(&self) -> HandshakePhase {
        self.phase
    }

    pub fn terminal(&self) -> Option<TerminalKind> {
        self.terminal
    }

    pub fn replica(&self) -> Option<&str> {
        self.replica.as_deref()
    }

    pub fn is_closed(&self) -> bool {
        self.phase == HandshakePhase::Closed
    }

    pub fn outcome_name(&self) -> String {
        match (self.phase, self.terminal) {
            (HandshakePhase::Closed, Some(terminal)) => format!("{terminal:?}"),
            (phase, None) => format!("{phase:?}"),
            _ => "Closed".to_string(),
        }
    }

    fn set_terminal(&mut self, kind: TerminalKind) {
        debug_assert!(self.terminal.is_none(), "terminal already set");
        self.terminal = Some(kind);
        self.phase = HandshakePhase::Closed;
    }

    /// The `router.bootstrap` frame was written successfully (Accepted ->
    /// BootstrapSent).
    pub fn on_bootstrap_written(&mut self) -> Result<(), TerminalKind> {
        if self.phase != HandshakePhase::Accepted {
            return Err(TerminalKind::WrongOrder);
        }
        self.phase = HandshakePhase::BootstrapSent;
        Ok(())
    }

    pub fn on_bootstrap_write_failed(&mut self) -> TerminalKind {
        let terminal = TerminalKind::BootstrapWriteFail;
        self.set_terminal(terminal);
        terminal
    }

    pub fn on_capabilities(&mut self, runtime_id: &str) -> CapabilitiesEvent {
        match self.phase {
            HandshakePhase::Accepted => {
                let terminal = TerminalKind::WrongOrder;
                self.set_terminal(terminal);
                CapabilitiesEvent::Terminal(terminal)
            }
            HandshakePhase::BootstrapSent => {
                self.replica = Some(runtime_id.to_string());
                self.phase = HandshakePhase::CapabilitiesBound;
                CapabilitiesEvent::Bound
            }
            HandshakePhase::CapabilitiesBound | HandshakePhase::Registered => {
                if self.replica.as_deref() == Some(runtime_id) {
                    CapabilitiesEvent::Refreshed
                } else {
                    let terminal = TerminalKind::IdentityChange;
                    self.set_terminal(terminal);
                    CapabilitiesEvent::Terminal(terminal)
                }
            }
            HandshakePhase::Closed => {
                let terminal = TerminalKind::WrongOrder;
                self.set_terminal(terminal);
                CapabilitiesEvent::Terminal(terminal)
            }
        }
    }

    pub fn on_health(&mut self, runtime_id: &str) -> HealthEvent {
        match self.phase {
            HandshakePhase::Registered => {
                if self.replica.as_deref() != Some(runtime_id) {
                    let terminal = TerminalKind::IdentityChange;
                    self.set_terminal(terminal);
                    HealthEvent::Terminal(terminal)
                } else {
                    HealthEvent::Observed
                }
            }
            _ => {
                let terminal = TerminalKind::WrongOrder;
                self.set_terminal(terminal);
                HealthEvent::Terminal(terminal)
            }
        }
    }

    /// The `runtime.registered` ACK was written (CapabilitiesBound ->
    /// Registered).
    pub fn on_ack_written(&mut self) -> Result<(), TerminalKind> {
        match self.phase {
            HandshakePhase::CapabilitiesBound => {
                self.phase = HandshakePhase::Registered;
                Ok(())
            }
            HandshakePhase::Registered => Ok(()),
            _ => Err(TerminalKind::WrongOrder),
        }
    }

    pub fn on_ack_write_failed(&mut self) -> TerminalKind {
        let terminal = TerminalKind::AckLoss;
        self.set_terminal(terminal);
        terminal
    }

    pub fn on_timeout(&mut self, kind: TimeoutKind) -> TerminalKind {
        let terminal = match (kind, self.phase) {
            (TimeoutKind::Bootstrap, HandshakePhase::Accepted) => TerminalKind::BootstrapTimeout,
            (TimeoutKind::Capabilities, HandshakePhase::BootstrapSent) => {
                TerminalKind::CapabilitiesTimeout
            }
            _ => TerminalKind::Disconnect,
        };
        self.set_terminal(terminal);
        terminal
    }

    pub fn on_disconnect(&mut self) -> TerminalKind {
        let terminal = TerminalKind::Disconnect;
        self.set_terminal(terminal);
        terminal
    }

    pub fn terminal_with(&mut self, kind: TerminalKind) -> TerminalKind {
        self.set_terminal(kind);
        kind
    }
}

impl Default for HandshakeState {
    fn default() -> Self {
        Self::new()
    }
}
