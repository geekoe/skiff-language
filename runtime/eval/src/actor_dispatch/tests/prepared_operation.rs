use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::Poll,
};

use serde_json::json;
use skiff_artifact_model::{ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity};
use skiff_canonical_json::canonical_json_bytes;
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorCapabilityApi, ActorCapabilityContext, ActorFindControlRequest,
    ActorGetOrCreateControlRequest, ActorInvocationCancellation, ActorInvocationDeadline,
    ActorInvocationDeclarationOwner, ActorInvocationError, ActorInvocationIdentity,
    ActorInvocationOutcome, ActorInvocationOwnerFile, ActorInvocationOwnerUnit,
    ActorInvocationRequest, ActorRemoveControlRequest, ActorReplaceControlRequest, CapabilityError,
    CapabilityFuture, CapabilityResult, OwnedActorCapabilityContext, OwnedExecutionControl,
    OwnedRequestCapabilityContext, RequestCapabilityApi, RequestCapabilityContext,
    TaskSubmitControlRequest, TaskSubmitResponseControl,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::{ActorRef, RuntimeValue},
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};
use tokio::sync::oneshot;

use super::*;

fn assert_heap_free_wait<F>(_: &F)
where
    F: Future + Send + 'static,
{
}

async fn poll_once<F>(mut future: Pin<&mut F>) -> Option<F::Output>
where
    F: Future,
{
    std::future::poll_fn(|context| {
        Poll::Ready(match future.as_mut().poll(context) {
            Poll::Ready(output) => Some(output),
            Poll::Pending => None,
        })
    })
    .await
}

enum ActorReply {
    Ready(CapabilityResult<ActorInvocationOutcome>),
    Pending(oneshot::Receiver<CapabilityResult<ActorInvocationOutcome>>),
}

#[derive(Clone)]
struct RecordingActor {
    state: Arc<RecordingActorState>,
}

struct RecordingActorState {
    reply: Mutex<Option<ActorReply>>,
    starts: AtomicUsize,
    cancelled_waits: AtomicUsize,
}

impl RecordingActor {
    fn ready(outcome: CapabilityResult<ActorInvocationOutcome>) -> Self {
        Self {
            state: Arc::new(RecordingActorState {
                reply: Mutex::new(Some(ActorReply::Ready(outcome))),
                starts: AtomicUsize::new(0),
                cancelled_waits: AtomicUsize::new(0),
            }),
        }
    }

    fn pending() -> (
        Self,
        oneshot::Sender<CapabilityResult<ActorInvocationOutcome>>,
    ) {
        let (sender, receiver) = oneshot::channel();
        (
            Self {
                state: Arc::new(RecordingActorState {
                    reply: Mutex::new(Some(ActorReply::Pending(receiver))),
                    starts: AtomicUsize::new(0),
                    cancelled_waits: AtomicUsize::new(0),
                }),
            },
            sender,
        )
    }

    fn starts(&self) -> usize {
        self.state.starts.load(Ordering::Acquire)
    }

    fn cancelled_waits(&self) -> usize {
        self.state.cancelled_waits.load(Ordering::Acquire)
    }
}

struct PendingInvocationGuard {
    state: Arc<RecordingActorState>,
    completed: bool,
}

impl Drop for PendingInvocationGuard {
    fn drop(&mut self) {
        if !self.completed {
            self.state.cancelled_waits.fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl ActorCapabilityApi for RecordingActor {
    fn owned(&self) -> OwnedActorCapabilityContext {
        ActorCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> ActorCapabilityContext<'_> {
        ActorCapabilityContext::new(self.clone())
    }

    fn get_or_create_actor<'a>(
        &'a self,
        _request: ActorGetOrCreateControlRequest,
        _bootstrap_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn replace_actor<'a>(
        &'a self,
        _request: ActorReplaceControlRequest,
        _bootstrap_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorRef> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn find_actor<'a>(
        &'a self,
        _request: ActorFindControlRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, Option<ActorRef>> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn remove_actor<'a>(
        &'a self,
        _request: ActorRemoveControlRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, bool> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }

    fn invoke_actor<'a>(
        &'a self,
        _request: ActorInvocationRequest,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, ActorInvocationOutcome> {
        self.state.starts.fetch_add(1, Ordering::AcqRel);
        let reply = self
            .state
            .reply
            .lock()
            .expect("reply lock")
            .take()
            .expect("invocation must start only once");
        match reply {
            ActorReply::Ready(outcome) => Box::pin(async move { outcome }),
            ActorReply::Pending(receiver) => {
                let state = self.state.clone();
                Box::pin(async move {
                    let mut guard = PendingInvocationGuard {
                        state,
                        completed: false,
                    };
                    let outcome = receiver.await.map_err(|_| {
                        CapabilityError::provider_unavailable("actor.test", "closed")
                    })?;
                    guard.completed = true;
                    Ok(outcome?)
                })
            }
        }
    }
}

impl RequestCapabilityApi for RecordingActor {
    fn owned(&self) -> OwnedRequestCapabilityContext {
        RequestCapabilityContext::new(self.clone())
    }

    fn borrow(&self) -> RequestCapabilityContext<'_> {
        RequestCapabilityContext::new(self.clone())
    }

    fn runtime_id(&self) -> &str {
        "runtime:test"
    }

    fn service_id(&self) -> &str {
        "skiff.test/actor"
    }

    fn service_version(&self) -> &str {
        "1.0.0"
    }

    fn request_id(&self) -> &str {
        "request:test"
    }

    fn request_target(&self) -> &str {
        "actor.test"
    }

    fn request_build_id(&self) -> &str {
        "build:test"
    }

    fn task_service_protocol_identity(&self) -> &str {
        "task-protocol:test"
    }

    fn request_service_protocol_identity(&self) -> &str {
        "request-protocol:test"
    }

    fn operation_service_protocol_identity(&self) -> Option<&str> {
        None
    }

    fn activation_identity(&self) -> Option<&ActivationIdentityControl> {
        None
    }

    fn trace_id(&self) -> Option<&str> {
        None
    }

    fn submit_task<'a>(
        &'a self,
        _request: TaskSubmitControlRequest,
        _args_payload: Vec<u8>,
        _execution_control: OwnedExecutionControl,
    ) -> CapabilityFuture<'a, TaskSubmitResponseControl> {
        Box::pin(async { Err(CapabilityError::unsupported("not used")) })
    }
}

fn string_plan() -> RuntimeTypePlan {
    RuntimeTypePlan::synthetic_named_builtin("string", RuntimeTypeNode::String, Vec::new())
}

fn invocation_request() -> ActorInvocationRequest {
    ActorInvocationRequest {
        actor_ref: ActorRef::new(
            "skiff.test/actor",
            "actor-type:test",
            "actor-id-type:test",
            "canonical-json-v1",
            br#""actor-1""#.to_vec(),
            "actor-id-hash:test",
            Some(7),
        ),
        declaration_owner: ActorInvocationDeclarationOwner {
            unit: ActorInvocationOwnerUnit::Service,
            file: ActorInvocationOwnerFile::FileIrIdentity("file:actor-test".to_string()),
            actor_symbol: "TestActor".to_string(),
        },
        identity: ActorInvocationIdentity {
            invocation_id: "invocation:test".to_string(),
            expected_epoch: 7,
            actor_abi_identity: ActorAbiIdentity::new("actor-abi:test"),
            requested_implementation_identity: ActorImplementationIdentity::new(
                "actor-implementation:test",
            ),
            method_identity: ActorMethodIdentity::new("actor-method:test"),
            cancellation_correlation: "invocation:test:cancel".to_string(),
        },
        deadline: ActorInvocationDeadline { timeout_ms: 30_000 },
        arguments_payload: b"[]".to_vec(),
    }
}

fn prepared(actor: RecordingActor, return_plan: RuntimeTypePlan) -> PreparedActorMethodInvocation {
    PreparedActorMethodInvocation::new(
        ActorCapabilityContext::new(actor).owned(),
        invocation_request(),
        return_plan,
        "run".to_string(),
        30_000,
        crate::assembly_execution::ordinary::tests::test_runtime::execution_control().owned(),
    )
}

fn returned(value: serde_json::Value) -> ActorInvocationOutcome {
    ActorInvocationOutcome::Returned(
        canonical_json_bytes(&value).expect("return fixture should encode"),
    )
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_ready_preserves_prepared_segment_and_starts_once() {
    let actor = RecordingActor::ready(Ok(returned(json!("ready"))));
    let wait = prepared(actor.clone(), string_plan()).into_wait();
    assert_heap_free_wait(&wait);
    assert_eq!(actor.starts(), 0);
    let completion = wait.await;
    assert_eq!(actor.starts(), 1);
    let mut heap = RequestHeap::default();
    let value = completion
        .finalize(&mut heap)
        .expect("ready Actor return should finalize");
    assert_eq!(value.value(), &RuntimeValue::String("ready".to_string()));
    assert_eq!(actor.cancelled_waits(), 0);
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_pending_preserves_continuation_heap_boundary() {
    let (actor, sender) = RecordingActor::pending();
    let mut wait = Box::pin(prepared(actor.clone(), string_plan()).into_wait());
    assert!(poll_once(wait.as_mut()).await.is_none());
    assert_eq!(actor.starts(), 1);

    let mut heap = RequestHeap::default();
    heap.alloc_bytes(vec![4, 5, 6])
        .expect("caller heap must remain independently mutable");
    let before_finalize = heap.stats();
    sender
        .send(Ok(returned(json!("pending"))))
        .expect("pending Actor response should send");
    let completion = wait.await;
    assert_eq!(
        heap.stats(),
        before_finalize,
        "Actor wait must not write the caller heap"
    );
    let value = completion
        .finalize(&mut heap)
        .expect("pending Actor return should finalize");
    assert_eq!(value.value(), &RuntimeValue::String("pending".to_string()));
    assert_eq!(heap.stats(), before_finalize);
    assert_eq!(actor.starts(), 1);
    assert_eq!(actor.cancelled_waits(), 0);
}

#[tokio::test]
async fn actor_cancel_error_and_replacement_are_finalized_after_the_wait() {
    let mut heap = RequestHeap::default();
    let cancelled = prepared(
        RecordingActor::ready(Ok(ActorInvocationOutcome::Cancelled(
            ActorInvocationCancellation::Cancelled,
        ))),
        string_plan(),
    )
    .into_wait()
    .await
    .finalize(&mut heap)
    .expect_err("Actor cancellation should remain terminal");
    assert!(cancelled.is_cancellation_terminal());

    let replaced = prepared(
        RecordingActor::ready(Ok(ActorInvocationOutcome::ActorError(
            ActorInvocationError::ActorIncarnationReplaced {
                requested_epoch: 7,
                current_epoch: 8,
            },
        ))),
        string_plan(),
    )
    .into_wait()
    .await
    .finalize(&mut heap)
    .expect_err("replacement should remain an Actor error");
    let payload = replaced
        .ordinary_payload()
        .expect("replacement is an ordinary typed runtime error");
    assert_eq!(payload.code, "ActorIncarnationReplacedError");
    assert_eq!(
        payload.details,
        Some(json!({ "requestedEpoch": 7, "currentEpoch": 8 }))
    );

    let transport = prepared(
        RecordingActor::ready(Err(CapabilityError::provider_unavailable(
            "actor.test",
            "offline",
        ))),
        string_plan(),
    )
    .into_wait()
    .await
    .finalize(&mut heap)
    .expect_err("capability failure should remain opaque");
    assert!(matches!(transport, RuntimeError::Opaque(_)));
}

#[tokio::test]
async fn f445h_i6_actor_scope_method_drop_releases_the_single_invocation_owner() {
    let (actor, sender) = RecordingActor::pending();
    let mut wait = Box::pin(prepared(actor.clone(), string_plan()).into_wait());
    assert!(poll_once(wait.as_mut()).await.is_none());
    let mut heap = RequestHeap::default();
    heap.alloc_bytes(vec![9])
        .expect("caller heap must remain independently mutable");
    let before_drop = heap.stats();

    drop(wait);

    assert_eq!(actor.starts(), 1);
    assert_eq!(actor.cancelled_waits(), 1);
    assert!(
        sender.send(Ok(returned(json!("late")))).is_err(),
        "late outcome must be isolated after wait drop"
    );
    assert_eq!(heap.stats(), before_drop);
}

#[tokio::test]
async fn actor_finalize_heap_failure_rolls_back_partial_decode() {
    let actor = RecordingActor::ready(Ok(returned(json!([[1]]))));
    let completion = prepared(actor.clone(), RuntimeTypePlan::json_value_plan())
        .into_wait()
        .await;
    let mut heap = RequestHeap::new(RequestHeapLimits {
        max_nodes: 1,
        ..RequestHeapLimits::default()
    });
    let before = heap.stats();
    let before_len = heap.len();

    assert!(
        completion.finalize(&mut heap).is_err(),
        "nested return should exceed the heap node budget"
    );
    assert_eq!(heap.stats(), before);
    assert_eq!(heap.len(), before_len);
    assert_eq!(actor.starts(), 1);
}
