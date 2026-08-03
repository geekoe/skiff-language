use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

#[cfg(test)]
use std::sync::OnceLock;

use base64::Engine as _;
use skiff_runtime_eval::{
    actor_executor::{ActorMethodExecutionRequest, ActorMethodExecutor, ActorMethodExecutorError},
    actor_instance::{
        ActorIncarnationKey, ActorInstanceFence, ActorInstanceStoreError, ActorLogicalKey,
        ACTOR_BOOTSTRAP_ENCODING_V1,
    },
};
use skiff_runtime_linked_program::{FileAddr, LinkedActorDeclarationOwner, UnitAddr};
use skiff_runtime_request::{ExecutionBudget, RouterWriterMessage};
use skiff_runtime_transport::{
    actor_method::{
        encode_actor_method_frame, ActorMethodCancelFrameHeader, ActorMethodCancelReason,
        ActorMethodErrorFrameHeader, ActorMethodErrorFramePayload, ActorMethodFrame,
        ActorMethodReturnFrameHeader, ActorOwnerFileFrameHeader, ActorOwnerUnitFrameHeader,
        ACTOR_RETURN_ENCODING_V1,
    },
    actor_owner::{
        encode_actor_owner_failure_frame, ActorOwnerControlFrameHeader,
        ActorOwnerFailureFrameHeader, ActorOwnerFailureReasonFrameHeader,
        ActorOwnerInvokeFrameHeader, ACTOR_OWNER_FAILURE_FRAME_TYPE,
    },
    protocol::RUNTIME_FRAME_SCHEMA_VERSION,
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::sync::mpsc;
use tracing::error;

use crate::{
    eval_capability_adapter::{ActorMethodEvalExecution, ActorMethodEvalExecutionInput},
    host::actor_owner_invocations::ActorOwnerCancellationReason,
};

use super::{
    actor_method_handoff::{
        AdmittedActorBootstrap, AdmittedActorMethodInput, AdmittedActorOwnerFence,
    },
    actor_route_holds::ActorRouteHoldGuard,
    RuntimeHost,
};

pub(super) enum ActorOwnerControlAcceptance {
    Accepted,
    Rejected(Option<ActorOwnerFailureReasonFrameHeader>),
}

#[cfg(test)]
pub(super) fn expired_actor_owner_terminal_barrier() -> &'static tokio::sync::Barrier {
    static BARRIER: OnceLock<tokio::sync::Barrier> = OnceLock::new();
    BARRIER.get_or_init(|| tokio::sync::Barrier::new(2))
}

#[cfg(test)]
pub(super) fn pending_actor_owner_after_admission_barrier() -> &'static tokio::sync::Barrier {
    static BARRIER: OnceLock<tokio::sync::Barrier> = OnceLock::new();
    BARRIER.get_or_init(|| tokio::sync::Barrier::new(2))
}

struct ActorOwnerInvocationTaskLease {
    registry: Arc<super::actor_owner_invocations::ActorOwnerInvocationRegistry>,
    identity: super::actor_owner_invocations::ActorOwnerInvocationIdentity,
    finished: bool,
}

impl ActorOwnerInvocationTaskLease {
    fn new(
        registry: Arc<super::actor_owner_invocations::ActorOwnerInvocationRegistry>,
        identity: super::actor_owner_invocations::ActorOwnerInvocationIdentity,
    ) -> Self {
        Self {
            registry,
            identity,
            finished: false,
        }
    }

    fn finish(&mut self) -> Option<ActorOwnerCancellationReason> {
        if self.finished {
            return None;
        }
        self.finished = true;
        self.registry.finish(&self.identity)
    }
}

impl Drop for ActorOwnerInvocationTaskLease {
    fn drop(&mut self) {
        if !self.finished {
            self.registry
                .cancel_registered(&self.identity, ActorOwnerCancellationReason::Cancelled);
        }
        let _ = self.finish();
    }
}

impl RuntimeHost {
    fn finish_actor_owner_activation(
        &self,
        session: &skiff_runtime_eval::actor_instance::ActorInstanceSessionLease,
        handle: skiff_runtime_eval::actor_instance::ActorInstanceHandle,
    ) -> ActorOwnerControlAcceptance {
        match self.track_actor_instance_with_lease(session, handle) {
            Ok(()) => ActorOwnerControlAcceptance::Accepted,
            Err(
                skiff_runtime_eval::actor_instance::ActorInstanceSessionTrackError::AlreadyTracked {
                    owner_session_id,
                },
            ) if owner_session_id == session.router_session_id() => {
                ActorOwnerControlAcceptance::Accepted
            }
            Err(error) => ActorOwnerControlAcceptance::Rejected(Some(control_reason(
                "ActorCreateFailed",
                &error.to_string(),
            ))),
        }
    }

    pub(super) async fn activate_actor_owner_control(
        &self,
        session: &skiff_runtime_eval::actor_instance::ActorInstanceSessionLease,
        control: &ActorOwnerControlFrameHeader,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> bool {
        let router_session_id = session.router_session_id();
        let Some(transition) = control.transition.as_ref() else {
            return false;
        };
        let Some((execution, _route_hold)) =
            build_owner_control_execution(self, router_session_id, control, sender, None, None)
        else {
            return false;
        };
        let Ok(context) = execution.context() else {
            return false;
        };
        let Ok(bootstrap) =
            base64::engine::general_purpose::STANDARD.decode(&transition.bootstrap_payload_base64)
        else {
            return false;
        };
        let executor = ActorMethodExecutor::new(self.actor_instances.store());
        let Ok(handle) = executor
            .activate_for_session(
                &self.actor_instances,
                session,
                execution.interpreter(),
                &context,
                match control_instance_fence(control) {
                    Ok(fence) => fence,
                    Err(_) => return false,
                },
                &transition.bootstrap_encoding_version,
                &bootstrap,
            )
            .await
        else {
            return false;
        };
        match self.track_actor_instance_with_lease(session, handle) {
            Ok(()) => true,
            Err(
                skiff_runtime_eval::actor_instance::ActorInstanceSessionTrackError::AlreadyTracked {
                    owner_session_id,
                },
            ) => owner_session_id == router_session_id,
            Err(_) => false,
        }
    }

    pub(super) async fn activate_actor_owner_initial(
        &self,
        session: &skiff_runtime_eval::actor_instance::ActorInstanceSessionLease,
        control: &ActorOwnerControlFrameHeader,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        test_effect_context: Option<crate::capability_context::ActorMethodTestEffectContext>,
    ) -> ActorOwnerControlAcceptance {
        let router_session_id = session.router_session_id();
        let Some(bootstrap) = control.bootstrap.as_ref() else {
            return ActorOwnerControlAcceptance::Rejected(Some(control_reason(
                "ActorCreateFailed",
                "actor.owner.control activateInitial requires bootstrap",
            )));
        };
        let Some(deadline) = control.deadline.as_ref() else {
            return ActorOwnerControlAcceptance::Rejected(Some(control_reason(
                "ActorCreateFailed",
                "actor.owner.control activateInitial requires deadline",
            )));
        };
        let Some(duration) = effective_deadline(deadline) else {
            return ActorOwnerControlAcceptance::Rejected(Some(control_reason(
                "ActorCreateTimeout",
                "actor create deadline has already expired",
            )));
        };
        let Some((execution, _route_hold)) = build_owner_control_execution(
            self,
            router_session_id,
            control,
            sender,
            Some(Instant::now() + duration),
            test_effect_context,
        ) else {
            return ActorOwnerControlAcceptance::Rejected(Some(control_reason(
                "ActorCreateFailed",
                "actor owner execution context is unavailable",
            )));
        };
        let Ok(context) = execution.context() else {
            return ActorOwnerControlAcceptance::Rejected(Some(control_reason(
                "ActorCreateFailed",
                "actor owner execution context is unavailable",
            )));
        };
        let Ok(bootstrap_payload) = bootstrap.decode_payload() else {
            return ActorOwnerControlAcceptance::Rejected(Some(control_reason(
                "ActorCreateFailed",
                "actor activation bootstrap payload is invalid",
            )));
        };
        let fence = match control_instance_fence(control) {
            Ok(fence) => fence,
            Err(message) => {
                return ActorOwnerControlAcceptance::Rejected(Some(control_reason(
                    "ActorCreateFailed",
                    &message,
                )))
            }
        };
        let executor = ActorMethodExecutor::new(self.actor_instances.store());
        let handle = match executor
            .activate_for_session(
                &self.actor_instances,
                session,
                execution.interpreter(),
                &context,
                fence,
                &bootstrap.encoding_version,
                &bootstrap_payload,
            )
            .await
        {
            Ok(handle) => handle,
            Err(error) => {
                return ActorOwnerControlAcceptance::Rejected(Some(control_reason(
                    "ActorCreateFailed",
                    &error.to_string(),
                )))
            }
        };
        self.finish_actor_owner_activation(session, handle)
    }

    pub(super) fn begin_actor_owner_invoke(
        &self,
        router_session_id: String,
        header: ActorOwnerInvokeFrameHeader,
        arguments_payload: Vec<u8>,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> crate::error::Result<
        std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>>,
    > {
        // Admission must complete in the synchronous Router frame handler. A later cancel,
        // disconnect, or root finalization is then guaranteed to observe this invocation even if
        // the session-owned future has not received its first poll yet.
        let test_effect_execution = match (
            header.invoke.test_case_capability.as_deref(),
            header.invoke.test_case_parent_request_id.as_deref(),
        ) {
            (Some(capability), Some(parent_request_id)) => {
                Some(self.test_http_entries.begin_actor_method(
                    capability,
                    parent_request_id,
                    &router_session_id,
                    header.invoke.invocation_id.clone(),
                )?)
            }
            (None, None) => None,
            _ => {
                return Err(crate::error::RuntimeError::Decode(
                    "Actor test capability and parent request id must be present together"
                        .to_string(),
                ))
            }
        };
        let test_request_revoker = test_effect_execution
            .as_ref()
            .map(crate::capability_context::ActorMethodTestEffectExecution::revoker);
        let actor_session = self
            .actor_instance_session_lease(&router_session_id)
            .map_err(|error| crate::error::RuntimeError::Decode(error.to_string()))?;
        let registration = self
            .actor_owner_invocations
            .register_with_test_revoker(
                header.invoke.invocation_id.clone(),
                router_session_id.clone(),
                header.invoke.cancellation_correlation.clone(),
                test_request_revoker.clone(),
            )
            .ok_or_else(|| {
                crate::error::RuntimeError::Decode("duplicate Actor invocation id".to_string())
            })?;
        let invocation_lease = ActorOwnerInvocationTaskLease::new(
            Arc::clone(&self.actor_owner_invocations),
            registration.identity().clone(),
        );
        let cancellation = registration.cancellation();
        let host = self.clone();
        let task = Box::pin(async move {
            let mut invocation_lease = invocation_lease;
            let invocation_id = header.invoke.invocation_id.clone();
            let trace_id = header
                .invoke
                .trace_id
                .as_deref()
                .unwrap_or_default()
                .to_string();
            let owner_runtime_id = header.owner_fence.owner_runtime_id.clone();
            let owner_lease_id = header.owner_fence.owner_lease_id.clone();
            let epoch = header.owner_fence.epoch;
            let actor_implementation_identity =
                header.owner_fence.actor_implementation_identity.clone();
            #[cfg(test)]
            if trace_id == "skiff-test:panic-after-actor-owner-admission" {
                panic!("injected Actor owner task panic after synchronous admission");
            }
            #[cfg(test)]
            if trace_id == "skiff-test:pending-after-actor-owner-admission" {
                pending_actor_owner_after_admission_barrier().wait().await;
                std::future::pending::<()>().await;
            }
            if let Err(failure) = host
                .execute_actor_owner_invoke(
                    &actor_session,
                    header,
                    arguments_payload,
                    &sender,
                    cancellation,
                    &mut invocation_lease,
                    test_request_revoker.as_ref(),
                    test_effect_execution
                        .as_ref()
                        .map(crate::capability_context::ActorMethodTestEffectExecution::context),
                )
                .await
            {
                error!(
                    event = "runtime.actor_owner_invoke_failed",
                    invocation_id = %invocation_id,
                    trace_id = %trace_id,
                    error = %failure.message
                );
                let bytes = if let Some(frame) = failure.terminal {
                    encode_actor_method_frame(&frame)
                } else {
                    encode_actor_owner_failure_frame(&ActorOwnerFailureFrameHeader {
                        schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
                        envelope_type: ACTOR_OWNER_FAILURE_FRAME_TYPE.into(),
                        invocation_id,
                        owner_runtime_id,
                        owner_lease_id,
                        epoch,
                        actor_implementation_identity,
                        reason: ActorOwnerFailureReasonFrameHeader {
                            code: "runtimeExecutionFailed".into(),
                            message: bounded_failure_message(&failure.message),
                        },
                    })
                };
                if let Ok(bytes) = bytes {
                    let _ = sender.send(RouterWriterMessage::Binary(bytes));
                }
            }
            // Keep the Host-private test execution owner alive through terminal encode/send and
            // failure logging, not merely through Eval's last poll.
            drop(test_effect_execution);
        });
        Ok(task)
    }

    #[cfg(test)]
    pub(super) fn spawn_actor_owner_invoke(
        &self,
        router_session_id: String,
        header: ActorOwnerInvokeFrameHeader,
        arguments_payload: Vec<u8>,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> crate::error::Result<tokio::task::JoinHandle<()>> {
        Ok(tokio::spawn(self.begin_actor_owner_invoke(
            router_session_id,
            header,
            arguments_payload,
            sender,
        )?))
    }

    async fn execute_actor_owner_invoke(
        &self,
        actor_session: &skiff_runtime_eval::actor_instance::ActorInstanceSessionLease,
        header: ActorOwnerInvokeFrameHeader,
        arguments_payload: Vec<u8>,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        cancellation: skiff_runtime_capability_context::CancellationToken,
        invocation_lease: &mut ActorOwnerInvocationTaskLease,
        test_request_revoker: Option<&crate::capability_context::TestRequestRevoker>,
        test_effect_context: Option<crate::capability_context::ActorMethodTestEffectContext>,
    ) -> Result<(), ActorOwnerExecutionFailure> {
        let route_hold_cell = Arc::new(Mutex::new(None::<ActorRouteHoldGuard>));
        let invocation_id = header.invoke.invocation_id.clone();
        let cancellation_correlation = header.invoke.cancellation_correlation.clone();
        let effective_deadline = effective_deadline(&header.invoke.deadline);
        let (result, terminal_fallback, deadline_at) = if let Some(timeout) = effective_deadline {
            let deadline_at = Instant::now() + timeout;
            let mut deadline = Box::pin(tokio::time::sleep_until(tokio::time::Instant::from_std(
                deadline_at,
            )));
            let mut execution = Box::pin(self.execute_actor_owner_invoke_inner(
                actor_session,
                &header,
                arguments_payload,
                sender,
                cancellation.clone(),
                test_effect_context,
                Arc::clone(&route_hold_cell),
            ));
            let (result, terminal_fallback) = tokio::select! {
                biased;
                _ = cancellation.wait_cancelled() => {
                    (None, Some(ActorOwnerCancellationReason::Cancelled))
                }
                _ = &mut deadline => {
                    self.actor_owner_invocations.cancel_registered(
                        &invocation_lease.identity,
                        ActorOwnerCancellationReason::DeadlineExceeded,
                    );
                    (None, Some(ActorOwnerCancellationReason::DeadlineExceeded))
                }
                result = &mut execution => (Some(result), None),
            };
            (result, terminal_fallback, Some(deadline_at))
        } else {
            // An already-expired or unparseable expiresAt is a deadline terminal, not an early
            // validation return. Record the winner while this exact registration is active, then
            // converge through the same authority revocation and lease finish path below.
            self.actor_owner_invocations.cancel_registered(
                &invocation_lease.identity,
                ActorOwnerCancellationReason::DeadlineExceeded,
            );
            (
                None,
                Some(ActorOwnerCancellationReason::DeadlineExceeded),
                None,
            )
        };
        // Eval's own execution budget can observe the same deadline before Tokio polls the
        // deadline branch above. Record the deadline while this exact registration is still
        // active so the terminal remains typed instead of degrading to owner.failure.
        if terminal_fallback.is_none()
            && deadline_at.is_some_and(|deadline_at| Instant::now() >= deadline_at)
        {
            self.actor_owner_invocations.cancel_registered(
                &invocation_lease.identity,
                ActorOwnerCancellationReason::DeadlineExceeded,
            );
        }
        // Once execution has a terminal winner, the invocation is no longer a valid parent for
        // recursive Actor/task work. Its ownership lease remains alive through terminal send.
        if let Some(revoker) = test_request_revoker {
            revoker.revoke();
        }
        let cancellation_reason = invocation_lease.finish().or(terminal_fallback);
        #[cfg(test)]
        if deadline_at.is_none()
            && header.invoke.trace_id.as_deref()
                == Some("skiff-test:pause-expired-actor-owner-before-terminal")
        {
            // Let the regression test observe the authority/registry state after the terminal
            // winner has converged but before the outer task can encode or send the terminal.
            expired_actor_owner_terminal_barrier().wait().await;
            expired_actor_owner_terminal_barrier().wait().await;
        }
        if let Some(reason) = cancellation_reason {
            return Err(ActorOwnerExecutionFailure::cancelled(
                &invocation_id,
                &cancellation_correlation,
                match reason {
                    ActorOwnerCancellationReason::Cancelled => ActorMethodCancelReason::Cancelled,
                    ActorOwnerCancellationReason::DeadlineExceeded => {
                        ActorMethodCancelReason::DeadlineExceeded
                    }
                },
            ));
        }
        let payload =
            result.expect("Actor owner execution result must exist without cancellation")?;
        let frame = ActorMethodFrame::Return(
            ActorMethodReturnFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
                envelope_type: "actor.method.return".into(),
                invocation_id,
                return_encoding_version: ACTOR_RETURN_ENCODING_V1.into(),
            },
            payload,
        );
        let bytes = encode_actor_method_frame(&frame)
            .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?;
        sender
            .send(RouterWriterMessage::Binary(bytes))
            .map_err(|_| ActorOwnerExecutionFailure::new("Router writer is closed"))
    }

    async fn execute_actor_owner_invoke_inner(
        &self,
        actor_session: &skiff_runtime_eval::actor_instance::ActorInstanceSessionLease,
        header: &ActorOwnerInvokeFrameHeader,
        arguments_payload: Vec<u8>,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        cancellation: skiff_runtime_capability_context::CancellationToken,
        test_effect_context: Option<crate::capability_context::ActorMethodTestEffectContext>,
        route_hold_cell: Arc<Mutex<Option<ActorRouteHoldGuard>>>,
    ) -> Result<Vec<u8>, ActorOwnerExecutionFailure> {
        let router_session_id = actor_session.router_session_id();
        let route = self
            .actor_execution_route(&header.route_authority, &header.invoke.actor_ref.service_id)
            .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?
            .ok_or_else(|| ActorOwnerExecutionFailure::new("Actor service is not active"))?;
        *route_hold_cell
            .lock()
            .expect("Actor owner route hold cell lock poisoned") =
            Some(self.actor_route_holds.acquire(route.active_assembly()));
        let budget = Arc::new(ExecutionBudget::new(
            skiff_runtime_request::execution_budget::ExecutionBudgetConfig::runtime_default(),
            effective_deadline(&header.invoke.deadline).map(|duration| Instant::now() + duration),
        ));
        let execution = ActorMethodEvalExecution::new(ActorMethodEvalExecutionInput {
            runtime_id: self.base_runtime_id.clone(),
            invocation_id: header.invoke.invocation_id.clone(),
            trace_id: header.invoke.trace_id.clone(),
            service_protocol_identity: route
                .service_protocol_identity()
                .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?
                .as_str()
                .to_string(),
            activation: Arc::clone(route.activation()),
            execution_image: Arc::clone(route.execution_image()),
            contexts: Arc::clone(route.context_set()),
            config_views: route
                .config_views()
                .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?,
            db_source: route
                .db_source()
                .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?,
            file_source: crate::capability_context::FileCapabilitySource::new(self.file_runtime()),
            http_options: self.http_runtime_options.clone(),
            outbound_requests: Arc::clone(&self.outbound_requests),
            actor_method_outbound: Arc::clone(&self.actor_method_outbound),
            telemetry_context: None,
            router_sender: Some(sender.clone()),
            connection_requests: Arc::clone(&self.connection_requests),
            router_session: skiff_runtime_capability_context::ConnectionRequestSession::new(
                router_session_id.to_string(),
            )
            .map_err(ActorOwnerExecutionFailure::new)?,
            http_response_max_bytes: self.default_http_response_max_bytes,
            cancellation,
            execution_budget: budget,
            request_heap_limits: self.request_heap_limits(),
            test_http_entries: self.test_http_entries.clone(),
            test_effect_context,
        })
        .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?;
        let context = execution
            .context()
            .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?;
        let input = admitted_input(header.clone(), arguments_payload)?;
        let fence = actor_instance_fence(&input)?;
        let bootstrap = input.activation_bootstrap.as_ref();
        let executor = ActorMethodExecutor::new(self.actor_instances.store());
        let handle = executor
            .activate_for_session(
                &self.actor_instances,
                actor_session,
                execution.interpreter(),
                &context,
                fence,
                bootstrap
                    .map(|value| value.encoding_version.as_str())
                    .unwrap_or(ACTOR_BOOTSTRAP_ENCODING_V1),
                bootstrap
                    .map(|value| value.payload.as_slice())
                    .unwrap_or(&[]),
            )
            .await
            .map_err(|error| {
                execution_error(
                    &header.invoke.invocation_id,
                    &header.invoke.actor_ref,
                    error,
                )
            })?;
        // Session-aware activation provisionally claims this exact Actor Arc before create can
        // suspend and atomically commits that ownership with admission. Re-publishing by the
        // string session id here would let a stale same-id connection generation cross the lease
        // fence after reconnect.
        executor
            .execute(
                execution.interpreter(),
                ActorMethodExecutionRequest {
                    instance: &handle,
                    method_identity: &input.invoke.method_identity,
                    arguments_payload: &input.arguments_payload,
                    context,
                },
            )
            .await
            .map_err(|error| {
                execution_error(
                    &header.invoke.invocation_id,
                    &header.invoke.actor_ref,
                    error,
                )
            })
    }
}

fn bounded_failure_message(message: &str) -> String {
    if message.is_empty() {
        return "Actor owner execution failed".to_string();
    }
    if message.len() <= 4096 {
        return message.to_string();
    }
    let mut end = 4096;
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message[..end].to_string()
}

pub(super) fn control_reason(code: &str, message: &str) -> ActorOwnerFailureReasonFrameHeader {
    ActorOwnerFailureReasonFrameHeader {
        code: code.to_string(),
        message: bounded_failure_message(message),
    }
}

fn build_owner_control_execution(
    host: &RuntimeHost,
    router_session_id: &str,
    control: &ActorOwnerControlFrameHeader,
    sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    budget_deadline: Option<Instant>,
    test_effect_context: Option<crate::capability_context::ActorMethodTestEffectContext>,
) -> Option<(ActorMethodEvalExecution, ActorRouteHoldGuard)> {
    let route = host
        .actor_execution_route(&control.route_authority, &control.fence.service_id)
        .ok()??;
    let route_hold = host.actor_route_holds.acquire(route.active_assembly());
    let cancellation = skiff_runtime_capability_context::CancellationToken::new();
    let budget = Arc::new(ExecutionBudget::new(
        skiff_runtime_request::execution_budget::ExecutionBudgetConfig::runtime_default(),
        budget_deadline,
    ));
    let execution = ActorMethodEvalExecution::new(ActorMethodEvalExecutionInput {
        runtime_id: host.base_runtime_id.clone(),
        invocation_id: control.request_id.clone(),
        trace_id: None,
        service_protocol_identity: route.service_protocol_identity().ok()?.as_str().to_string(),
        activation: Arc::clone(route.activation()),
        execution_image: Arc::clone(route.execution_image()),
        contexts: Arc::clone(route.context_set()),
        config_views: route.config_views().ok()?,
        db_source: route.db_source().ok()?,
        file_source: crate::capability_context::FileCapabilitySource::new(host.file_runtime()),
        http_options: host.http_runtime_options.clone(),
        outbound_requests: Arc::clone(&host.outbound_requests),
        actor_method_outbound: Arc::clone(&host.actor_method_outbound),
        telemetry_context: None,
        router_sender: Some(sender.clone()),
        connection_requests: Arc::clone(&host.connection_requests),
        router_session: skiff_runtime_capability_context::ConnectionRequestSession::new(
            router_session_id.to_string(),
        )
        .ok()?,
        http_response_max_bytes: host.default_http_response_max_bytes,
        cancellation,
        execution_budget: budget,
        request_heap_limits: host.request_heap_limits(),
        test_http_entries: host.test_http_entries.clone(),
        test_effect_context,
    })
    .ok()?;
    Some((execution, route_hold))
}

pub(super) fn control_instance_fence(
    control: &ActorOwnerControlFrameHeader,
) -> Result<ActorInstanceFence, String> {
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(&control.fence.canonical_actor_id_key_bytes_base64)
        .map_err(|_| "Actor id key is not base64".to_string())?;
    Ok(ActorInstanceFence {
        incarnation: ActorIncarnationKey {
            logical_key: ActorLogicalKey {
                service_id: control.fence.service_id.clone(),
                actor_type_identity: control.fence.actor_type_identity.clone(),
                actor_id_type_identity: control.fence.actor_id_type_identity.clone(),
                actor_id_encoding_version: control.fence.actor_id_encoding_version.clone(),
                canonical_actor_id_key_bytes: key_bytes,
                actor_id_hash: control.fence.actor_id_hash.clone(),
            },
            epoch: control.fence.epoch,
        },
        actor_abi_identity: control.fence.actor_abi_identity.clone(),
        actor_implementation_identity: control.fence.actor_implementation_identity.clone(),
        declaration_owner: linked_owner(&control.fence.declaration_owner),
    })
}

fn admitted_input(
    header: ActorOwnerInvokeFrameHeader,
    arguments_payload: Vec<u8>,
) -> Result<AdmittedActorMethodInput, ActorOwnerExecutionFailure> {
    let bootstrap = header
        .activation_bootstrap
        .map(|bootstrap| {
            bootstrap
                .decode_payload()
                .map(|payload| AdmittedActorBootstrap {
                    encoding_version: bootstrap.encoding_version,
                    payload,
                })
        })
        .transpose()
        .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?;
    Ok(AdmittedActorMethodInput {
        target_runtime_id: header.target_runtime_id,
        owner_fence: AdmittedActorOwnerFence {
            owner_runtime_id: header.owner_fence.owner_runtime_id,
            owner_lease_id: header.owner_fence.owner_lease_id,
            epoch: header.owner_fence.epoch,
            actor_abi_identity: header.owner_fence.actor_abi_identity,
            actor_implementation_identity: header.owner_fence.actor_implementation_identity,
            declaration_owner: header.owner_fence.declaration_owner,
        },
        invoke: header.invoke,
        arguments_payload,
        activation_bootstrap: bootstrap,
    })
}

fn actor_instance_fence(
    input: &AdmittedActorMethodInput,
) -> Result<ActorInstanceFence, ActorOwnerExecutionFailure> {
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(&input.invoke.actor_ref.canonical_actor_id_key_bytes_base64)
        .map_err(|_| ActorOwnerExecutionFailure::new("Actor id key is not base64"))?;
    Ok(ActorInstanceFence {
        incarnation: ActorIncarnationKey {
            logical_key: ActorLogicalKey {
                service_id: input.invoke.actor_ref.service_id.clone(),
                actor_type_identity: input.invoke.actor_ref.actor_type_identity.clone(),
                actor_id_type_identity: input.invoke.actor_ref.actor_id_type_identity.clone(),
                actor_id_encoding_version: input.invoke.actor_ref.actor_id_encoding_version.clone(),
                canonical_actor_id_key_bytes: key_bytes,
                actor_id_hash: input.invoke.actor_ref.actor_id_hash.clone(),
            },
            epoch: input.owner_fence.epoch,
        },
        actor_abi_identity: input.owner_fence.actor_abi_identity.clone(),
        actor_implementation_identity: input.owner_fence.actor_implementation_identity.clone(),
        declaration_owner: linked_owner(&input.owner_fence.declaration_owner),
    })
}

pub(super) fn linked_owner(
    owner: &skiff_runtime_transport::actor_method::ActorDeclarationOwnerFrameHeader,
) -> LinkedActorDeclarationOwner {
    LinkedActorDeclarationOwner {
        unit: match owner.unit {
            ActorOwnerUnitFrameHeader::Service => UnitAddr::Service,
            ActorOwnerUnitFrameHeader::Package(slot) => UnitAddr::Package(slot as usize),
        },
        file: match &owner.file {
            ActorOwnerFileFrameHeader::LoadedFileIndex(index) => {
                FileAddr::LoadedFileIndex(*index as usize)
            }
            ActorOwnerFileFrameHeader::FileIrIdentity(identity) => {
                FileAddr::FileIrIdentity(identity.clone())
            }
        },
        actor_symbol: owner.actor_symbol.clone(),
    }
}

fn effective_deadline(
    deadline: &skiff_runtime_transport::actor_method::ActorMethodDeadlineFrameHeader,
) -> Option<std::time::Duration> {
    let timeout = std::time::Duration::from_millis(deadline.timeout_ms);
    let expires_at = OffsetDateTime::parse(&deadline.expires_at, &Rfc3339).ok()?;
    let remaining = expires_at - OffsetDateTime::now_utc();
    if remaining.is_negative() || remaining.is_zero() {
        return None;
    }
    let remaining = std::time::Duration::try_from(remaining).ok()?;
    Some(timeout.min(remaining))
}

fn execution_error(
    invocation_id: &str,
    actor_ref: &skiff_runtime_transport::actor_method::ActorLogicalRefFrameHeader,
    error: ActorMethodExecutorError,
) -> ActorOwnerExecutionFailure {
    let current_epoch = match &error {
        ActorMethodExecutorError::Store(ActorInstanceStoreError::StaleEpoch { latest, .. }) => {
            Some(*latest)
        }
        ActorMethodExecutorError::Store(ActorInstanceStoreError::InstanceReplaced) => {
            actor_ref.epoch.checked_add(1)
        }
        _ => None,
    };
    let terminal = current_epoch.map(|current_epoch| {
        ActorMethodFrame::Error(ActorMethodErrorFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
            envelope_type: "actor.method.error".into(),
            invocation_id: invocation_id.to_string(),
            error: ActorMethodErrorFramePayload::ActorIncarnationReplacedError {
                actor_ref: actor_ref.clone(),
                current_epoch,
            },
        })
    });
    ActorOwnerExecutionFailure {
        message: error.to_string(),
        terminal,
    }
}

struct ActorOwnerExecutionFailure {
    message: String,
    terminal: Option<ActorMethodFrame>,
}

impl ActorOwnerExecutionFailure {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            terminal: None,
        }
    }

    fn cancelled(
        invocation_id: &str,
        cancellation_correlation: &str,
        reason: ActorMethodCancelReason,
    ) -> Self {
        Self {
            message: format!("Actor invocation ended with {reason:?}"),
            terminal: Some(ActorMethodFrame::Cancel(ActorMethodCancelFrameHeader {
                schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
                envelope_type: "actor.method.cancel".into(),
                invocation_id: invocation_id.to_string(),
                cancellation_correlation: cancellation_correlation.to_string(),
                reason,
            })),
        }
    }
}
