use std::{sync::Arc, time::Instant};

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
    RuntimeHost,
};

impl RuntimeHost {
    pub(super) async fn activate_actor_owner_control(
        &self,
        router_session_id: &str,
        control: &ActorOwnerControlFrameHeader,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> bool {
        let Some(transition) = control.transition.as_ref() else {
            return false;
        };
        let Ok(Some(route)) = self.active_actor_execution_route(&control.fence.service_id) else {
            return false;
        };
        let cancellation = skiff_runtime_capability_context::CancellationToken::new();
        let budget = Arc::new(ExecutionBudget::new(
            skiff_runtime_request::execution_budget::ExecutionBudgetConfig::runtime_default(),
            None,
        ));
        let Ok(execution) = ActorMethodEvalExecution::new(ActorMethodEvalExecutionInput {
            runtime_id: self.base_runtime_id.clone(),
            invocation_id: control.request_id.clone(),
            service_protocol_identity: transition.actor_abi_identity.as_str().to_string(),
            activation: Arc::clone(route.activation()),
            execution_image: Arc::clone(route.execution_image()),
            contexts: Arc::clone(route.context_set()),
            config_views: match route.config_views() {
                Ok(views) => views,
                Err(_) => return false,
            },
            db_source: match route.db_source() {
                Ok(source) => source,
                Err(_) => return false,
            },
            file_source: crate::capability_context::FileCapabilitySource::new(self.file_runtime()),
            http_options: self.http_runtime_options.clone(),
            outbound_requests: Arc::clone(&self.outbound_requests),
            actor_method_outbound: Arc::clone(&self.actor_method_outbound),
            telemetry_context: None,
            router_sender: Some(sender.clone()),
            connection_requests: Arc::clone(&self.connection_requests),
            router_session: match skiff_runtime_capability_context::ConnectionRequestSession::new(
                router_session_id.to_string(),
            ) {
                Ok(session) => session,
                Err(_) => return false,
            },
            http_response_max_bytes: self.default_http_response_max_bytes,
            cancellation,
            execution_budget: budget,
            request_heap_limits: self.request_heap_limits(),
            test_http_entries: self.test_http_entries.clone(),
        }) else {
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
            .activate(
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
        match self.track_actor_instance(router_session_id, handle) {
            Ok(()) => true,
            Err(error) => error.to_string().contains("already tracked"),
        }
    }

    pub(super) fn spawn_actor_owner_invoke(
        &self,
        router_session_id: String,
        header: ActorOwnerInvokeFrameHeader,
        arguments_payload: Vec<u8>,
        sender: mpsc::UnboundedSender<RouterWriterMessage>,
    ) {
        let host = self.clone();
        tokio::spawn(async move {
            let invocation_id = header.invoke.invocation_id.clone();
            let owner_runtime_id = header.owner_fence.owner_runtime_id.clone();
            let owner_lease_id = header.owner_fence.owner_lease_id.clone();
            let epoch = header.owner_fence.epoch;
            let actor_implementation_identity =
                header.owner_fence.actor_implementation_identity.clone();
            if let Err(failure) = host
                .execute_actor_owner_invoke(&router_session_id, header, arguments_payload, &sender)
                .await
            {
                error!(
                    event = "runtime.actor_owner_invoke_failed",
                    invocation_id = %invocation_id,
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
        });
    }

    async fn execute_actor_owner_invoke(
        &self,
        router_session_id: &str,
        header: ActorOwnerInvokeFrameHeader,
        arguments_payload: Vec<u8>,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
    ) -> Result<(), ActorOwnerExecutionFailure> {
        let invocation_id = header.invoke.invocation_id.clone();
        let cancellation_correlation = header.invoke.cancellation_correlation.clone();
        let cancellation = self
            .actor_owner_invocations
            .register(invocation_id.clone(), cancellation_correlation.clone())
            .ok_or_else(|| ActorOwnerExecutionFailure::new("duplicate Actor invocation id"))?;

        let timeout = effective_deadline(&header.invoke.deadline).ok_or_else(|| {
            ActorOwnerExecutionFailure::cancelled(
                &invocation_id,
                &cancellation_correlation,
                ActorMethodCancelReason::DeadlineExceeded,
            )
        })?;
        let deadline_registry = Arc::clone(&self.actor_owner_invocations);
        let deadline_invocation = invocation_id.clone();
        let deadline_correlation = cancellation_correlation.clone();
        let deadline_task = tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            deadline_registry.cancel(
                &deadline_invocation,
                &deadline_correlation,
                ActorOwnerCancellationReason::DeadlineExceeded,
            );
        });

        let result = self
            .execute_actor_owner_invoke_inner(
                router_session_id,
                &header,
                arguments_payload,
                sender,
                cancellation,
            )
            .await;
        deadline_task.abort();
        let cancellation_reason = self.actor_owner_invocations.finish(&invocation_id);
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
        let payload = result?;
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
        router_session_id: &str,
        header: &ActorOwnerInvokeFrameHeader,
        arguments_payload: Vec<u8>,
        sender: &mpsc::UnboundedSender<RouterWriterMessage>,
        cancellation: skiff_runtime_capability_context::CancellationToken,
    ) -> Result<Vec<u8>, ActorOwnerExecutionFailure> {
        let route = self
            .active_actor_execution_route(&header.invoke.actor_ref.service_id)
            .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?
            .ok_or_else(|| ActorOwnerExecutionFailure::new("Actor service is not active"))?;
        let budget = Arc::new(ExecutionBudget::new(
            skiff_runtime_request::execution_budget::ExecutionBudgetConfig::runtime_default(),
            effective_deadline(&header.invoke.deadline).map(|duration| Instant::now() + duration),
        ));
        let execution = ActorMethodEvalExecution::new(ActorMethodEvalExecutionInput {
            runtime_id: self.base_runtime_id.clone(),
            invocation_id: header.invoke.invocation_id.clone(),
            service_protocol_identity: header.invoke.actor_abi_identity.as_str().to_string(),
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
            .activate(
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
        self.track_actor_instance(router_session_id, handle.clone())
            .or_else(|error| {
                if error.to_string().contains("already tracked") {
                    Ok(())
                } else {
                    Err(error)
                }
            })
            .map_err(|error| ActorOwnerExecutionFailure::new(error.to_string()))?;
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
