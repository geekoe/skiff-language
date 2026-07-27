use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll, Wake, Waker},
};

use bytes::Bytes;
use serde_json::{json, Value};
use skiff_artifact_model::{AssemblyIdentity, DeploymentRevision};
use skiff_runtime_boundary::file::{FileCreateOptions, ImmutableFileRef};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorFindControlRequest, ActorGetOrCreateControlRequest,
    ActorRemoveControlRequest, ActorReplaceControlRequest, ConnectionRequestTerminal,
    FileCapabilityFuture, NativeCapabilityContexts, StreamConsumerCleanup,
};
use skiff_runtime_model::{
    addr::{FileAddr, TypeAddr, UnitAddr},
    runtime_value::RuntimeObject,
    service_error::{LocalExecutionTypeIdentity, NamedUnionOwnerIdentity},
    type_plan::RuntimeRecordFieldPlan,
};
use skiff_runtime_native_contract::{
    NativeBindingKey, NativeCallPlan, NativeRequiredContext, TARGET_STD_HTTP_REQUEST,
    TARGET_STD_HTTP_SSE, TARGET_STD_HTTP_STREAM,
};

use super::{
    actor::ActorNativeDispatch, file::FileNativeDispatch, http::HttpNativeDispatch,
    time::TimeNativeDispatch, websocket::WebsocketNativeDispatch, NativeExternalOutcome,
    NativeExternalWait, PreparedNativeCall, RuntimeActorNativeMetadata, RuntimeNativeInvocation,
};
use crate::{
    capability::{
        NativeActorCapability, NativeCapabilityFuture, NativeFileCapability,
        NativeFileCapabilityBundle, NativeFileChunkSource, NativeFileSourceStreamCapability,
        NativeHttpClientCapability, NativeHttpResponseStreamCapability, NativeTelemetryCapability,
        NativeTimeCapability, NativeWebsocketCapability,
    },
    error::{Result, RuntimeError},
    runtime_value_facade::{
        bytes_value, ActorRef, RequestHeap, RequestHeapLimits, RuntimeTypeNode, RuntimeTypePlan,
        RuntimeValue,
    },
};

#[derive(Clone)]
struct CountingTimeContext {
    polls: Arc<AtomicUsize>,
}

impl NativeTimeCapability for CountingTimeContext {
    fn poll_execution_budget(&self) -> crate::error::Result<()> {
        self.polls.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

fn sleep_invocation() -> RuntimeNativeInvocation {
    RuntimeNativeInvocation::new(
        "std.time.sleep".to_string(),
        "std.time.sleep",
        Some(NativeCallPlan::new(
            NativeBindingKey::from_static("std.time.sleep"),
            vec![RuntimeTypePlan::new(
                "number",
                None,
                RuntimeTypeNode::Number,
            )],
            RuntimeTypePlan::new("null", None, RuntimeTypeNode::Null),
            NativeRequiredContext::Time,
        )),
        None,
        None,
    )
}

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

struct PendingOnce<T> {
    value: Option<T>,
    first_pending: bool,
    polls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}

impl<T> PendingOnce<T> {
    fn new(
        value: T,
        first_pending: bool,
        polls: Arc<AtomicUsize>,
        drops: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            value: Some(value),
            first_pending,
            polls,
            drops,
        }
    }
}

impl<T: Unpin> Future for PendingOnce<T> {
    type Output = Result<T>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let poll = self.polls.fetch_add(1, Ordering::AcqRel);
        if self.first_pending && poll == 0 {
            context.waker().wake_by_ref();
            return Poll::Pending;
        }
        Poll::Ready(Ok(self.value.take().expect("future completes once")))
    }
}

impl<T> Drop for PendingOnce<T> {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}

fn poll_external_wait(wait: &mut NativeExternalWait<'_>) -> Poll<Result<NativeExternalOutcome>> {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    wait.as_mut().poll(&mut context)
}

fn scalar_plan(name: &str, node: RuntimeTypeNode) -> RuntimeTypePlan {
    RuntimeTypePlan::new(name, None, node)
}

#[tokio::test]
async fn prepared_time_wait_does_not_borrow_caller_heap_and_observes_actual_pending() {
    let polls = Arc::new(AtomicUsize::new(0));
    let context = CountingTimeContext {
        polls: Arc::clone(&polls),
    };
    let mut heap = RequestHeap::default();
    let prepared = TimeNativeDispatch::prepare(
        context,
        sleep_invocation(),
        "std.time.sleep".to_string(),
        vec![RuntimeValue::Number(1_000.0)],
        &mut heap,
    )
    .expect("sleep should prepare");

    // This independent mutation is the ownership proof: the prepared wait is
    // live, but no borrow of the caller heap escaped prepare.
    heap.alloc_bytes(b"caller remains usable".to_vec())
        .expect("caller heap remains independently mutable");

    let PreparedNativeCall::ExternalWait(operation) = prepared else {
        panic!("sleep must defer the external timer to first poll");
    };
    let (mut wait, _finalize) = operation.into_parts();
    assert!(matches!(poll_external_wait(&mut wait), Poll::Pending));
    assert!(polls.load(Ordering::Acquire) > 0);
}

#[tokio::test]
async fn prepared_zero_time_wait_is_ready_on_its_real_first_poll() {
    let polls = Arc::new(AtomicUsize::new(0));
    let mut heap = RequestHeap::default();
    let prepared = TimeNativeDispatch::prepare(
        CountingTimeContext {
            polls: Arc::clone(&polls),
        },
        sleep_invocation(),
        "std.time.sleep".to_string(),
        vec![RuntimeValue::Number(0.0)],
        &mut heap,
    )
    .expect("zero sleep should prepare");
    let PreparedNativeCall::ExternalWait(operation) = prepared else {
        panic!("the dispatcher must not guess readiness from the binding name");
    };
    let (mut wait, finalize) = operation.into_parts();
    let Poll::Ready(outcome) = poll_external_wait(&mut wait) else {
        panic!("zero sleep should be Ready on its real first poll");
    };
    let outcome = outcome.expect("zero sleep should succeed");
    assert_eq!(polls.load(Ordering::Acquire), 1);
    assert_eq!(
        finalize
            .finalize(outcome, &mut heap)
            .expect("zero sleep finalizes"),
        RuntimeValue::Null
    );
}

mod actor;
mod core;
mod file;
mod http;
mod websocket;
