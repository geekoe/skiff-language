use skiff_artifact_model::{
    AssemblyActivationControl, RuntimeAssemblyRef, RuntimeConfigSnapshotRef,
};
use skiff_runtime_capability_context::CancellationSource;
use skiff_runtime_transport::{
    assembly_activation::{
        decode_assembly_activation_frame, encode_assembly_activation_frame,
        AssemblyActivationFrameDirection,
    },
    protocol::{decode_typed_binary_frame, TypedEnvelope},
};
use tokio::task::JoinSet;

use super::{ConnectionBootstrap, RuntimeError};
use crate::{
    error::Result,
    host::{RouterWriterMessage, RuntimeHost},
};

pub(super) enum SessionActivationState {
    Idle,
    Preparing(PendingActivationPrepare),
    TerminalReady(ReadyActivationTerminal),
}

pub(super) struct PendingActivationPrepare {
    prepare: AssemblyActivationControl,
    abort: AssemblyActivationControl,
    bootstrap: ConnectionBootstrap,
    cancellation: CancellationSource,
}

pub(super) struct ReadyActivationTerminal {
    prepare: AssemblyActivationControl,
    abort: AssemblyActivationControl,
    bootstrap: ConnectionBootstrap,
    reply: AssemblyActivationControl,
    inbound_abort_probe_pending: bool,
}

pub(super) type ActivationPrepareTaskResult = Result<Option<AssemblyActivationControl>>;

impl SessionActivationState {
    pub(super) fn is_preparing(&self) -> bool {
        matches!(self, Self::Preparing(_))
    }

    pub(super) fn is_terminal_ready(&self) -> bool {
        matches!(self, Self::TerminalReady(_))
    }

    pub(super) fn should_probe_terminal_abort(&self) -> bool {
        matches!(
            self,
            Self::TerminalReady(ReadyActivationTerminal {
                inbound_abort_probe_pending: true,
                ..
            })
        )
    }

    pub(super) fn finish_terminal_abort_probe(&mut self) -> Result<()> {
        let Self::TerminalReady(ready) = self else {
            return Err(RuntimeError::Decode(
                "assembly activation terminal Abort probe completed outside TerminalReady state"
                    .to_string(),
            ));
        };
        ready.inbound_abort_probe_pending = false;
        Ok(())
    }

    pub(super) fn assert_task_invariant(
        &self,
        tasks: &JoinSet<ActivationPrepareTaskResult>,
    ) -> Result<()> {
        let expected = usize::from(self.is_preparing());
        if tasks.len() != expected {
            return Err(RuntimeError::Decode(format!(
                "assembly activation session state expected {expected} prepare task(s), found {}",
                tasks.len()
            )));
        }
        Ok(())
    }

    pub(super) fn complete_prepare(
        &mut self,
        result: Option<AssemblyActivationControl>,
    ) -> Result<()> {
        let Self::Preparing(pending) = self else {
            return Err(RuntimeError::Decode(
                "assembly activation prepare completed outside Preparing state".to_string(),
            ));
        };
        let reply = result.ok_or_else(|| {
            RuntimeError::Decode(
                "assembly activation prepare completed without a terminal reply".to_string(),
            )
        })?;
        if pending.cancellation.is_cancelled() {
            return Err(RuntimeError::Decode(
                "cancelled assembly activation prepare produced a terminal reply".to_string(),
            ));
        }
        let pending = match std::mem::replace(self, Self::Idle) {
            Self::Preparing(pending) => pending,
            _ => unreachable!("Preparing state was checked above"),
        };
        *self = Self::TerminalReady(ReadyActivationTerminal {
            prepare: pending.prepare,
            abort: pending.abort,
            bootstrap: pending.bootstrap,
            reply,
            inbound_abort_probe_pending: true,
        });
        Ok(())
    }

    pub(super) fn mark_terminal_sent(&mut self) -> Result<()> {
        if !self.is_terminal_ready() {
            return Err(RuntimeError::Decode(
                "assembly activation terminal send completed outside TerminalReady state"
                    .to_string(),
            ));
        }
        *self = Self::Idle;
        Ok(())
    }
}

pub(super) fn router_binary_frame_type(bytes: &[u8]) -> Result<String> {
    let (typed, _) = decode_typed_binary_frame::<TypedEnvelope>(bytes)
        .map_err(super::super::transport_error_into_runtime_error)?;
    Ok(typed.envelope_type)
}

pub(super) async fn dispatch_session_activation_frame(
    host: &RuntimeHost,
    bytes: &[u8],
    bootstrap: &Option<ConnectionBootstrap>,
    state: &mut SessionActivationState,
    tasks: &mut JoinSet<ActivationPrepareTaskResult>,
) -> Result<Option<RouterWriterMessage>> {
    let bootstrap = bootstrap.as_ref().ok_or_else(|| {
        RuntimeError::Decode("assembly activation requires router.bootstrap first".to_string())
    })?;
    let control =
        decode_assembly_activation_frame(AssemblyActivationFrameDirection::RouterToRuntime, bytes)
            .map_err(super::super::transport_error_into_runtime_error)?;

    match &control {
        AssemblyActivationControl::Prepare { .. } => {
            match state {
                SessionActivationState::Idle => {}
                SessionActivationState::Preparing(current) => {
                    if activation_transaction_matches(&current.prepare, &control) {
                        return Ok(None);
                    }
                    return Err(RuntimeError::Decode(
                        "a different assembly activation prepare is already pending".to_string(),
                    ));
                }
                SessionActivationState::TerminalReady(current) => {
                    if activation_transaction_matches(&current.prepare, &control) {
                        return Ok(None);
                    }
                    return Err(RuntimeError::Decode(
                        "a different assembly activation terminal is awaiting send".to_string(),
                    ));
                }
            }
            if !tasks.is_empty() {
                return Err(RuntimeError::Decode(
                    "assembly activation task existed while session state was Idle".to_string(),
                ));
            }
            let abort = abort_control_for_prepare(&control)?;
            let cancellation = CancellationSource::new();
            let task_cancellation = cancellation.token();
            let task_host = host.clone();
            let task_bootstrap = bootstrap.clone();
            let task_control = control.clone();
            #[cfg(test)]
            let injected_fault = injected_prepare_fault(&control);
            tasks.spawn(async move {
                let result = task_host
                    .apply_cancellable_bootstrapped_assembly_activation_control(
                        task_control,
                        &task_bootstrap.resolver,
                        &task_bootstrap.config_snapshot_store,
                        Some(&task_bootstrap.service_db),
                        &task_cancellation,
                    )
                    .await
                    .map_err(|error| RuntimeError::Decode(error.to_string()));
                #[cfg(test)]
                if injected_fault == InjectedPrepareFault::CompleteWithoutTerminal {
                    return result.map(|_| None);
                }
                result
            });
            *state = SessionActivationState::Preparing(PendingActivationPrepare {
                prepare: control,
                abort,
                bootstrap: bootstrap.clone(),
                cancellation,
            });
            #[cfg(test)]
            match injected_fault {
                InjectedPrepareFault::MultipleTasks => {
                    tasks.spawn(std::future::pending());
                }
                InjectedPrepareFault::MissingTask => {
                    tokio::task::yield_now().await;
                    tasks.abort_all();
                    while tasks.join_next().await.is_some() {}
                }
                InjectedPrepareFault::None | InjectedPrepareFault::CompleteWithoutTerminal => {}
            }
            Ok(None)
        }
        AssemblyActivationControl::Abort { .. } => match state {
            SessionActivationState::Preparing(current) => {
                if !activation_transaction_matches(&current.prepare, &control) {
                    return Err(RuntimeError::Decode(
                        "activation abort tuple does not match pending prepare".to_string(),
                    ));
                }
                cancel_preparing_and_apply_abort(host, state, &control, tasks).await?;
                Ok(None)
            }
            SessionActivationState::TerminalReady(current) => {
                if !activation_transaction_matches(&current.prepare, &control) {
                    return Err(RuntimeError::Decode(
                        "activation abort tuple does not match ready prepare terminal".to_string(),
                    ));
                }
                // Keep the terminal guard installed until the exact Abort applies successfully.
                apply_control(host, control, &current.bootstrap).await?;
                *state = SessionActivationState::Idle;
                Ok(None)
            }
            SessionActivationState::Idle => {
                let reply = apply_control(host, control, bootstrap).await?;
                encode_optional_terminal(reply)
            }
        },
        _ if !matches!(state, SessionActivationState::Idle) => Err(RuntimeError::Decode(
            "assembly activation transition received before pending prepare completed".to_string(),
        )),
        _ => {
            let reply = apply_control(host, control, bootstrap).await?;
            encode_optional_terminal(reply)
        }
    }
}

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum InjectedPrepareFault {
    None,
    MissingTask,
    MultipleTasks,
    CompleteWithoutTerminal,
}

#[cfg(test)]
fn injected_prepare_fault(control: &AssemblyActivationControl) -> InjectedPrepareFault {
    let AssemblyActivationControl::Prepare { activation_id, .. } = control else {
        return InjectedPrepareFault::None;
    };
    match activation_id.as_str() {
        "test-fault-missing-prepare-task" => InjectedPrepareFault::MissingTask,
        "test-fault-multiple-prepare-tasks" => InjectedPrepareFault::MultipleTasks,
        "test-fault-complete-without-terminal" => InjectedPrepareFault::CompleteWithoutTerminal,
        _ => InjectedPrepareFault::None,
    }
}

pub(super) fn terminal_message(state: &SessionActivationState) -> Result<RouterWriterMessage> {
    let SessionActivationState::TerminalReady(ready) = state else {
        return Err(RuntimeError::Decode(
            "activation terminal outbox is not ready".to_string(),
        ));
    };
    encode_terminal(&ready.reply)
}

#[cfg(test)]
pub(super) fn inject_terminal_send_failure(state: &SessionActivationState) -> Result<()> {
    let SessionActivationState::TerminalReady(ready) = state else {
        return Ok(());
    };
    if matches!(
        &ready.prepare,
        AssemblyActivationControl::Prepare { activation_id, .. }
            if activation_id == "test-fault-terminal-send"
    ) {
        return Err(RuntimeError::Decode(
            "injected assembly activation terminal send failure".to_string(),
        ));
    }
    Ok(())
}

fn encode_optional_terminal(
    reply: Option<AssemblyActivationControl>,
) -> Result<Option<RouterWriterMessage>> {
    reply.map(|reply| encode_terminal(&reply)).transpose()
}

fn encode_terminal(reply: &AssemblyActivationControl) -> Result<RouterWriterMessage> {
    let frame =
        encode_assembly_activation_frame(AssemblyActivationFrameDirection::RuntimeToRouter, reply)
            .map_err(super::super::transport_error_into_runtime_error)?;
    Ok(RouterWriterMessage::Binary(frame))
}

async fn apply_control(
    host: &RuntimeHost,
    control: AssemblyActivationControl,
    bootstrap: &ConnectionBootstrap,
) -> Result<Option<AssemblyActivationControl>> {
    host.apply_bootstrapped_assembly_activation_control(
        control,
        &bootstrap.resolver,
        &bootstrap.config_snapshot_store,
        Some(&bootstrap.service_db),
    )
    .await
    .map_err(|error| RuntimeError::Decode(error.to_string()))
}

pub(super) async fn cleanup_session_activation(
    host: &RuntimeHost,
    state: &mut SessionActivationState,
    tasks: &mut JoinSet<ActivationPrepareTaskResult>,
) -> Result<()> {
    match state {
        SessionActivationState::Idle => {
            if tasks.is_empty() {
                return Ok(());
            }
            let primary = RuntimeError::Decode(
                "assembly activation task existed without a guarded transaction".to_string(),
            );
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            Err(primary)
        }
        SessionActivationState::Preparing(_) => {
            cancel_preparing_and_apply_synthetic_abort(host, state, tasks).await
        }
        SessionActivationState::TerminalReady(ready) => {
            let primary = (!tasks.is_empty()).then(|| {
                RuntimeError::Decode(format!(
                    "assembly activation TerminalReady state retained {} prepare task(s)",
                    tasks.len()
                ))
            });
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
            let abort_result = apply_control(host, ready.abort.clone(), &ready.bootstrap).await;
            if abort_result.is_ok() {
                *state = SessionActivationState::Idle;
            }
            if let Some(primary) = primary {
                return Err(primary);
            }
            abort_result.map(|_| ())
        }
    }
}

async fn cancel_preparing_and_apply_synthetic_abort(
    host: &RuntimeHost,
    state: &mut SessionActivationState,
    tasks: &mut JoinSet<ActivationPrepareTaskResult>,
) -> Result<()> {
    let abort = match state {
        SessionActivationState::Preparing(pending) => pending.abort.clone(),
        _ => {
            return Err(RuntimeError::Decode(
                "synthetic activation Abort requires Preparing state".to_string(),
            ));
        }
    };
    cancel_preparing_and_apply_abort(host, state, &abort, tasks).await
}

async fn cancel_preparing_and_apply_abort(
    host: &RuntimeHost,
    state: &mut SessionActivationState,
    abort: &AssemblyActivationControl,
    tasks: &mut JoinSet<ActivationPrepareTaskResult>,
) -> Result<()> {
    let SessionActivationState::Preparing(pending) = state else {
        return Err(RuntimeError::Decode(
            "activation cancellation requires Preparing state".to_string(),
        ));
    };
    pending.cancellation.cancel();

    let task_count = tasks.len();
    let mut primary = (task_count != 1).then(|| {
        RuntimeError::Decode(format!(
            "assembly activation Preparing state owned {task_count} tasks instead of one"
        ))
    });
    if task_count > 1 {
        tasks.abort_all();
    }
    while let Some(joined) = tasks.join_next().await {
        let error = match joined {
            Ok(Ok(_)) => None,
            Ok(Err(error)) => Some(error),
            Err(error) => Some(RuntimeError::Decode(format!(
                "assembly activation prepare task failed during cancellation: {error}"
            ))),
        };
        if primary.is_none() {
            primary = error;
        }
    }

    // The guard remains in `state` while the exact Abort is applied. This also repairs
    // loader state after a panic or an abruptly aborted prepare task.
    let bootstrap = match state {
        SessionActivationState::Preparing(pending) => &pending.bootstrap,
        _ => unreachable!("Preparing guard remains installed during cancellation"),
    };
    let abort_result = apply_control(host, abort.clone(), bootstrap)
        .await
        .map(|_| ());
    if abort_result.is_ok() {
        *state = SessionActivationState::Idle;
    }
    if let Some(primary) = primary {
        return Err(primary);
    }
    abort_result
}

fn activation_transaction_matches(
    left: &AssemblyActivationControl,
    right: &AssemblyActivationControl,
) -> bool {
    activation_transaction_tuple(left)
        .zip(activation_transaction_tuple(right))
        .is_some_and(|(left, right)| left == right)
}

type ActivationTransactionTuple<'a> = (
    &'a str,
    &'a str,
    u64,
    u64,
    &'a RuntimeAssemblyRef,
    &'a RuntimeConfigSnapshotRef,
    &'a str,
);

fn activation_transaction_tuple(
    control: &AssemblyActivationControl,
) -> Option<ActivationTransactionTuple<'_>> {
    match control {
        AssemblyActivationControl::Prepare {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            ..
        }
        | AssemblyActivationControl::Prepared {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
        }
        | AssemblyActivationControl::Reject {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            ..
        }
        | AssemblyActivationControl::Commit {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
            ..
        }
        | AssemblyActivationControl::Abort {
            environment,
            activation_id,
            expected_generation,
            candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
        } => Some((
            environment,
            activation_id,
            *expected_generation,
            *candidate_generation,
            assembly,
            config_snapshot,
            replica_id,
        )),
        AssemblyActivationControl::Register { .. } => None,
    }
}

fn abort_control_for_prepare(
    prepare: &AssemblyActivationControl,
) -> Result<AssemblyActivationControl> {
    let AssemblyActivationControl::Prepare {
        environment,
        activation_id,
        expected_generation,
        candidate_generation,
        assembly,
        config_snapshot,
        replica_id,
        ..
    } = prepare
    else {
        return Err(RuntimeError::Decode(
            "activation cancellation requires a Prepare control".to_string(),
        ));
    };
    Ok(AssemblyActivationControl::Abort {
        environment: environment.clone(),
        activation_id: activation_id.clone(),
        expected_generation: *expected_generation,
        candidate_generation: *candidate_generation,
        assembly: assembly.clone(),
        config_snapshot: config_snapshot.clone(),
        replica_id: replica_id.clone(),
    })
}
