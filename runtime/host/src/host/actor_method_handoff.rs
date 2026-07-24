use std::{future::Future, pin::Pin};

use serde_json::Value;
use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_transport::{
    actor_method::{
        decode_actor_method_frame, ActorDeclarationOwnerFrameHeader, ActorLogicalRefFrameHeader,
        ActorMethodFrame, ActorMethodInvokeFrameHeader, ACTOR_ARGUMENTS_ENCODING_V1,
        ACTOR_RETURN_ENCODING_V1,
    },
    BinaryFrameError,
};
use thiserror::Error;

const ACTOR_BOOTSTRAP_ENCODING_V1: &str = "skiff-canonical-v1";

/// Result of decoding the public Actor method protocol.
///
/// Decoding this frame never grants execution permission. Router admission
/// must separately construct [`AdmittedActorMethodInput`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorMethodHandoff {
    ExternalInvoke(ActorMethodInvokeFrameHeader, Vec<u8>),
    ExternalCancel,
    UnexpectedRuntimeResponse,
}

pub fn receive_actor_method_frame(bytes: &[u8]) -> Result<ActorMethodHandoff, BinaryFrameError> {
    match decode_actor_method_frame(bytes)? {
        ActorMethodFrame::Invoke(header, payload) => {
            Ok(ActorMethodHandoff::ExternalInvoke(header, payload))
        }
        ActorMethodFrame::Cancel(_) => Ok(ActorMethodHandoff::ExternalCancel),
        ActorMethodFrame::Return(_, _) | ActorMethodFrame::Error(_) => {
            Ok(ActorMethodHandoff::UnexpectedRuntimeResponse)
        }
    }
}

/// Router-owned fence produced only after Actor method admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedActorOwnerFence {
    pub owner_runtime_id: String,
    pub owner_lease_id: String,
    pub epoch: u64,
    pub actor_abi_identity: ActorAbiIdentity,
    pub actor_implementation_identity: ActorImplementationIdentity,
    pub declaration_owner: ActorDeclarationOwnerFrameHeader,
}

/// Bootstrap is present only when this admission may activate the exact
/// incarnation for the first time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedActorBootstrap {
    pub encoding_version: String,
    pub payload: Vec<u8>,
}

/// Internal Router -> Runtime input. It is deliberately not a public wire
/// header and cannot be manufactured by decoding `actor.method.invoke`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdmittedActorMethodInput {
    pub target_runtime_id: String,
    pub owner_fence: AdmittedActorOwnerFence,
    pub invoke: ActorMethodInvokeFrameHeader,
    pub arguments_payload: Vec<u8>,
    pub activation_bootstrap: Option<AdmittedActorBootstrap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorExecutorRequest {
    pub invocation_id: String,
    pub actor_ref: ActorLogicalRefFrameHeader,
    pub owner_fence: AdmittedActorOwnerFence,
    pub method_identity: ActorMethodIdentity,
    pub arguments_payload: Vec<u8>,
    pub activation_bootstrap: Option<AdmittedActorBootstrap>,
    pub deadline: skiff_runtime_transport::actor_method::ActorMethodDeadlineFrameHeader,
    pub cancellation_correlation: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorExecutorOutput {
    Returned {
        encoding_version: String,
        payload: Vec<u8>,
    },
    TypedError(ActorExecutorTypedError),
    CoroutineNotImplemented,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorExecutorTypedError {
    ActorUpgrading {
        retry_after_ms: u64,
    },
    ActorVersionRejected {
        requested_implementation_identity: ActorImplementationIdentity,
        accepted_implementation_identity: ActorImplementationIdentity,
    },
    ActorIncarnationReplaced {
        current_epoch: u64,
    },
}

/// Narrow adapter seam for the eval-owned executor. The host handoff does not
/// depend on its concrete scheduler or field-frame implementation.
pub trait ActorMethodExecutor: Send + Sync {
    fn execute(
        &self,
        request: ActorExecutorRequest,
    ) -> Pin<Box<dyn Future<Output = ActorExecutorOutput> + Send + '_>>;
}

pub async fn execute_admitted_actor_method(
    local_runtime_id: &str,
    input: AdmittedActorMethodInput,
    executor: &impl ActorMethodExecutor,
) -> Result<ActorExecutorOutput, ActorMethodAdmissionError> {
    let request = prepare_admitted_actor_execution(local_runtime_id, input)?;
    validate_actor_executor_output(executor.execute(request).await)
}

pub fn prepare_admitted_actor_execution(
    local_runtime_id: &str,
    input: AdmittedActorMethodInput,
) -> Result<ActorExecutorRequest, ActorMethodAdmissionError> {
    validate_token(local_runtime_id, "local runtime id")?;
    validate_token(&input.target_runtime_id, "target runtime id")?;
    validate_token(
        &input.owner_fence.owner_runtime_id,
        "owner fence runtime id",
    )?;
    validate_token(&input.owner_fence.owner_lease_id, "owner lease id")?;

    if local_runtime_id != input.target_runtime_id
        || input.target_runtime_id != input.owner_fence.owner_runtime_id
    {
        return Err(ActorMethodAdmissionError::RuntimeMismatch);
    }
    if input.owner_fence.epoch == 0 {
        return Err(ActorMethodAdmissionError::InvalidEpoch);
    }
    if input.invoke.actor_ref.epoch != input.owner_fence.epoch {
        return Err(ActorMethodAdmissionError::EpochMismatch);
    }
    if input.invoke.actor_abi_identity != input.owner_fence.actor_abi_identity {
        return Err(ActorMethodAdmissionError::ActorAbiMismatch);
    }
    if input.invoke.actor_implementation_identity != input.owner_fence.actor_implementation_identity
    {
        return Err(ActorMethodAdmissionError::ActorImplementationMismatch);
    }
    if input.invoke.declaration_owner != input.owner_fence.declaration_owner {
        return Err(ActorMethodAdmissionError::DeclarationOwnerMismatch);
    }
    if input.invoke.arguments_encoding_version != ACTOR_ARGUMENTS_ENCODING_V1 {
        return Err(ActorMethodAdmissionError::ArgumentsEncodingMismatch);
    }
    if let Some(bootstrap) = &input.activation_bootstrap {
        validate_bootstrap(bootstrap)?;
    }

    Ok(ActorExecutorRequest {
        invocation_id: input.invoke.invocation_id,
        actor_ref: input.invoke.actor_ref,
        owner_fence: input.owner_fence,
        method_identity: input.invoke.method_identity,
        arguments_payload: input.arguments_payload,
        activation_bootstrap: input.activation_bootstrap,
        deadline: input.invoke.deadline,
        cancellation_correlation: input.invoke.cancellation_correlation,
    })
}

pub fn validate_actor_executor_output(
    output: ActorExecutorOutput,
) -> Result<ActorExecutorOutput, ActorMethodAdmissionError> {
    match &output {
        ActorExecutorOutput::Returned {
            encoding_version, ..
        } if encoding_version != ACTOR_RETURN_ENCODING_V1 => {
            Err(ActorMethodAdmissionError::ReturnEncodingMismatch)
        }
        ActorExecutorOutput::TypedError(ActorExecutorTypedError::ActorIncarnationReplaced {
            current_epoch,
        }) if *current_epoch == 0 => Err(ActorMethodAdmissionError::InvalidEpoch),
        _ => Ok(output),
    }
}

fn validate_bootstrap(bootstrap: &AdmittedActorBootstrap) -> Result<(), ActorMethodAdmissionError> {
    if bootstrap.encoding_version != ACTOR_BOOTSTRAP_ENCODING_V1 {
        return Err(ActorMethodAdmissionError::BootstrapEncodingMismatch);
    }
    let value: Value = serde_json::from_slice(&bootstrap.payload)
        .map_err(|_| ActorMethodAdmissionError::InvalidBootstrapPayload)?;
    if !value.is_object() {
        return Err(ActorMethodAdmissionError::InvalidBootstrapPayload);
    }
    let canonical = canonical_json_bytes(&value)
        .map_err(|_| ActorMethodAdmissionError::InvalidBootstrapPayload)?;
    if canonical != bootstrap.payload {
        return Err(ActorMethodAdmissionError::NonCanonicalBootstrapPayload);
    }
    Ok(())
}

fn validate_token(value: &str, label: &'static str) -> Result<(), ActorMethodAdmissionError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return Err(ActorMethodAdmissionError::InvalidToken { label });
    }
    Ok(())
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ActorMethodAdmissionError {
    #[error("{label} must be a non-empty canonical token")]
    InvalidToken { label: &'static str },
    #[error("Actor method admission targets a different Runtime")]
    RuntimeMismatch,
    #[error("Actor owner fence epoch must be positive")]
    InvalidEpoch,
    #[error("Actor invoke epoch does not match the admitted owner fence")]
    EpochMismatch,
    #[error("Actor invoke ABI does not match the admitted owner fence")]
    ActorAbiMismatch,
    #[error("Actor invoke implementation does not match the admitted owner fence")]
    ActorImplementationMismatch,
    #[error("Actor invoke declaration owner does not match the admitted owner fence")]
    DeclarationOwnerMismatch,
    #[error("Actor arguments encoding is unsupported")]
    ArgumentsEncodingMismatch,
    #[error("Actor activation bootstrap encoding is unsupported")]
    BootstrapEncodingMismatch,
    #[error("Actor activation bootstrap payload is invalid")]
    InvalidBootstrapPayload,
    #[error("Actor activation bootstrap payload is not canonical")]
    NonCanonicalBootstrapPayload,
    #[error("Actor return encoding is unsupported")]
    ReturnEncodingMismatch,
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    use skiff_runtime_transport::{
        actor_method::{
            encode_actor_method_frame, ActorMethodDeadlineFrameHeader, ActorOwnerFileFrameHeader,
            ActorOwnerUnitFrameHeader,
        },
        protocol::RUNTIME_FRAME_SCHEMA_VERSION,
    };

    use super::*;

    fn identity(prefix: &str, byte: char) -> String {
        format!("{prefix}:{}", byte.to_string().repeat(64))
    }

    fn invoke() -> ActorMethodInvokeFrameHeader {
        ActorMethodInvokeFrameHeader {
            schema_version: RUNTIME_FRAME_SCHEMA_VERSION.into(),
            envelope_type: "actor.method.invoke".into(),
            invocation_id: "inv:1".into(),
            actor_ref: ActorLogicalRefFrameHeader {
                service_id: "svc".into(),
                actor_type_identity: "actor".into(),
                actor_id_type_identity: "id".into(),
                actor_id_encoding_version: "skiff-canonical-v1".into(),
                canonical_actor_id_key_bytes_base64: "MQ==".into(),
                actor_id_hash: format!("sha256:{}", "d".repeat(64)),
                epoch: 7,
            },
            declaration_owner: ActorDeclarationOwnerFrameHeader {
                unit: ActorOwnerUnitFrameHeader::Service,
                file: ActorOwnerFileFrameHeader::FileIrIdentity("file".into()),
                actor_symbol: "Counter".into(),
            },
            actor_abi_identity: ActorAbiIdentity::new(identity("skiff-actor-abi-v1:sha256", 'a')),
            actor_implementation_identity: ActorImplementationIdentity::new(identity(
                "skiff-actor-implementation-v1:sha256",
                'b',
            )),
            method_identity: ActorMethodIdentity::new(identity(
                "skiff-actor-method-v1:sha256",
                'c',
            )),
            arguments_encoding_version: ACTOR_ARGUMENTS_ENCODING_V1.into(),
            deadline: ActorMethodDeadlineFrameHeader {
                timeout_ms: 10,
                expires_at: "2026-07-25T00:00:00Z".into(),
            },
            cancellation_correlation: "cancel:1".into(),
        }
    }

    fn admitted() -> AdmittedActorMethodInput {
        let invoke = invoke();
        AdmittedActorMethodInput {
            target_runtime_id: "runtime-1".into(),
            owner_fence: AdmittedActorOwnerFence {
                owner_runtime_id: "runtime-1".into(),
                owner_lease_id: "lease-1".into(),
                epoch: invoke.actor_ref.epoch,
                actor_abi_identity: invoke.actor_abi_identity.clone(),
                actor_implementation_identity: invoke.actor_implementation_identity.clone(),
                declaration_owner: invoke.declaration_owner.clone(),
            },
            invoke,
            arguments_payload: vec![1, 2],
            activation_bootstrap: Some(AdmittedActorBootstrap {
                encoding_version: ACTOR_BOOTSTRAP_ENCODING_V1.into(),
                payload: br#"{"count":0}"#.to_vec(),
            }),
        }
    }

    struct RecordingExecutor {
        calls: AtomicUsize,
        output: Mutex<Option<ActorExecutorOutput>>,
    }

    impl RecordingExecutor {
        fn new(output: ActorExecutorOutput) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                output: Mutex::new(Some(output)),
            }
        }
    }

    impl ActorMethodExecutor for RecordingExecutor {
        fn execute(
            &self,
            _request: ActorExecutorRequest,
        ) -> Pin<Box<dyn Future<Output = ActorExecutorOutput> + Send + '_>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let output = self
                .output
                .lock()
                .expect("recording executor lock poisoned")
                .take()
                .expect("recording executor should be called once");
            Box::pin(async move { output })
        }
    }

    #[test]
    fn external_invoke_remains_unadmitted() {
        let frame = ActorMethodFrame::Invoke(invoke(), vec![1]);
        let wire = encode_actor_method_frame(&frame).unwrap();
        assert!(matches!(
            receive_actor_method_frame(&wire).unwrap(),
            ActorMethodHandoff::ExternalInvoke(_, payload) if payload == vec![1]
        ));
    }

    #[test]
    fn admitted_input_retains_complete_fence_and_bootstrap() {
        let request = prepare_admitted_actor_execution("runtime-1", admitted()).unwrap();
        assert_eq!(request.owner_fence.owner_lease_id, "lease-1");
        assert_eq!(request.owner_fence.epoch, 7);
        assert_eq!(
            request.activation_bootstrap.unwrap().payload,
            br#"{"count":0}"#
        );
    }

    #[test]
    fn wrong_target_runtime_fails_closed() {
        assert_eq!(
            prepare_admitted_actor_execution("runtime-2", admitted()).unwrap_err(),
            ActorMethodAdmissionError::RuntimeMismatch
        );
    }

    #[test]
    fn every_external_identity_must_match_the_owner_fence() {
        let mut input = admitted();
        input.owner_fence.epoch += 1;
        assert_eq!(
            prepare_admitted_actor_execution("runtime-1", input).unwrap_err(),
            ActorMethodAdmissionError::EpochMismatch
        );

        let mut input = admitted();
        input.owner_fence.actor_abi_identity =
            ActorAbiIdentity::new(identity("skiff-actor-abi-v1:sha256", 'e'));
        assert_eq!(
            prepare_admitted_actor_execution("runtime-1", input).unwrap_err(),
            ActorMethodAdmissionError::ActorAbiMismatch
        );

        let mut input = admitted();
        input.owner_fence.declaration_owner.actor_symbol = "Other".into();
        assert_eq!(
            prepare_admitted_actor_execution("runtime-1", input).unwrap_err(),
            ActorMethodAdmissionError::DeclarationOwnerMismatch
        );
    }

    #[test]
    fn bootstrap_must_be_supported_canonical_record_encoding() {
        let mut input = admitted();
        input
            .activation_bootstrap
            .as_mut()
            .unwrap()
            .encoding_version = "other".into();
        assert_eq!(
            prepare_admitted_actor_execution("runtime-1", input).unwrap_err(),
            ActorMethodAdmissionError::BootstrapEncodingMismatch
        );

        let mut input = admitted();
        input.activation_bootstrap.as_mut().unwrap().payload = br#"{ "count": 0 }"#.to_vec();
        assert_eq!(
            prepare_admitted_actor_execution("runtime-1", input).unwrap_err(),
            ActorMethodAdmissionError::NonCanonicalBootstrapPayload
        );

        let mut input = admitted();
        input.activation_bootstrap.as_mut().unwrap().payload = br#"[0]"#.to_vec();
        assert_eq!(
            prepare_admitted_actor_execution("runtime-1", input).unwrap_err(),
            ActorMethodAdmissionError::InvalidBootstrapPayload
        );
    }

    #[test]
    fn executor_output_is_typed_and_validated() {
        let returned = ActorExecutorOutput::Returned {
            encoding_version: ACTOR_RETURN_ENCODING_V1.into(),
            payload: vec![1],
        };
        assert_eq!(
            validate_actor_executor_output(returned.clone()).unwrap(),
            returned
        );
        assert_eq!(
            validate_actor_executor_output(ActorExecutorOutput::Returned {
                encoding_version: "json".into(),
                payload: vec![],
            })
            .unwrap_err(),
            ActorMethodAdmissionError::ReturnEncodingMismatch
        );
        assert_eq!(
            validate_actor_executor_output(ActorExecutorOutput::CoroutineNotImplemented).unwrap(),
            ActorExecutorOutput::CoroutineNotImplemented
        );
    }

    #[tokio::test]
    async fn admitted_execution_calls_only_the_dedicated_executor() {
        let expected = ActorExecutorOutput::Returned {
            encoding_version: ACTOR_RETURN_ENCODING_V1.into(),
            payload: vec![9],
        };
        let executor = RecordingExecutor::new(expected.clone());
        let actual = execute_admitted_actor_method("runtime-1", admitted(), &executor)
            .await
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_admission_never_calls_the_executor() {
        let executor = RecordingExecutor::new(ActorExecutorOutput::CoroutineNotImplemented);
        let error = execute_admitted_actor_method("wrong-runtime", admitted(), &executor)
            .await
            .unwrap_err();
        assert_eq!(error, ActorMethodAdmissionError::RuntimeMismatch);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn invalid_executor_return_encoding_fails_closed() {
        let executor = RecordingExecutor::new(ActorExecutorOutput::Returned {
            encoding_version: "unsupported".into(),
            payload: vec![9],
        });
        let error = execute_admitted_actor_method("runtime-1", admitted(), &executor)
            .await
            .unwrap_err();
        assert_eq!(error, ActorMethodAdmissionError::ReturnEncodingMismatch);
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
    }
}
