use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_recursion::async_recursion;
use serde_json::Value;
use skiff_runtime_boundary::stream::stream_id;
use skiff_runtime_boundary::type_descriptor::bare_type_name;
use skiff_runtime_capability_context::StreamRuntimeError;
use skiff_runtime_linked_program::{
    CallIr, ConstAddr, ExecutableAddr, LinkedCallTarget, LinkedExecutable, LinkedExprIr,
    LinkedFileUnit, LinkedStmtIr, LinkedTypeRef, ReceiverCallAbi,
};
use skiff_runtime_model::{
    request_heap::{deep_clone_runtime_value_between_heaps, RequestHeap},
    runtime_value::RuntimeValue,
    type_plan::RuntimeTypePlan,
};

use super::type_descriptor::TypeSubstitutions;
use super::{
    capabilities::{StreamCancelSignal, StreamPoll, StreamSink},
    env::{check_cancelled, Env, Flow},
    program_execution::{OwnedProgramExecutionContext, ProgramExecutionContext},
    program_ir::{program_call_target_kind, program_expression_ref},
    runtime_ops::{runtime_from_wire, runtime_from_wire_required_plan},
    Interpreter,
};
use crate::{
    error::{Result, RuntimeError},
    invocation::EvalProgramProjection,
    type_projection::EvalTypeProjection,
};

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
        let execution = context.execution();
        loop {
            execution.add_instruction_units(1)?;
            check_cancelled(&execution, env)?;
            let item = self
                .stream_runtime
                .next_with_cancel(&stream_value, cancel_signals, &[execution.cancel_flag()])
                .await?;
            let item = match item {
                StreamPoll::Item(item) => item,
                StreamPoll::End => return Ok(Flow::Continue),
            };
            let item_value = if let Some(item_type) = item_type.as_ref() {
                runtime_from_wire_required_plan(&item, Some(item_type), "stream item", heap)?
            } else {
                runtime_from_wire(&item, heap)?
            };
            let flow = self
                .exec_program_for_in_body(
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
                Err(error) => {
                    self.stream_runtime.cancel(&stream_value);
                    return Err(error);
                }
            };
            match flow {
                Flow::Continue | Flow::LoopContinue => continue,
                Flow::Break => {
                    self.stream_runtime.cancel(&stream_value);
                    return Ok(Flow::Continue);
                }
                Flow::Return(value) => {
                    self.stream_runtime.cancel(&stream_value);
                    return Ok(Flow::Return(value));
                }
                Flow::Parked => {
                    self.stream_runtime.cancel(&stream_value);
                    return Ok(Flow::Parked);
                }
                Flow::ContinueConsumer => {
                    self.stream_runtime.cancel(&stream_value);
                    return Ok(Flow::ContinueConsumer);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn exec_program_stream_producer_for_in(
        &self,
        program: EvalProgramProjection<'_>,
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
        // Whatever the consumer does (finish, break, return, error, cancel) the
        // producer task is signalled through the stream's cancel flag/notify so
        // it stops emitting and exits. This is the cross-task equivalent of the
        // old inline `select!` that cancelled the producer when the consumer
        // resolved first.
        self.stream_runtime.cancel(&stream_value);
        consumer_result
    }

    /// Prepares a stream-producer call whose result is bound to a value rather
    /// than consumed inline (e.g. `const s = producer(...)`). The producer is
    /// parked in the deferred registry keyed by the stream id it feeds, and the
    /// returned `RuntimeValue` is the stream the caller can iterate later. The
    /// parked producer is driven concurrently the first time that stream value
    /// is consumed by a `for-in` (see `exec_program_stream_for_in`).
    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_deferred_stream_producer(
        &self,
        program: EvalProgramProjection<'_>,
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
        let id = stream_id(&prepared.stream_value)
            .ok_or_else(|| {
                RuntimeError::Decode(
                    "deferred stream producer was not assigned a stream id".to_string(),
                )
            })?
            .to_string();
        // Hand the consumer a stream value backed by the parked producer's
        // channel, expressed in the caller's heap.
        let stream_value = match runtime_from_wire(&prepared.stream_value, heap) {
            Ok(value) => value,
            Err(error) => {
                self.stream_runtime.cancel(&prepared.stream_value);
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
    pub async fn prepare_deferred_stream_producer_from_values(
        &self,
        program: EvalProgramProjection<'_>,
        context: ProgramExecutionContext<'_>,
        heap: &mut RequestHeap,
        env: &Env,
        caller_addr: &ExecutableAddr,
        producer_addr: &ExecutableAddr,
        producer_executable: &LinkedExecutable,
        producer_type_args: &BTreeMap<String, LinkedTypeRef>,
        producer_self: RuntimeValue,
        producer_args: Vec<RuntimeValue>,
    ) -> Result<Option<RuntimeValue>> {
        if !executable_body_contains_emit(producer_executable) {
            return Ok(None);
        }
        let type_projection = EvalTypeProjection::new(program);
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
        let producer_self =
            deep_clone_runtime_value_between_heaps(heap, &mut producer_heap, &producer_self)?;
        let mut cloned_args = Vec::with_capacity(producer_args.len());
        for arg in &producer_args {
            cloned_args.push(deep_clone_runtime_value_between_heaps(
                heap,
                &mut producer_heap,
                arg,
            )?);
        }

        let mut producer_env = env.clone();
        let (stream_value, sink) = self.stream_runtime.channel_stream();
        let cancel_signal = sink.cancel_signal();
        producer_env.stream_sink = Some(sink.clone());
        producer_env.current_stream_item_type = Some(item_type.clone());
        let prepared = StreamProducerExecution {
            stream_value,
            cancel_signal,
            item_type,
            arg_producers: Vec::new(),
            producer_heap,
            producer_env,
            producer_addr: producer_addr.clone(),
            producer_self: Some(producer_self),
            producer_type_args: producer_type_args.clone(),
            producer_args: cloned_args,
            sink,
        };

        let id = stream_id(&prepared.stream_value)
            .ok_or_else(|| {
                RuntimeError::Decode(
                    "deferred stream producer was not assigned a stream id".to_string(),
                )
            })?
            .to_string();
        let stream_value = match runtime_from_wire(&prepared.stream_value, heap) {
            Ok(value) => value,
            Err(error) => {
                self.stream_runtime.cancel(&prepared.stream_value);
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
            PreparedNativeStreamProducer(prepared),
            consumer,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_native_stream_producer_arg(
        &self,
        program: EvalProgramProjection<'_>,
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
        .map(PreparedNativeStreamProducer)
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
        let stream_value = prepared.0.stream_value.clone();
        let cancel_signal = prepared.0.cancel_signal.clone();
        let owned_context = Arc::new(OwnedProgramExecutionContext::capture(&context));
        spawn_stream_producer(self, owned_context, addr.clone(), prepared.0);

        tokio::pin!(consumer);
        let consumer_result = consumer.await;
        match consumer_result {
            Ok(value) => {
                self.stream_runtime.cancel(&stream_value);
                Ok(value)
            }
            Err(error) if error.is_cancelled() => {
                self.stream_runtime.cancel(&stream_value);
                Err(error)
            }
            Err(error) => {
                // The consumer errored on its own (not via cancellation). Drain
                // the producer's pending output so a trailing producer error is
                // surfaced (preferred over the consumer error), then cancel.
                let drain_result = self
                    .drain_stream_producer_output(context, &stream_value, &cancel_signal)
                    .await;
                self.stream_runtime.cancel(&stream_value);
                match drain_result {
                    Ok(()) => Err(error),
                    Err(producer_error) => Err(producer_error),
                }
            }
        }
    }

    pub fn cancel_prepared_native_stream_producer_arg(
        &self,
        prepared: &PreparedNativeStreamProducer,
    ) {
        self.stream_runtime.cancel(prepared.stream_value());
    }

    async fn drain_stream_producer_output(
        &self,
        context: ProgramExecutionContext<'_>,
        stream_value: &Value,
        cancel_signal: &StreamCancelSignal,
    ) -> Result<()> {
        let execution = context.execution();
        loop {
            match self
                .stream_runtime
                .next_with_cancel(
                    stream_value,
                    std::slice::from_ref(cancel_signal),
                    &[execution.cancel_flag()],
                )
                .await
                .map_err(RuntimeError::from)
            {
                Ok(StreamPoll::Item(_)) => continue,
                Ok(StreamPoll::End) => return Ok(()),
                Err(RuntimeError::Decode(message))
                    if message == "Stream value has already been consumed" =>
                {
                    return Ok(())
                }
                Err(error) => return Err(error),
            }
        }
    }

    #[async_recursion]
    #[allow(clippy::too_many_arguments)]
    async fn prepare_stream_producer(
        &self,
        program: EvalProgramProjection<'async_recursion>,
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
        let mut arg_producers: Vec<StreamProducerExecution> = Vec::new();
        let mut args = Vec::with_capacity(producer.call.args.len());
        for arg in &producer.call.args {
            let expr = program_expression_ref(_executable, *arg)?;
            if let Some(arg_producer) =
                self.resolve_stream_producer_call(program, addr, heap, env, _executable, expr)?
            {
                if !arg_producers.is_empty() {
                    for producer in &arg_producers {
                        self.stream_runtime.cancel(&producer.stream_value);
                    }
                    return Err(RuntimeError::Unsupported(
                        "multiple stream-producing producer call arguments are not supported"
                            .to_string(),
                    ));
                }
                let nested = match self
                    .prepare_stream_producer(
                        program,
                        context.clone(),
                        heap,
                        env,
                        addr,
                        _file,
                        _executable,
                        arg_producer,
                    )
                    .await
                {
                    Ok(value) => value,
                    Err(error) => {
                        for producer in &arg_producers {
                            self.stream_runtime.cancel(&producer.stream_value);
                        }
                        return Err(error.into());
                    }
                };
                let stream_value = match runtime_from_wire(&nested.stream_value, &mut producer_heap)
                {
                    Ok(value) => value,
                    Err(error) => {
                        self.stream_runtime.cancel(&nested.stream_value);
                        for producer in &arg_producers {
                            self.stream_runtime.cancel(&producer.stream_value);
                        }
                        return Err(error.into());
                    }
                };
                args.push(stream_value);
                arg_producers.push(nested);
            } else {
                let arg = match self
                    .eval_program_expr_ref(
                        context.clone(),
                        heap,
                        env,
                        addr,
                        _file,
                        _executable,
                        *arg,
                    )
                    .await
                {
                    Ok(value) => value,
                    Err(error) => {
                        for producer in &arg_producers {
                            self.stream_runtime.cancel(&producer.stream_value);
                        }
                        return Err(error.into());
                    }
                };
                let arg =
                    match deep_clone_runtime_value_between_heaps(heap, &mut producer_heap, &arg) {
                        Ok(value) => value,
                        Err(error) => {
                            for producer in &arg_producers {
                                self.stream_runtime.cancel(&producer.stream_value);
                            }
                            return Err(error.into());
                        }
                    };
                args.push(arg);
            }
        }
        let producer_self = receiver
            .as_ref()
            .map(|receiver| {
                deep_clone_runtime_value_between_heaps(heap, &mut producer_heap, receiver)
            })
            .transpose()?;
        let mut producer_env = env.clone();
        let (stream_value, sink) = self.stream_runtime.channel_stream();
        let cancel_signal = sink.cancel_signal();
        producer_env.stream_sink = Some(sink.clone());
        producer_env.current_stream_item_type = Some(producer.item_type.clone());
        Ok(StreamProducerExecution {
            stream_value,
            cancel_signal,
            item_type: producer.item_type,
            arg_producers,
            producer_heap,
            producer_env,
            producer_addr: producer.addr,
            producer_self,
            producer_type_args: producer.call.type_args,
            producer_args: args,
            sink,
        })
    }

    pub fn resolve_stream_producer_call(
        &self,
        program: EvalProgramProjection<'_>,
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

    pub fn resolve_stream_producer_from_call(
        &self,
        program: EvalProgramProjection<'_>,
        current_addr: &ExecutableAddr,
        _heap: &RequestHeap,
        env: &Env,
        _executable: &LinkedExecutable,
        call: &CallIr,
    ) -> Result<Option<StreamProducerCall>> {
        let type_projection = EvalTypeProjection::new(program);
        let (addr, receiver_const, producer_self, call) = match &call.target {
            LinkedCallTarget::Executable { addr } => (addr.clone(), None, None, call.clone()),
            LinkedCallTarget::LocalExecutable { .. }
            | LinkedCallTarget::ExternalServiceSymbol { .. }
            | LinkedCallTarget::PackageSymbol { .. } => {
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
        let resolved = program.executable_at(&addr)?;
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
    pub producer_self: Option<RuntimeValue>,
    pub call: CallIr,
    pub item_type: RuntimeTypePlan,
}

pub struct PreparedNativeStreamProducer(StreamProducerExecution);

impl PreparedNativeStreamProducer {
    pub fn stream_value(&self) -> &Value {
        &self.0.stream_value
    }
}

pub struct StreamProducerExecution {
    stream_value: Value,
    cancel_signal: StreamCancelSignal,
    item_type: RuntimeTypePlan,
    arg_producers: Vec<StreamProducerExecution>,
    producer_heap: RequestHeap,
    producer_env: Env,
    producer_addr: ExecutableAddr,
    producer_self: Option<RuntimeValue>,
    producer_type_args: std::collections::BTreeMap<String, LinkedTypeRef>,
    producer_args: Vec<RuntimeValue>,
    sink: StreamSink,
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
    let interpreter = interpreter.clone();
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
        producer_self,
        producer_type_args,
        producer_args,
        sink,
        ..
    } = producer;

    let arg_streams = arg_producers
        .iter()
        .map(|producer| producer.stream_value.clone())
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
    let result = if let Some(producer_self) = producer_self {
        interpreter
            .call_program_executable_with_self_direct(
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
            .call_program_executable(
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
        Err(error) => sink.fail(StreamRuntimeError::producer(error)).await,
    }
    for stream_value in arg_streams {
        interpreter.stream_runtime.cancel(&stream_value);
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
            package_files: Vec::new(),
            service_resources: Default::default(),
            package_resources: Vec::new(),
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
            &program.package_files,
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
            args: Vec::new(),
            type_args: BTreeMap::from([("T".to_string(), builtin("string"))]),
            metadata: BTreeMap::new(),
        };

        let routes = HashMap::<String, ExecutableAddr>::new();
        let type_projection = EvalTypeProjection::new(EvalProgramProjection::new(
            &program.service_id,
            &program.service_files,
            &program.packages,
            &program.package_files,
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
            args: Vec::new(),
            type_args: BTreeMap::new(),
            metadata: BTreeMap::new(),
        };

        let routes = HashMap::<String, ExecutableAddr>::new();
        let type_projection = EvalTypeProjection::new(EvalProgramProjection::new(
            &program.service_id,
            &program.service_files,
            &program.packages,
            &program.package_files,
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
