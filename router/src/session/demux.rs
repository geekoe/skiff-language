//! `RuntimeFrameDemux` and the stateless `RegistrationFrameSink`
//! (authority design §5.5, C-session §6, C-model-registration §5.1).
//!
//! The demux owner performs framing, direction, payload-presence and
//! source-session fence checks, then dispatches over the CLOSED
//! `RuntimeFrameFamily` registry from `skiff-runtime-transport`. A new
//! family is a shared-model change, not a session feature. Families without
//! an installed sink terminate the exact session (authority design §6.1);
//! `assembly.activation:Register` is delegated to `RegistrationFrameSink`,
//! other activation transaction variants are not installed in W-session.

use skiff_artifact_model::AssemblyActivationControl;
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::protocol::{
    decode_binary_frame, decode_typed_binary_frame, RuntimeCapabilitiesFrameHeader,
    RuntimeFrameFamily, RuntimeHealthFrameHeader,
};

use super::consumer::ConsumerKind;
use super::directory::{PublishError, RuntimeRegistrationDirectory, TransitionOutcome};
use super::handshake::{EpochContext, HandshakeState, RegisterControl, TerminalKind};
use super::identity::RuntimeSessionEpoch;

#[derive(Debug, Clone, PartialEq)]
pub enum DemuxEvent {
    Capabilities(RuntimeCapabilitiesFrameHeader),
    Health(RuntimeHealthFrameHeader),
    Register(RegisterControl),
    LegacyRegister,
    Unimplemented { family: RuntimeFrameFamily },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DemuxOutcome {
    Handled(DemuxEvent),
    Terminal(TerminalKind),
}

/// Closed family demux. Stateless: the session task owns phase/session state.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeFrameDemux;

impl RuntimeFrameDemux {
    pub fn classify(&self, raw: &[u8]) -> DemuxOutcome {
        let frame = match decode_binary_frame(raw) {
            Ok(frame) => frame,
            Err(_) => return DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
        };
        let Some(frame_type) = frame.header.get("type").and_then(|value| value.as_str()) else {
            return DemuxOutcome::Terminal(TerminalKind::MalformedFrame);
        };

        let Some(family) = RuntimeFrameFamily::ALL
            .into_iter()
            .find(|family| frame_type.starts_with(family.wire_type_prefix()))
        else {
            return DemuxOutcome::Terminal(TerminalKind::MalformedFrame);
        };

        // Inbound frames must be Runtime->Router. The Spawn family is
        // Router->Runtime only; all others are Either at family level and are
        // narrowed frame-level below.
        if !matches!(
            family.direction(),
            skiff_runtime_transport::protocol::FrameDirection::Either
        ) {
            return DemuxOutcome::Terminal(TerminalKind::MalformedFrame);
        }

        // Session and activation families require an empty payload.
        if matches!(
            family.payload_presence(),
            skiff_runtime_transport::protocol::PayloadPresenceRule::Empty
        ) && !frame.payload_bytes.is_empty()
        {
            return DemuxOutcome::Terminal(TerminalKind::MalformedFrame);
        }

        match family {
            RuntimeFrameFamily::Session => self.classify_session(frame_type, raw),
            RuntimeFrameFamily::Activation => self.classify_activation(raw),
            RuntimeFrameFamily::Request
            | RuntimeFrameFamily::Connection
            | RuntimeFrameFamily::Actor
            | RuntimeFrameFamily::Spawn => {
                DemuxOutcome::Handled(DemuxEvent::Unimplemented { family })
            }
        }
    }

    fn classify_session(&self, frame_type: &str, raw: &[u8]) -> DemuxOutcome {
        match frame_type {
            "runtime.capabilities" => {
                let (header, _) =
                    match decode_typed_binary_frame::<RuntimeCapabilitiesFrameHeader>(raw) {
                        Ok(parts) => parts,
                        Err(_) => return DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
                    };
                DemuxOutcome::Handled(DemuxEvent::Capabilities(header))
            }
            "runtime.health" => {
                let (header, _) = match decode_typed_binary_frame::<RuntimeHealthFrameHeader>(raw) {
                    Ok(parts) => parts,
                    Err(_) => return DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
                };
                DemuxOutcome::Handled(DemuxEvent::Health(header))
            }
            "runtime.register" => DemuxOutcome::Handled(DemuxEvent::LegacyRegister),
            // Outbound-only frames (or unknown session frames) arriving from
            // the Runtime are direction/protocol violations.
            "router.bootstrap" | "runtime.registered" => {
                DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
            }
            _ => DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
        }
    }

    fn classify_activation(&self, raw: &[u8]) -> DemuxOutcome {
        let control = match decode_assembly_activation_frame(
            AssemblyActivationFrameDirection::RuntimeToRouter,
            raw,
        ) {
            Ok(control) => control,
            Err(_) => return DemuxOutcome::Terminal(TerminalKind::MalformedFrame),
        };
        match control {
            AssemblyActivationControl::Register {
                environment,
                generation,
                assembly,
                config_snapshot,
                replica_id,
            } => DemuxOutcome::Handled(DemuxEvent::Register(RegisterControl {
                environment,
                generation,
                assembly,
                config_snapshot,
                replica_id,
            })),
            _ => DemuxOutcome::Handled(DemuxEvent::Unimplemented {
                family: RuntimeFrameFamily::Activation,
            }),
        }
    }
}

/// Stateless registration adapter (C-model-registration §5.1): decodes
/// `assembly.activation:Register`, runs `RuntimeRegistrationTransition` and
/// reports whether an ACK is required. It never calls `ActivationCoordinator`.
#[derive(Debug, Clone, Copy, Default)]
pub struct RegistrationFrameSink;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationSinkOutput {
    PendingPublished {
        revision: u64,
        cancelled_old: Option<RuntimeSessionEpoch>,
    },
    TransitionPublished {
        revision: u64,
    },
    Idempotent,
    Terminal(TerminalKind),
}

impl RegistrationFrameSink {
    pub fn handle_register(
        &self,
        machine: &mut HandshakeState,
        directory: &mut RuntimeRegistrationDirectory,
        session: &RuntimeSessionEpoch,
        register: &RegisterControl,
        context: &EpochContext,
        permits: &[ConsumerKind],
    ) -> RegistrationSinkOutput {
        match machine.on_register(register, context) {
            super::handshake::RegisterEvent::Validated { tuple } => {
                match directory.publish_pending(session, tuple, permits) {
                    Ok(published) => RegistrationSinkOutput::PendingPublished {
                        revision: published.revision,
                        cancelled_old: published.cancelled_old,
                    },
                    Err(PublishError::PermitUnavailable) => {
                        let terminal = machine.terminal_with(TerminalKind::RegistrationRefused);
                        RegistrationSinkOutput::Terminal(terminal)
                    }
                    Err(PublishError::DuplicateSession) => {
                        let terminal = machine.terminal_with(TerminalKind::DuplicateRegister);
                        RegistrationSinkOutput::Terminal(terminal)
                    }
                }
            }
            super::handshake::RegisterEvent::Idempotent => RegistrationSinkOutput::Idempotent,
            super::handshake::RegisterEvent::Transition { tuple } => {
                let current = match context.current.as_ref() {
                    Some(current) => current,
                    None => {
                        let terminal = machine.terminal_with(TerminalKind::StaleRegister);
                        return RegistrationSinkOutput::Terminal(terminal);
                    }
                };
                match directory.transition(session, tuple, current, context.pending.as_ref()) {
                    TransitionOutcome::Published { revision } => {
                        RegistrationSinkOutput::TransitionPublished { revision }
                    }
                    TransitionOutcome::Idempotent => RegistrationSinkOutput::Idempotent,
                    TransitionOutcome::NewGenerationRejected => {
                        let terminal =
                            machine.terminal_with(TerminalKind::NewGenerationBeforeEpochSwap);
                        RegistrationSinkOutput::Terminal(terminal)
                    }
                    TransitionOutcome::StaleClosed => {
                        let terminal = machine.terminal_with(TerminalKind::StaleRegister);
                        RegistrationSinkOutput::Terminal(terminal)
                    }
                }
            }
            super::handshake::RegisterEvent::Terminal(kind) => {
                RegistrationSinkOutput::Terminal(kind)
            }
        }
    }
}
