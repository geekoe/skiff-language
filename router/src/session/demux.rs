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

use std::fmt;
use std::sync::Arc;

use skiff_artifact_model::AssemblyActivationControl;
use skiff_runtime_transport::assembly_activation::{
    decode_assembly_activation_frame, AssemblyActivationFrameDirection,
};
use skiff_runtime_transport::protocol::{
    decode_binary_frame, decode_typed_binary_frame, spawn_submit_frame_direction, FrameDirection,
    RuntimeCapabilitiesFrameHeader, RuntimeFrameFamily, RuntimeHealthFrameHeader,
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
    /// One raw frame of an installed lane family (plan §5.5 sink bundle).
    /// The frame already passed the closed family registry framing, direction
    /// and payload-presence checks; the lane sink owns its codec.
    Sink {
        family: RuntimeFrameFamily,
        raw: Vec<u8>,
    },
    Unimplemented {
        family: RuntimeFrameFamily,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DemuxOutcome {
    Handled(DemuxEvent),
    Terminal(TerminalKind),
}

/// Closed family demux. Stateless: the session task owns phase/session state.
///
/// The central match stays closed: framing, direction and payload-presence
/// checks are unchanged. Installed lane sinks are an additive injection
/// point (`classify_with_sinks`); absent sinks preserve the W-session
/// `Unimplemented` fail-closed behavior.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuntimeFrameDemux;

impl RuntimeFrameDemux {
    pub fn classify(&self, raw: &[u8]) -> DemuxOutcome {
        self.classify_with_sinks(raw, &InboundSinkSet::default())
    }

    /// Same classification with the installed lane sink bundle (plan §5.5).
    pub fn classify_with_sinks(&self, raw: &[u8], sinks: &InboundSinkSet) -> DemuxOutcome {
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
            // Injection point: a well-framed type outside the closed family
            // registry may still belong to an installed lane sink (the frozen
            // registry prefixes cover `request.*`/`connection.*` but not the
            // Runtime→Router `response.*` subfamily or the lifecycle control
            // type). Absent an accepting sink the original fail-closed
            // behavior is preserved.
            if let Some(family) = sink_family_for_frame_type(sinks, frame_type) {
                return DemuxOutcome::Handled(DemuxEvent::Sink {
                    family,
                    raw: raw.to_vec(),
                });
            }
            return DemuxOutcome::Terminal(TerminalKind::MalformedFrame);
        };

        // Inbound frames must be Runtime->Router. The Spawn family is
        // mixed-direction (submit.request = Runtime->Router;
        // submit.response/error = Router->Runtime) and is narrowed
        // frame-level below; all others are Either at family level and are
        // also narrowed frame-level.
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
            RuntimeFrameFamily::Activation => self.classify_activation(raw, sinks),
            RuntimeFrameFamily::Request => {
                sink_or_unimplemented(sinks.request.as_ref(), family, raw)
            }
            RuntimeFrameFamily::Connection => {
                sink_or_unimplemented(sinks.connection.as_ref(), family, raw)
            }
            RuntimeFrameFamily::Actor => sink_or_unimplemented(sinks.actor.as_ref(), family, raw),
            RuntimeFrameFamily::Spawn => self.classify_spawn(frame_type, raw, sinks),
        }
    }

    /// Spawn family frame-level direction narrowing (C-model-spawn §3.0).
    ///
    /// Only `spawn.submit.request` may arrive from the Runtime and reach the
    /// installed spawn sink; `spawn.submit.response/error` are Router->Runtime
    /// frames and are direction violations here (fail closed, even with a
    /// sink installed). Without an installed sink the request terminates the
    /// exact session via `Unimplemented` (authority design §6.1).
    fn classify_spawn(&self, frame_type: &str, raw: &[u8], sinks: &InboundSinkSet) -> DemuxOutcome {
        match spawn_submit_frame_direction(frame_type) {
            Some(FrameDirection::RuntimeToRouter) => {
                sink_or_unimplemented(sinks.spawn.as_ref(), RuntimeFrameFamily::Spawn, raw)
            }
            Some(FrameDirection::RouterToRuntime) | Some(FrameDirection::Either) | None => {
                DemuxOutcome::Terminal(TerminalKind::MalformedFrame)
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

    fn classify_activation(&self, raw: &[u8], sinks: &InboundSinkSet) -> DemuxOutcome {
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
            _ => sink_or_unimplemented(
                sinks.activation_transaction.as_ref(),
                RuntimeFrameFamily::Activation,
                raw,
            ),
        }
    }
}

/// One installed lane sink (plan §5.5). The session task hands over a raw
/// frame already validated for family direction/payload presence; the sink
/// owns its family codec and returns a terminal kind when the frame must
/// close the exact session.
pub trait InboundFrameSink: Send + Sync + fmt::Debug {
    fn family(&self) -> RuntimeFrameFamily;

    /// Optional injection-point declaration: accepts frame types the closed
    /// family registry cannot classify (`response.*` for the Request family,
    /// `websocket.generation.lifecycle` for the Connection family). The
    /// central family match is never changed; this is only consulted after
    /// the central match fails and only while a sink is installed.
    fn accepts_frame_type(&self, _frame_type: &str) -> bool {
        false
    }

    fn handle(&self, session: &RuntimeSessionEpoch, raw: &[u8]) -> Result<(), TerminalKind>;
}

/// Injectable sink bundle (plan §5.5 `RuntimeFrameSinks` shape). Each slot is
/// optional; absent slots preserve the W-session `Unimplemented` fail-closed
/// behavior (the exact session terminates). The composition installs the
/// bundle before the listeners start; the bundle is never extended at
/// runtime.
#[derive(Debug, Default)]
pub struct InboundSinkSet {
    pub request: Option<Arc<dyn InboundFrameSink>>,
    pub connection: Option<Arc<dyn InboundFrameSink>>,
    pub activation_transaction: Option<Arc<dyn InboundFrameSink>>,
    pub actor: Option<Arc<dyn InboundFrameSink>>,
    pub spawn: Option<Arc<dyn InboundFrameSink>>,
}

impl InboundSinkSet {
    pub fn is_empty(&self) -> bool {
        self.request.is_none()
            && self.connection.is_none()
            && self.activation_transaction.is_none()
            && self.actor.is_none()
            && self.spawn.is_none()
    }

    pub fn sink_for(&self, family: RuntimeFrameFamily) -> Option<&Arc<dyn InboundFrameSink>> {
        match family {
            RuntimeFrameFamily::Session => None,
            // E-activation: the activation-transaction sink delivers
            // Runtime→Router Prepared/Reject ACKs to the coordinator. The
            // frozen demux central match still routes Register internally;
            // an absent sink keeps the Unimplemented fail-closed behavior.
            RuntimeFrameFamily::Activation => self.activation_transaction.as_ref(),
            RuntimeFrameFamily::Request => self.request.as_ref(),
            RuntimeFrameFamily::Connection => self.connection.as_ref(),
            RuntimeFrameFamily::Actor => self.actor.as_ref(),
            RuntimeFrameFamily::Spawn => self.spawn.as_ref(),
        }
    }
}

fn sink_or_unimplemented(
    sink: Option<&Arc<dyn InboundFrameSink>>,
    family: RuntimeFrameFamily,
    raw: &[u8],
) -> DemuxOutcome {
    match sink {
        Some(_) => DemuxOutcome::Handled(DemuxEvent::Sink {
            family,
            raw: raw.to_vec(),
        }),
        None => DemuxOutcome::Handled(DemuxEvent::Unimplemented { family }),
    }
}

fn sink_family_for_frame_type(
    sinks: &InboundSinkSet,
    frame_type: &str,
) -> Option<RuntimeFrameFamily> {
    let candidates = [
        sinks.request.as_ref(),
        sinks.connection.as_ref(),
        sinks.activation_transaction.as_ref(),
        sinks.actor.as_ref(),
        sinks.spawn.as_ref(),
    ];
    let mut matched = None;
    for sink in candidates.into_iter().flatten() {
        if sink.accepts_frame_type(frame_type) {
            if matched.is_some() {
                // Ambiguous sink claims fail closed.
                return None;
            }
            matched = Some(sink.family());
        }
    }
    matched
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
