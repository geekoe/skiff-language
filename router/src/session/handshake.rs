//! Per-connection handshake phase state machine
//! (C-model-registration §2, authority design §3.5).
//!
//! This is the single phase authority for one physical connection. It is
//! deliberately free of sockets, directories and time: the session task
//! drives it, and the `RegistrationFrameSink` performs the corresponding
//! directory mutations. The corpus reference model (`runtime/transport/tests/
//! registration_handshake_corpus.rs`) and this machine implement the same
//! frozen semantics; post-commit re-register follows authority design §3.2
//! (`RuntimeRegistrationTransition`) as required by C-session §3.3.

use skiff_artifact_model::{RuntimeAssemblyRef, RuntimeConfigSnapshotRef};

use super::identity::RegisteredAssemblyTuple;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakePhase {
    Accepted,
    BootstrapSent,
    CapabilitiesBound,
    RegisterValidated,
    Registered,
    Closed,
}

/// Strict terminal classification (C-model-registration §2.3) plus the
/// implementation-level terminals documented in the W-session leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    WrongOrder,
    IdentityChange,
    DuplicateRegister,
    StaleRegister,
    NewGenerationBeforeEpochSwap,
    LegacyRegisterRejected,
    BootstrapWriteFail,
    AckLoss,
    BootstrapTimeout,
    CapabilitiesTimeout,
    RegisterTimeout,
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
            Self::DuplicateRegister => "second register before the ACK",
            Self::StaleRegister => "register tuple does not match the committed epoch",
            Self::NewGenerationBeforeEpochSwap => {
                "register matches a pending epoch that is not yet committed"
            }
            Self::LegacyRegisterRejected => "legacy runtime.register is not a handshake frame",
            Self::BootstrapWriteFail => "router.bootstrap write failed",
            Self::AckLoss => "runtime.registered ACK write failed or timed out",
            Self::BootstrapTimeout => "bootstrap deadline expired",
            Self::CapabilitiesTimeout => "capabilities deadline expired",
            Self::RegisterTimeout => "register deadline expired",
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
pub struct RegisterControl {
    pub profile: String,
    pub generation: u64,
    pub assembly: RuntimeAssemblyRef,
    pub config_snapshot: RuntimeConfigSnapshotRef,
    pub replica_id: String,
}

impl RegisterControl {
    pub fn tuple(&self) -> RegisteredAssemblyTuple {
        RegisteredAssemblyTuple {
            profile: self.profile.clone(),
            generation: self.generation,
            assembly: self.assembly.clone(),
            config_snapshot: self.config_snapshot.clone(),
        }
    }
}

/// Captured routing epoch context for register validation. `pending` is the
/// activation epoch before durable commit/swap (None in W-session until the
/// activation lane supplies it).
#[derive(Debug, Clone, Default)]
pub struct EpochContext {
    pub current: Option<RegisteredAssemblyTuple>,
    pub pending: Option<RegisteredAssemblyTuple>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilitiesEvent {
    Bound,
    Terminal(TerminalKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegisterEvent {
    /// Register validated against the committed epoch; pending publish.
    Validated {
        tuple: RegisteredAssemblyTuple,
    },
    /// Exact duplicate on the same tuple: idempotent, no revision bump.
    Idempotent,
    /// Same physical session re-registers a new committed tuple after an
    /// activation commit; `RuntimeRegistrationTransition` must run.
    Transition {
        tuple: RegisteredAssemblyTuple,
    },
    Terminal(TerminalKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthEvent {
    Observed,
    DroppedBeforeAck,
    Terminal(TerminalKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutKind {
    Bootstrap,
    Capabilities,
    Register,
}

/// Pure per-connection handshake state.
#[derive(Debug, Clone)]
pub struct HandshakeState {
    phase: HandshakePhase,
    terminal: Option<TerminalKind>,
    replica: Option<String>,
    registered_tuple: Option<RegisteredAssemblyTuple>,
    health_before_ack: u64,
}

impl HandshakeState {
    pub fn new() -> Self {
        Self {
            phase: HandshakePhase::Accepted,
            terminal: None,
            replica: None,
            registered_tuple: None,
            health_before_ack: 0,
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

    pub fn registered_tuple(&self) -> Option<&RegisteredAssemblyTuple> {
        self.registered_tuple.as_ref()
    }

    pub fn health_before_ack(&self) -> u64 {
        self.health_before_ack
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
                if let Some(bound) = self.replica.as_deref() {
                    let terminal = if bound == runtime_id {
                        TerminalKind::WrongOrder
                    } else {
                        TerminalKind::IdentityChange
                    };
                    self.set_terminal(terminal);
                    CapabilitiesEvent::Terminal(terminal)
                } else {
                    self.replica = Some(runtime_id.to_string());
                    self.phase = HandshakePhase::CapabilitiesBound;
                    CapabilitiesEvent::Bound
                }
            }
            HandshakePhase::CapabilitiesBound
            | HandshakePhase::RegisterValidated
            | HandshakePhase::Registered => {
                let terminal = if self.replica.as_deref() == Some(runtime_id) {
                    TerminalKind::WrongOrder
                } else {
                    TerminalKind::IdentityChange
                };
                self.set_terminal(terminal);
                CapabilitiesEvent::Terminal(terminal)
            }
            HandshakePhase::Closed => {
                let terminal = TerminalKind::WrongOrder;
                self.set_terminal(terminal);
                CapabilitiesEvent::Terminal(terminal)
            }
        }
    }

    pub fn on_register(
        &mut self,
        register: &RegisterControl,
        context: &EpochContext,
    ) -> RegisterEvent {
        let snapshot = (
            self.phase,
            self.replica.clone(),
            self.registered_tuple.clone(),
        );
        let tuple = register.tuple();
        match snapshot.0 {
            HandshakePhase::Accepted | HandshakePhase::BootstrapSent => {
                let terminal = TerminalKind::WrongOrder;
                self.set_terminal(terminal);
                RegisterEvent::Terminal(terminal)
            }
            HandshakePhase::CapabilitiesBound => {
                if snapshot.1.as_deref() != Some(register.replica_id.as_str()) {
                    let terminal = TerminalKind::IdentityChange;
                    self.set_terminal(terminal);
                    return RegisterEvent::Terminal(terminal);
                }
                if context.current.as_ref() == Some(&tuple) {
                    self.registered_tuple = Some(tuple.clone());
                    self.phase = HandshakePhase::RegisterValidated;
                    RegisterEvent::Validated { tuple }
                } else if context.pending.as_ref() == Some(&tuple) {
                    let terminal = TerminalKind::NewGenerationBeforeEpochSwap;
                    self.set_terminal(terminal);
                    RegisterEvent::Terminal(terminal)
                } else {
                    let terminal = TerminalKind::StaleRegister;
                    self.set_terminal(terminal);
                    RegisterEvent::Terminal(terminal)
                }
            }
            HandshakePhase::RegisterValidated => {
                let terminal = TerminalKind::DuplicateRegister;
                self.set_terminal(terminal);
                RegisterEvent::Terminal(terminal)
            }
            HandshakePhase::Registered => {
                if snapshot.1.as_deref() != Some(register.replica_id.as_str()) {
                    let terminal = TerminalKind::IdentityChange;
                    self.set_terminal(terminal);
                    return RegisterEvent::Terminal(terminal);
                }
                if snapshot.2.as_ref() == Some(&tuple) {
                    RegisterEvent::Idempotent
                } else if context.current.as_ref() == Some(&tuple) {
                    // Post-commit re-register on the same physical session:
                    // RuntimeRegistrationTransition publishes the new revision
                    // (authority design §3.2, C-session §3.3).
                    self.registered_tuple = Some(tuple.clone());
                    RegisterEvent::Transition { tuple }
                } else if context.pending.as_ref() == Some(&tuple) {
                    let terminal = TerminalKind::NewGenerationBeforeEpochSwap;
                    self.set_terminal(terminal);
                    RegisterEvent::Terminal(terminal)
                } else {
                    let terminal = TerminalKind::StaleRegister;
                    self.set_terminal(terminal);
                    RegisterEvent::Terminal(terminal)
                }
            }
            HandshakePhase::Closed => {
                let terminal = TerminalKind::WrongOrder;
                self.set_terminal(terminal);
                RegisterEvent::Terminal(terminal)
            }
        }
    }

    pub fn on_legacy_register(&mut self) -> TerminalKind {
        let terminal = TerminalKind::LegacyRegisterRejected;
        self.set_terminal(terminal);
        terminal
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
            HandshakePhase::RegisterValidated => {
                if self.replica.as_deref() != Some(runtime_id) {
                    let terminal = TerminalKind::IdentityChange;
                    self.set_terminal(terminal);
                    HealthEvent::Terminal(terminal)
                } else {
                    self.health_before_ack += 1;
                    HealthEvent::DroppedBeforeAck
                }
            }
            _ => {
                let terminal = TerminalKind::WrongOrder;
                self.set_terminal(terminal);
                HealthEvent::Terminal(terminal)
            }
        }
    }

    /// The `runtime.registered` ACK was written (RegisterValidated ->
    /// Registered).
    pub fn on_ack_written(&mut self) -> Result<(), TerminalKind> {
        match self.phase {
            HandshakePhase::RegisterValidated => {
                self.phase = HandshakePhase::Registered;
                Ok(())
            }
            // E-activation §4.1 step 9/§8: a post-commit same-session
            // re-register publishes a transition and writes a fresh
            // `runtime.registered` ACK while the session is already
            // Registered; that ACK completion must not terminate the
            // exact session.
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
            (TimeoutKind::Register, HandshakePhase::CapabilitiesBound) => {
                TerminalKind::RegisterTimeout
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
