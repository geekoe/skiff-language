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
        method_identity: ActorMethodIdentity::new(identity("skiff-actor-method-v1:sha256", 'c')),
        arguments_encoding_version: ACTOR_ARGUMENTS_ENCODING_V1.into(),
        deadline: ActorMethodDeadlineFrameHeader {
            timeout_ms: 10,
            expires_at: "2026-07-25T00:00:00Z".into(),
        },
        cancellation_correlation: "cancel:1".into(),
        trace_id: None,
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
    assert_eq!(request.trace_id, None);
    assert_eq!(request.owner_fence.owner_lease_id, "lease-1");
    assert_eq!(request.owner_fence.epoch, 7);
    assert_eq!(
        request.activation_bootstrap.unwrap().payload,
        br#"{"count":0}"#
    );
}

#[test]
fn admitted_input_forwards_invoke_trace_id() {
    let mut input = admitted();
    input.invoke.trace_id = Some("trace:actor-invoke:1".to_string());
    let request = prepare_admitted_actor_execution("runtime-1", input).unwrap();
    assert_eq!(request.trace_id.as_deref(), Some("trace:actor-invoke:1"));
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
