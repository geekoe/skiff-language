use std::{
    collections::BTreeMap,
    fmt,
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use serde_json::Value;
use skiff_artifact_model::{
    BoundaryCancellationContract, BoundaryStreamContract, BoundaryValueCarrier,
    BoundaryValueLifetime, BoundaryValueOwner, BoundaryValuePlan, ContractTypeRef,
};
use skiff_runtime_activation::RequestStreamLease;
use skiff_runtime_boundary::package_schema_records::PackageSchemaRecords;
use skiff_runtime_boundary::service_linkable::{
    ServiceLinkableContractPlan, ServiceLinkableMaterializationError,
    ServiceLinkableMaterializationScope,
};
use skiff_runtime_capability_context::{
    CancellationToken, StreamCancelSignal, StreamInternalItem, StreamLifetimeGuard,
    StreamLifetimeGuardApi, StreamRuntimeError, StreamRuntimeResult,
};
use skiff_runtime_linked_program::CallIr;
use skiff_runtime_model::{
    request_heap::RequestHeap, runtime_value::RuntimeValue, type_plan::RuntimeTypePlan,
};

use super::{
    boundary_materialization::CanonicalServiceBoundaryPlan,
    callback_native::CallbackNativeCapabilityHooks, AssemblyExecutionHandoffError,
    AssemblyExecutionLaneKind, RuntimeExecutionProjection,
};
use crate::{
    capabilities::{StreamRuntimeOwner, StreamSink, StreamSinkApi},
    env::Env,
    error::{Result, RuntimeError},
    eval_context::EvalContext,
    program_execution::{OwnedProgramExecutionContext, ProgramExecutionContext},
    program_stream::{executable_body_contains_emit, linked_stream_item_type},
    runtime_ops::{runtime_from_wire, runtime_from_wire_required_plan, runtime_to_wire},
    type_projection::EvalTypeProjection,
    RuntimeAssemblyServiceCallTarget,
};

static PROVIDER_STREAM_TASKS_ACTIVE: AtomicUsize = AtomicUsize::new(0);

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn provider_stream_tasks_active_for_test() -> usize {
    PROVIDER_STREAM_TASKS_ACTIVE.load(Ordering::Acquire)
}

pub(crate) async fn execute_service_call(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    context.execution.check_cancelled()?;
    match AsyncStreamSpawn::for_target(&target) {
        AsyncStreamSpawn::ProviderUnary => {
            execute_provider_unary(context, call, target, args).await
        }
        AsyncStreamSpawn::ProviderStreamProducer => {
            start_provider_stream(context, call, target, args)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AsyncStreamSpawn {
    ProviderUnary,
    ProviderStreamProducer,
}

impl AsyncStreamSpawn {
    fn for_target(target: &RuntimeAssemblyServiceCallTarget) -> Self {
        match target.descriptor().contract.stream {
            BoundaryStreamContract::ServerStream { .. } => Self::ProviderStreamProducer,
            BoundaryStreamContract::Unary | BoundaryStreamContract::Unsupported { .. } => {
                Self::ProviderUnary
            }
        }
    }
}

async fn execute_provider_unary(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let boundary = CanonicalServiceBoundaryPlan::new(
        target.descriptor(),
        target.schema_records().as_ref(),
        args.len(),
    )?;
    let mut provider_heap = boundary.fresh_provider_heap(context.context.request_heap_limits());
    let caller_hooks = CallbackNativeCapabilityHooks::new(&context.context);
    let provider_args =
        boundary.materialize_parameters(&args, context.heap, &mut provider_heap, &caller_hooks)?;
    let provider_context = provider_execution_context(&context.context, &target)?;
    // Capture on every async-lane call. `may_suspend` never controls whether the future owns its
    // provider activation; it only describes the contract surface.
    let owned_provider = OwnedProgramExecutionContext::capture(&provider_context);
    let provider_addr = target.executable_addr().clone();
    let caller_addr = context.addr.clone();
    let caller_env = context.env.clone();
    let type_args = call.type_args.clone();
    let cancellation = context.execution.cancellation_token();
    let provider_request = target.provider_request().clone();
    let provider_result = {
        let provider_context = owned_provider.borrow();
        let provider_future = context.interpreter.call_program_executable(
            provider_context,
            &mut provider_heap,
            &caller_env,
            &caller_addr,
            &provider_addr,
            &type_args,
            provider_args,
        );
        await_provider_unary(
            target.descriptor().operation_id.as_str(),
            &target.descriptor().contract.cancellation,
            &cancellation,
            provider_future,
        )
        .await
    };
    let provider_result = match provider_result {
        Err(error) if error.is_cancelled() => {
            provider_request.cancel();
            return Err(error);
        }
        result => result,
    };

    let provider_context = owned_provider.borrow();
    let hooks = CallbackNativeCapabilityHooks::new(&provider_context);
    boundary.materialize_provider_result(provider_result, &mut provider_heap, context.heap, &hooks)
}

async fn await_provider_unary<F>(
    operation_id: &str,
    cancellation_contract: &BoundaryCancellationContract,
    cancellation: &CancellationToken,
    provider_future: F,
) -> Result<RuntimeValue>
where
    F: Future<Output = Result<RuntimeValue>>,
{
    tokio::pin!(provider_future);
    match cancellation_contract {
        BoundaryCancellationContract::Cooperative => {
            tokio::select! {
                biased;
                _ = cancellation.wait_cancelled() => Err(RuntimeError::Cancelled),
                result = &mut provider_future => result,
            }
        }
        BoundaryCancellationContract::NotCancellable => provider_future.await,
        BoundaryCancellationContract::Unsupported { reason } => {
            Err(RuntimeError::Unsupported(format!(
                "canonical service operation {operation_id} has unsupported cancellation semantics: {reason:?}"
            )))
        }
    }
}

fn start_provider_stream(
    context: &mut EvalContext<'_>,
    call: &CallIr,
    target: RuntimeAssemblyServiceCallTarget,
    args: Vec<RuntimeValue>,
) -> Result<RuntimeValue> {
    let BoundaryStreamContract::ServerStream {
        item_type,
        item_value_plan,
    } = &target.descriptor().contract.stream
    else {
        return Err(AssemblyExecutionHandoffError::unavailable_at(
            AssemblyExecutionLaneKind::AsyncStreamCancel,
            "stream-contract",
        ));
    };
    let boundary = CanonicalServiceBoundaryPlan::new(
        target.descriptor(),
        target.schema_records().as_ref(),
        args.len(),
    )?;
    canonical_scope(
        item_value_plan,
        BoundaryValueOwner::Provider,
        BoundaryValueLifetime::Stream,
    )?;
    if let BoundaryCancellationContract::Unsupported { reason } =
        &target.descriptor().contract.cancellation
    {
        return Err(RuntimeError::Unsupported(format!(
            "canonical service operation {} has unsupported cancellation semantics: {reason:?}",
            target.descriptor().operation_id
        )));
    }
    // Open the request's stream lifetime before parameter materialization: T06 may register a
    // stream-scoped callback while projecting a parameter, and registration must observe the
    // already-live stream lease. Every preparation error below drops this lease immediately.
    let lease = target.provider_request().open_stream().ok_or_else(|| {
        RuntimeError::ProviderUnavailable {
            target: target.descriptor().operation_id.to_string(),
            reason: "request stream lifetime is already terminal".to_string(),
        }
    })?;

    let mut provider_heap = boundary.fresh_provider_heap(context.context.request_heap_limits());
    let hooks = CallbackNativeCapabilityHooks::new(&context.context);
    let provider_args =
        boundary.materialize_parameters(&args, context.heap, &mut provider_heap, &hooks)?;
    let mut provider_context = provider_execution_context(&context.context, &target)?;
    let provider_stream_item_type = provider_stream_item_execution_plan(
        context.interpreter,
        &provider_context,
        context.addr,
        target.executable_addr(),
        context.env,
        &call.type_args,
    )?;
    let provider_stream_runtime_owner = provider_context.take_stream_runtime_owner();
    let owned_provider = Arc::new(OwnedProgramExecutionContext::capture(&provider_context));
    let stream_runtime = context.context.stream_runtime();
    let (stream_value, concrete_sink) = stream_runtime.channel_stream_with_lifetime(
        StreamLifetimeGuard::new(ProviderStreamLifetime { _lease: lease }),
    );
    let stream_cancel = concrete_sink.cancel_signal();
    let sink = StreamSink::new(BoundaryStreamSink {
        inner: concrete_sink,
        item_type: item_type.clone(),
        item_value_plan: item_value_plan.clone(),
        schema_records: Arc::clone(target.schema_records()),
        execution_item_type: provider_stream_item_type.clone(),
        provider_context: Arc::clone(&owned_provider),
    });
    let receiver_value = match runtime_from_wire(&stream_value, context.heap) {
        Ok(value) => value,
        Err(error) => {
            stream_runtime.cancel(&stream_value);
            return Err(error);
        }
    };

    let mut producer_env = context.env.clone();
    producer_env.stream_sink = Some(sink.clone());
    // This plan only serializes the provider File-IR value passed to `emit`. BoundaryStreamSink
    // remains the semantic owner and applies the canonical contract plan before publication.
    producer_env.current_stream_item_type = provider_stream_item_type;
    let producer = ProviderStreamTask {
        interpreter: context.interpreter.clone_for_stream_producer(),
        provider_context: owned_provider,
        provider_heap,
        provider_env: producer_env,
        caller_addr: context.addr.clone(),
        provider_addr: target.executable_addr().clone(),
        type_args: call.type_args.clone(),
        args: provider_args,
        stream_value,
        sink,
        stream_cancel,
        cancellation: context.execution.cancellation_token(),
        cancellation_contract: target.descriptor().contract.cancellation.clone(),
        request: target.provider_request().clone(),
        _stream_runtime_owner: provider_stream_runtime_owner,
    };
    spawn_provider_stream(producer);
    Ok(receiver_value)
}

fn provider_stream_item_execution_plan(
    interpreter: &crate::Interpreter,
    provider_context: &ProgramExecutionContext<'_>,
    caller_addr: &skiff_runtime_linked_program::ExecutableAddr,
    provider_addr: &skiff_runtime_linked_program::ExecutableAddr,
    caller_env: &Env,
    type_args: &BTreeMap<String, skiff_runtime_linked_program::LinkedTypeRef>,
) -> Result<Option<RuntimeTypePlan>> {
    let projection = RuntimeExecutionProjection::for_context(interpreter, provider_context)?;
    let resolved = projection.resolve_nested_executable(provider_addr)?;
    if !executable_body_contains_emit(resolved.executable) {
        return Ok(None);
    }
    let item_type =
        linked_stream_item_type(resolved.executable.return_type.as_ref()).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "canonical server-stream provider {} emits values but does not return Stream<T>",
                resolved.executable.symbol
            ))
        })?;
    let type_projection = EvalTypeProjection::from_execution_projection(projection.clone());
    let substitutions = type_projection.call_type_substitutions(
        caller_addr,
        &caller_env.type_substitutions,
        resolved.executable,
        type_args,
    );
    type_projection
        .plan_from_linked_nested_ref_with_substitutions(item_type, &resolved.addr, &substitutions)
        .map(Some)
}

struct ProviderStreamTask {
    interpreter: crate::Interpreter,
    provider_context: Arc<OwnedProgramExecutionContext>,
    provider_heap: RequestHeap,
    provider_env: Env,
    caller_addr: skiff_runtime_linked_program::ExecutableAddr,
    provider_addr: skiff_runtime_linked_program::ExecutableAddr,
    type_args: BTreeMap<String, skiff_runtime_linked_program::LinkedTypeRef>,
    args: Vec<RuntimeValue>,
    stream_value: Value,
    sink: StreamSink,
    stream_cancel: StreamCancelSignal,
    cancellation: CancellationToken,
    cancellation_contract: BoundaryCancellationContract,
    request: skiff_runtime_activation::RequestActivationContext,
    _stream_runtime_owner: Option<StreamRuntimeOwner>,
}

fn spawn_provider_stream(producer: ProviderStreamTask) {
    tokio::spawn(async move {
        let _active = ProviderStreamTaskGuard::new();
        run_provider_stream(producer).await;
    });
}

async fn run_provider_stream(mut producer: ProviderStreamTask) {
    let args = std::mem::take(&mut producer.args);
    let terminal = {
        let provider_context = producer.provider_context.borrow();
        let provider_future = producer.interpreter.call_program_executable(
            provider_context,
            &mut producer.provider_heap,
            &producer.provider_env,
            &producer.caller_addr,
            &producer.provider_addr,
            &producer.type_args,
            args,
        );
        tokio::pin!(provider_future);
        match producer.cancellation_contract {
            BoundaryCancellationContract::Cooperative => {
                tokio::select! {
                    biased;
                    _ = producer.stream_cancel.wait_cancelled() => ProviderTerminal::ConsumerCancelled,
                    _ = producer.cancellation.wait_cancelled() => ProviderTerminal::RequestCancelled,
                    result = &mut provider_future => ProviderTerminal::Provider(result),
                }
            }
            BoundaryCancellationContract::NotCancellable => {
                tokio::select! {
                    biased;
                    _ = producer.stream_cancel.wait_cancelled() => ProviderTerminal::ConsumerCancelled,
                    result = &mut provider_future => ProviderTerminal::Provider(result),
                }
            }
            BoundaryCancellationContract::Unsupported { reason } => {
                ProviderTerminal::Provider(Err(RuntimeError::Unsupported(format!(
                    "unsupported stream cancellation semantics: {reason:?}"
                ))))
            }
        }
    };

    match terminal {
        ProviderTerminal::Provider(Ok(_)) => {
            publish_provider_terminal(&producer, ProviderStreamPublication::End).await;
        }
        ProviderTerminal::Provider(Err(error)) if error.is_cancelled() => {
            producer.request.cancel();
            producer
                .interpreter
                .stream_runtime
                .cancel(&producer.stream_value);
        }
        ProviderTerminal::Provider(Err(error)) => {
            publish_provider_terminal(
                &producer,
                ProviderStreamPublication::Error(StreamRuntimeError::producer(error)),
            )
            .await;
        }
        ProviderTerminal::ConsumerCancelled => {
            producer
                .interpreter
                .stream_runtime
                .cancel(&producer.stream_value);
        }
        ProviderTerminal::RequestCancelled => {
            producer.request.cancel();
            producer
                .interpreter
                .stream_runtime
                .cancel(&producer.stream_value);
        }
    }
}

async fn publish_provider_terminal(
    producer: &ProviderStreamTask,
    publication: ProviderStreamPublication,
) {
    let publication = async {
        match publication {
            ProviderStreamPublication::End => producer.sink.end().await,
            ProviderStreamPublication::Error(error) => producer.sink.fail(error).await,
        }
    };
    tokio::pin!(publication);
    match producer.cancellation_contract {
        BoundaryCancellationContract::Cooperative => {
            tokio::select! {
                biased;
                _ = producer.stream_cancel.wait_cancelled() => {},
                _ = producer.cancellation.wait_cancelled() => {
                    producer.request.cancel();
                    producer.interpreter.stream_runtime.cancel(&producer.stream_value);
                },
                _ = &mut publication => {},
            }
        }
        BoundaryCancellationContract::NotCancellable => publication.await,
        BoundaryCancellationContract::Unsupported { .. } => {
            producer
                .interpreter
                .stream_runtime
                .cancel(&producer.stream_value);
        }
    }
}

enum ProviderTerminal {
    Provider(Result<RuntimeValue>),
    ConsumerCancelled,
    RequestCancelled,
}

enum ProviderStreamPublication {
    End,
    Error(StreamRuntimeError),
}

fn provider_execution_context<'a>(
    receiver: &ProgramExecutionContext<'a>,
    target: &RuntimeAssemblyServiceCallTarget,
) -> Result<ProgramExecutionContext<'a>> {
    let provider_target = receiver
        .runtime_assembly_target()?
        .with_request_activation(target.provider_request().clone())?;
    Ok(receiver
        .clone()
        .with_runtime_assembly_target(provider_target))
}

#[allow(clippy::too_many_arguments)]
fn materialize_value(
    stage: &str,
    ty: &ContractTypeRef,
    schema_records: &PackageSchemaRecords,
    value_plan: &BoundaryValuePlan,
    value: &RuntimeValue,
    source_heap: &RequestHeap,
    destination_heap: &mut RequestHeap,
    scope: ServiceLinkableMaterializationScope,
    hooks: &dyn skiff_runtime_boundary::service_linkable::ServiceLinkableCapabilityHooks,
) -> Result<RuntimeValue> {
    ServiceLinkableContractPlan::new(ty, schema_records, value_plan)
        .and_then(|plan| plan.materialize(value, source_heap, destination_heap, scope, hooks))
        .map_err(|error| materialization_error(stage, error))
}

fn canonical_scope(
    value_plan: &BoundaryValuePlan,
    detached_owner: BoundaryValueOwner,
    detached_lifetime: BoundaryValueLifetime,
) -> Result<ServiceLinkableMaterializationScope> {
    match value_plan {
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            owner,
            lifetime,
            ..
        } if *owner == detached_owner && *lifetime == detached_lifetime => {
            Ok(ServiceLinkableMaterializationScope {
                owner: *owner,
                lifetime: *lifetime,
            })
        }
        BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::CallbackCapability,
            owner,
            lifetime,
            ..
        } if *owner == BoundaryValueOwner::CapabilityOwner
            && matches!(
                lifetime,
                BoundaryValueLifetime::Request | BoundaryValueLifetime::Stream
            ) => Ok(ServiceLinkableMaterializationScope {
            owner: *owner,
            lifetime: *lifetime,
        }),
        BoundaryValuePlan::Unsupported { .. } | BoundaryValuePlan::Linkable { .. } => {
            Err(RuntimeError::InvalidArtifact(format!(
                "canonical boundary value plan does not match {detached_owner:?}/{detached_lifetime:?} lane ownership"
            )))
        }
    }
}

fn materialization_error(stage: &str, error: ServiceLinkableMaterializationError) -> RuntimeError {
    RuntimeError::InvalidArtifact(format!(
        "canonical in-process {stage} materialization failed: {error}"
    ))
}

#[derive(Debug)]
struct ProviderStreamLifetime {
    _lease: RequestStreamLease,
}

impl StreamLifetimeGuardApi for ProviderStreamLifetime {}

struct BoundaryStreamSink {
    inner: StreamSink,
    item_type: ContractTypeRef,
    item_value_plan: BoundaryValuePlan,
    schema_records: crate::AdmittedPackageSchemaRecords,
    execution_item_type: Option<RuntimeTypePlan>,
    provider_context: Arc<OwnedProgramExecutionContext>,
}

pub(crate) fn is_canonical_boundary_stream_sink(sink: &StreamSink) -> bool {
    sink.downcast_ref::<BoundaryStreamSink>().is_some()
}

impl BoundaryStreamSink {
    fn materialize_item(&self, item: Value) -> StreamRuntimeResult<Value> {
        let mut source_heap = RequestHeap::default();
        let source = runtime_from_wire_required_plan(
            &item,
            self.execution_item_type.as_ref(),
            "provider stream emit item",
            &mut source_heap,
        )
        .map_err(StreamRuntimeError::producer)?;
        let mut receiver_heap = RequestHeap::default();
        let provider_context = self.provider_context.borrow();
        let hooks = CallbackNativeCapabilityHooks::new(&provider_context);
        let materialized = materialize_value(
            "stream item",
            &self.item_type,
            self.schema_records.as_ref(),
            &self.item_value_plan,
            &source,
            &source_heap,
            &mut receiver_heap,
            canonical_scope(
                &self.item_value_plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .map_err(StreamRuntimeError::producer)?,
            &hooks,
        )
        .map_err(StreamRuntimeError::producer)?;
        runtime_to_wire(&materialized, &receiver_heap).map_err(StreamRuntimeError::producer)
    }

    fn project_internal_item(
        &self,
        item: RuntimeValue,
        source_heap: &RequestHeap,
    ) -> StreamRuntimeResult<Option<StreamInternalItem>> {
        if !matches!(
            self.item_value_plan,
            BoundaryValuePlan::Linkable {
                carrier: BoundaryValueCarrier::CallbackCapability,
                ..
            }
        ) {
            return Ok(None);
        }
        let provider_context = self.provider_context.borrow();
        let mut receiver_heap = provider_context.request_heap();
        let hooks = CallbackNativeCapabilityHooks::new(&provider_context);
        let materialized = materialize_value(
            "stream item",
            &self.item_type,
            self.schema_records.as_ref(),
            &self.item_value_plan,
            &item,
            source_heap,
            &mut receiver_heap,
            canonical_scope(
                &self.item_value_plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .map_err(StreamRuntimeError::producer)?,
            &hooks,
        )
        .map_err(StreamRuntimeError::producer)?;
        Ok(Some(StreamInternalItem::new(materialized, receiver_heap)))
    }
}

impl fmt::Debug for BoundaryStreamSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BoundaryStreamSink")
    }
}

impl StreamSinkApi for BoundaryStreamSink {
    fn project_runtime_item(
        &self,
        item: RuntimeValue,
        source_heap: &RequestHeap,
    ) -> StreamRuntimeResult<Option<StreamInternalItem>> {
        self.project_internal_item(item, source_heap)
    }

    fn send_internal_with_cancellation<'a>(
        &'a self,
        item: StreamInternalItem,
        signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.inner
                .send_internal_with_cancellation(item, signals, cancel_tokens)
                .await
        })
    }

    fn send<'a>(
        &'a self,
        item: Value,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move { self.inner.send(self.materialize_item(item)?).await })
    }

    fn send_with_cancel<'a>(
        &'a self,
        item: Value,
        cancel_flags: &'a [Arc<std::sync::atomic::AtomicBool>],
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.inner
                .send_with_cancel(self.materialize_item(item)?, cancel_flags)
                .await
        })
    }

    fn send_with_cancellation<'a>(
        &'a self,
        item: Value,
        signals: &'a [StreamCancelSignal],
        cancel_tokens: Vec<CancellationToken>,
    ) -> Pin<Box<dyn Future<Output = StreamRuntimeResult<()>> + Send + 'a>> {
        Box::pin(async move {
            self.inner
                .send_with_cancellation(self.materialize_item(item)?, signals, cancel_tokens)
                .await
        })
    }

    fn end<'a>(&'a self) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.inner.end().await })
    }

    fn fail<'a>(
        &'a self,
        error: StreamRuntimeError,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move { self.inner.fail(error).await })
    }

    fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    fn is_same_stream(&self, other: &StreamSink) -> bool {
        other
            .downcast_ref::<Self>()
            .map(|other| self.inner.is_same_stream(&other.inner))
            .unwrap_or_else(|| self.inner.is_same_stream(other))
    }

    fn cancel_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.inner.cancel_flag()
    }

    fn cancel_signal(&self) -> StreamCancelSignal {
        self.inner.cancel_signal()
    }
}

struct ProviderStreamTaskGuard;

impl ProviderStreamTaskGuard {
    fn new() -> Self {
        PROVIDER_STREAM_TASKS_ACTIVE.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ProviderStreamTaskGuard {
    fn drop(&mut self) {
        PROVIDER_STREAM_TASKS_ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use skiff_artifact_model::{
        ActivationPolicy, AssemblyIdentity, DeploymentArtifactIdentity, DeploymentPolicy,
        DeploymentRevision, PackageBuildId, PackageSchemaCanonicalDescriptor, PackageSchemaTypeId,
        PackageSchemaTypeRecord, ResourcePolicy, ServiceDeploymentRef,
    };
    use skiff_runtime_activation::{
        ActivationContext, ActivationIdentity, ActivationOwnedBindings, RequestActivationContext,
    };
    use skiff_runtime_boundary::service_linkable::FailClosedServiceLinkableCapabilityHooks;
    use skiff_runtime_model::{
        runtime_value::{HeapNode, RuntimeObject, RuntimeObjectFields},
        type_plan::RuntimeTypeNode,
    };

    use crate::runtime_ops::runtime_to_wire_required_plan;

    use super::*;

    #[test]
    fn in_process_stream_spawn_matrix_is_exhaustive() {
        let variants = BTreeSet::from([
            AsyncStreamSpawn::ProviderUnary,
            AsyncStreamSpawn::ProviderStreamProducer,
        ]);
        assert_eq!(variants.len(), 2);
    }

    #[tokio::test]
    async fn in_process_stream_cooperative_cancel_wakes_pending_provider_unary() {
        let cancellation = CancellationToken::new();
        let waiter = tokio::spawn({
            let cancellation = cancellation.clone();
            async move {
                await_provider_unary(
                    "operation:pending",
                    &BoundaryCancellationContract::Cooperative,
                    &cancellation,
                    std::future::pending(),
                )
                .await
            }
        });

        cancellation.cancel();

        let error = tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("cooperative cancellation should wake the pending provider")
            .expect("provider waiter should not panic")
            .expect_err("pending provider should terminate as cancelled");
        assert!(error.is_cancelled());
    }

    #[tokio::test]
    async fn in_process_stream_not_cancellable_waits_for_provider_terminal() {
        let cancellation = CancellationToken::new();
        let (complete, completed) = tokio::sync::oneshot::channel();
        let waiter = tokio::spawn({
            let cancellation = cancellation.clone();
            async move {
                await_provider_unary(
                    "operation:not-cancellable",
                    &BoundaryCancellationContract::NotCancellable,
                    &cancellation,
                    async move {
                        completed
                            .await
                            .expect("completion sender should remain alive");
                        Ok(RuntimeValue::Bool(true))
                    },
                )
                .await
            }
        });

        cancellation.cancel();
        tokio::task::yield_now().await;
        assert!(
            !waiter.is_finished(),
            "NotCancellable must not install the cooperative cancellation select"
        );
        complete.send(()).unwrap();
        assert_eq!(waiter.await.unwrap().unwrap(), RuntimeValue::Bool(true));
    }

    #[test]
    fn in_process_stream_task_counter_returns_to_baseline_exactly_once() {
        let baseline = PROVIDER_STREAM_TASKS_ACTIVE.load(Ordering::Acquire);
        let guard = ProviderStreamTaskGuard::new();
        assert_eq!(
            PROVIDER_STREAM_TASKS_ACTIVE.load(Ordering::Acquire),
            baseline + 1
        );
        drop(guard);
        assert_eq!(
            PROVIDER_STREAM_TASKS_ACTIVE.load(Ordering::Acquire),
            baseline
        );
    }

    #[tokio::test]
    async fn activation_context_across_suspend_keeps_explicit_provider_then_restores_receiver() {
        let receiver = activation("receiver", "receiver-build");
        let provider = activation("provider", "provider-build");
        let receiver_request = RequestActivationContext::begin(Arc::clone(&receiver)).unwrap();
        let provider_request = receiver_request.switch_to(Arc::clone(&provider)).unwrap();
        let generation = receiver_request.generation();

        let resumed = tokio::spawn(async move {
            assert!(Arc::ptr_eq(provider_request.current(), &provider));
            tokio::task::yield_now().await;
            assert!(Arc::ptr_eq(provider_request.current(), &provider));
            provider_request.restore_receiver()
        })
        .await
        .expect("owned provider continuation should not panic");

        assert!(Arc::ptr_eq(resumed.current(), &receiver));
        assert_eq!(resumed.generation(), generation);
    }

    #[test]
    fn in_process_stream_items_use_canonical_detached_plan() {
        let ty = ContractTypeRef::Record {
            fields: BTreeMap::from([
                (
                    "first".to_string(),
                    ContractTypeRef::Builtin {
                        name: "Array".to_string(),
                        arguments: vec![ContractTypeRef::builtin("string")],
                    },
                ),
                (
                    "second".to_string(),
                    ContractTypeRef::Builtin {
                        name: "Array".to_string(),
                        arguments: vec![ContractTypeRef::builtin("string")],
                    },
                ),
            ]),
        };
        let plan = BoundaryValuePlan::Linkable {
            carrier: skiff_artifact_model::BoundaryValueCarrier::DetachedValueGraph,
            encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
            owner: BoundaryValueOwner::Provider,
            lifetime: BoundaryValueLifetime::Stream,
        };
        let mut provider_heap = RequestHeap::default();
        let shared = provider_heap
            .alloc_array(vec![RuntimeValue::String("provider".to_string())])
            .unwrap();
        let root = provider_heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::from([
                ("first".to_string(), RuntimeValue::Heap(shared)),
                ("second".to_string(), RuntimeValue::Heap(shared)),
            ])))
            .unwrap();
        let mut receiver_heap = RequestHeap::default();

        let item = materialize_value(
            "stream item",
            &ty,
            &BTreeMap::new(),
            &plan,
            &RuntimeValue::Heap(root),
            &provider_heap,
            &mut receiver_heap,
            canonical_scope(
                &plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .unwrap(),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .unwrap();
        let RuntimeValue::Heap(root) = item else {
            panic!("materialized stream item should remain a record")
        };
        let HeapNode::Object(object) = receiver_heap.get(root).unwrap() else {
            panic!("materialized stream item should remain an object")
        };
        let RuntimeValue::Heap(first) = object.fields()["first"] else {
            panic!("first stream field should be an array")
        };
        let RuntimeValue::Heap(second) = object.fields()["second"] else {
            panic!("second stream field should be an array")
        };
        assert_ne!(first, second, "stream item aliases must be detached");
        receiver_heap
            .set_array_item(first, 0, RuntimeValue::String("receiver".to_string()))
            .unwrap();
        let HeapNode::Array(provider_items) = provider_heap.get(shared).unwrap() else {
            panic!("provider array should remain allocated")
        };
        assert_eq!(
            provider_items,
            &[RuntimeValue::String("provider".to_string())],
            "consumer mutation must not reach the provider heap"
        );
    }

    #[test]
    fn in_process_stream_typed_emit_decodes_before_canonical_plan() {
        let execution_plan =
            RuntimeTypePlan::synthetic_named_builtin("Date", RuntimeTypeNode::Date, Vec::new());
        let mut provider_heap = RequestHeap::default();
        let wire = runtime_to_wire_required_plan(
            &RuntimeValue::Date(1_234),
            Some(&execution_plan),
            "provider stream emit item",
            &mut provider_heap,
        )
        .unwrap();
        let mut decoded_heap = RequestHeap::default();
        let decoded = runtime_from_wire_required_plan(
            &wire,
            Some(&execution_plan),
            "provider stream emit item",
            &mut decoded_heap,
        )
        .unwrap();
        let plan = BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
            owner: BoundaryValueOwner::Provider,
            lifetime: BoundaryValueLifetime::Stream,
        };
        let mut receiver_heap = RequestHeap::default();

        let materialized = materialize_value(
            "stream item",
            &ContractTypeRef::builtin("Date"),
            &BTreeMap::new(),
            &plan,
            &decoded,
            &decoded_heap,
            &mut receiver_heap,
            canonical_scope(
                &plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .unwrap(),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .unwrap();

        assert_eq!(materialized, RuntimeValue::Date(1_234));
    }

    #[test]
    fn in_process_stream_named_item_uses_admitted_package_record() {
        let type_id = PackageSchemaTypeId::new("schema:stream-item");
        let ty =
            ContractTypeRef::package_schema("example.stream", "api.StreamItem", type_id.clone());
        let schema = BTreeMap::from([(
            type_id.clone(),
            Arc::new(PackageSchemaTypeRecord {
                package_id: "example.stream".to_string(),
                stable_schema_key: "api.StreamItem".to_string(),
                package_schema_type_id: type_id,
                canonical_descriptor: PackageSchemaCanonicalDescriptor {
                    type_params: Vec::new(),
                    descriptor: skiff_artifact_model::ContractTypeDescriptor::Record {
                        fields: BTreeMap::new(),
                    },
                },
            }),
        )]);
        let plan = BoundaryValuePlan::Linkable {
            carrier: BoundaryValueCarrier::DetachedValueGraph,
            encoding: skiff_artifact_model::BoundaryValueEncoding::CanonicalValue,
            owner: BoundaryValueOwner::Provider,
            lifetime: BoundaryValueLifetime::Stream,
        };
        let mut provider_heap = RequestHeap::default();
        let source = provider_heap
            .alloc_object(RuntimeObject::unshaped(RuntimeObjectFields::new()))
            .unwrap();
        let mut receiver_heap = RequestHeap::default();

        materialize_value(
            "stream item",
            &ty,
            &schema,
            &plan,
            &RuntimeValue::Heap(source),
            &provider_heap,
            &mut receiver_heap,
            canonical_scope(
                &plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .unwrap(),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .expect("admitted Package stream item should materialize");

        assert!(materialize_value(
            "stream item",
            &ty,
            &BTreeMap::new(),
            &plan,
            &RuntimeValue::Heap(source),
            &provider_heap,
            &mut receiver_heap,
            canonical_scope(
                &plan,
                BoundaryValueOwner::Provider,
                BoundaryValueLifetime::Stream,
            )
            .unwrap(),
            &FailClosedServiceLinkableCapabilityHooks,
        )
        .is_err());
    }

    fn activation(service: &str, package_build: &str) -> Arc<ActivationContext> {
        ActivationContext::new(
            ActivationIdentity {
                assembly_identity: AssemblyIdentity::new("assembly:async-stream"),
                assembly_generation: 7,
                runtime_replica_id: "replica:async-stream".to_string(),
                deployment: ServiceDeploymentRef {
                    service_id: service.to_string(),
                    contract_version: "1.0.0".to_string(),
                    deployment_revision: DeploymentRevision::new(format!("{service}-r1")),
                    deployment_artifact_identity: DeploymentArtifactIdentity::new(format!(
                        "deployment:{service}"
                    )),
                },
            },
            PackageBuildId::new(package_build),
            ActivationOwnedBindings {
                config_literals: Vec::new(),
                secret_refs: Vec::new(),
                state_bindings: Vec::new(),
                resource_bindings: Vec::new(),
                policy: DeploymentPolicy {
                    timeout_ms: Some(1_000),
                    resources: ResourcePolicy {
                        cpu_millis: 100,
                        memory_bytes: 1_024,
                    },
                    activation: ActivationPolicy {
                        max_concurrency: 1,
                        idle_timeout_ms: None,
                    },
                    principal: "test".to_string(),
                },
            },
            Vec::new(),
        )
        .unwrap()
    }
}
