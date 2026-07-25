use async_recursion::async_recursion;
use skiff_runtime_linked_program::{
    AssignTargetIr, CallIr, ExecutableAddr, ExprRefIr, LinkedBoxSourceIr, LinkedCallTarget,
    LinkedExecutable, LinkedExprIr, LinkedFileUnit, LinkedInterfaceInstantiationRef,
    LinkedRemoteOperationSlotPlanIr, LinkedRemoteOperationTablePlanIr, LinkedStmtIr,
    LinkedTestEffectOutcomeIr, LinkedTypeRef, NativeTarget, ReceiverCallAbi, UnaryOpIr,
};
use skiff_runtime_linked_type_plan::{
    linked_interface_instantiation_runtime_id, linked_type_ref_runtime_key,
};
use skiff_runtime_model::{
    request_heap::RequestHeap,
    runtime_value::{
        HeapNode, InterfaceCarrier, InterfaceMethodTarget, InterfaceReceiverCallAbi,
        InterfaceValue, RemoteOperationSlot, RemoteOperationTable, RuntimeMap, RuntimeObjectFields,
        RuntimeValue, RuntimeValueKey,
    },
    type_plan::{RuntimeTypeNode, RuntimeTypePlan},
};

use super::{
    assembly_execution::RuntimeExecutionProjection,
    capabilities::{ExecutionControl, RuntimeNativeConfigCapabilityContext},
    env::{check_cancelled, Env, Flow},
    exceptions::{catch_err, catch_ok, exception_envelope_for_catch},
    flow_completion::FlowCompletionPolicy,
    native_capability::{
        project_runtime_execution_native_capability_context,
        project_runtime_execution_native_capability_context_supervised,
    },
    native_invocation::{
        resolve_config_builtin_type_arg_plan, resolve_runtime_execution_native_invocation,
    },
    program_db::{is_db_builtin_op, program_call_db_op},
    program_execution::ProgramExecutionContext,
    program_ir::{
        bind_program_pattern, program_binary_operator, program_block, program_call_target_kind,
        program_expression_ref, program_literal, program_pattern_matches, program_statement_ref,
        program_u32_to_usize,
    },
    program_mutation::assign_program_index_target,
    receiver_methods::ReceiverMethodDispatch,
    recoverable_behavior::interface_method_table_from_linked,
    runtime_ops::{
        runtime_from_wire, runtime_object_from_fields, runtime_to_wire,
        runtime_to_wire_required_plan,
    },
    spawn_ops,
    test_effect_registry::{RegisteredTestEffect, RegisteredTestEffectOutcome, TestEffectTarget},
    type_projection::EvalTypeProjection,
    *,
};
use crate::error::RuntimeError;
use promoted_runtime::dispatch::NativeDispatch;
use skiff_artifact_model::STD_NATIVE_CALLABLE_SEMANTICS;
use skiff_runtime_boundary::stream::is_stream_value;
use skiff_runtime_native as promoted_runtime;
use skiff_runtime_native_contract::{native_target_binding_key, native_target_name};

pub(crate) fn native_call_suspends(binding_key: &str) -> bool {
    if let Some(semantics) = STD_NATIVE_CALLABLE_SEMANTICS
        .iter()
        .find(|semantics| semantics.binding_key == binding_key)
    {
        return semantics.effects.may_suspend;
    }
    // The callable-semantics table is intentionally sparse: it covers native
    // detachment safety, not the complete capability route matrix. These
    // contexts return real futures and therefore form coroutine boundaries.
    binding_key.starts_with("std.file.")
        || binding_key.starts_with("std.actor.")
        || matches!(
            binding_key,
            "std.http.client.stream" | "std.http.client.sse" | "std.http.stream.emitResponse"
        )
}

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
        })
    }

    fn type_projection(&self) -> EvalTypeProjection<'a> {
        EvalTypeProjection::from_execution_projection(self.projection.clone())
    }

    fn suspend_actor_segment(
        &mut self,
    ) -> Result<Option<crate::actor_executor::ActorExecutionFrame>> {
        let Some(frame) = self.context.actor_execution_frame().cloned() else {
            return Ok(None);
        };
        frame.suspend(self.heap)?;
        Ok(Some(frame))
    }

    async fn resume_actor_segment(
        &mut self,
        frame: Option<crate::actor_executor::ActorExecutionFrame>,
    ) -> Result<()> {
        if let Some(frame) = frame {
            frame.resume(self.heap, &self.execution).await?;
        }
        Ok(())
    }

    pub(crate) fn execution_projection(&self) -> &RuntimeExecutionProjection<'a> {
        &self.projection
    }

    fn ensure_legacy_service_path_allowed(&self, path: &str) -> Result<()> {
        if self.projection.assembly().is_some() {
            return Err(RuntimeError::InvalidArtifact(format!(
                "assembly execution cannot use legacy {path}"
            )));
        }
        Ok(())
    }

    pub async fn exec_program_executable(&mut self) -> Result<Flow> {
        self.exec_program_block("entry").await
    }

    #[async_recursion]
    pub async fn exec_program_block(&mut self, label: &str) -> Result<Flow> {
        self.execution.add_instruction_units(1)?;
        check_cancelled(&self.execution, self.env)?;
        let block = program_block(self.executable, label)?;
        self.env.push();
        for statement_ref in &block.statements {
            self.execution.poll_execution_budget()?;
            let statement = program_statement_ref(self.executable, statement_ref)?;
            let flow = match self.exec_program_statement(statement).await {
                Ok(flow) => flow,
                Err(error) => {
                    self.env.pop();
                    return Err(self
                        .interpreter
                        .attach_program_source_context(error, self.addr, self.file, None));
                }
            };
            if !matches!(flow, Flow::Continue) {
                self.env.pop();
                return Ok(flow);
            }
        }
        self.env.pop();
        Ok(Flow::Continue)
    }

    #[async_recursion]
    pub async fn exec_program_statement(&mut self, statement: &LinkedStmtIr) -> Result<Flow> {
        self.execution.add_instruction_units(1)?;
        check_cancelled(&self.execution, self.env)?;
        match statement {
            LinkedStmtIr::Let { slot, value } => {
                let value = self.eval_program_expr_ref(*value).await?;
                self.env.declare_binding(
                    "slot",
                    Some(program_u32_to_usize(*slot, "let.slot")?),
                    value,
                )?;
                Ok(Flow::Continue)
            }
            LinkedStmtIr::Assign { target, value } => {
                let value = self.eval_program_expr_ref(*value).await?;
                self.assign_program_target(target, value).await?;
                Ok(Flow::Continue)
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
                    return Ok(Flow::Continue);
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
            LinkedStmtIr::Break => Ok(Flow::Break),
            LinkedStmtIr::Continue => Ok(Flow::LoopContinue),
            LinkedStmtIr::Spawn { call } => {
                spawn_ops::submit_spawn_statement(self, *call).await?;
                Ok(Flow::Continue)
            }
            LinkedStmtIr::Expr { value } => {
                self.eval_program_expr_ref(*value).await?;
                Ok(Flow::Continue)
            }
            LinkedStmtIr::Return { value } => {
                let value = match value {
                    Some(value_ref) => self.eval_program_expr_ref(*value_ref).await?,
                    None => RuntimeValue::Null,
                };
                Ok(Flow::Return(value))
            }
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
                    return Ok(Flow::Continue);
                };
                self.exec_program_block(block).await
            }
            LinkedStmtIr::Match { value, arms } => {
                let value = self.eval_program_expr_ref(*value).await?;
                for arm in arms {
                    self.execution.poll_execution_budget()?;
                    if !program_pattern_matches(&arm.pattern, &value, self.heap)? {
                        continue;
                    }
                    self.env.push();
                    if let Err(error) = bind_program_pattern(self.env, &arm.pattern, value.clone())
                    {
                        self.env.pop();
                        return Err(error);
                    }
                    let flow = self.exec_program_block(&arm.body).await;
                    self.env.pop();
                    return flow;
                }
                Ok(Flow::Continue)
            }
            LinkedStmtIr::Emit { value, .. } => {
                let value = self.eval_program_expr_ref(*value).await?;
                let sink = self
                    .env
                    .stream_sink
                    .as_ref()
                    .ok_or_else(|| {
                        RuntimeError::Decode(
                            "emit used outside a stream output context".to_string(),
                        )
                    })?
                    .clone();
                if let Some(item) = sink.project_runtime_item(value.clone(), self.heap)? {
                    let frame = self.suspend_actor_segment()?;
                    let result = sink
                        .send_internal_with_cancellation(
                            item,
                            &[],
                            [self.execution.cancellation_token()],
                        )
                        .await;
                    self.resume_actor_segment(frame).await?;
                    result?;
                    return Ok(Flow::Continue);
                }
                let value = runtime_to_wire_required_plan(
                    &value,
                    self.env.current_stream_item_type.as_ref(),
                    "stream emit item",
                    self.heap,
                )?;
                let frame = self.suspend_actor_segment()?;
                let result = sink
                    .send_with_cancellation(value, &[], [self.execution.cancellation_token()])
                    .await;
                self.resume_actor_segment(frame).await?;
                result?;
                Ok(Flow::Continue)
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
                let effect_target = match target {
                    LinkedCallTarget::PackageDirect { call: target } => {
                        TestEffectTarget::package_callable(
                            target.dependency_package_build_id().clone(),
                            target.package_callable_id().clone(),
                        )
                    }
                    LinkedCallTarget::ActivationRelativeService { instruction } => {
                        TestEffectTarget::contract_operation(
                            instruction.operation_id().clone(),
                            instruction.expected_protocol_identity().clone(),
                        )
                    }
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
                        let projection = self.type_projection();
                        let payload_plan =
                            projection.plan_from_linked_nested_ref(payload_type, self.addr)?;
                        let payload = runtime_to_wire_required_plan(
                            &payload,
                            Some(&payload_plan),
                            "test effect typed throw",
                            self.heap,
                        )?;
                        RegisteredTestEffectOutcome::Throw {
                            payload,
                            payload_plan,
                            identity: projection.throw_payload_actual_type(payload_type)?,
                        }
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
                Ok(Flow::Continue)
            }
            LinkedStmtIr::Throw {
                value,
                payload_type,
            } => self.eval_program_throw(*value, payload_type).await,
            LinkedStmtIr::Rethrow { exception_slot } => self.interpreter.eval_program_rethrow_slot(
                self.env,
                program_u32_to_usize(*exception_slot, "rethrow.exceptionSlot")?,
                self.heap,
            ),
        }
    }

    #[async_recursion]
    pub async fn eval_program_expr_ref(&mut self, expr_ref: ExprRefIr) -> Result<RuntimeValue> {
        let expr = program_expression_ref(self.executable, expr_ref)?;
        self.eval_program_expr(expr).await
    }

    #[async_recursion]
    pub async fn eval_program_expr(&mut self, expr: &LinkedExprIr) -> Result<RuntimeValue> {
        self.execution.add_instruction_units(1)?;
        check_cancelled(&self.execution, self.env)?;
        match expr {
            LinkedExprIr::Literal { value } => program_literal(value),
            LinkedExprIr::LoadSlot { slot } => self
                .env
                .get_slot(program_u32_to_usize(*slot, "loadSlot.slot")?),
            LinkedExprIr::Field { object, field } => {
                let object = self.eval_program_expr_ref(*object).await?;
                runtime_member_access(&object, field, self.heap)
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
                .read_field(field),
            LinkedExprIr::Construct { type_ref, fields } => {
                self.eval_program_construct(type_ref, fields).await
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
                    items.push(self.eval_program_expr_ref(*item_ref).await?);
                }
                runtime_array_from_items(items, self.heap)
            }
            LinkedExprIr::Unary { op, value } => {
                let value = self.eval_program_expr_ref(*value).await?;
                match op {
                    UnaryOpIr::Not => Ok(RuntimeValue::Bool(!runtime_truthy(&value, self.heap)?)),
                    UnaryOpIr::Negate => Ok(runtime_number_value(-runtime_numeric(&value)?)),
                }
            }
            LinkedExprIr::Binary { op, left, right } => {
                let op = program_binary_operator(*op);
                if op == "&&" || op == "||" {
                    let left = self.eval_program_expr_ref(*left).await?;
                    return match op {
                        "&&" if !runtime_truthy(&left, self.heap)? => Ok(RuntimeValue::Bool(false)),
                        "&&" => {
                            let right = self.eval_program_expr_ref(*right).await?;
                            Ok(RuntimeValue::Bool(runtime_truthy(&right, self.heap)?))
                        }
                        "||" if runtime_truthy(&left, self.heap)? => Ok(RuntimeValue::Bool(true)),
                        "||" => {
                            let right = self.eval_program_expr_ref(*right).await?;
                            Ok(RuntimeValue::Bool(runtime_truthy(&right, self.heap)?))
                        }
                        _ => unreachable!("checked logical operator"),
                    };
                }
                let left = self.eval_program_expr_ref(*left).await?;
                let right = self.eval_program_expr_ref(*right).await?;
                runtime_eval_binary(op, left, right, self.heap)
            }
            LinkedExprIr::Call { call } => self.eval_program_call(call).await,
            LinkedExprIr::ValueBlock { block, result } => {
                let flow = self.exec_program_block(block).await?;
                if let Some(value) = FlowCompletionPolicy::value_block_value(flow)? {
                    Ok(value)
                } else {
                    self.eval_program_expr_ref(*result).await
                }
            }
            LinkedExprIr::DbOperation { operation } => {
                let frame = self.suspend_actor_segment()?;
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
                self.resume_actor_segment(frame).await?;
                result
            }
            LinkedExprIr::DbQuery {
                target,
                query,
                projection,
                ..
            } => {
                let frame = self.suspend_actor_segment()?;
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
                self.resume_actor_segment(frame).await?;
                result
            }
            LinkedExprIr::DbTransaction { transaction } => {
                let frame = self.suspend_actor_segment()?;
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
                self.resume_actor_segment(frame).await?;
                result
            }
            LinkedExprIr::DbLeaseClaim { claim } => {
                let frame = self.suspend_actor_segment()?;
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
                self.resume_actor_segment(frame).await?;
                result
            }
            LinkedExprIr::DbLeaseRead { read } => {
                let frame = self.suspend_actor_segment()?;
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
                self.resume_actor_segment(frame).await?;
                result
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
            } => {
                let flow = self.eval_program_throw(*value, payload_type).await?;
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
            } => {
                self.eval_program_catch(*try_expression, catch_type.as_ref())
                    .await
            }
        }
    }

    pub async fn exec_program_for_in(
        &mut self,
        item_slot: usize,
        item_type: Option<&LinkedTypeRef>,
        value_slot: Option<usize>,
        iterable_ref: ExprRefIr,
        body: &str,
    ) -> Result<Flow> {
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
                    .await;
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

        if let Some(items) = runtime_array_items(&items, self.heap)? {
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
            let consumer = interpreter.exec_program_stream_for_in(
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
            );
            // If this stream value is backed by a deferred producer (a producer
            // call bound to a value rather than consumed inline), co-drive that
            // producer here so its `emit`s run with their own stream sink.
            return interpreter
                .drive_deferred_stream_producer(drive_context, addr, &stream_value, consumer)
                .await;
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
        items: Vec<RuntimeValue>,
    ) -> Result<Flow> {
        for item_value in items {
            self.execution.add_instruction_units(1)?;
            check_cancelled(&self.execution, self.env)?;
            let flow = self
                .exec_program_for_in_body(item_slot, body, item_value)
                .await?;
            match flow {
                Flow::Continue | Flow::LoopContinue => continue,
                Flow::Break => break,
                Flow::Return(value) => return Ok(Flow::Return(value)),
                Flow::Parked => return Ok(Flow::Parked),
                Flow::ContinueConsumer => return Ok(Flow::ContinueConsumer),
            }
        }
        Ok(Flow::Continue)
    }

    async fn exec_program_map_entry_for_in(
        &mut self,
        item_slot: usize,
        value_slot: usize,
        body: &str,
        entries: Vec<(RuntimeValue, RuntimeValue)>,
    ) -> Result<Flow> {
        for (key_value, entry_value) in entries {
            self.execution.add_instruction_units(1)?;
            check_cancelled(&self.execution, self.env)?;
            let flow = self
                .exec_program_for_in_entry_body(item_slot, value_slot, body, key_value, entry_value)
                .await?;
            match flow {
                Flow::Continue | Flow::LoopContinue => continue,
                Flow::Break => break,
                Flow::Return(value) => return Ok(Flow::Return(value)),
                Flow::Parked => return Ok(Flow::Parked),
                Flow::ContinueConsumer => return Ok(Flow::ContinueConsumer),
            }
        }
        Ok(Flow::Continue)
    }

    pub async fn exec_program_for_in_body(
        &mut self,
        item_slot: usize,
        body: &str,
        item_value: RuntimeValue,
    ) -> Result<Flow> {
        self.env.push();
        if let Err(error) = self
            .env
            .declare_binding("slot", Some(item_slot), item_value)
        {
            self.env.pop();
            return Err(error);
        }
        let flow = self.exec_program_block(body).await;
        self.env.pop();
        flow
    }

    async fn exec_program_for_in_entry_body(
        &mut self,
        item_slot: usize,
        value_slot: usize,
        body: &str,
        key_value: RuntimeValue,
        entry_value: RuntimeValue,
    ) -> Result<Flow> {
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
        let flow = self.exec_program_block(body).await;
        self.env.pop();
        flow
    }

    async fn eval_program_construct(
        &mut self,
        type_ref: &LinkedTypeRef,
        field_refs: &std::collections::BTreeMap<String, ExprRefIr>,
    ) -> Result<RuntimeValue> {
        let mut object_fields = RuntimeObjectFields::new();
        for (field, value_ref) in field_refs {
            let value = self.eval_program_expr_ref(*value_ref).await?;
            object_fields.insert(field.to_string(), value);
        }
        self.validate_construct_type_ref(type_ref)?;
        runtime_object_from_fields(object_fields, self.heap)
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
    ) -> Result<RuntimeValue> {
        let interface_id = linked_interface_instantiation_runtime_id(interface);
        let carrier = match source {
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
                InterfaceCarrier::Local {
                    concrete_type: linked_type_ref_runtime_key(concrete_type),
                    method_table: table,
                    payload,
                }
            }
            LinkedBoxSourceIr::Remote {
                dependency_ref,
                public_instance_key,
                operations,
                ..
            } => {
                self.ensure_legacy_service_path_allowed("remote interface boxing")?;
                let table = self.remote_operation_table_from_linked(
                    dependency_ref,
                    public_instance_key,
                    operations,
                )?;
                if interface_id != table.interface_abi_id() {
                    return Err(RuntimeError::InvalidArtifact(format!(
                        "InterfaceBox target {} does not match remote operation table interface {}",
                        interface_id,
                        table.interface_abi_id()
                    )));
                }
                InterfaceCarrier::Remote {
                    dependency_ref: dependency_ref.clone(),
                    public_instance_key: public_instance_key.clone(),
                    operations: table,
                }
            }
        };

        let handle = self
            .heap
            .alloc_interface(InterfaceValue::new(interface_id, carrier))
            .map_err(RuntimeError::from)?;
        Ok(RuntimeValue::Heap(handle))
    }

    fn remote_operation_table_from_linked(
        &self,
        dependency_ref: &str,
        public_instance_key: &str,
        operations: &LinkedRemoteOperationTablePlanIr,
    ) -> Result<RemoteOperationTable> {
        let interface_id = linked_interface_instantiation_runtime_id(&operations.interface);
        let slots = operations
            .slots
            .iter()
            .map(remote_operation_slot_from_linked)
            .collect::<Result<Vec<_>>>()?;
        Ok(RemoteOperationTable::new(
            remote_operation_table_id(dependency_ref, public_instance_key, &interface_id),
            interface_id,
            slots,
        ))
    }

    async fn eval_program_interface_method_call(
        &mut self,
        call: &CallIr,
        interface: &LinkedInterfaceInstantiationRef,
        method_abi_id: &str,
        slot: u32,
        values: Vec<RuntimeValue>,
    ) -> Result<RuntimeValue> {
        let (receiver, args) = values.split_first().ok_or_else(|| {
            RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram interface method {method_abi_id} missing receiver argument"
            ))
        })?;
        let interface_value = self.interface_receiver_value(receiver)?;
        let expected_interface = linked_interface_instantiation_runtime_id(interface);
        if interface_value.interface() != expected_interface {
            return Err(RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram interface method {method_abi_id} expected receiver {}, got {}",
                expected_interface,
                interface_value.interface()
            )));
        }

        match interface_value.carrier() {
            InterfaceCarrier::Local {
                method_table,
                payload,
                ..
            } => {
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
                let payload = payload.clone();
                match target {
                    InterfaceMethodTarget::LocalExecutable {
                        executable,
                        receiver_call_abi,
                    } => match receiver_call_abi {
                        InterfaceReceiverCallAbi::ExplicitSelfFirst => {
                            self.interpreter
                                .call_program_executable_with_self(
                                    self.context.clone(),
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
            InterfaceCarrier::Remote {
                dependency_ref,
                operations,
                ..
            } => {
                self.ensure_legacy_service_path_allowed("remote interface invocation")?;
                let slot_index = program_u32_to_usize(slot, "interfaceMethod.slot")?;
                let Some(remote_slot) = operations.slots().get(slot_index) else {
                    return Err(RuntimeError::InvalidArtifact(format!(
                        "RuntimeProgram interface method {method_abi_id} slot {slot} is out of bounds"
                    )));
                };
                if remote_slot.slot() != slot || remote_slot.method_abi_id() != method_abi_id {
                    return Err(RuntimeError::InvalidArtifact(format!(
                        "RuntimeProgram interface method {method_abi_id} slot {slot} does not match remote operation table slot {} ({})",
                        remote_slot.slot(),
                        remote_slot.method_abi_id()
                    )));
                }
                let operation_abi_id = remote_slot.operation_abi_id().to_string();
                let outbound_context = self.context.outbound_context();
                let stream_runtime = self.context.stream_runtime();
                let frame = self.suspend_actor_segment()?;
                let result = super::service_dispatch::call_outbound_service_operation(
                    self.interpreter,
                    &outbound_context,
                    &stream_runtime,
                    self.heap,
                    self.env,
                    self.addr,
                    dependency_ref,
                    &operation_abi_id,
                    args.to_vec(),
                )
                .await;
                self.resume_actor_segment(frame).await?;
                result
            }
            InterfaceCarrier::CallbackCapability(carrier) => {
                let frame = self.suspend_actor_segment()?;
                let result = super::assembly_execution::dispatch_callback_capability(
                    self,
                    call,
                    carrier,
                    method_abi_id,
                    slot,
                    args.to_vec(),
                )
                .await;
                self.resume_actor_segment(frame).await?;
                result
            }
        }
    }

    fn interface_receiver_value(&self, receiver: &RuntimeValue) -> Result<InterfaceValue> {
        let RuntimeValue::Heap(handle) = receiver else {
            return Err(RuntimeError::Decode(
                "interface method receiver is not an interface value".to_string(),
            ));
        };
        match self.heap.get(*handle)? {
            HeapNode::Interface(value) => Ok(value.clone()),
            _ => Err(RuntimeError::Decode(
                "interface method receiver is not an interface value".to_string(),
            )),
        }
    }

    async fn eval_program_map_literal(
        &mut self,
        entry_refs: &std::collections::BTreeMap<String, ExprRefIr>,
    ) -> Result<RuntimeValue> {
        let mut entries = RuntimeMap::new();
        for (key, value_ref) in entry_refs {
            let value = self.eval_program_expr_ref(*value_ref).await?;
            entries.insert(RuntimeValueKey::string(key.to_string()), value);
        }
        runtime_map_from_entries(entries, self.heap)
    }

    async fn eval_program_call(&mut self, call: &CallIr) -> Result<RuntimeValue> {
        if let Some(op) = program_call_db_op(&call.target) {
            return Err(RuntimeError::Unsupported(format!(
                "old RuntimeProgram db builtin {op} is not supported for object DB; use explicit DbOperation IR"
            )));
        }
        if let LinkedCallTarget::Native { target } = &call.target {
            if let Some(value) = self
                .eval_native_call_with_stream_producer_arg(call, target)
                .await?
            {
                return Ok(value);
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
            return Ok(value);
        }

        let mut values = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            values.push(self.eval_program_expr_ref(*arg).await?);
        }

        match &call.target {
            LinkedCallTarget::Executable { addr } => {
                self.interpreter
                    .call_program_executable(
                        self.context.clone(),
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
                if self.interpreter.test_effects_enabled {
                    let effect_target = TestEffectTarget::package_callable(
                        target.dependency_package_build_id().clone(),
                        target.package_callable_id().clone(),
                    );
                    if let Some(result) = self.interpreter.runtime_test_effects.dispatch(
                        &effect_target,
                        &values,
                        Some(&self.interpreter.stream_runtime),
                        self.heap,
                    ) {
                        return result;
                    }
                }
                super::assembly_execution::dispatch_package_direct(self, call, target, values).await
            }
            LinkedCallTarget::ActivationRelativeService { instruction } => {
                if self.interpreter.test_effects_enabled {
                    let effect_target = TestEffectTarget::contract_operation(
                        instruction.operation_id().clone(),
                        instruction.expected_protocol_identity().clone(),
                    );
                    if let Some(result) = self.interpreter.runtime_test_effects.dispatch(
                        &effect_target,
                        &values,
                        Some(&self.interpreter.stream_runtime),
                        self.heap,
                    ) {
                        return result;
                    }
                }
                let frame = self.suspend_actor_segment()?;
                let result = super::assembly_execution::dispatch_service_call(
                    self,
                    call,
                    instruction,
                    values,
                )
                .await;
                self.resume_actor_segment(frame).await?;
                result
            }
            LinkedCallTarget::LocalExecutable { .. }
            | LinkedCallTarget::PublicationExecutable { .. }
            | LinkedCallTarget::PackageSymbol { .. }
            | LinkedCallTarget::ActorMethod { .. } => Err(RuntimeError::InvalidArtifact(format!(
                "RuntimeProgram call target {} was not linked before execution",
                program_call_target_kind(&call.target)
            ))),
            LinkedCallTarget::ActorDispatch { plan } => {
                let frame = self.suspend_actor_segment()?;
                let result = crate::actor_dispatch::dispatch_actor_method(self, plan, values).await;
                self.resume_actor_segment(frame).await?;
                result
            }
            LinkedCallTarget::ExternalServiceSymbol { symbol } => {
                Err(RuntimeError::InvalidArtifact(format!(
                    "RuntimeProgram external service call {} must use service dependency symbols",
                    symbol.symbol_path()
                )))
            }
            LinkedCallTarget::ServiceDependencySymbol { symbol } => {
                self.ensure_legacy_service_path_allowed("service dependency dispatch")?;
                let outbound_context = self.context.outbound_context();
                let stream_runtime = self.context.stream_runtime();
                let frame = self.suspend_actor_segment()?;
                let result = super::service_dispatch::call_outbound_service(
                    self.interpreter,
                    &outbound_context,
                    &stream_runtime,
                    self.heap,
                    self.env,
                    self.addr,
                    call,
                    symbol,
                    values,
                )
                .await;
                self.resume_actor_segment(frame).await?;
                result
            }
            LinkedCallTarget::Native { target } => {
                if is_db_builtin_op(&native_target_name(target)) {
                    return Err(RuntimeError::Unsupported(format!(
                        "old RuntimeProgram db builtin {} is not supported for object DB; use explicit DbOperation IR",
                        native_target_name(target)
                    )));
                }
                let native_dispatch = NativeDispatch::new();
                let invocation = resolve_runtime_execution_native_invocation(
                    self.interpreter,
                    &self.projection,
                    self.addr,
                    self.env,
                    call,
                    target,
                )?;
                let suspends = native_call_suspends(invocation.binding_key());
                let frame = if suspends {
                    self.suspend_actor_segment()?
                } else {
                    None
                };
                let native_capability_context = project_runtime_execution_native_capability_context(
                    &self.context,
                    self.projection.clone(),
                    self.env.stream_capability_context(),
                    invocation.required_context(),
                );
                let result = native_dispatch
                    .dispatch_resolved_native_call(
                        native_capability_context,
                        invocation,
                        values,
                        self.heap,
                    )
                    .await
                    .map_err(RuntimeError::from);
                self.resume_actor_segment(frame).await?;
                result
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
                    NativeDispatch::new()
                        .dispatch_builtin(
                            &config_context,
                            self.addr,
                            op,
                            config_type_arg_plan,
                            values,
                            self.heap,
                        )
                        .map_err(RuntimeError::from)
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
                ReceiverMethodDispatch::new(self.heap).dispatch_op(op, receiver, args)
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
                            .call_program_executable_with_self(
                                self.context.clone(),
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
        }
    }

    async fn eval_executable_call_with_stream_producer_arg(
        &mut self,
        call: &CallIr,
        callee_addr: &ExecutableAddr,
    ) -> Result<Option<RuntimeValue>> {
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
                values.push(stream_value);
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
        let consumer = self.interpreter.call_program_executable(
            self.context.clone(),
            self.heap,
            self.env,
            self.addr,
            callee_addr,
            &call.type_args,
            values,
        );
        let result = self
            .interpreter
            .exec_prepared_native_stream_producer_arg(
                self.context.clone(),
                self.addr,
                prepared,
                consumer,
            )
            .await?;
        Ok(Some(result))
    }

    async fn eval_native_call_with_stream_producer_arg(
        &mut self,
        call: &CallIr,
        target: &NativeTarget,
    ) -> Result<Option<RuntimeValue>> {
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
                Ok(value) => values.push(value),
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
        let interpreter = self.interpreter;
        let context = self.context.clone();
        let addr = self.addr;
        let heap = &mut *self.heap;
        let consumer = async move {
            native_dispatch
                .dispatch_resolved_native_call(native_capability_context, invocation, values, heap)
                .await
                .map_err(RuntimeError::from)
        };
        let result = interpreter
            .exec_prepared_native_stream_producer_arg(context, addr, prepared, consumer)
            .await?;
        Ok(Some(result))
    }

    async fn eval_program_throw(
        &mut self,
        value: ExprRefIr,
        payload_type: &LinkedTypeRef,
    ) -> Result<Flow> {
        let payload = self.eval_program_expr_ref(value).await?;
        let payload_json = runtime_to_wire(&payload, self.heap)?;
        let actual_payload_type = self.resolve_throw_payload_actual_type(payload_type)?;
        Err(RuntimeError::UserException(
            UserException::from_typed_payload(
                payload_json,
                actual_payload_type.clone(),
                Some(actual_payload_type),
            )?,
        ))
    }

    fn resolve_throw_payload_actual_type(
        &self,
        payload_type: &LinkedTypeRef,
    ) -> Result<crate::error::TypeIdentity> {
        self.type_projection()
            .throw_payload_actual_type(payload_type)
    }

    async fn eval_program_catch(
        &mut self,
        try_expression: ExprRefIr,
        catch_type: Option<&LinkedTypeRef>,
    ) -> Result<RuntimeValue> {
        let leaves = match catch_type {
            Some(ty) => self.type_projection().catch_type_leaves(ty)?,
            None => Vec::new(),
        };

        match self.eval_program_expr_ref(try_expression).await {
            Ok(value) => catch_ok(value, self.heap),
            Err(error) => {
                if let Some(envelope) = exception_envelope_for_catch(&error, &leaves)? {
                    return catch_err(envelope, self.heap);
                }
                Err(error)
            }
        }
    }

    async fn assign_program_target(
        &mut self,
        target: &AssignTargetIr,
        value: RuntimeValue,
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
                apply_collection_mutation(
                    self.heap,
                    handle,
                    CollectionMutation::ObjectSetField {
                        field: field.to_string(),
                        value,
                    },
                )?;
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
                frame.write_field(field, field_type, type_view, self.addr, &value, self.heap)
            }
            AssignTargetIr::Index { object, index } => {
                let object = self.eval_program_expr_ref(*object).await?;
                let index = self.eval_program_expr_ref(*index).await?;
                assign_program_index_target(self.heap, &object, &index, value)
            }
        }
    }
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

fn remote_operation_table_id(
    dependency_ref: &str,
    public_instance_key: &str,
    interface_id: &str,
) -> String {
    format!("remote-operation-table:{dependency_ref}/{public_instance_key}:{interface_id}")
}

fn remote_operation_slot_from_linked(
    slot: &LinkedRemoteOperationSlotPlanIr,
) -> Result<RemoteOperationSlot> {
    Ok(RemoteOperationSlot::new(
        slot.slot,
        slot.method_abi_id.clone(),
        slot.operation_abi_id.clone(),
    ))
}

fn runtime_map_key_snapshot(
    value: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<Option<Vec<RuntimeValue>>> {
    let RuntimeValue::Heap(handle) = value else {
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
    Ok(Some(map.keys().map(runtime_value_from_map_key).collect()))
}

fn runtime_map_entry_snapshot(
    value: &RuntimeValue,
    heap: &RequestHeap,
) -> Result<Option<Vec<(RuntimeValue, RuntimeValue)>>> {
    let RuntimeValue::Heap(handle) = value else {
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
        map.iter()
            .map(|(key, value)| (runtime_value_from_map_key(key), value.clone()))
            .collect(),
    ))
}

fn runtime_value_from_map_key(key: &RuntimeValueKey) -> RuntimeValue {
    match key {
        RuntimeValueKey::String(value) => RuntimeValue::String(value.clone()),
    }
}
