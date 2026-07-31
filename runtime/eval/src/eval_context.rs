use async_recursion::async_recursion;
use skiff_runtime_linked_program::{
    ActivationRelativeServiceCall, AssignTargetIr, BlockIr, CallIr, ExecutableAddr, ExprRefIr,
    LinkedBoxSourceIr, LinkedCallTarget, LinkedExecutable, LinkedExprIr, LinkedFileUnit,
    LinkedInterfaceInstantiationRef, LinkedStmtIr, LinkedTestEffectOutcomeIr, LinkedTypeRef,
    NativeTarget, ReceiverCallAbi, UnaryOpIr,
};
use skiff_runtime_linked_type_plan::{
    linked_interface_instantiation_runtime_id, linked_type_ref_runtime_key,
};
use skiff_runtime_model::{
    request_heap::{deep_clone_runtime_value_carrier_between_heaps, RequestHeap},
    runtime_value::{
        HeapNode, InterfaceCarrier, InterfaceMethodTarget, InterfaceReceiverCallAbi,
        InterfaceValue, RuntimeValue, RuntimeValueCarrier, RuntimeValueKey,
    },
    service_error::{ExceptionStackFrame, RequestException},
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};

use super::{
    assembly_execution::{
        service_error_channel::{
            CanonicalServiceErrorChannel, RestrictedServiceDiagnosticExportContext,
            ServiceErrorExportContext, ServiceErrorImportContext,
        },
        RuntimeExecutionProjection,
    },
    capabilities::{ExecutionControl, RuntimeNativeConfigCapabilityContext},
    env::{Env, Flow},
    exceptions::{
        catch_err, catch_identity_matches, catch_ok, request_exception_for_catch,
        request_exception_for_resource_error, user_exception_for_catch,
    },
    flow_completion::FlowCompletionPolicy,
    native_capability::{
        project_runtime_execution_native_capability_context,
        project_runtime_execution_native_capability_context_supervised,
    },
    native_invocation::{
        resolve_config_builtin_type_arg_plan, resolve_runtime_execution_native_invocation,
    },
    program_db::{is_db_builtin_op, program_call_db_op},
    program_execution::{EvaluatorControl, ProgramExecutionContext},
    program_ir::{
        bind_program_pattern, program_binary_operator, program_block, program_call_target_kind,
        program_expression_ref, program_literal, program_pattern_matches, program_statement_ref,
        program_u32_to_usize,
    },
    program_mutation::assign_program_index_target_carrier,
    receiver_methods::ReceiverMethodDispatch,
    recoverable_behavior::interface_method_table_from_linked,
    runtime_ops::{
        runtime_array_from_carriers, runtime_array_item_carriers, runtime_carrier_for_plan,
        runtime_from_wire, runtime_map_from_carriers, runtime_member_access_carrier,
        runtime_object_from_carriers, runtime_representation_wrap_for_plan, runtime_to_wire,
        runtime_to_wire_required_plan,
    },
    spawn_ops,
    test_effect_registry::{
        RegisteredTestEffect, RegisteredTestEffectFailure, RegisteredTestEffectOutcome,
        RegisteredTestEffectThrow, ServiceTestEffectDispatch, TestEffectTarget,
    },
    type_projection::EvalTypeProjection,
    *,
};
use crate::error::{materialize_request_heap_owned_runtime_error, RuntimeError, UserException};
use promoted_runtime::dispatch::NativeDispatch;
use skiff_artifact_model::{InstructionSourceSite, SyntheticInstructionSiteReason};
use skiff_runtime_boundary::stream::is_stream_value;
use skiff_runtime_capability_context::StreamInternalItem;
use skiff_runtime_native as promoted_runtime;
use skiff_runtime_native_contract::{native_target_binding_key, native_target_name};

mod actual_pending;
mod checkpoint;
mod concurrent;
mod timeout;

#[cfg(test)]
pub(crate) use actual_pending::tests::legacy_native_call_expected_to_suspend as native_call_suspends;

pub struct EvalContext<'a> {
    pub interpreter: &'a Interpreter,
    projection: RuntimeExecutionProjection<'a>,
    pub context: ProgramExecutionContext<'a>,
    pub execution: ExecutionControl<'a>,
    pub heap: &'a mut RequestHeap,
    pub env: &'a mut Env,
    pub addr: &'a ExecutableAddr,
    pub file: &'a LinkedFileUnit,
    pub executable: &'a LinkedExecutable,
    tail_call_context: TailCallContext,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TailCallContext {
    Transparent,
    Barrier,
}

pub(crate) fn promote_call_site_error<T>(
    projection: &RuntimeExecutionProjection<'_>,
    context: &ProgramExecutionContext<'_>,
    heap: &mut RequestHeap,
    addr: &ExecutableAddr,
    result: Result<T>,
    site: &InstructionSourceSite,
) -> Result<T> {
    let Err(error) = result else {
        return result;
    };
    let error = materialize_request_heap_owned_runtime_error(error, heap)?;
    if error.is_cancellation_terminal() {
        return Err(error);
    }
    if user_exception_for_catch(&error).is_some() {
        return Err(error);
    }
    if let Some(exception) = request_exception_for_resource_error(
        &error,
        projection,
        addr,
        site.clone(),
        context.exception_stack_for_site(site.clone()),
        || context.next_exception_correlation(),
        heap,
    )? {
        return Err(RuntimeError::UserException(UserException::new(exception)));
    }
    let Some((identity, _)) = error.ordinary_catch_projection() else {
        return Err(error);
    };
    let exception = request_exception_for_catch(
        &error,
        std::slice::from_ref(&identity),
        site.clone(),
        context.exception_stack_for_site(site.clone()),
        context.next_exception_correlation()?,
        heap,
    )?
    .ok_or_else(|| {
        RuntimeError::InvalidArtifact(
            "platform catch projection did not match its own exact identity".to_string(),
        )
    })?;
    Err(RuntimeError::UserException(UserException::new(exception)))
}

impl<'a> EvalContext<'a> {
    pub fn new(
        interpreter: &'a Interpreter,
        context: ProgramExecutionContext<'a>,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
        addr: &'a ExecutableAddr,
        file: &'a LinkedFileUnit,
        executable: &'a LinkedExecutable,
    ) -> Result<Self> {
        Self::new_with_tail_call_context(
            interpreter,
            context,
            heap,
            env,
            addr,
            file,
            executable,
            TailCallContext::Barrier,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_callable_with_projection(
        interpreter: &'a Interpreter,
        projection: RuntimeExecutionProjection<'a>,
        context: ProgramExecutionContext<'a>,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
        addr: &'a ExecutableAddr,
        file: &'a LinkedFileUnit,
        executable: &'a LinkedExecutable,
    ) -> Self {
        let execution = context.execution();
        Self {
            interpreter,
            projection,
            context,
            execution,
            heap,
            env,
            addr,
            file,
            executable,
            tail_call_context: TailCallContext::Transparent,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_tail_call_context(
        interpreter: &'a Interpreter,
        context: ProgramExecutionContext<'a>,
        heap: &'a mut RequestHeap,
        env: &'a mut Env,
        addr: &'a ExecutableAddr,
        file: &'a LinkedFileUnit,
        executable: &'a LinkedExecutable,
        tail_call_context: TailCallContext,
    ) -> Result<Self> {
        let projection = RuntimeExecutionProjection::for_context(interpreter, &context)?;
        let execution = context.execution();
        Ok(Self {
            interpreter,
            projection,
            context,
            execution,
            heap,
            env,
            addr,
            file,
            executable,
            tail_call_context,
        })
    }

    pub(crate) async fn exec_program_block_with_tail_call_barrier(
        &mut self,
        label: &str,
    ) -> Result<Flow> {
        let previous = self.tail_call_context;
        self.tail_call_context = TailCallContext::Barrier;
        let result = self
            .exec_program_block_control(label)
            .await
            .and_then(|control| control.into_flow("tail-call barrier"));
        self.tail_call_context = previous;
        result
    }

    fn type_projection(&self) -> EvalTypeProjection<'a> {
        EvalTypeProjection::from_execution_projection(self.projection.clone())
    }

    fn materialize_service_test_throw(
        &mut self,
        call: &CallIr,
        instruction: &ActivationRelativeServiceCall,
        throw: RegisteredTestEffectThrow,
    ) -> Result<RuntimeValueCarrier> {
        let provider_source = InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerGeneratedTestHarness,
        };
        let provider_error = match throw.failure {
            RegisteredTestEffectFailure::LocalPayload(payload) => {
                let provider_exception = RequestException::local(
                    payload,
                    provider_source.clone(),
                    vec![ExceptionStackFrame::Local {
                        site: provider_source.clone(),
                    }],
                    self.context.next_exception_correlation()?,
                )
                .map_err(RuntimeError::InvalidArtifact)?;
                RuntimeError::UserException(UserException::new(provider_exception))
            }
            RegisteredTestEffectFailure::FixedService(error) => {
                RuntimeError::FixedServiceFailure(error)
            }
            RegisteredTestEffectFailure::ProviderFailure(error) => error,
        };

        let execution_image = self
            .context
            .runtime_assembly_target()?
            .execution_image()
            .clone();
        let projection = self.projection.clone();
        let synthetic_service_id = format!(
            "test-effect:{}",
            instruction.expected_protocol_identity().as_str()
        );
        let operation_id = instruction.operation_id().as_str();
        let telemetry = self.context.telemetry_context();
        let fallback_stack = vec![ExceptionStackFrame::Local {
            site: provider_source.clone(),
        }];
        let provider_activation_id = format!("test-effect-activation:{synthetic_service_id}");
        let request_generation = self
            .context
            .runtime_assembly_target()?
            .request_activation()
            .generation();
        let fixed = CanonicalServiceErrorChannel::export_provider_failure_with_diagnostic(
            &provider_error,
            ServiceErrorExportContext {
                execution_image: &execution_image,
                type_view: projection.type_view(),
                provider_heap: &throw.setup_heap,
                provider_package_build_id: &throw.setup_package_build_id,
                caller_package_build_id: Some(instruction.caller_package_build_id()),
                provider_service_id: &synthetic_service_id,
                operation_id,
            },
            RestrictedServiceDiagnosticExportContext {
                telemetry: &telemetry,
                provider_activation_id: &provider_activation_id,
                request_generation,
                fallback_source: &provider_source,
                fallback_stack: &fallback_stack,
            },
            || self.context.next_exception_correlation(),
        )?;

        let caller_stack = self.context.exception_stack_for_site(call.site.clone());
        let imported = CanonicalServiceErrorChannel::import_caller_failure(
            fixed,
            ServiceErrorImportContext {
                execution_image: &execution_image,
                type_view: projection.type_view(),
                caller_heap: self.heap,
                caller_package_build_id: instruction.caller_package_build_id(),
                caller_executable_addr: self.addr,
                call_site: &call.site,
                caller_stack_at_site: &caller_stack,
                remote_service_id: &synthetic_service_id,
                remote_operation_id: operation_id,
            },
        )?;
        Err(RuntimeError::UserException(imported))
    }

    pub(crate) fn execution_projection(&self) -> &RuntimeExecutionProjection<'a> {
        &self.projection
    }

    pub async fn exec_program_executable(&mut self) -> Result<Flow> {
        self.exec_program_executable_control()
            .await?
            .into_flow("EvalContext executable")
    }

    pub(crate) async fn exec_program_executable_control(&mut self) -> Result<EvaluatorControl> {
        let block = self.prepare_program_executable_entry()?;
        self.exec_program_block_body(block).await
    }

    pub(crate) async fn exec_tail_entry_control(
        &mut self,
        tail_caller: &ExecutableAddr,
        tail_site: &InstructionSourceSite,
    ) -> Result<EvaluatorControl> {
        let entry = self.prepare_program_executable_entry();
        let block = promote_call_site_error(
            &self.projection,
            &self.context,
            self.heap,
            tail_caller,
            entry,
            tail_site,
        )?;
        self.exec_program_block_body(block).await
    }

    fn prepare_program_executable_entry(&self) -> Result<&'a BlockIr> {
        self.checkpoint_function_entry()?;
        self.prepare_program_block("entry")
    }

    #[async_recursion]
    pub async fn exec_program_block(&mut self, label: &str) -> Result<Flow> {
        self.exec_program_block_control(label)
            .await?
            .into_flow("EvalContext block")
    }

    #[async_recursion]
    async fn exec_program_block_control(&mut self, label: &str) -> Result<EvaluatorControl> {
        let block = self.prepare_program_block(label)?;
        self.exec_program_block_body(block).await
    }

    fn prepare_program_block(&self, label: &str) -> Result<&'a BlockIr> {
        self.checkpoint_function_entry()?;
        program_block(self.executable, label)
    }

    async fn exec_program_block_body(&mut self, block: &'a BlockIr) -> Result<EvaluatorControl> {
        self.env.push();
        for statement_ref in &block.statements {
            let statement = program_statement_ref(self.executable, statement_ref)?;
            let control = match self.exec_program_statement_control(statement).await {
                Ok(control) => control,
                Err(error) => {
                    self.env.pop();
                    return Err(self
                        .interpreter
                        .attach_program_source_context(error, self.addr, self.file, None));
                }
            };
            if !matches!(control, EvaluatorControl::Complete(Flow::Continue)) {
                self.env.pop();
                return Ok(control);
            }
        }
        self.env.pop();
        Ok(Flow::Continue.into())
    }

    #[async_recursion]
    pub async fn exec_program_statement(&mut self, statement: &LinkedStmtIr) -> Result<Flow> {
        self.exec_program_statement_control(statement)
            .await?
            .into_flow("EvalContext statement")
    }

    #[async_recursion]
    async fn exec_program_statement_control(
        &mut self,
        statement: &LinkedStmtIr,
    ) -> Result<EvaluatorControl> {
        self.checkpoint_generated_chunk(1)?;
        match statement {
            LinkedStmtIr::Timeout {
                duration_ms,
                body,
                site,
            } => self
                .exec_timeout_statement(*duration_ms, body, site)
                .await
                .map(EvaluatorControl::from),
            LinkedStmtIr::Concurrent { plan } => self
                .exec_concurrent_statement(plan)
                .await
                .map(EvaluatorControl::from),
            LinkedStmtIr::Let { slot, value } => {
                let value = self.eval_program_expr_ref(*value).await?;
                self.env.declare_binding(
                    "slot",
                    Some(program_u32_to_usize(*slot, "let.slot")?),
                    value,
                )?;
                Ok(Flow::Continue.into())
            }
            LinkedStmtIr::Assign { target, value } => {
                let value = self.eval_program_expr_ref(*value).await?;
                self.assign_program_target(target, value).await?;
                Ok(Flow::Continue.into())
            }
            LinkedStmtIr::ForIn {
                item_slot,
                item_type,
                value_slot,
                iterable,
                body,
            } => {
                self.exec_program_for_in(
                    program_u32_to_usize(*item_slot, "forIn.itemSlot")?,
                    item_type.as_ref(),
                    value_slot
                        .map(|slot| program_u32_to_usize(slot, "forIn.valueSlot"))
                        .transpose()?,
                    *iterable,
                    body,
                )
                .await
            }
            LinkedStmtIr::Assert { condition, message } => {
                let condition = self.eval_program_expr_ref(*condition).await?;
                if runtime_truthy(&condition, self.heap)? {
                    return Ok(Flow::Continue.into());
                }
                let message = match message {
                    Some(message_ref) => {
                        let message = self.eval_program_expr_ref(*message_ref).await?;
                        runtime_stringify_key(&message, self.heap)?
                    }
                    _ => "assertion failed".to_string(),
                };
                Err(RuntimeError::Decode(message))
            }
            LinkedStmtIr::Break => Ok(Flow::Break.into()),
            LinkedStmtIr::Continue => Ok(Flow::LoopContinue.into()),
            LinkedStmtIr::Spawn { call } => {
                spawn_ops::submit_spawn_statement(self, *call).await?;
                Ok(Flow::Continue.into())
            }
            LinkedStmtIr::Expr { value } => {
                self.eval_program_expr_ref(*value).await?;
                Ok(Flow::Continue.into())
            }
            LinkedStmtIr::Return { value } => self.exec_program_return(*value).await,
            LinkedStmtIr::If {
                condition,
                then_block,
                else_block,
            } => {
                let condition = self.eval_program_expr_ref(*condition).await?;
                let block = if runtime_truthy(&condition, self.heap)? {
                    then_block
                } else if let Some(block) = else_block {
                    block
                } else {
                    return Ok(Flow::Continue.into());
                };
                self.exec_program_block_control(block).await
            }
            LinkedStmtIr::Match { value, arms } => {
                let value = self.eval_program_expr_ref(*value).await?;
                for arm in arms {
                    self.checkpoint_generated_chunk(0)?;
                    if !program_pattern_matches(&arm.pattern, &value, self.heap)? {
                        continue;
                    }
                    self.env.push();
                    if let Err(error) = bind_program_pattern(self.env, &arm.pattern, value.clone())
                    {
                        self.env.pop();
                        return Err(error);
                    }
                    let control = self.exec_program_block_control(&arm.body).await;
                    self.env.pop();
                    return control;
                }
                Ok(Flow::Continue.into())
            }
            LinkedStmtIr::Emit { value, .. } => {
                self.exec_emit(*value).await.map(EvaluatorControl::from)
            }
            LinkedStmtIr::TestEffectRegister {
                target,
                expect,
                step_expect,
                outcome,
            } => {
                if !self.interpreter.test_effects_enabled {
                    return Err(RuntimeError::Unsupported(
                        "test effect setup cannot run outside test execution".to_string(),
                    ));
                }
                let (effect_target, setup_package_build_id) = match target {
                    LinkedCallTarget::PackageDirect { call: target } => (
                        TestEffectTarget::package_callable(
                            target.dependency_package_build_id().clone(),
                            target.package_callable_id().clone(),
                        ),
                        target.caller_package_build_id().clone(),
                    ),
                    LinkedCallTarget::ActivationRelativeService { instruction } => (
                        TestEffectTarget::contract_operation(
                            instruction.operation_id().clone(),
                            instruction.expected_protocol_identity().clone(),
                        ),
                        instruction.caller_package_build_id().clone(),
                    ),
                    _ => {
                        return Err(RuntimeError::InvalidArtifact(
                            "test effect target did not link to a package callable or contract operation"
                                .to_string(),
                        ));
                    }
                };
                let expect = match expect {
                    Some(expected) => {
                        let value = self.eval_program_expr_ref(expected.value).await?;
                        Some(runtime_to_wire(&value, self.heap)?)
                    }
                    None => None,
                };
                let step_expect = match step_expect {
                    Some(expected) => {
                        let value = self.eval_program_expr_ref(expected.value).await?;
                        Some(runtime_to_wire(&value, self.heap)?)
                    }
                    None => None,
                };
                let outcome = match outcome {
                    LinkedTestEffectOutcomeIr::Respond { value, value_type } => {
                        let value = self.eval_program_expr_ref(*value).await?;
                        let value_plan = self
                            .type_projection()
                            .plan_from_linked_nested_ref(value_type, self.addr)?;
                        let value = runtime_to_wire_required_plan(
                            &value,
                            Some(&value_plan),
                            "test effect response",
                            self.heap,
                        )?;
                        RegisteredTestEffectOutcome::Respond { value, value_plan }
                    }
                    LinkedTestEffectOutcomeIr::Throw {
                        value,
                        payload_type,
                    } => {
                        let payload = self.eval_program_expr_ref(*value).await?;
                        let payload_plan = self
                            .type_projection()
                            .plan_from_linked_nested_ref_with_substitutions(
                                payload_type,
                                self.addr,
                                &self.env.type_substitutions,
                            )?;
                        let payload = runtime_carrier_for_plan(
                            payload,
                            &payload_plan,
                            "test effect typed throw",
                            self.heap,
                        )?;
                        RegisteredTestEffectOutcome::Throw(RegisteredTestEffectThrow {
                            failure: RegisteredTestEffectFailure::LocalPayload(payload),
                            setup_heap: self.heap.clone(),
                            setup_package_build_id: setup_package_build_id.clone(),
                        })
                    }
                    LinkedTestEffectOutcomeIr::Stream { values, item_type } => {
                        let item_plan = self
                            .type_projection()
                            .plan_from_linked_nested_ref(item_type, self.addr)?;
                        let mut runtime_values = Vec::with_capacity(values.len());
                        for value in values {
                            let value = self.eval_program_expr_ref(*value).await?;
                            runtime_values.push(runtime_to_wire_required_plan(
                                &value,
                                Some(&item_plan),
                                "test effect stream item",
                                self.heap,
                            )?);
                        }
                        RegisteredTestEffectOutcome::Stream {
                            values: runtime_values,
                            item_plan,
                        }
                    }
                };
                self.interpreter.runtime_test_effects.register(
                    effect_target,
                    RegisteredTestEffect {
                        expect,
                        step_expect,
                        outcome,
                    },
                );
                Ok(Flow::Continue.into())
            }
            LinkedStmtIr::Throw {
                value,
                payload_type,
                site,
            } => self
                .eval_program_throw(*value, payload_type, site)
                .await
                .map(EvaluatorControl::from),
            LinkedStmtIr::Rethrow { exception_slot } => self
                .interpreter
                .eval_program_rethrow_slot(
                    self.env,
                    program_u32_to_usize(*exception_slot, "rethrow.exceptionSlot")?,
                    self.heap,
                )
                .map(EvaluatorControl::from),
        }
    }

    async fn exec_program_return(&mut self, value: Option<ExprRefIr>) -> Result<EvaluatorControl> {
        let Some(value_ref) = value else {
            return Ok(Flow::Return(RuntimeValue::Null.into()).into());
        };
        if self.tail_call_context == TailCallContext::Transparent {
            let expression = program_expression_ref(self.executable, value_ref)?;
            if let LinkedExprIr::Call { call } = expression {
                if let LinkedCallTarget::Executable { addr } = &call.target {
                    if !self.tail_call_has_stream_semantics(call)? {
                        self.checkpoint_generated_chunk(1)?;
                        let mut args = Vec::with_capacity(call.args.len());
                        for arg in &call.args {
                            args.push(self.eval_program_expr_ref(*arg).await?);
                        }
                        let prepared = self.interpreter.prepare_tail_call(
                            self.projection.clone(),
                            self.env,
                            self.addr,
                            self.executable,
                            addr,
                            &call.type_args,
                            &args,
                            call.site.clone(),
                        );
                        let prepared = self.promote_call_site_error(prepared, &call.site)?;
                        if let Some(prepared) = prepared {
                            return Ok(EvaluatorControl::TailCall(prepared));
                        }

                        let result = self
                            .interpreter
                            .call_program_executable_carriers(
                                self.context.clone().with_local_call_site(call.site.clone()),
                                self.heap,
                                self.env,
                                self.addr,
                                addr,
                                &call.type_args,
                                args,
                            )
                            .await;
                        let value = self.promote_call_site_error(result, &call.site)?;
                        return Ok(Flow::Return(value).into());
                    }
                }
            }
        }

        self.eval_program_expr_ref(value_ref)
            .await
            .map(|value| Flow::Return(value).into())
    }

    fn tail_call_has_stream_semantics(&mut self, call: &CallIr) -> Result<bool> {
        for arg in &call.args {
            let expression = program_expression_ref(self.executable, *arg)?;
            if self
                .interpreter
                .resolve_stream_producer_call(
                    self.projection.clone(),
                    self.addr,
                    self.heap,
                    self.env,
                    self.executable,
                    expression,
                )?
                .is_some()
            {
                return Ok(true);
            }
        }
        Ok(self
            .interpreter
            .resolve_stream_producer_from_call(
                self.projection.clone(),
                self.addr,
                self.heap,
                self.env,
                self.executable,
                call,
            )?
            .is_some())
    }

    #[async_recursion]
    pub async fn eval_program_expr_ref(
        &mut self,
        expr_ref: ExprRefIr,
    ) -> Result<RuntimeValueCarrier> {
        let expr = program_expression_ref(self.executable, expr_ref)?;
        self.eval_program_expr(expr).await
    }

    #[async_recursion]
    pub async fn eval_program_expr(&mut self, expr: &LinkedExprIr) -> Result<RuntimeValueCarrier> {
        self.checkpoint_generated_chunk(1)?;
        match expr {
            LinkedExprIr::Timeout {
                duration_ms,
                value,
                site,
            } => {
                self.eval_timeout_expression(*duration_ms, *value, site)
                    .await
            }
            LinkedExprIr::ConcurrentValue { plan } => self.eval_concurrent_value(plan).await,
            LinkedExprIr::Literal { value } => program_literal(value).map(Into::into),
            LinkedExprIr::LoadSlot { slot } => self
                .env
                .get_slot(program_u32_to_usize(*slot, "loadSlot.slot")?),
            LinkedExprIr::Field { object, field } => {
                let object = self.eval_program_expr_ref(*object).await?;
                runtime_member_access_carrier(&object, field, self.heap)
            }
            LinkedExprIr::ActorSelfField { field, .. } => self
                .context
                .actor_execution_frame()
                .ok_or_else(|| {
                    RuntimeError::InvalidArtifact(
                        "Actor self field read requires the current Actor execution token"
                            .to_string(),
                    )
                })?
                .read_field(field)
                .map(Into::into),
            LinkedExprIr::Construct { type_ref, fields } => {
                self.eval_program_construct(type_ref, fields).await
            }
            LinkedExprIr::RepresentationWrap { value, type_ref } => {
                let value = self.eval_program_expr_ref(*value).await?;
                let plan = self.type_projection().representation_wrap_target_plan(
                    self.addr,
                    type_ref,
                    &self.env.type_substitutions,
                )?;
                runtime_representation_wrap_for_plan(value, &plan, "representation wrap", self.heap)
            }
            LinkedExprIr::InterfaceBox {
                value,
                interface,
                source,
            } => {
                self.eval_program_interface_box(*value, interface, source)
                    .await
            }
            LinkedExprIr::MapLiteral { entries } => self.eval_program_map_literal(entries).await,
            LinkedExprIr::ArrayLiteral { items: item_refs } => {
                let mut items = Vec::new();
                for item_ref in item_refs {
                    self.checkpoint_generated_chunk(0)?;
                    items.push(self.eval_program_expr_ref(*item_ref).await?);
                }
                runtime_array_from_carriers(items, self.heap)
            }
            LinkedExprIr::Unary { op, value } => {
                let value = self.eval_program_expr_ref(*value).await?;
                match op {
                    UnaryOpIr::Not => {
                        Ok(RuntimeValue::Bool(!runtime_truthy(&value, self.heap)?).into())
                    }
                    UnaryOpIr::Negate => Ok(runtime_number_value(-runtime_numeric(&value)?).into()),
                }
            }
            LinkedExprIr::Binary { op, left, right } => {
                let op = program_binary_operator(*op);
                if op == "&&" || op == "||" {
                    let left = self.eval_program_expr_ref(*left).await?;
                    return match op {
                        "&&" if !runtime_truthy(&left, self.heap)? => {
                            Ok(RuntimeValue::Bool(false).into())
                        }
                        "&&" => {
                            let right = self.eval_program_expr_ref(*right).await?;
                            Ok(RuntimeValue::Bool(runtime_truthy(&right, self.heap)?).into())
                        }
                        "||" if runtime_truthy(&left, self.heap)? => {
                            Ok(RuntimeValue::Bool(true).into())
                        }
                        "||" => {
                            let right = self.eval_program_expr_ref(*right).await?;
                            Ok(RuntimeValue::Bool(runtime_truthy(&right, self.heap)?).into())
                        }
                        _ => unreachable!("checked logical operator"),
                    };
                }
                let left = self.eval_program_expr_ref(*left).await?;
                let right = self.eval_program_expr_ref(*right).await?;
                runtime_eval_binary(op, left.into_value(), right.into_value(), self.heap)
                    .map(Into::into)
            }
            LinkedExprIr::Call { call } => self.eval_program_call(call).await,
            LinkedExprIr::ValueBlock { block, result } => {
                let flow = self
                    .exec_program_block_with_tail_call_barrier(block)
                    .await?;
                if let Some(value) = FlowCompletionPolicy::value_block_value(flow)? {
                    Ok(value)
                } else {
                    self.eval_program_expr_ref(*result).await
                }
            }
            LinkedExprIr::DbOperation { operation } => {
                let result = self
                    .interpreter
                    .eval_program_db_operation(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        operation,
                    )
                    .await;
                result.map(Into::into)
            }
            LinkedExprIr::DbQuery {
                target,
                query,
                projection,
                ..
            } => {
                let result = self
                    .interpreter
                    .eval_program_db_query_value(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        target,
                        query,
                        projection.as_ref(),
                    )
                    .await;
                result.map(Into::into)
            }
            LinkedExprIr::DbTransaction { transaction } => {
                let result = self
                    .interpreter
                    .eval_program_explicit_db_transaction(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        transaction,
                    )
                    .await;
                result
            }
            LinkedExprIr::DbLeaseClaim { claim } => {
                let result = self
                    .interpreter
                    .eval_program_db_lease_claim(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        claim,
                    )
                    .await;
                result.map(Into::into)
            }
            LinkedExprIr::DbLeaseRead { read } => {
                let result = self
                    .interpreter
                    .eval_program_db_lease_read(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        read,
                    )
                    .await;
                result.map(Into::into)
            }
            LinkedExprIr::LoadConst { const_index } => {
                self.interpreter
                    .eval_program_const(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        *const_index,
                    )
                    .await
            }
            LinkedExprIr::LoadConstAddress { const_addr } => {
                self.interpreter
                    .eval_program_const_addr(self.context.clone(), self.heap, self.env, const_addr)
                    .await
            }
            LinkedExprIr::LoadPackageConst { .. } => Err(RuntimeError::InvalidArtifact(
                "unlinked package constant reached evaluation".to_string(),
            )),
            LinkedExprIr::Throw {
                value,
                payload_type,
                site,
            } => {
                let flow = self.eval_program_throw(*value, payload_type, site).await?;
                FlowCompletionPolicy::non_returning_expression_value(flow, "throw")
            }
            LinkedExprIr::Rethrow { exception_slot } => {
                let flow = self.interpreter.eval_program_rethrow_slot(
                    self.env,
                    program_u32_to_usize(*exception_slot, "rethrow.exceptionSlot")?,
                    self.heap,
                )?;
                FlowCompletionPolicy::non_returning_expression_value(flow, "rethrow")
            }
            LinkedExprIr::Catch {
                try_expression,
                catch_type,
                ..
            } => self.eval_program_catch(*try_expression, catch_type).await,
        }
    }

    async fn exec_program_for_in(
        &mut self,
        item_slot: usize,
        item_type: Option<&LinkedTypeRef>,
        value_slot: Option<usize>,
        iterable_ref: ExprRefIr,
        body: &str,
    ) -> Result<EvaluatorControl> {
        let iterable_expr = program_expression_ref(self.executable, iterable_ref)?;
        if value_slot.is_none() {
            if let Some(producer) = self.interpreter.resolve_stream_producer_call(
                self.projection.clone(),
                self.addr,
                self.heap,
                self.env,
                self.executable,
                iterable_expr,
            )? {
                return self
                    .interpreter
                    .exec_program_stream_producer_for_in(
                        self.projection.clone(),
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        item_slot,
                        body,
                        producer,
                    )
                    .await
                    .map(EvaluatorControl::from);
            }
        }

        let items = self.eval_program_expr_ref(iterable_ref).await?;
        if let Some(value_slot) = value_slot {
            if let Some(entries) = runtime_map_entry_snapshot(&items, self.heap)? {
                return self
                    .exec_program_map_entry_for_in(item_slot, value_slot, body, entries)
                    .await;
            }
            return Err(RuntimeError::Decode(
                "for entry binding requires Map".to_string(),
            ));
        }

        if let Some(items) = runtime_array_item_carriers(&items, self.heap)? {
            return self.exec_program_array_for_in(item_slot, body, items).await;
        }

        let stream_value = runtime_to_wire(&items, self.heap)?;
        if is_stream_value(&stream_value) {
            let stream_item_type = item_type
                .map(|item_type| {
                    self.type_projection()
                        .plan_from_linked_nested_ref(item_type, self.addr)
                })
                .transpose()?;
            let mut cancel_signals = Vec::new();
            if let Some(sink) = self.env.stream_sink.as_ref() {
                cancel_signals.push(sink.cancel_signal());
            }
            let interpreter = self.interpreter;
            let drive_context = self.context.clone();
            let addr = self.addr;
            // If this stream value is backed by a deferred producer (a producer
            // call bound to a value rather than consumed inline), co-drive that
            // producer here so its `emit`s run with their own stream sink.
            return interpreter
                .drive_deferred_stream_producer(drive_context, addr, &stream_value, |supervision| {
                    interpreter.exec_program_stream_for_in(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        item_slot,
                        body,
                        stream_value.clone(),
                        stream_item_type,
                        &cancel_signals,
                        supervision,
                    )
                })
                .await
                .map(EvaluatorControl::from);
        }

        if let Some(keys) = runtime_map_key_snapshot(&items, self.heap)? {
            return self.exec_program_array_for_in(item_slot, body, keys).await;
        }

        Err(RuntimeError::Decode(
            "for iterable must evaluate to array, Map, or Stream".to_string(),
        ))
    }

    async fn exec_program_array_for_in(
        &mut self,
        item_slot: usize,
        body: &str,
        items: Vec<RuntimeValueCarrier>,
    ) -> Result<EvaluatorControl> {
        for item_value in items {
            self.checkpoint_loop_condition(1)?;
            let control = self
                .exec_program_for_in_body_control(item_slot, body, item_value)
                .await?;
            match control {
                EvaluatorControl::Complete(Flow::Continue | Flow::LoopContinue) => {
                    self.checkpoint_loop_backedge(0)?;
                    continue;
                }
                EvaluatorControl::Complete(Flow::Break) => break,
                EvaluatorControl::Complete(flow) => return Ok(flow.into()),
                EvaluatorControl::TailCall(prepared) => {
                    return Ok(EvaluatorControl::TailCall(prepared))
                }
            }
        }
        Ok(Flow::Continue.into())
    }

    async fn exec_program_map_entry_for_in(
        &mut self,
        item_slot: usize,
        value_slot: usize,
        body: &str,
        entries: Vec<(RuntimeValueCarrier, RuntimeValueCarrier)>,
    ) -> Result<EvaluatorControl> {
        for (key_value, entry_value) in entries {
            self.checkpoint_loop_condition(1)?;
            let control = self
                .exec_program_for_in_entry_body(item_slot, value_slot, body, key_value, entry_value)
                .await?;
            match control {
                EvaluatorControl::Complete(Flow::Continue | Flow::LoopContinue) => {
                    self.checkpoint_loop_backedge(0)?;
                    continue;
                }
                EvaluatorControl::Complete(Flow::Break) => break,
                EvaluatorControl::Complete(flow) => return Ok(flow.into()),
                EvaluatorControl::TailCall(prepared) => {
                    return Ok(EvaluatorControl::TailCall(prepared))
                }
            }
        }
        Ok(Flow::Continue.into())
    }

    pub async fn exec_program_for_in_body(
        &mut self,
        item_slot: usize,
        body: &str,
        item_value: RuntimeValueCarrier,
    ) -> Result<Flow> {
        self.exec_program_for_in_body_control(item_slot, body, item_value)
            .await?
            .into_flow("for-in body")
    }

    async fn exec_program_for_in_body_control(
        &mut self,
        item_slot: usize,
        body: &str,
        item_value: RuntimeValueCarrier,
    ) -> Result<EvaluatorControl> {
        self.env.push();
        if let Err(error) = self
            .env
            .declare_binding("slot", Some(item_slot), item_value)
        {
            self.env.pop();
            return Err(error);
        }
        let control = self.exec_program_block_control(body).await;
        self.env.pop();
        control
    }

    async fn exec_program_for_in_entry_body(
        &mut self,
        item_slot: usize,
        value_slot: usize,
        body: &str,
        key_value: RuntimeValueCarrier,
        entry_value: RuntimeValueCarrier,
    ) -> Result<EvaluatorControl> {
        self.env.push();
        if let Err(error) = self.env.declare_binding("slot", Some(item_slot), key_value) {
            self.env.pop();
            return Err(error);
        }
        if let Err(error) = self
            .env
            .declare_binding("slot", Some(value_slot), entry_value)
        {
            self.env.pop();
            return Err(error);
        }
        let control = self.exec_program_block_control(body).await;
        self.env.pop();
        control
    }

    async fn eval_program_construct(
        &mut self,
        type_ref: &LinkedTypeRef,
        field_refs: &std::collections::BTreeMap<String, ExprRefIr>,
    ) -> Result<RuntimeValueCarrier> {
        let mut object_fields = std::collections::BTreeMap::new();
        for (field, value_ref) in field_refs {
            self.checkpoint_generated_chunk(0)?;
            let value = self.eval_program_expr_ref(*value_ref).await?;
            object_fields.insert(field.to_string(), value);
        }
        self.validate_construct_type_ref(type_ref)?;
        let plan = self
            .type_projection()
            .plan_from_linked_nested_ref_with_substitutions(
                type_ref,
                self.addr,
                &self.env.type_substitutions,
            )?;
        let value = runtime_object_from_carriers(object_fields, self.heap)?;
        runtime_carrier_for_plan(value, &plan, "construct", self.heap)
    }

    fn validate_construct_type_ref(&self, type_ref: &LinkedTypeRef) -> Result<()> {
        self.type_projection().validate_construct_type_ref(
            self.addr,
            type_ref,
            &self.env.type_substitutions,
        )
    }

    async fn eval_program_interface_box(
        &mut self,
        value: ExprRefIr,
        interface: &LinkedInterfaceInstantiationRef,
        source: &LinkedBoxSourceIr,
    ) -> Result<RuntimeValueCarrier> {
        let interface_id = linked_interface_instantiation_runtime_id(interface);
        let (carrier, payload_identity) = match source {
            LinkedBoxSourceIr::Local {
                concrete_type,
                method_table,
            } => {
                let payload = self.eval_program_expr_ref(value).await?;
                let table = interface_method_table_from_linked(self.addr, method_table)?;
                if interface_id != table.interface_abi_id() {
                    return Err(RuntimeError::InvalidArtifact(format!(
                        "InterfaceBox target {} does not match method table interface {}",
                        interface_id,
                        table.interface_abi_id()
                    )));
                }
                let (payload, payload_identity) = payload.into_parts();
                (
                    InterfaceCarrier::Local {
                        concrete_type: linked_type_ref_runtime_key(concrete_type),
                        method_table: table,
                        payload,
                    },
                    payload_identity,
                )
            }
            LinkedBoxSourceIr::Remote { .. } => {
                return Err(RuntimeError::InvalidArtifact(
                    "legacy remote interface boxing is not executable".to_string(),
                ))
            }
        };

        let handle = self
            .heap
            .alloc_interface_with_local_payload_identity(
                InterfaceValue::new(interface_id, carrier),
                payload_identity,
            )
            .map_err(RuntimeError::from)?;
        Ok(RuntimeValue::Heap(handle).into())
    }

    async fn eval_program_interface_method_call(
        &mut self,
        call: &CallIr,
        interface: &LinkedInterfaceInstantiationRef,
        method_abi_id: &str,
        slot: u32,
        values: Vec<RuntimeValueCarrier>,
    ) -> Result<RuntimeValueCarrier> {
        let (receiver, args) = values.split_first().ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram interface method {method_abi_id} missing receiver argument"
            ))
        })?;
        let (receiver_handle, interface_value) = self.interface_receiver_value(receiver)?;
        let expected_interface = linked_interface_instantiation_runtime_id(interface);
        if interface_value.interface() != expected_interface {
            return Err(RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram interface method {method_abi_id} expected receiver {}, got {}",
                expected_interface,
                interface_value.interface()
            )));
        }

        match interface_value.carrier() {
            InterfaceCarrier::Local { method_table, .. } => {
                let slot_index = program_u32_to_usize(slot, "interfaceMethod.slot")?;
                let Some(method_slot) = method_table.slots().get(slot_index) else {
                    return Err(RuntimeError::InvalidArtifact(format!(
                        "RuntimeProgram interface method {method_abi_id} slot {slot} is out of bounds"
                    )));
                };
                if method_slot.slot() != slot || method_slot.method_abi_id() != method_abi_id {
                    return Err(RuntimeError::InvalidArtifact(format!(
                        "RuntimeProgram interface method {method_abi_id} slot {slot} does not match method table slot {} ({})",
                        method_slot.slot(),
                        method_slot.method_abi_id()
                    )));
                }
                let target = method_slot.target().clone();
                let payload = self
                    .heap
                    .interface_local_payload_carrier(receiver_handle)?
                    .ok_or_else(|| {
                        RuntimeError::InvalidArtifact(
                            "local interface carrier has no local payload".to_string(),
                        )
                    })?;
                match target {
                    InterfaceMethodTarget::LocalExecutable {
                        executable,
                        receiver_call_abi,
                    } => match receiver_call_abi {
                        InterfaceReceiverCallAbi::ExplicitSelfFirst => {
                            self.interpreter
                                .call_program_executable_with_self_carriers(
                                    self.context.clone().with_local_call_site(call.site.clone()),
                                    self.heap,
                                    self.env,
                                    self.addr,
                                    &executable,
                                    &call.type_args,
                                    payload,
                                    args.to_vec(),
                                )
                                .await
                        }
                    },
                }
            }
            InterfaceCarrier::CallbackCapability(carrier) => {
                self.eval_callback_interface_call(call, carrier, method_abi_id, slot, args)
                    .await
            }
        }
    }

    fn interface_receiver_value(
        &self,
        receiver: &RuntimeValueCarrier,
    ) -> Result<(
        skiff_runtime_model::runtime_value::HeapHandle,
        InterfaceValue,
    )> {
        let RuntimeValue::Heap(handle) = receiver.value() else {
            return Err(RuntimeError::Decode(
                "interface method receiver is not an interface value".to_string(),
            ));
        };
        match self.heap.get(*handle)? {
            HeapNode::Interface(value) => Ok((*handle, value.clone())),
            _ => Err(RuntimeError::Decode(
                "interface method receiver is not an interface value".to_string(),
            )),
        }
    }

    async fn eval_program_map_literal(
        &mut self,
        entry_refs: &std::collections::BTreeMap<String, ExprRefIr>,
    ) -> Result<RuntimeValueCarrier> {
        let mut entries = std::collections::BTreeMap::new();
        for (key, value_ref) in entry_refs {
            self.checkpoint_generated_chunk(0)?;
            let value = self.eval_program_expr_ref(*value_ref).await?;
            entries.insert(RuntimeValueKey::string(key.to_string()), value);
        }
        runtime_map_from_carriers(entries, self.heap)
    }

    async fn eval_program_call(&mut self, call: &CallIr) -> Result<RuntimeValueCarrier> {
        if let Some(op) = program_call_db_op(&call.target) {
            return Err(RuntimeError::Unsupported(format!(
                "old RuntimeProgram db builtin {op} is not supported for object DB; use explicit DbOperation IR"
            )));
        }
        if let LinkedCallTarget::Native { target } = &call.target {
            match self
                .eval_native_call_with_stream_producer_arg(call, target)
                .await
            {
                Ok(Some(value)) => return Ok(value),
                Ok(None) => {}
                Err(error) => return self.promote_call_site_error(Err(error), &call.site),
            }
        }
        if let LinkedCallTarget::Executable { addr } = &call.target {
            if let Some(value) = self
                .eval_executable_call_with_stream_producer_arg(call, addr)
                .await?
            {
                return Ok(value);
            }
        }

        // A stream-producer call whose result is bound to a value (e.g. `const s
        // = producer(...)`) rather than consumed inline by a `for-in` must not
        // run its body eagerly here: its `emit`s need a stream sink. Park it as a
        // deferred producer and hand back the stream value; it is driven when the
        // stream is later consumed.
        if let Some(producer) = self.interpreter.resolve_stream_producer_from_call(
            self.projection.clone(),
            self.addr,
            self.heap,
            self.env,
            self.executable,
            call,
        )? {
            let value = self
                .interpreter
                .prepare_deferred_stream_producer(
                    self.projection.clone(),
                    self.context.clone(),
                    self.heap,
                    self.env,
                    self.addr,
                    self.file,
                    self.executable,
                    producer,
                )
                .await?;
            return Ok(value.into());
        }

        let mut values = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            values.push(self.eval_program_expr_ref(*arg).await?);
        }

        let result = match &call.target {
            LinkedCallTarget::Executable { addr } => {
                self.interpreter
                    .call_program_executable_carriers(
                        self.context.clone().with_local_call_site(call.site.clone()),
                        self.heap,
                        self.env,
                        self.addr,
                        addr,
                        &call.type_args,
                        values,
                    )
                    .await
            }
            LinkedCallTarget::PackageDirect { call: target } => {
                let bypass_test_effect =
                    is_std_http_self_ingress_call(target, &values, self.heap, &self.context)?;
                if self.interpreter.test_effects_enabled && !bypass_test_effect {
                    let effect_target = TestEffectTarget::package_callable(
                        target.dependency_package_build_id().clone(),
                        target.package_callable_id().clone(),
                    );
                    let stream_runtime = self.context.stream_runtime();
                    if let Some(result) = self.interpreter.runtime_test_effects.dispatch_package(
                        &effect_target,
                        &values,
                        Some(&stream_runtime),
                        self.heap,
                        &self.context,
                        &call.site,
                    ) {
                        return result;
                    }
                }
                super::assembly_execution::dispatch_package_direct(self, call, target, values).await
            }
            LinkedCallTarget::ActivationRelativeService { instruction } => {
                self.eval_activation_relative_service_call(call, instruction, values)
                    .await
            }
            LinkedCallTarget::LocalExecutable { .. }
            | LinkedCallTarget::PublicationExecutable { .. }
            | LinkedCallTarget::PackageSymbol { .. }
            | LinkedCallTarget::ActorMethod { .. } => Err(RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram call target {} was not linked before execution",
                program_call_target_kind(&call.target)
            ))),
            LinkedCallTarget::ActorDispatch { plan } => {
                self.eval_actor_dispatch(plan, values).await
            }
            LinkedCallTarget::ServiceDependencySymbol { .. } => Err(RuntimeError::InvalidArtifact(
                "legacy service dependency symbols are not executable".to_string(),
            )),
            LinkedCallTarget::Native { target } => {
                if is_db_builtin_op(&native_target_name(target)) {
                    return Err(RuntimeError::Unsupported(format!(
                        "old RuntimeProgram db builtin {} is not supported for object DB; use explicit DbOperation IR",
                        native_target_name(target)
                    )));
                }
                self.eval_native_prepared_call(call, target, values).await
            }
            LinkedCallTarget::Builtin { op } => {
                if is_db_builtin_op(op) {
                    Err(RuntimeError::Unsupported(format!(
                        "old RuntimeProgram db builtin {op} is not supported for object DB; use explicit DbOperation IR"
                    )))
                } else {
                    let config_context =
                        RuntimeNativeConfigCapabilityContext::new(self.context.config_context());
                    let config_type_arg_plan = resolve_config_builtin_type_arg_plan(
                        self.projection.type_view(),
                        self.addr,
                        self.env.type_substitutions.as_linked_map(),
                        call,
                        op,
                    )?;
                    let return_plan = config_type_arg_plan.clone();
                    let value = NativeDispatch::new()
                        .dispatch_builtin(
                            &config_context,
                            self.addr,
                            op,
                            config_type_arg_plan,
                            values
                                .into_iter()
                                .map(RuntimeValueCarrier::into_value)
                                .collect(),
                            self.heap,
                        )
                        .map_err(RuntimeError::from)?;
                    match return_plan {
                        Some(plan) => runtime_carrier_for_plan(
                            value,
                            &plan,
                            "config builtin return",
                            self.heap,
                        ),
                        None => Ok(value.into()),
                    }
                }
            }
            LinkedCallTarget::ReceiverBuiltin { op } => {
                let receiver = values.first().cloned().ok_or_else(|| {
                    RuntimeError::InvalidArtifact(format!(
                        "RuntimeProgram receiver builtin {} missing receiver argument",
                        op.canonical_key
                    ))
                })?;
                let args = values.into_iter().skip(1).collect::<Vec<_>>();
                ReceiverMethodDispatch::new(self.heap).dispatch_op_carriers(op, receiver, args)
            }
            LinkedCallTarget::InterfaceMethod {
                interface,
                method_abi_id,
                slot,
            } => {
                self.eval_program_interface_method_call(
                    call,
                    interface,
                    method_abi_id,
                    *slot,
                    values,
                )
                .await
            }
            LinkedCallTarget::LocalConstReceiverExecutable {
                const_addr,
                executable_addr,
                receiver_call_abi,
                ..
            } => {
                let receiver = self
                    .interpreter
                    .eval_program_const_addr(self.context.clone(), self.heap, self.env, const_addr)
                    .await?;
                match receiver_call_abi {
                    ReceiverCallAbi::ExplicitSelfFirst => {
                        self.interpreter
                            .call_program_executable_with_self_carriers(
                                self.context.clone().with_local_call_site(call.site.clone()),
                                self.heap,
                                self.env,
                                self.addr,
                                executable_addr,
                                &call.type_args,
                                receiver,
                                values,
                            )
                            .await
                    }
                }
            }
        };
        self.promote_call_site_error(result, &call.site)
    }

    pub(crate) fn account_tail_transfer(&mut self, site: &InstructionSourceSite) -> Result<()> {
        let result: Result<()> = (|| {
            self.context.execution().add_instruction_units(1)?;
            self.context.execution().poll_execution_budget()?;
            Ok(())
        })();
        self.promote_call_site_error(result, site)
    }

    fn promote_call_site_error<T>(
        &mut self,
        result: Result<T>,
        site: &InstructionSourceSite,
    ) -> Result<T> {
        promote_call_site_error(
            &self.projection,
            &self.context,
            self.heap,
            self.addr,
            result,
            site,
        )
    }

    async fn eval_executable_call_with_stream_producer_arg(
        &mut self,
        call: &CallIr,
        callee_addr: &ExecutableAddr,
    ) -> Result<Option<RuntimeValueCarrier>> {
        let mut producers = Vec::with_capacity(call.args.len());
        let mut producer_count = 0usize;
        for arg in &call.args {
            let expr = program_expression_ref(self.executable, *arg)?;
            let producer = self.interpreter.resolve_stream_producer_call(
                self.projection.clone(),
                self.addr,
                self.heap,
                self.env,
                self.executable,
                expr,
            )?;
            producer_count += usize::from(producer.is_some());
            producers.push(producer);
        }
        if producer_count == 0 {
            return Ok(None);
        }
        if producer_count > 1 {
            return Err(RuntimeError::Unsupported(
                "multiple stream-producing executable call arguments are not supported".to_string(),
            ));
        }

        let mut prepared: Option<super::program_stream::PreparedNativeStreamProducer> = None;
        let mut values = Vec::with_capacity(call.args.len());
        for (arg, producer) in call.args.iter().zip(producers) {
            if let Some(producer) = producer {
                let next_prepared = self
                    .interpreter
                    .prepare_native_stream_producer_arg(
                        self.projection.clone(),
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        producer,
                    )
                    .await?;
                let stream_value = match runtime_from_wire(next_prepared.stream_value(), self.heap)
                {
                    Ok(value) => value,
                    Err(error) => {
                        self.interpreter
                            .cancel_prepared_native_stream_producer_arg(&next_prepared);
                        return Err(error);
                    }
                };
                values.push(stream_value.into());
                prepared = Some(next_prepared);
            } else {
                match self.eval_program_expr_ref(*arg).await {
                    Ok(value) => values.push(value),
                    Err(error) => {
                        if let Some(existing) = prepared.as_ref() {
                            self.interpreter
                                .cancel_prepared_native_stream_producer_arg(existing);
                        }
                        return Err(error);
                    }
                }
            }
        }

        let prepared = prepared.expect("producer count was validated before argument evaluation");
        let mut consumer_env = self.env.clone();
        consumer_env.supervise_stream_consumer(
            prepared.stream_value().clone(),
            prepared.consumption_child(),
        );
        let consumer = self.interpreter.call_program_executable_carriers(
            self.context.clone().with_local_call_site(call.site.clone()),
            self.heap,
            &consumer_env,
            self.addr,
            callee_addr,
            &call.type_args,
            values,
        );
        let result = match self
            .interpreter
            .exec_prepared_native_stream_producer_arg(
                self.context.clone(),
                self.addr,
                prepared,
                consumer,
            )
            .await
        {
            Ok(result) => result,
            Err(error) => {
                return Err(materialize_request_heap_owned_runtime_error(
                    error, self.heap,
                )?)
            }
        };
        Ok(Some(result))
    }

    async fn eval_native_call_with_stream_producer_arg(
        &mut self,
        call: &CallIr,
        target: &NativeTarget,
    ) -> Result<Option<RuntimeValueCarrier>> {
        let target_name = native_target_name(target);
        let binding_key = native_target_binding_key(target).unwrap_or(target_name.as_str());
        if binding_key != "std.file.createFromStream" {
            return Ok(None);
        }
        let Some(first_arg) = call.args.first() else {
            return Ok(None);
        };
        let expr = program_expression_ref(self.executable, *first_arg)?;
        let Some(producer) = self.interpreter.resolve_stream_producer_call(
            self.projection.clone(),
            self.addr,
            self.heap,
            self.env,
            self.executable,
            expr,
        )?
        else {
            return Ok(None);
        };

        let native_dispatch = NativeDispatch::new();
        let invocation = resolve_runtime_execution_native_invocation(
            self.interpreter,
            &self.projection,
            self.addr,
            self.env,
            call,
            target,
        )?;
        let stream_arg_plan = invocation.arg_plan(0)?.clone();
        let return_plan = invocation.return_plan()?.clone();
        if !stream_item_plans_match(&producer.item_type, &stream_arg_plan) {
            return Err(RuntimeError::Decode(format!(
                "{target_name} stream producer item type {} is not assignable to {}",
                producer.item_type.label(),
                stream_arg_plan.label()
            )));
        }

        let prepared = self
            .interpreter
            .prepare_native_stream_producer_arg(
                self.projection.clone(),
                self.context.clone(),
                self.heap,
                self.env,
                self.addr,
                self.file,
                self.executable,
                producer,
            )
            .await?;
        let stream_value = match runtime_from_wire_required_plan(
            prepared.stream_value(),
            Some(&stream_arg_plan),
            "std.file.createFromStream source",
            self.heap,
        ) {
            Ok(value) => value,
            Err(error) => {
                self.interpreter
                    .cancel_prepared_native_stream_producer_arg(&prepared);
                return Err(error);
            }
        };
        let mut values = Vec::with_capacity(call.args.len());
        values.push(stream_value);
        for arg in call.args.iter().skip(1) {
            match self.eval_program_expr_ref(*arg).await {
                Ok(value) => values.push(value.into_value()),
                Err(error) => {
                    self.interpreter
                        .cancel_prepared_native_stream_producer_arg(&prepared);
                    return Err(error);
                }
            }
        }
        let native_capability_context =
            project_runtime_execution_native_capability_context_supervised(
                &self.context,
                self.projection.clone(),
                self.env.stream_capability_context(),
                invocation.required_context(),
                prepared.consumption_child(),
            );
        let prepared_native = native_dispatch
            .prepare_resolved_native_call(native_capability_context, invocation, values, self.heap)
            .map_err(RuntimeError::from)?;
        let interpreter = self.interpreter;
        let context = self.context.clone();
        let addr = self.addr;
        let frame = self.context.actor_execution_frame().cloned();
        let execution = self.execution.clone();
        let heap = &mut *self.heap;
        let consumer = async move {
            match prepared_native {
                promoted_runtime::dispatch::PreparedNativeCall::Ready(value) => Ok(value),
                promoted_runtime::dispatch::PreparedNativeCall::ExternalWait(operation) => {
                    let (wait, finalize) = operation.into_parts();
                    let outcome =
                        actual_pending::await_operation(&context, frame, heap, &execution, wait)
                            .await??;
                    finalize.finalize(outcome, heap).map_err(RuntimeError::from)
                }
            }
        };
        let result = interpreter
            .exec_prepared_native_stream_producer_arg(
                self.context.clone(),
                addr,
                prepared,
                consumer,
            )
            .await?;
        runtime_carrier_for_plan(result, &return_plan, "native stream return", self.heap).map(Some)
    }

    async fn eval_program_throw(
        &mut self,
        value: ExprRefIr,
        payload_type: &LinkedTypeRef,
        site: &InstructionSourceSite,
    ) -> Result<Flow> {
        let payload = self.eval_program_expr_ref(value).await?;
        let actual_identity = payload.catch_identity().ok_or_else(|| {
            RuntimeError::InvalidArtifact(
                "throw payload is missing its required runtime catch identity".to_string(),
            )
        })?;
        let allowed = self.type_projection().catch_type_leaves(
            payload_type,
            self.addr,
            &self.env.type_substitutions,
        )?;
        if !catch_identity_matches(actual_identity, &allowed) {
            return Err(RuntimeError::InvalidArtifact(
                "throw payload runtime identity does not match its fully-instantiated linked payload type"
                    .to_string(),
            ));
        }
        let exception = RequestException::local(
            payload,
            site.clone(),
            self.context.exception_stack_for_site(site.clone()),
            self.context.next_exception_correlation()?,
        )
        .map_err(RuntimeError::InvalidArtifact)?;
        Err(RuntimeError::UserException(UserException::new(exception)))
    }

    async fn eval_program_catch(
        &mut self,
        try_expression: ExprRefIr,
        catch_type: &LinkedTypeRef,
    ) -> Result<RuntimeValueCarrier> {
        let leaves = self.type_projection().catch_type_leaves(
            catch_type,
            self.addr,
            &self.env.type_substitutions,
        )?;

        match self.eval_program_expr_ref(try_expression).await {
            Ok(value) => catch_ok(value, self.heap),
            Err(error) => {
                if let Some(exception) = user_exception_for_catch(&error) {
                    if exception
                        .actual_payload_type()
                        .is_some_and(|identity| catch_identity_matches(identity, &leaves))
                    {
                        return catch_err(exception.request().clone(), self.heap);
                    }
                }
                Err(error)
            }
        }
    }

    async fn assign_program_target(
        &mut self,
        target: &AssignTargetIr,
        value: RuntimeValueCarrier,
    ) -> Result<()> {
        match target {
            AssignTargetIr::Slot { slot } => self.env.assign_binding(
                "slot",
                Some(program_u32_to_usize(*slot, "assign target slot")?),
                value,
            ),
            AssignTargetIr::Field { object, field } => {
                let object = self.eval_program_expr_ref(*object).await?;
                let handle = object.as_heap_handle().ok_or_else(|| {
                    RuntimeError::Decode(
                        "field assignment target object must be a heap value".to_string(),
                    )
                })?;
                self.heap
                    .set_object_field_carrier(handle, field.to_string(), value)?;
                Ok(())
            }
            AssignTargetIr::ActorSelfField { field, field_type } => {
                let frame = self
                    .context
                    .actor_execution_frame()
                    .cloned()
                    .ok_or_else(|| {
                        RuntimeError::InvalidArtifact(
                            "Actor self field write requires the current Actor execution token"
                                .to_string(),
                        )
                    })?;
                let projection = self.execution_projection().clone();
                let type_view = projection.type_view();
                frame.write_field(
                    field,
                    field_type,
                    type_view,
                    self.addr,
                    value.value(),
                    self.heap,
                )
            }
            AssignTargetIr::Index { object, index } => {
                let object = self.eval_program_expr_ref(*object).await?;
                let index = self.eval_program_expr_ref(*index).await?;
                assign_program_index_target_carrier(self.heap, &object, &index, value)
            }
        }
    }
}

fn is_std_http_self_ingress_call(
    target: &skiff_runtime_linked_program::LinkedPackageDirectCall,
    values: &[RuntimeValueCarrier],
    heap: &RequestHeap,
    context: &ProgramExecutionContext<'_>,
) -> Result<bool> {
    if !matches!(
        target.package_callable_id().as_str(),
        "pkg-callable:skiff.run/std:std.http.request"
            | "pkg-callable:skiff.run/std:std.http.stream"
    ) {
        return Ok(false);
    }
    let [input] = values else {
        return Ok(false);
    };
    let input = runtime_to_wire(input, heap)?;
    context
        .http_client_context()
        .is_test_http_self_ingress(&input)
        .map_err(RuntimeError::from)
}

fn stream_item_plans_match(
    actual_item: &RuntimeTypePlan,
    expected_stream: &RuntimeTypePlan,
) -> bool {
    match expected_stream.node() {
        RuntimeTypeNode::Stream(expected_item) => {
            runtime_type_plans_match(actual_item, expected_item)
        }
        _ => false,
    }
}

fn runtime_type_plans_match(actual: &RuntimeTypePlan, expected: &RuntimeTypePlan) -> bool {
    match (actual.node(), expected.node()) {
        (RuntimeTypeNode::Alias(actual), RuntimeTypeNode::Alias(expected))
        | (RuntimeTypeNode::Nullable(actual), RuntimeTypeNode::Nullable(expected))
        | (RuntimeTypeNode::Stream(actual), RuntimeTypeNode::Stream(expected))
        | (RuntimeTypeNode::Array(actual), RuntimeTypeNode::Array(expected)) => {
            runtime_type_plans_match(actual, expected)
        }
        (RuntimeTypeNode::Union(actual), RuntimeTypeNode::Union(expected)) => {
            actual.len() == expected.len()
                && actual
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| runtime_type_plans_match(actual, expected))
        }
        (
            RuntimeTypeNode::Map {
                key: actual_key,
                value: actual_value,
            },
            RuntimeTypeNode::Map {
                key: expected_key,
                value: expected_value,
            },
        ) => {
            runtime_type_plans_match(actual_key, expected_key)
                && runtime_type_plans_match(actual_value, expected_value)
        }
        (
            RuntimeTypeNode::Record { fields: actual, .. },
            RuntimeTypeNode::Record {
                fields: expected, ..
            },
        ) => {
            actual.len() == expected.len()
                && actual.iter().zip(expected).all(|(actual, expected)| {
                    actual.name == expected.name
                        && actual.required == expected.required
                        && runtime_type_plans_match(&actual.ty, &expected.ty)
                })
        }
        (
            RuntimeTypeNode::Representation {
                type_name: actual,
                payload: actual_payload,
            },
            RuntimeTypeNode::Representation {
                type_name: expected,
                payload: expected_payload,
            },
        ) => actual == expected && runtime_type_plans_match(actual_payload, expected_payload),
        (RuntimeTypeNode::LiteralString(actual), RuntimeTypeNode::LiteralString(expected)) => {
            actual == expected
        }
        (RuntimeTypeNode::Json, RuntimeTypeNode::Json)
        | (RuntimeTypeNode::JsonObject, RuntimeTypeNode::JsonObject)
        | (RuntimeTypeNode::Bytes, RuntimeTypeNode::Bytes)
        | (RuntimeTypeNode::String, RuntimeTypeNode::String)
        | (RuntimeTypeNode::Bool, RuntimeTypeNode::Bool)
        | (RuntimeTypeNode::Number, RuntimeTypeNode::Number)
        | (RuntimeTypeNode::Integer, RuntimeTypeNode::Integer)
        | (RuntimeTypeNode::Null, RuntimeTypeNode::Null) => true,
        _ => false,
    }
}

fn runtime_map_key_snapshot(
    value: &RuntimeValueCarrier,
    heap: &RequestHeap,
) -> Result<Option<Vec<RuntimeValueCarrier>>> {
    let RuntimeValue::Heap(handle) = value.value() else {
        return Ok(None);
    };
    let node = heap.get(*handle)?;
    let HeapNode::Map(map) = node else {
        return match node {
            HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
                "{} is not iterable as a Map",
                value.diagnostic_label()
            ))),
            _ => Ok(None),
        };
    };
    Ok(Some(
        map.keys()
            .map(runtime_value_from_map_key)
            .map(Into::into)
            .collect(),
    ))
}

fn runtime_map_entry_snapshot(
    value: &RuntimeValueCarrier,
    heap: &RequestHeap,
) -> Result<Option<Vec<(RuntimeValueCarrier, RuntimeValueCarrier)>>> {
    let RuntimeValue::Heap(handle) = value.value() else {
        return Ok(None);
    };
    let node = heap.get(*handle)?;
    let HeapNode::Map(map) = node else {
        return match node {
            HeapNode::Interface(value) => Err(RuntimeError::Decode(format!(
                "{} is not iterable as a Map",
                value.diagnostic_label()
            ))),
            _ => Ok(None),
        };
    };
    map.keys()
        .map(|key| {
            let value = heap.map_entry_carrier(*handle, key)?.ok_or_else(|| {
                RuntimeError::InvalidArtifact(
                    "Map carrier sidecar is missing an existing entry".to_string(),
                )
            })?;
            Ok((runtime_value_from_map_key(key).into(), value))
        })
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn runtime_value_from_map_key(key: &RuntimeValueKey) -> RuntimeValue {
    match key {
        RuntimeValueKey::String(value) => RuntimeValue::String(value.clone()),
    }
}
