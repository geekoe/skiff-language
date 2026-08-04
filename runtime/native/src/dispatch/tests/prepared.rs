use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll, Wake, Waker},
};

use bytes::Bytes;
use serde_json::{json, Value};
use skiff_artifact_model::{AssemblyIdentity, DeploymentRevision};
use skiff_runtime_boundary::file::{FileCreateOptions, ImmutableFileRef};
use skiff_runtime_capability_context::{
    ActivationIdentityControl, ActorFindControlRequest, ActorGetOrCreateControlRequest,
    ActorRemoveControlRequest, ActorReplaceControlRequest, CancellationToken,
    ConnectionRequestTerminal, ExecutionControl, ExecutionControlApi, ExecutionControlResult,
    ExecutionScope, ExecutionScopeAccessError, FileCapabilityFuture, FileSourceStreamContext,
    NativeCapabilityContexts, OwnedExecutionControl, OwnedExecutionControlApi,
    StreamConsumerCleanup, StreamRuntime,
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
    execution_control: OwnedExecutionControl,
}

#[derive(Clone)]
struct PreparedTestExecutionControl {
    cancelled: Arc<AtomicBool>,
    cancellation: CancellationToken,
    scope: ExecutionScope,
}

impl PreparedTestExecutionControl {
    fn owned() -> OwnedExecutionControl {
        let cancellation = CancellationToken::new();
        OwnedExecutionControl::new(Self {
            cancelled: cancellation.cancel_flag(),
            scope: ExecutionScope::request(cancellation.clone(), None),
            cancellation,
        })
    }
}

impl ExecutionControlApi for PreparedTestExecutionControl {
    fn owned(&self) -> OwnedExecutionControl {
        OwnedExecutionControl::new(self.clone())
    }

    fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        self.scope
            .effective_deadline()
            .map(|deadline| deadline.at())
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }

    fn check_cancelled(&self) -> ExecutionControlResult<()> {
        Ok(())
    }

    fn add_instruction_units(&self, _units: u64) -> ExecutionControlResult<()> {
        Ok(())
    }

    fn poll_execution_budget(&self) -> ExecutionControlResult<()> {
        Ok(())
    }

    fn file_source_stream_context(
        &self,
        _stream_runtime: StreamRuntime,
    ) -> FileSourceStreamContext<'static> {
        panic!("prepared time tests do not create file source streams")
    }
}

impl OwnedExecutionControlApi for PreparedTestExecutionControl {
    fn borrow(&self) -> ExecutionControl<'_> {
        ExecutionControl::new(self.clone())
    }

    fn cancelled(&self) -> &AtomicBool {
        self.cancelled.as_ref()
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    fn deadline(&self) -> Option<std::time::Instant> {
        ExecutionControlApi::deadline(self)
    }

    fn execution_scope(&self) -> std::result::Result<ExecutionScope, ExecutionScopeAccessError> {
        Ok(self.scope.clone())
    }
}

impl NativeTimeCapability for CountingTimeContext {
    fn execution_control(&self) -> OwnedExecutionControl {
        self.execution_control.clone()
    }

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
        execution_control: PreparedTestExecutionControl::owned(),
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
            execution_control: PreparedTestExecutionControl::owned(),
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
