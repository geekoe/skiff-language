use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_recursion::async_recursion;
use serde_json::Value;
use skiff_runtime_boundary::stream::stream_id;
use skiff_runtime_boundary::type_descriptor::bare_type_name;
use skiff_runtime_capability_context::{
    StreamConsumptionTerminal, StreamRuntimeError, StreamRuntimeResult,
    SupervisedStreamConsumptionChild, SupervisedStreamConsumptionLease,
};
use skiff_runtime_linked_program::{
    CallIr, ConstAddr, ExecutableAddr, LinkedCallTarget, LinkedExecutable, LinkedExprIr,
    LinkedFileUnit, LinkedStmtIr, LinkedTypeRef, ReceiverCallAbi,
};
use skiff_runtime_model::{
    request_heap::{
        deep_clone_runtime_value_between_heaps, deep_clone_runtime_value_carrier_between_heaps,
        RequestHeap,
    },
    runtime_value::{RuntimeValue, RuntimeValueCarrier},
    type_plan::RuntimeTypePlan,
};

use super::type_descriptor::TypeSubstitutions;
use super::{
    capabilities::{StreamCancelSignal, StreamPoll, StreamRuntime, StreamSink, TypedStreamSink},
    env::{Env, Flow},
    program_execution::{OwnedProgramExecutionContext, ProgramExecutionContext},
    program_ir::{program_call_target_kind, program_expression_ref},
    runtime_ops::{
        runtime_carrier_for_plan, runtime_carrier_from_wire_required_plan, runtime_from_wire,
    },
    Interpreter,
};
use crate::{
    assembly_execution::{
        service_error_channel::{CanonicalServiceErrorChannel, ServiceErrorImportContext},
        RuntimeExecutionProjection,
    },
    capabilities::StreamConsumerCleanup,
    error::{
        materialize_stream_runtime_error, stream_runtime_error_from_eval,
        RequestHeapOwnedStreamError, Result, RuntimeError,
    },
    test_effect_registry::TestEffectTarget,
    type_projection::EvalTypeProjection,
};

mod current_scope;

#[cfg(test)]
#[path = "program_stream/current_scope_tests.rs"]
mod current_scope_tests;
#[cfg(test)]
#[path = "program_stream/supervised_executable_tests.rs"]
mod supervised_executable_tests;

impl Interpreter {
    #[allow(clippy::too_many_arguments)]
    pub async fn exec_program_stream_for_in(
        &self,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        item_slot: usize,
        body: &str,
        stream_value: Value,
        item_type: Option<RuntimeTypePlan>,
        cancel_signals: &[StreamCancelSignal],
    ) -> Result<Flow> {
        let stream_runtime = context.stream_runtime();
        let supervision = env.stream_consumer_supervision_for(&stream_value);
        let mut cleanup = match &supervision {
            Some(supervision) => supervision.consumer_cleanup(&stream_value),
            None => StreamConsumerCleanup::new(stream_runtime.clone(), &stream_value),
        };
        loop {
            let item = current_scope::next_with_actor(
                &context,
                heap,
                &stream_runtime,
                &stream_value,
                cancel_signals,
                1,
            )
            .await?;
            let item = match item {
                Ok(item) => item,
                Err(error) => {
                    if matches!(&error, StreamRuntimeError::Producer(_)) {
                        if let Some(supervision) = &supervision {
                            supervision.observe_producer_error(&stream_value);
                        }
                    }
                    return Err(materialize_consumed_stream_error(
                        self, &context, error, heap,
                    )?);
                }
            };
            let item_value = match materialize_runtime_stream_item(item, item_type.as_ref(), heap)?
            {
                Some(item) => item,
                None => {
                    cleanup.reached_end();
                    return Ok(Flow::Continue);
                }
            };
            let flow = self
                .exec_program_for_in_body_carrier(
                    context.clone(),
                    heap,
                    env,
                    addr,
                    file,
                    executable,
                    item_slot,
                    body,
                    item_value,
                )
                .await;
            let flow = match flow {
                Ok(flow) => flow,
                Err(error) => return Err(error),
            };
            match flow {
                Flow::Continue | Flow::LoopContinue => continue,
                Flow::Break => return Ok(Flow::Continue),
                Flow::Return(value) => return Ok(Flow::Return(value)),
                Flow::Parked => return Ok(Flow::Parked),
                Flow::ContinueConsumer => return Ok(Flow::ContinueConsumer),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn exec_program_stream_producer_for_in(
        &self,
        program: RuntimeExecutionProjection<'_>,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        item_slot: usize,
        body: &str,
        producer: StreamProducerCall,
    ) -> Result<Flow> {
        let prepared = self
            .prepare_stream_producer(
                program,
                context.clone(),
                heap,
                env,
                addr,
                file,
                executable,
                producer,
            )
            .await?;
        let stream_value = prepared.stream_value.clone();
        let cancel_signal = prepared.cancel_signal.clone();
        let item_type = prepared.item_type.clone();
        let owned_context = Arc::new(OwnedProgramExecutionContext::capture(&context));
        spawn_stream_producer(self, owned_context, addr.clone(), prepared);

        let consumer_result = self
            .exec_program_stream_for_in(
                context,
                heap,
                env,
                addr,
                file,
                executable,
                item_slot,
                body,
                stream_value.clone(),
                Some(item_type),
                std::slice::from_ref(&cancel_signal),
            )
            .await;
        consumer_result
    }

    /// Prepares a stream-producer call whose result is bound to a value rather
    /// than consumed inline (e.g. `const s = producer(...)`). The producer is
    /// parked in the deferred registry keyed by the stream id it feeds, and the
    /// returned `RuntimeValue` is the stream the caller can iterate later. The
    /// parked producer is driven concurrently the first time that stream value
    /// is consumed by a `for-in` (see `exec_program_stream_for_in`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn prepare_deferred_stream_producer(
        &self,
        program: RuntimeExecutionProjection<'_>,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        producer: StreamProducerCall,
    ) -> Result<RuntimeValue> {
        let prepared = self
            .prepare_stream_producer(
                program, context, heap, env, addr, file, executable, producer,
            )
            .await?;
        let id = deferred_stream_id(&prepared)?;
        // Hand the consumer a stream value backed by the parked producer's
        // channel, expressed in the caller's heap.
        let stream_value = match runtime_from_wire(&prepared.stream_value, heap) {
            Ok(value) => value,
            Err(error) => {
                prepared.cancel();
                return Err(error);
            }
        };
        self.deferred_stream_producers.insert(id, prepared);
        Ok(stream_value)
    }

    /// Prepares an already-evaluated explicit-self stream producer call. Dynamic
    /// dispatch forms such as local `any I` method calls only discover the
    /// executable after receiver dispatch, so the expression-level stream
    /// resolver cannot catch them before argument evaluation.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn prepare_deferred_stream_producer_from_values(
        &self,
        program: RuntimeExecutionProjection<'_>,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &Env,
        caller_addr: &ExecutableAddr,
        producer_addr: &ExecutableAddr,
        producer_executable: &LinkedExecutable,
        producer_type_args: &BTreeMap<String, LinkedTypeRef>,
        producer_self: RuntimeValueCarrier,
        producer_args: Vec<RuntimeValueCarrier>,
    ) -> Result<Option<RuntimeValue>> {
        if !executable_body_contains_emit(producer_executable) {
            return Ok(None);
        }
        let type_projection = EvalTypeProjection::from_execution_projection(program.clone());
        let Some(item_type) = stream_item_plan_from_return_type(
            &type_projection,
            caller_addr,
            producer_addr,
            producer_executable,
            producer_type_args,
            &env.type_substitutions,
        )?
        else {
            return Ok(None);
        };

        let mut producer_heap = context.request_heap();
        let producer_self = deep_clone_runtime_value_carrier_between_heaps(
            heap,
            &mut producer_heap,
            &producer_self,
        )?;
        let mut cloned_args = Vec::with_capacity(producer_args.len());
        for arg in &producer_args {
            cloned_args.push(deep_clone_runtime_value_carrier_between_heaps(
                heap,
                &mut producer_heap,
                arg,
            )?);
        }

        let mut producer_env = env.clone();
        let stream_runtime = context.stream_runtime();
        let (stream_value, sink) = stream_runtime.channel_stream();
        let cancel_signal = sink.cancel_signal();
        producer_env.stream_sink = Some(sink.clone());
        producer_env.current_stream_item_type = Some(item_type.clone());
        let prepared = StreamProducerExecution {
            stream_runtime,
            stream_value,
            cancel_signal,
            item_type,
            arg_producers: Vec::new(),
            producer_heap,
            producer_env,
            producer_addr: producer_addr.clone(),
            // The already-evaluated explicit-self path receives a context that
            // already contains the exact required call site.
            producer_site: None,
            producer_self: Some(producer_self),
            producer_type_args: producer_type_args.clone(),
            producer_args: cloned_args,
            sink,
        };

        let id = deferred_stream_id(&prepared)?;
        let stream_value = match runtime_from_wire(&prepared.stream_value, heap) {
            Ok(value) => value,
            Err(error) => {
                prepared.cancel();
                return Err(error);
            }
        };
        self.deferred_stream_producers.insert(id, prepared);
        Ok(Some(stream_value))
    }

    /// Takes the deferred producer registered for `stream_value` (if any) and
    /// runs it concurrently with `consumer`, mirroring how
    /// `exec_program_stream_producer_for_in` co-drives an inline producer. When
    /// no producer is parked for the stream this simply awaits the consumer.
    pub async fn drive_deferred_stream_producer<'fut, T, Fut>(
        &self,
        context: ProgramExecutionContext<'_>,
        addr: &ExecutableAddr,
        stream_value: &Value,
        consumer: Fut,
    ) -> Result<T>
    where
        Fut: std::future::Future<Output = Result<T>> + 'fut,
    {
        let Some(prepared) =
            stream_id(stream_value).and_then(|id| self.deferred_stream_producers.take(id))
        else {
            return consumer.await;
        };
        // The producer now runs on its own spawned task rather than being
        // co-driven with the consumer, so the consumer future no longer compounds
        // producer-stack frames and the previous `Box::pin` mitigation is no
        // longer required.
        self.exec_prepared_native_stream_producer_arg(
            context,
            addr,
            PreparedNativeStreamProducer::new(prepared),
            consumer,
        )
        .await
    }

    pub(crate) fn attach_deferred_http_response_sink(
        &self,
        stream_value: &Value,
        item_type: RuntimeTypePlan,
        request_generation: Option<u64>,
    ) -> Result<()> {
        let id = stream_id(stream_value).ok_or_else(|| {
            RuntimeError::Decode(
                "raw HTTP response stream is not a canonical Stream value".to_string(),
            )
        })?;
        self.deferred_stream_producers.attach_response_sink(
            id,
            stream_value,
            item_type,
            request_generation,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn prepare_native_stream_producer_arg(
        &self,
        program: RuntimeExecutionProjection<'_>,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        producer: StreamProducerCall,
    ) -> Result<PreparedNativeStreamProducer> {
        self.prepare_stream_producer(
            program, context, heap, env, addr, file, executable, producer,
        )
        .await
        .map(PreparedNativeStreamProducer::new)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn exec_prepared_native_stream_producer_arg<T, Fut>(
        &self,
        context: ProgramExecutionContext<'_>,
        addr: &ExecutableAddr,
        prepared: PreparedNativeStreamProducer,
        consumer: Fut,
    ) -> Result<T>
    where
        Fut: std::future::Future<Output = Result<T>>,
    {
        let PreparedNativeStreamProducer {
            producer,
            consumption,
        } = prepared;
        let stream_runtime = producer.stream_runtime.clone();
        let stream_value = producer.stream_value.clone();
        let cancel_signal = producer.cancel_signal.clone();
        let owned_context = Arc::new(OwnedProgramExecutionContext::capture(&context));
        spawn_stream_producer(self, owned_context, addr.clone(), producer);

        tokio::pin!(consumer);
        let consumer_result = consumer.await;
        match consumer_result {
            Ok(value) => {
                if consumption.status().stream_mismatch() {
                    consumption.hard_cancel();
                    return Err(prepared_stream_consumer_mismatch_error());
                }
                consumption.complete_success();
                Ok(value)
            }
            Err(error) if error.is_cancelled() => {
                consumption.hard_cancel();
                Err(error)
            }
            Err(error) => {
                let status = consumption.status();
                if status.stream_mismatch() {
                    consumption.hard_cancel();
                    return Err(prepared_stream_consumer_mismatch_error());
                }
                if status.terminal() != StreamConsumptionTerminal::Open {
                    // The native child already consumed the typed terminal. Its
                    // error (or a post-End commit error) is the authoritative
                    // consumer result, and a second registry lookup would lose
                    // that information.
                    consumption.complete_terminal();
                    return Err(error);
                }
                // The consumer errored on its own (not via cancellation). Drain
                // the producer's pending output so a trailing producer error is
                // surfaced (preferred over the consumer error).
                let drain_result = self
                    .drain_stream_producer_output(
                        context,
                        &stream_runtime,
                        &stream_value,
                        &cancel_signal,
                        &consumption,
                    )
                    .await;
                if consumption.status().terminal() == StreamConsumptionTerminal::Open {
                    consumption.hard_cancel();
                } else {
                    consumption.complete_terminal();
                }
                Err(prepared_stream_error_after_drain(error, drain_result))
            }
        }
    }

    pub fn cancel_prepared_native_stream_producer_arg(
        &self,
        prepared: &PreparedNativeStreamProducer,
    ) {
        prepared.consumption.hard_cancel();
    }

    async fn drain_stream_producer_output(
        &self,
        context: ProgramExecutionContext<'_>,
        stream_runtime: &StreamRuntime,
        stream_value: &Value,
        cancel_signal: &StreamCancelSignal,
        consumption: &SupervisedStreamConsumptionLease,
    ) -> StreamRuntimeResult<()> {
        loop {
            let item = current_scope::next(
                &context,
                stream_runtime,
                stream_value,
                std::slice::from_ref(cancel_signal),
                0,
            )
            .await
            .map_err(stream_runtime_error_from_eval)?;
            match item {
                Ok(StreamPoll::Item(_) | StreamPoll::InternalItem(_)) => continue,
                Ok(StreamPoll::End) => {
                    consumption.observe_end();
                    return Ok(());
                }
                Err(error) => {
                    if matches!(&error, StreamRuntimeError::Producer(_)) {
                        consumption.observe_producer_error();
                    }
                    return Err(error);
                }
            }
        }
    }

    #[async_recursion]
    #[allow(clippy::too_many_arguments)]
    async fn prepare_stream_producer(
        &self,
        program: RuntimeExecutionProjection<'async_recursion>,
        context: ProgramExecutionContext<'async_recursion>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        _file: &LinkedFileUnit,
        _executable: &LinkedExecutable,
        producer: StreamProducerCall,
    ) -> Result<StreamProducerExecution> {
        let receiver = match (
            producer.receiver_const.as_ref(),
            producer.producer_self.as_ref(),
        ) {
            (Some(const_addr), None) => Some(
                self.eval_program_const_addr(context.clone(), heap, env, const_addr)
                    .await?,
            ),
            (None, Some(receiver)) => Some(receiver.clone()),
            (None, None) => None,
            (Some(_), Some(_)) => return Err(RuntimeError::InvalidArtifact(
                "stream producer call cannot have both const receiver and dynamic receiver self"
                    .to_string(),
            )),
        };
        let mut producer_heap = context.request_heap();
        let mut arg_producers = PreparedStreamProducerArgs::default();
        let mut args = Vec::with_capacity(producer.call.args.len());
        for arg in &producer.call.args {
            let expr = program_expression_ref(_executable, *arg)?;
            let arg_producer = self.resolve_stream_producer_call(
                program.clone(),
                addr,
                heap,
                env,
                _executable,
                expr,
            )?;
            if let Some(arg_producer) = arg_producer {
                if !arg_producers.is_empty() {
                    return Err(RuntimeError::Unsupported(
                        "multiple stream-producing producer call arguments are not supported"
                            .to_string(),
                    ));
                }
                let nested = self
                    .prepare_stream_producer(
                        program.clone(),
                        context.clone(),
                        heap,
                        env,
                        addr,
                        _file,
                        _executable,
                        arg_producer,
                    )
                    .await?;
                let stream_value = match runtime_from_wire(&nested.stream_value, &mut producer_heap)
                {
                    Ok(value) => value,
                    Err(error) => {
                        nested.cancel();
                        return Err(error);
                    }
                };
                args.push(stream_value.into());
                arg_producers.push(nested);
            } else {
                let arg = self
                    .eval_program_expr_ref(
                        context.clone(),
                        heap,
                        env,
                        addr,
                        _file,
                        _executable,
                        *arg,
                    )
                    .await?;
                let arg =
                    deep_clone_runtime_value_carrier_between_heaps(heap, &mut producer_heap, &arg)?;
                args.push(arg);
            }
        }
        let producer_self = receiver
            .as_ref()
            .map(|receiver| {
                deep_clone_runtime_value_carrier_between_heaps(heap, &mut producer_heap, receiver)
            })
            .transpose()?;
        let mut producer_env = env.clone();
        let stream_runtime = context.stream_runtime();
        let (stream_value, sink) = stream_runtime.channel_stream();
        let cancel_signal = sink.cancel_signal();
        producer_env.stream_sink = Some(sink.clone());
        producer_env.current_stream_item_type = Some(producer.item_type.clone());
        Ok(StreamProducerExecution {
            stream_runtime,
            stream_value,
            cancel_signal,
            item_type: producer.item_type,
            arg_producers: arg_producers.into_producers(),
            producer_heap,
            producer_env,
            producer_addr: producer.addr,
            producer_site: Some(producer.call.site),
            producer_self,
            producer_type_args: producer.call.type_args,
            producer_args: args,
            sink,
        })
    }

    pub(crate) fn resolve_stream_producer_call(
        &self,
        program: RuntimeExecutionProjection<'_>,
        current_addr: &ExecutableAddr,
        heap: &RequestHeap,
        env: &Env,
        executable: &LinkedExecutable,
        expr: &LinkedExprIr,
    ) -> Result<Option<StreamProducerCall>> {
        let LinkedExprIr::Call { call } = expr else {
            return Ok(None);
        };
        self.resolve_stream_producer_from_call(program, current_addr, heap, env, executable, call)
    }

    pub(crate) fn resolve_stream_producer_from_call(
        &self,
        program: RuntimeExecutionProjection<'_>,
        current_addr: &ExecutableAddr,
        _heap: &RequestHeap,
        env: &Env,
        _executable: &LinkedExecutable,
        call: &CallIr,
    ) -> Result<Option<StreamProducerCall>> {
        let type_projection = EvalTypeProjection::from_execution_projection(program.clone());
        let (addr, receiver_const, producer_self, call) = match &call.target {
            LinkedCallTarget::Executable { addr } => (addr.clone(), None, None, call.clone()),
            LinkedCallTarget::PackageDirect { call: target } => {
                if self.test_effects_enabled {
                    let effect_target = TestEffectTarget::package_callable(
                        target.dependency_package_build_id().clone(),
                        target.package_callable_id().clone(),
                    );
                    if self.runtime_test_effects.contains_target(&effect_target) {
                        // Inline effects replace the package callable itself.
                        // Do not turn a real `emit` body into a deferred
                        // producer before ordinary call dispatch gets a chance
                        // to consume the registered stream outcome. Keeping an
                        // exhausted target in the registry also preserves the
                        // required sequence-exhaustion error instead of
                        // falling through to production code.
                        return Ok(None);
                    }
                }
                (
                    target.executable_addr().clone(),
                    target.receiver_const().cloned(),
                    None,
                    call.clone(),
                )
            }
            LinkedCallTarget::LocalExecutable { .. } | LinkedCallTarget::PackageSymbol { .. } => {
                return Err(RuntimeError::InvalidArtifact(format!(
                    "RuntimeProgram call target {} was not linked before execution",
                    program_call_target_kind(&call.target)
                )));
            }
            LinkedCallTarget::ServiceDependencySymbol { .. } => return Ok(None),
            LinkedCallTarget::LocalConstReceiverExecutable {
                const_addr,
                executable_addr,
                receiver_call_abi,
                ..
            } => match receiver_call_abi {
                ReceiverCallAbi::ExplicitSelfFirst => (
                    executable_addr.clone(),
                    Some(const_addr.clone()),
                    None,
                    call.clone(),
                ),
            },
            _ => return Ok(None),
        };
        let resolved = program.resolve_nested_executable(&addr)?;
        if !executable_body_contains_emit(resolved.executable) {
            return Ok(None);
        }
        let item_type = stream_item_plan_from_return_type(
            &type_projection,
            current_addr,
            &addr,
            resolved.executable,
            &call.type_args,
            &env.type_substitutions,
        )?;
        let Some(item_type) = item_type else {
            return Ok(None);
        };
        Ok(Some(StreamProducerCall {
            addr,
            receiver_const,
            producer_self,
            call,
            item_type,
        }))
    }
}

/// Moves either an in-process producer item or an external wire item into the consumer heap
/// under one exact item plan. Boundary consumers use this same transfer before applying their
/// protocol codec.
pub(crate) fn materialize_runtime_stream_item(
    item: StreamPoll,
    item_type: Option<&RuntimeTypePlan>,
    heap: &mut RequestHeap,
) -> Result<Option<RuntimeValueCarrier>> {
    match item {
        StreamPoll::InternalItem(item) => {
            let (value, source_heap) = item.into_parts();
            let local_carrier = match &value {
                RuntimeValue::Heap(handle) => source_heap.local_carrier_cell(*handle)?,
                _ => None,
            };
            let carrier = if let Some(carrier) = local_carrier {
                deep_clone_runtime_value_carrier_between_heaps(&source_heap, heap, &carrier)?
            } else {
                deep_clone_runtime_value_between_heaps(&source_heap, heap, &value)?.into()
            };
            match item_type {
                Some(plan) => {
                    runtime_carrier_for_plan(carrier, plan, "stream item", heap).map(Some)
                }
                None => Ok(Some(carrier)),
            }
        }
        StreamPoll::Item(item) => {
            let carrier = match item_type {
                Some(item_type) => runtime_carrier_from_wire_required_plan(
                    &item,
                    Some(item_type),
                    "stream item",
                    heap,
                )?,
                None => runtime_from_wire(&item, heap)?.into(),
            };
            Ok(Some(carrier))
        }
        StreamPoll::End => Ok(None),
    }
}

fn prepared_stream_consumer_mismatch_error() -> RuntimeError {
    RuntimeError::Decode(
        "supervised stream consumer used a different Stream value than its prepared producer"
            .to_string(),
    )
}

fn prepared_stream_error_after_drain(
    consumer_error: RuntimeError,
    drain_result: StreamRuntimeResult<()>,
) -> RuntimeError {
    match drain_result {
        Ok(()) => consumer_error,
        Err(error) => match error.fixed_service_failure_parts() {
            Some((error, _)) => RuntimeError::FixedServiceFailure(error.clone()),
            None => RuntimeError::from(error),
        },
    }
}

fn materialize_consumed_stream_error(
    interpreter: &Interpreter,
    context: &ProgramExecutionContext<'_>,
    error: StreamRuntimeError,
    caller_heap: &mut RequestHeap,
) -> Result<RuntimeError> {
    let Some((fixed, import)) = error.fixed_service_failure_parts() else {
        return materialize_stream_runtime_error(error, caller_heap);
    };
    let fixed = fixed.clone();
    let Some((
        caller_package_build_id,
        caller_executable_addr,
        call_site,
        caller_stack_at_site,
        remote_service_id,
        remote_operation_id,
    )) = import
    else {
        return Ok(RuntimeError::FixedServiceFailure(fixed));
    };
    let target = context.runtime_assembly_target()?;
    let projection = RuntimeExecutionProjection::for_context(interpreter, context)?;
    let exception = CanonicalServiceErrorChannel::import_caller_failure(
        fixed,
        ServiceErrorImportContext {
            execution_image: target.execution_image().as_ref(),
            type_view: projection.type_view(),
            caller_heap,
            caller_package_build_id,
            caller_executable_addr,
            call_site,
            caller_stack_at_site,
            remote_service_id,
            remote_operation_id,
        },
    )?;
    Ok(RuntimeError::UserException(exception))
}

fn stream_item_plan_from_return_type(
    type_projection: &EvalTypeProjection<'_>,
    caller_addr: &ExecutableAddr,
    callee_addr: &ExecutableAddr,
    executable: &LinkedExecutable,
    type_args: &BTreeMap<String, LinkedTypeRef>,
    caller_substitutions: &TypeSubstitutions,
) -> Result<Option<RuntimeTypePlan>> {
    let Some(item_type_ref) = linked_stream_item_type(executable.return_type.as_ref()) else {
        return Ok(None);
    };

    if linked_type_ref_contains_type_param(item_type_ref) {
        let substitutions = type_projection.call_type_substitutions(
            caller_addr,
            caller_substitutions,
            executable,
            type_args,
        );
        return type_projection
            .plan_from_linked_nested_ref_with_substitutions(
                item_type_ref,
                callee_addr,
                &substitutions,
            )
            .map(Some);
    }

    type_projection
        .plan_from_linked_nested_ref(item_type_ref, callee_addr)
        .map(Some)
}

pub struct StreamProducerCall {
    pub addr: ExecutableAddr,
    pub receiver_const: Option<ConstAddr>,
    pub producer_self: Option<RuntimeValueCarrier>,
    pub call: CallIr,
    pub item_type: RuntimeTypePlan,
}

pub struct PreparedNativeStreamProducer {
    producer: StreamProducerExecution,
    consumption: SupervisedStreamConsumptionLease,
}

impl PreparedNativeStreamProducer {
    fn new(producer: StreamProducerExecution) -> Self {
        let consumption = SupervisedStreamConsumptionLease::new(
            producer.stream_runtime.clone(),
            &producer.stream_value,
        );
        Self {
            producer,
            consumption,
        }
    }

    pub fn stream_value(&self) -> &Value {
        &self.producer.stream_value
    }

    pub fn consumption_child(&self) -> SupervisedStreamConsumptionChild {
        self.consumption.child()
    }
}

pub struct StreamProducerExecution {
    stream_runtime: StreamRuntime,
    stream_value: Value,
    cancel_signal: StreamCancelSignal,
    item_type: RuntimeTypePlan,
    arg_producers: Vec<StreamProducerExecution>,
    producer_heap: RequestHeap,
    producer_env: Env,
    producer_addr: ExecutableAddr,
    producer_site: Option<skiff_artifact_model::InstructionSourceSite>,
    producer_self: Option<RuntimeValueCarrier>,
    producer_type_args: std::collections::BTreeMap<String, LinkedTypeRef>,
    producer_args: Vec<RuntimeValueCarrier>,
    sink: StreamSink,
}

impl StreamProducerExecution {
    fn cancel(&self) {
        self.stream_runtime.cancel(&self.stream_value);
    }
}

#[derive(Default)]
struct PreparedStreamProducerArgs {
    producers: Vec<StreamProducerExecution>,
}

impl PreparedStreamProducerArgs {
    fn is_empty(&self) -> bool {
        self.producers.is_empty()
    }

    fn push(&mut self, producer: StreamProducerExecution) {
        self.producers.push(producer);
    }

    fn into_producers(mut self) -> Vec<StreamProducerExecution> {
        std::mem::take(&mut self.producers)
    }
}

impl Drop for PreparedStreamProducerArgs {
    fn drop(&mut self) {
        for producer in &self.producers {
            producer.cancel();
        }
    }
}

fn deferred_stream_id(producer: &StreamProducerExecution) -> Result<String> {
    match stream_id(&producer.stream_value) {
        Some(id) => Ok(id.to_string()),
        None => {
            producer.cancel();
            Err(RuntimeError::Decode(
                "deferred stream producer was not assigned a stream id".to_string(),
            ))
        }
    }
}

/// Registry of stream producers whose result was bound to a value instead of
/// being consumed inline. Keyed by the stream id the producer feeds; entries are
/// removed and driven the first time that stream is consumed by a `for-in`.
#[derive(Clone, Default)]
pub struct DeferredStreamProducerRegistry {
    entries: Arc<Mutex<HashMap<String, StreamProducerExecution>>>,
}

impl DeferredStreamProducerRegistry {
    fn insert(&self, id: String, producer: StreamProducerExecution) {
        self.entries
            .lock()
            .expect("deferred stream producer registry poisoned")
            .insert(id, producer);
    }

    fn take(&self, id: &str) -> Option<StreamProducerExecution> {
        self.entries
            .lock()
            .expect("deferred stream producer registry poisoned")
            .remove(id)
    }

    fn attach_response_sink(
        &self,
        id: &str,
        stream_value: &Value,
        item_type: RuntimeTypePlan,
        request_generation: Option<u64>,
    ) -> Result<()> {
        let mut entries = self
            .entries
            .lock()
            .expect("deferred stream producer registry poisoned");
        let producer = entries.get_mut(id).ok_or_else(|| {
            RuntimeError::Decode(format!(
                "raw HTTP response stream {id} is not a parked deferred producer"
            ))
        })?;
        if producer.stream_value != *stream_value
            || producer.stream_runtime.request_scope_generation() != request_generation
        {
            return Err(RuntimeError::Decode(format!(
                "raw HTTP response stream {id} does not belong to the current request"
            )));
        }
        if producer.producer_env.response_stream_sink.is_some() {
            return Err(RuntimeError::Decode(format!(
                "raw HTTP response stream {id} already has a response sink"
            )));
        }
        producer.producer_env.response_stream_sink = Some(TypedStreamSink {
            sink: producer.sink.clone(),
            item_type,
        });
        Ok(())
    }
}

/// Spawns `producer` (and, recursively, its argument producers) onto the tokio
/// runtime as independent tasks. This is the root fix for the stream
/// stack-overflow: a producer body that consumes an inner producer used to drive
/// that inner producer synchronously on the *same* native stack (via
/// `#[async_recursion]` + `tokio::select!`), so N nested producers kept N futures
/// alive on one stack. By giving every producer its own scheduling context, the
/// consumer only ever polls the bounded channel (`StreamSink`/`next_with_cancel`)
/// and native stack depth stays constant regardless of nesting depth.
///
/// The spawned task owns a clone of the `Arc<OwnedProgramExecutionContext>` and
/// re-borrows a `ProgramExecutionContext<'_>` from it for the duration of the
/// call. Cancellation, backpressure, error->throw, and one-shot semantics are
/// all carried by the existing stream channel/cancel-signal machinery and are
/// unchanged by where the producer runs.
fn spawn_stream_producer(
    interpreter: &Interpreter,
    owned_context: Arc<OwnedProgramExecutionContext>,
    caller_addr: ExecutableAddr,
    producer: StreamProducerExecution,
) {
    let interpreter = interpreter.clone_for_stream_producer();
    tokio::spawn(async move {
        run_stream_producer_task(&interpreter, &owned_context, &caller_addr, producer).await;
    });
}

/// Body of a spawned stream-producer task. Spawns argument producers as their
/// own tasks first, then runs the producer call to completion, feeding the sink.
/// When the main call finishes it cancels any argument streams (mirroring the
/// old co-driven `select!`, which cancelled arg producers once the main producer
/// completed).
async fn run_stream_producer_task(
    interpreter: &Interpreter,
    owned_context: &Arc<OwnedProgramExecutionContext>,
    caller_addr: &ExecutableAddr,
    producer: StreamProducerExecution,
) {
    let StreamProducerExecution {
        arg_producers,
        mut producer_heap,
        producer_env,
        producer_addr,
        producer_site,
        producer_self,
        producer_type_args,
        producer_args,
        sink,
        ..
    } = producer;

    let arg_streams = arg_producers
        .iter()
        .map(|producer| {
            (
                producer.stream_runtime.clone(),
                producer.stream_value.clone(),
            )
        })
        .collect::<Vec<_>>();
    for arg_producer in arg_producers {
        spawn_stream_producer(
            interpreter,
            owned_context.clone(),
            caller_addr.clone(),
            arg_producer,
        );
    }

    let context = owned_context.borrow();
    let context = match producer_site {
        Some(site) => context.with_local_call_site(site),
        None => context,
    };
    let result = if let Some(producer_self) = producer_self {
        interpreter
            .call_program_executable_with_self_direct_carriers(
                context,
                &mut producer_heap,
                &producer_env,
                caller_addr,
                &producer_addr,
                &producer_type_args,
                producer_self,
                producer_args,
            )
            .await
    } else {
        interpreter
            .call_program_executable_carriers(
                context,
                &mut producer_heap,
                &producer_env,
                caller_addr,
                &producer_addr,
                &producer_type_args,
                producer_args,
            )
            .await
    };
    match result {
        Ok(_) => sink.end().await,
        Err(error) if error.is_cancelled() && sink.is_cancelled() => {}
        Err(error) => match RequestHeapOwnedStreamError::try_new(error, producer_heap) {
            Ok(error) => sink.fail(StreamRuntimeError::producer(error)).await,
            Err(error) => {
                debug_assert!(error.is_cancellation_terminal());
                sink.fail(StreamRuntimeError::Cancelled).await;
            }
        },
    }
    for (stream_runtime, stream_value) in arg_streams {
        stream_runtime.cancel(&stream_value);
    }
}

pub fn executable_body_contains_emit(executable: &LinkedExecutable) -> bool {
    executable
        .body
        .statements
        .iter()
        .any(|statement| matches!(statement, LinkedStmtIr::Emit { .. }))
}

pub fn linked_stream_item_type(return_type: Option<&LinkedTypeRef>) -> Option<&LinkedTypeRef> {
    let LinkedTypeRef::Native { name, args } = return_type? else {
        return None;
    };
    (bare_type_name(name) == "Stream" && args.len() == 1).then(|| &args[0])
}

fn linked_type_ref_contains_type_param(type_ref: &LinkedTypeRef) -> bool {
    match type_ref {
        LinkedTypeRef::TypeParam { .. } => true,
        LinkedTypeRef::AppliedNominal { arguments, .. } => {
            arguments.iter().any(linked_type_ref_contains_type_param)
        }
        LinkedTypeRef::Native { args, .. } | LinkedTypeRef::Union { items: args } => {
            args.iter().any(linked_type_ref_contains_type_param)
        }
        LinkedTypeRef::Record { fields } => {
            fields.values().any(linked_type_ref_contains_type_param)
        }
        LinkedTypeRef::Nullable { inner } => linked_type_ref_contains_type_param(inner),
        LinkedTypeRef::AnyInterface { interface } => interface
            .canonical_type_args
            .iter()
            .any(linked_type_ref_contains_type_param),
        LinkedTypeRef::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| linked_type_ref_contains_type_param(&param.ty))
                || linked_type_ref_contains_type_param(return_type)
        }
        LinkedTypeRef::Literal { .. }
        | LinkedTypeRef::LocalType { .. }
        | LinkedTypeRef::PublicationType { .. }
        | LinkedTypeRef::ServiceSymbol { .. }
        | LinkedTypeRef::PackageSymbol { .. }
        | LinkedTypeRef::PackageSchema { .. }
        | LinkedTypeRef::DbObjectSymbol { .. }
        | LinkedTypeRef::Address { .. } => false,
    }
}

#[cfg(all(test, any()))]
mod tests {
    use std::{
        collections::{BTreeMap, HashMap},
        sync::Arc,
    };

    use super::*;
    use crate as runtime_root;
    use crate::{
        eval::invocation::EvalProgramProjection,
        eval::program::{
            anonymous_type_decl, CallIr, ExecutableAddr, ExecutableKind, FileAddr,
            LinkedCallTarget, LinkedExecutable, LinkedExecutableBody, LinkedTypeDescriptor,
            ParamIr, RuntimeProgram, RuntimeTypeContext, ServiceDependencySymbolRef, ServiceMeta,
            SlotLayoutIr, TypeAddr, UnitAddr,
        },
    };

    fn empty_program() -> RuntimeProgram {
        RuntimeProgram {
            service: ServiceMeta {
                id: "svc".to_string(),
                display_name: Some("Service".to_string()),
                metadata: Default::default(),
            },
            version: "v1".to_string(),
            build_id: "build:program".to_string(),
            service_files: Vec::new(),
            packages: Vec::new(),
            service_resources: Default::default(),
            service_dependencies: Vec::new(),
            timeout: Default::default(),
            operation_route_bindings: Vec::new(),
            routes: Default::default(),
            spawn_routes: Default::default(),
            operations: Default::default(),
            operation_receivers: Default::default(),
            db: Vec::new(),
            actors: Vec::new(),
            link_overlay: Default::default(),
            gateway: Default::default(),
            types: RuntimeTypeContext::default(),
        }
    }

    fn builtin(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::Native {
            name: name.to_string(),
            args: Vec::new(),
        }
    }

    fn type_param(name: &str) -> LinkedTypeRef {
        LinkedTypeRef::TypeParam {
            name: name.to_string(),
        }
    }

    fn service_type_addr(file: usize, type_index: usize) -> TypeAddr {
        TypeAddr {
            unit: UnitAddr::Service,
            file: FileAddr::LoadedFileIndex(file),
            type_index,
        }
    }

    #[test]
    fn service_dependency_stream_call_is_not_treated_as_unlinked_producer() {
        let program = Arc::new(empty_program());
        let projection = EvalProgramProjection::new(
            &program.service_id,
            &program.service_files,
            &program.packages,
            &program.spawn_routes,
            &program.link_overlay,
            &program.types,
        );
        let interpreter = Interpreter::with_program(
            program.clone(),
            runtime_root::eval_capability_adapter::runtime_factory(),
        );
        let executable = LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "svc.main.run".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: None,
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        };
        let heap = RequestHeap::default();
        let expr = LinkedExprIr::Call {
            call: CallIr {
                target: LinkedCallTarget::ServiceDependencySymbol {
                    symbol: ServiceDependencySymbolRef {
                        dependency_ref: "remoteLlm".to_string(),
                        operation: skiff_artifact_model::OperationAbiRef {
                            operation_abi_id: "operation:remoteLlm:streamChat".to_string(),
                            kind: skiff_artifact_model::PublicationOperationKind::PublicFunction,
                            public_path: "streamChat".to_string(),
                            public_instance_key: None,
                            interface: None,
                            method_abi_id: None,
                            display_name: "streamChat".to_string(),
                        },
                    },
                },
                site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                    reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
                },
                args: Vec::new(),
                type_args: BTreeMap::new(),
                metadata: BTreeMap::new(),
            },
        };

        let producer = interpreter
            .resolve_stream_producer_call(
                projection,
                &ExecutableAddr::service(0, 0),
                &heap,
                &Env::default(),
                &executable,
                &expr,
            )
            .expect("service dependency call should fall back to normal stream eval");

        assert!(producer.is_none());
    }

    #[test]
    fn stream_producer_generic_item_type_uses_structured_substitutions() {
        let program = empty_program();
        let caller_addr = ExecutableAddr::service(0, 0);
        let callee_addr = ExecutableAddr::service(0, 1);
        let executable = LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "svc.main.produce".to_string(),
            type_params: Vec::new(),
            params: vec![ParamIr {
                name: "value".to_string(),
                slot: 0,
                ty: type_param("T"),
            }],
            return_type: Some(LinkedTypeRef::Native {
                name: "Stream".to_string(),
                args: vec![type_param("T")],
            }),
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        };
        let call = CallIr {
            target: LinkedCallTarget::Executable {
                addr: callee_addr.clone(),
            },
            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
            args: Vec::new(),
            type_args: BTreeMap::from([("T".to_string(), builtin("string"))]),
            metadata: BTreeMap::new(),
        };

        let routes = HashMap::<String, ExecutableAddr>::new();
        let type_projection = EvalTypeProjection::new(EvalProgramProjection::new(
            &program.service_id,
            &program.service_files,
            &program.packages,
            &routes,
            &program.link_overlay,
            &program.types,
        ));
        let plan = stream_item_plan_from_return_type(
            &type_projection,
            &caller_addr,
            &callee_addr,
            &executable,
            &call.type_args,
            &TypeSubstitutions::new(),
        )
        .expect("stream item plan should build")
        .expect("Stream<T> should have an item plan");

        assert!(
            format!("{plan:?}").contains("node: String"),
            "Stream<T> item type should use the call binding"
        );
    }

    #[test]
    fn stream_producer_local_type_item_uses_nested_callee_resolution() {
        let mut program = empty_program();
        program.types.descriptors.insert(
            service_type_addr(0, 1),
            anonymous_type_decl(
                "CallerLocal",
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::from([("caller_only".to_string(), builtin("number"))]),
                },
            ),
        );
        program.types.descriptors.insert(
            service_type_addr(1, 1),
            anonymous_type_decl(
                "CalleeLocal",
                LinkedTypeDescriptor::Record {
                    fields: BTreeMap::from([("callee_only".to_string(), builtin("string"))]),
                },
            ),
        );
        let caller_addr = ExecutableAddr::service(0, 0);
        let callee_addr = ExecutableAddr::service(1, 0);
        let executable = LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: "svc.main.produceLocal".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(LinkedTypeRef::Native {
                name: "Stream".to_string(),
                args: vec![LinkedTypeRef::LocalType { type_index: 1 }],
            }),
            self_type: None,
            slots: SlotLayoutIr::default(),
            may_suspend: false,
            body: LinkedExecutableBody::default(),
        };
        let call = CallIr {
            target: LinkedCallTarget::Executable {
                addr: callee_addr.clone(),
            },
            site: skiff_artifact_model::InstructionSourceSite::Synthetic {
                reason: skiff_artifact_model::SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
            },
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };

        let routes = HashMap::<String, ExecutableAddr>::new();
        let type_projection = EvalTypeProjection::new(EvalProgramProjection::new(
            &program.service_id,
            &program.service_files,
            &program.packages,
            &routes,
            &program.link_overlay,
            &program.types,
        ));
        let plan = stream_item_plan_from_return_type(
            &type_projection,
            &caller_addr,
            &callee_addr,
            &executable,
            &call.type_args,
            &TypeSubstitutions::new(),
        )
        .expect("stream item plan should build")
        .expect("Stream<LocalType> should have an item plan");
        let debug = format!("{plan:?}");

        assert!(
            debug.contains("callee_only"),
            "Stream<LocalType> item type should resolve against the callee file"
        );
        assert!(
            !debug.contains("caller_only"),
            "Stream<LocalType> item type must not resolve against the caller file"
        );
    }
}

#[cfg(test)]
mod prepared_stream_drain_tests {
    use skiff_runtime_capability_context::StreamRuntimeError;
    use skiff_runtime_model::service_error::OpaqueServiceError;

    use super::prepared_stream_error_after_drain;
    use crate::error::RuntimeError;

    fn fixed_service_error() -> OpaqueServiceError {
        OpaqueServiceError::decode(
            br#"{"kind":"internalError","payload":{"message":"Internal service error","traceId":"trace-stream","errorId":"trace-stream:error"}}"#
                .to_vec(),
        )
        .expect("fixed service error fixture should decode")
    }

    #[test]
    fn unknown_stream_after_consumer_error_fails_closed() {
        let error = prepared_stream_error_after_drain(
            RuntimeError::DecodeTarget {
                target: "std.json.decode".to_string(),
                message: "producer failed before its first event".to_string(),
            },
            Err(StreamRuntimeError::decode("unknown Stream value")),
        );

        assert!(matches!(
            error,
            RuntimeError::Decode(message) if message == "unknown Stream value"
        ));
    }

    #[test]
    fn consumer_error_is_preserved_after_normal_producer_end() {
        let error = prepared_stream_error_after_drain(
            RuntimeError::FileError {
                message: "consumer failed".to_string(),
            },
            Ok(()),
        );

        assert!(matches!(
            error,
            RuntimeError::FileError { message } if message == "consumer failed"
        ));
    }

    #[test]
    fn genuine_drain_error_still_overrides_consumer_error() {
        let error = prepared_stream_error_after_drain(
            RuntimeError::FileError {
                message: "consumer failed".to_string(),
            },
            Err(StreamRuntimeError::decode(
                "unexpected stream registry failure",
            )),
        );

        assert!(matches!(
            error,
            RuntimeError::Decode(message) if message == "unexpected stream registry failure"
        ));
    }

    #[test]
    fn fixed_drain_terminal_overrides_consumer_error_without_reencoding() {
        let fixed = fixed_service_error();
        let exact = fixed.encoded_bytes().to_vec();
        let error = prepared_stream_error_after_drain(
            RuntimeError::FileError {
                message: "consumer failed".to_string(),
            },
            Err(StreamRuntimeError::fixed_service_failure(fixed)),
        );

        match error {
            RuntimeError::FixedServiceFailure(error) => {
                assert_eq!(error.encoded_bytes(), exact)
            }
            _ => panic!("fixed producer terminal must remain typed"),
        }
    }
}
