use std::{collections::BTreeMap, sync::Arc};

use async_recursion::async_recursion;
use skiff_runtime_activation::RuntimeActivation;
use skiff_runtime_linked_program::{
    ConstAddr, ExecutableAddr, ExecutableKind, ExprRefIr, LinkedExecutable, LinkedExprIr,
    LinkedFileUnit, LinkedStmtIr, LinkedTypeRef,
};
use skiff_runtime_model::{
    request_heap::{RequestHeap, RequestHeapLimits},
    runtime_value::RuntimeValue,
};

#[allow(unused_imports)]
pub use super::program_types::executable_type_param_names;
use super::{
    capabilities::{
        ActorCapabilityContext, ConfigCapabilityContext, DbCapabilityContext,
        EffectDispatchContext, ExecutionControl, FileCapabilityContext, FileCapabilitySource,
        FileSourceStreamContext, HttpClientCapabilityContext, OutboundServiceContext,
        OwnedActorCapabilityContext, OwnedConfigCapabilityContext, OwnedExecutionControl,
        OwnedWebsocketCapabilityContext, StreamRuntime, TelemetryCapabilityContext,
        TestEffectDoubleContext, TimeCapabilityContext, WebsocketCapabilityContext,
    },
    error::attach_source_frame,
    eval_context::EvalContext,
    flow_completion::FlowCompletionPolicy,
    invocation::{EvalInvocation, EvalProgramProjection},
    program_ir::{
        executable_has_explicit_self_binding, program_assembly_index, program_u32_to_usize,
        validate_program_call_arg_count,
    },
    program_types::call_type_substitutions,
    source_context::program_source_context_frame,
    *,
};

pub struct ProgramExecutionInput<'a> {
    pub execution: ExecutionControl<'a>,
    pub config: ConfigCapabilityContext<'a>,
    pub db: DbCapabilityContext,
    pub file: FileCapabilityContext,
    pub file_source_stream: FileSourceStreamContext<'a>,
    pub time: TimeCapabilityContext<'a>,
    pub websocket: WebsocketCapabilityContext<'a>,
    pub effects: EffectDispatchContext,
    pub http_client: HttpClientCapabilityContext,
    pub test_effect_doubles: TestEffectDoubleContext,
    pub runtime_activation: Arc<RuntimeActivation>,
    pub actor: ActorCapabilityContext<'a>,
    pub spawn: ActorCapabilityContext<'a>,
    pub outbound: OutboundServiceContext,
    pub request_heap_limits: RequestHeapLimits,
}

#[derive(Clone)]
pub struct ProgramExecutionContext<'a> {
    execution: ExecutionControl<'a>,
    config: ConfigCapabilityContext<'a>,
    db: DbCapabilityContext,
    file: FileCapabilityContext,
    file_source_stream: FileSourceStreamContext<'a>,
    time: TimeCapabilityContext<'a>,
    websocket: WebsocketCapabilityContext<'a>,
    effects: EffectDispatchContext,
    http_client: HttpClientCapabilityContext,
    test_effect_doubles: TestEffectDoubleContext,
    runtime_activation: Arc<RuntimeActivation>,
    actor: ActorCapabilityContext<'a>,
    spawn: ActorCapabilityContext<'a>,
    outbound: OutboundServiceContext,
    request_heap_limits: RequestHeapLimits,
}

impl<'a> ProgramExecutionContext<'a> {
    pub fn new(input: ProgramExecutionInput<'a>) -> Self {
        Self {
            execution: input.execution,
            config: input.config,
            db: input.db,
            file: input.file,
            file_source_stream: input.file_source_stream,
            time: input.time,
            websocket: input.websocket,
            effects: input.effects,
            http_client: input.http_client,
            test_effect_doubles: input.test_effect_doubles,
            runtime_activation: input.runtime_activation,
            actor: input.actor,
            spawn: input.spawn,
            outbound: input.outbound,
            request_heap_limits: input.request_heap_limits,
        }
    }

    pub fn execution(&self) -> ExecutionControl<'a> {
        self.execution.clone()
    }

    pub fn config_context(&self) -> ConfigCapabilityContext<'a> {
        self.config.clone()
    }

    pub fn db_context(&self) -> DbCapabilityContext {
        self.db.clone()
    }

    pub fn file_context(&self) -> FileCapabilityContext {
        self.file.clone()
    }

    pub fn file_source_stream_context(&self) -> FileSourceStreamContext<'a> {
        self.file_source_stream.clone()
    }

    pub fn time_context(&self) -> TimeCapabilityContext<'a> {
        self.time.clone()
    }

    pub fn websocket_context(&self) -> WebsocketCapabilityContext<'a> {
        self.websocket.clone()
    }

    pub fn telemetry_context(&self) -> TelemetryCapabilityContext {
        self.effects.telemetry_context()
    }

    pub fn http_client_context(&self) -> HttpClientCapabilityContext {
        self.http_client.clone()
    }

    pub fn test_effect_double_context(&self) -> TestEffectDoubleContext {
        self.test_effect_doubles.clone()
    }

    pub fn runtime_activation(&self) -> &RuntimeActivation {
        &self.runtime_activation
    }

    pub fn actor_context(&self) -> ActorCapabilityContext<'a> {
        self.actor.clone()
    }

    pub fn spawn_context(&self) -> ActorCapabilityContext<'a> {
        self.spawn.clone()
    }

    pub fn outbound_context(&self) -> OutboundServiceContext {
        self.outbound.clone()
    }

    pub fn request_heap(&self) -> RequestHeap {
        RequestHeap::new(self.request_heap_limits.clone())
    }

    pub fn request_heap_limits(&self) -> RequestHeapLimits {
        self.request_heap_limits.clone()
    }
}

/// Owned, `'static` mirror of every borrow held by [`ProgramExecutionContext`].
///
/// A `ProgramExecutionContext<'a>` borrows almost entirely from data that lives
/// for the whole request inside service-level and per-request `Arc`s; the
/// borrows are just convenient views. To run a stream producer in
/// its own `tokio::spawn`ed task (so native stack depth stays constant no matter
/// how deeply stream producers nest) the producer future must be `Send +
/// 'static`, which a borrowed context can never be. This struct holds owned/
/// `Arc` copies of that underlying data, and [`OwnedProgramExecutionContext::borrow`]
/// reconstructs a borrowed `ProgramExecutionContext<'_>` from it. Wrap it in an
/// `Arc` and clone the `Arc` into each spawned task.
///
/// The owned `actor` strings are shared by both the actor and spawn contexts —
/// they are identical at the construction site (`runner.rs`).
pub struct OwnedProgramExecutionContext {
    execution: OwnedExecutionControl,
    config: OwnedConfigCapabilityContext,
    db: DbCapabilityContext,
    file_source: FileCapabilitySource,
    stream_runtime: StreamRuntime,
    websocket: OwnedWebsocketCapabilityContext,
    effects: EffectDispatchContext,
    http_client: HttpClientCapabilityContext,
    test_effect_doubles: TestEffectDoubleContext,
    runtime_activation: Arc<RuntimeActivation>,
    actor: OwnedActorCapabilityContext,
    spawn: OwnedActorCapabilityContext,
    outbound: OutboundServiceContext,
    request_heap_limits: RequestHeapLimits,
}

impl OwnedProgramExecutionContext {
    /// Captures owned copies of everything `context` borrows so the resulting
    /// value can outlive the original borrow scope (e.g. inside a spawned task).
    pub fn capture(context: &ProgramExecutionContext<'_>) -> Self {
        let execution = context.execution.clone();
        let actor = context.actor.clone();
        Self {
            execution: execution.owned(),
            config: ConfigCapabilityContext::owned(&context.config),
            db: context.db.clone(),
            file_source: context.file.source(),
            stream_runtime: context.file_source_stream.stream_runtime_handle(),
            websocket: context.websocket.owned(),
            effects: context.effects.clone(),
            http_client: context.http_client.clone(),
            test_effect_doubles: context.test_effect_doubles.clone(),
            runtime_activation: context.runtime_activation.clone(),
            actor: actor.owned(),
            spawn: context.spawn.owned(),
            outbound: context.outbound.clone(),
            request_heap_limits: context.request_heap_limits.clone(),
        }
    }

    /// Reconstructs a borrowed execution context that views this owned data.
    pub fn borrow(&self) -> ProgramExecutionContext<'_> {
        let execution = self.execution.borrow();
        let config = self.config.borrow();
        let file = self.file_source.context_for_request(self.db.clone());
        let file_source_stream =
            FileSourceStreamContext::new(self.stream_runtime.clone(), execution.clone());
        let time = TimeCapabilityContext::new(execution.clone());
        let websocket = self.websocket.borrow();
        let actor = self.actor.borrow();
        let spawn = self.spawn.borrow();
        ProgramExecutionContext::new(ProgramExecutionInput {
            execution,
            config,
            db: self.db.clone(),
            file,
            file_source_stream,
            time,
            websocket,
            effects: self.effects.clone(),
            http_client: self.http_client.clone(),
            test_effect_doubles: self.test_effect_doubles.clone(),
            runtime_activation: self.runtime_activation.clone(),
            actor,
            spawn,
            outbound: self.outbound.clone(),
            request_heap_limits: self.request_heap_limits.clone(),
        })
    }
}

pub trait IntoProgramExecutionContext<'a> {
    fn into_program_execution_context(self) -> ProgramExecutionContext<'a>;
}

impl<'a> IntoProgramExecutionContext<'a> for ProgramExecutionContext<'a> {
    fn into_program_execution_context(self) -> ProgramExecutionContext<'a> {
        self
    }
}

pub struct ExecutableInvocation<'a> {
    program: EvalProgramProjection<'a>,
    pub addr: &'a ExecutableAddr,
    pub file: &'a LinkedFileUnit,
    pub executable: &'a LinkedExecutable,
    pub explicit_self_param: bool,
}

impl<'a> ExecutableInvocation<'a> {
    pub fn from_eval_invocation(invocation: EvalInvocation<'a>) -> Self {
        let executable_body = invocation.executable_body();
        Self {
            program: invocation.program_projection(),
            addr: invocation.addr(),
            file: executable_body.file(),
            executable: executable_body.executable(),
            explicit_self_param: executable_body.explicit_self_param(),
        }
    }

    pub fn resolve(interpreter: &'a Interpreter, addr: &'a ExecutableAddr) -> Result<Self> {
        let program = interpreter.program_projection()?;
        let resolved = program.resolve_executable(addr)?;
        Ok(Self {
            program,
            addr,
            file: resolved.file,
            executable: resolved.executable,
            explicit_self_param: executable_has_explicit_self_binding(resolved.executable),
        })
    }

    pub fn program_projection(&self) -> EvalProgramProjection<'a> {
        self.program
    }

    pub fn validate_arg_count(&self, arg_count: usize) -> Result<()> {
        let expected_args = if self.explicit_self_param {
            arg_count.saturating_add(1)
        } else {
            arg_count
        };
        validate_program_call_arg_count(self.executable, expected_args)
    }

    pub fn validate_raw_arg_count(&self, arg_count: usize) -> Result<()> {
        validate_program_call_arg_count(self.executable, arg_count)
    }

    fn accepts_separate_self_argument_without_self_param(&self, arg_count: usize) -> bool {
        matches!(self.executable.kind, ExecutableKind::ImplMethod)
            && self.executable.self_type.is_some()
            && !self.explicit_self_param
            && arg_count == self.executable.params.len() + 1
    }

    pub fn env(&self) -> Result<Env> {
        Env::for_program_executable(
            self.executable,
            Some(self.file.module_path.clone()),
            program_assembly_index(self.addr),
        )
    }

    pub fn env_for_call(
        &self,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        type_args: &BTreeMap<String, LinkedTypeRef>,
    ) -> Result<Env> {
        let mut env = self.env()?;
        env.stream_sink = caller_env.stream_sink.clone();
        env.current_stream_item_type = caller_env.current_stream_item_type.clone();
        env.response_stream_sink = caller_env.response_stream_sink.clone();
        env.type_substitutions = call_type_substitutions(
            self.program_projection().type_view(),
            caller_addr,
            &caller_env.type_substitutions,
            self.executable,
            type_args,
        );
        Ok(env)
    }

    pub fn declare_self(&self, env: &mut Env, self_value: RuntimeValue) -> Result<()> {
        if self.explicit_self_param {
            env.declare_program_parameter(self.executable, "self", self_value)?;
        } else {
            env.declare_program_self(self.executable, self_value)?;
        }
        Ok(())
    }

    pub fn declare_args(&self, env: &mut Env, args: &[RuntimeValue]) -> Result<()> {
        for (index, parameter) in self
            .executable
            .params
            .iter()
            .skip(usize::from(self.explicit_self_param))
            .enumerate()
        {
            env.declare_program_parameter(
                self.executable,
                &parameter.name,
                args.get(index).cloned().unwrap_or(RuntimeValue::Null),
            )?;
        }
        Ok(())
    }

    pub async fn exec<'ctx>(
        &self,
        interpreter: &Interpreter,
        context: impl IntoProgramExecutionContext<'ctx> + Send,
        heap: &mut RequestHeap,
        env: &mut Env,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        interpreter
            .exec_program_executable(context, heap, env, self.addr, self.file, self.executable)
            .await
    }
}

impl Interpreter {
    pub fn program_projection(&self) -> Result<EvalProgramProjection<'_>> {
        Ok(EvalProgramProjection::new(
            &self.program.service_id,
            &self.program.service_files,
            &self.program.packages,
            &self.program.package_files,
            &self.program.spawn_routes,
            &self.program.link_overlay,
            &self.program.types,
        ))
    }

    #[async_recursion]
    pub async fn call_program_executable(
        &self,
        context: ProgramExecutionContext<'async_recursion>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        context.execution().add_instruction_units(1)?;
        context.execution().poll_execution_budget()?;

        let invocation = ExecutableInvocation::resolve(self, addr)?;
        let has_separate_self_arg =
            invocation.accepts_separate_self_argument_without_self_param(args.len());
        if !has_separate_self_arg {
            invocation.validate_raw_arg_count(args.len())?;
        }

        let mut env = invocation.env_for_call(caller_env, caller_addr, type_args)?;
        let (self_value, args) = if invocation.explicit_self_param || has_separate_self_arg {
            let Some((self_value, args)) = args.split_first() else {
                return Err(RuntimeError::Decode(format!(
                    "callable {} missing self argument",
                    invocation.executable.symbol
                )));
            };
            (self_value.clone(), args)
        } else {
            (
                caller_env
                    .self_value()
                    .unwrap_or_else(|| RuntimeValue::Null),
                args.as_slice(),
            )
        };
        invocation.declare_self(&mut env, self_value)?;
        invocation.declare_args(&mut env, args)?;

        let flow = invocation
            .exec(self, context.clone(), heap, &mut env)
            .await?;
        context.execution().poll_execution_budget()?;
        FlowCompletionPolicy::callable_value(flow, &invocation.executable.symbol)
    }

    #[async_recursion]
    pub async fn call_program_executable_with_self(
        &self,
        context: ProgramExecutionContext<'async_recursion>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        self_value: RuntimeValue,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        self.call_program_executable_with_self_inner(
            context,
            heap,
            caller_env,
            caller_addr,
            addr,
            type_args,
            self_value,
            args,
            true,
        )
        .await
    }

    #[async_recursion]
    pub async fn call_program_executable_with_self_direct(
        &self,
        context: ProgramExecutionContext<'async_recursion>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        self_value: RuntimeValue,
        args: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        self.call_program_executable_with_self_inner(
            context,
            heap,
            caller_env,
            caller_addr,
            addr,
            type_args,
            self_value,
            args,
            false,
        )
        .await
    }

    #[async_recursion]
    async fn call_program_executable_with_self_inner(
        &self,
        context: ProgramExecutionContext<'async_recursion>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        caller_addr: &ExecutableAddr,
        addr: &ExecutableAddr,
        type_args: &std::collections::BTreeMap<String, LinkedTypeRef>,
        self_value: RuntimeValue,
        args: Vec<RuntimeValue>,
        allow_stream_defer: bool,
    ) -> Result<RuntimeValue> {
        context.execution().add_instruction_units(1)?;
        context.execution().poll_execution_budget()?;

        let invocation = ExecutableInvocation::resolve(self, addr)?;
        invocation.validate_arg_count(args.len())?;

        if allow_stream_defer {
            if let Some(value) = self
                .prepare_deferred_stream_producer_from_values(
                    self.program_projection()?,
                    context.clone(),
                    heap,
                    caller_env,
                    caller_addr,
                    addr,
                    invocation.executable,
                    type_args,
                    self_value.clone(),
                    args.clone(),
                )
                .await?
            {
                context.execution().poll_execution_budget()?;
                return Ok(value);
            }
        }

        let mut env = invocation.env_for_call(caller_env, caller_addr, type_args)?;
        invocation.declare_self(&mut env, self_value)?;
        invocation.declare_args(&mut env, &args)?;

        let flow = invocation
            .exec(self, context.clone(), heap, &mut env)
            .await?;
        context.execution().poll_execution_budget()?;
        FlowCompletionPolicy::callable_value(flow, &invocation.executable.symbol)
    }

    pub async fn exec_program_executable<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)
            .exec_program_executable()
            .await
    }

    pub async fn eval_program_const<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        const_index: u32,
    ) -> Result<RuntimeValue> {
        let context = context.into_program_execution_context();
        let const_index = program_u32_to_usize(const_index, "const ref")?;
        self.eval_program_const_in_file(context, heap, caller_env, addr, file, const_index)
            .await
    }

    pub async fn eval_program_const_addr<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        const_addr: &ConstAddr,
    ) -> Result<RuntimeValue> {
        let context = context.into_program_execution_context();
        let program = self.program_projection()?;
        let file = program.resolve_file(&const_addr.unit, &const_addr.file)?;
        let addr = ExecutableAddr {
            unit: const_addr.unit.clone(),
            file: const_addr.file.clone(),
            executable: 0,
        };
        self.eval_program_const_in_file(
            context,
            heap,
            caller_env,
            &addr,
            file.as_ref(),
            const_addr.const_index,
        )
        .await
    }

    async fn eval_program_const_in_file<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        caller_env: &Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        const_index: usize,
    ) -> Result<RuntimeValue> {
        let context = context.into_program_execution_context();
        let constant = file.constants.get(const_index).ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!("RuntimeProgram const {const_index} is missing"))
        })?;
        let executable = LinkedExecutable {
            kind: ExecutableKind::Function,
            symbol: constant.name.clone(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: Some(constant.ty.clone()),
            self_type: None,
            slots: Default::default(),
            may_suspend: false,
            body: constant.body.clone(),
        };
        let mut env = Env::for_program_executable(
            &executable,
            Some(file.module_path.clone()),
            program_assembly_index(addr),
        )?;
        env.stream_sink = caller_env.stream_sink.clone();
        env.current_stream_item_type = caller_env.current_stream_item_type.clone();
        env.response_stream_sink = caller_env.response_stream_sink.clone();
        env.type_substitutions = caller_env.type_substitutions.clone();
        let flow = self
            .exec_program_executable(context, heap, &mut env, addr, file, &executable)
            .await?;
        FlowCompletionPolicy::const_value(flow, &constant.name)
    }

    pub async fn exec_program_block<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        label: &str,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)
            .exec_program_block(label)
            .await
    }

    async fn exec_program_statement<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        statement: &LinkedStmtIr,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)
            .exec_program_statement(statement)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn exec_program_for_in_body<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        item_slot: usize,
        body: &str,
        item_value: RuntimeValue,
    ) -> Result<Flow> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)
            .exec_program_for_in_body(item_slot, body, item_value)
            .await
    }

    pub async fn eval_program_expr_ref<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        expr_ref: ExprRefIr,
    ) -> Result<RuntimeValue> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)
            .eval_program_expr_ref(expr_ref)
            .await
    }

    async fn eval_program_expr<'ctx>(
        &self,
        context: impl IntoProgramExecutionContext<'ctx>,
        heap: &mut RequestHeap,
        env: &mut Env,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        executable: &LinkedExecutable,
        expr: &LinkedExprIr,
    ) -> Result<RuntimeValue> {
        let context = context.into_program_execution_context();
        EvalContext::new(self, context, heap, env, addr, file, executable)
            .eval_program_expr(expr)
            .await
    }

    pub fn eval_program_rethrow_slot(
        &self,
        env: &Env,
        exception_slot: usize,
        heap: &RequestHeap,
    ) -> Result<Flow> {
        let exception = env.get_slot(exception_slot)?;
        let exception = runtime_to_wire(&exception, heap)?;
        Err(RuntimeError::UserException(UserException::from_envelope(
            exception,
        )?))
    }

    pub fn attach_program_source_context(
        &self,
        error: RuntimeError,
        addr: &ExecutableAddr,
        file: &LinkedFileUnit,
        source_id: Option<u64>,
    ) -> RuntimeError {
        let Some(source_id) = source_id else {
            return error;
        };
        let frame = program_source_context_frame(addr, file, source_id);
        attach_source_frame(error, source_id, frame)
    }
}
