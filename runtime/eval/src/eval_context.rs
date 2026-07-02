use async_recursion::async_recursion;
use skiff_runtime_linked_program::{
    AssignTargetIr, CallIr, ExecutableAddr, ExprRefIr, LinkedBoxSourceIr, LinkedCallTarget,
    LinkedExecutable, LinkedExprIr, LinkedFileUnit, LinkedInterfaceInstantiationRef,
    LinkedRemoteOperationSlotPlanIr, LinkedRemoteOperationTablePlanIr, LinkedStmtIr, LinkedTypeRef,
    NativeTarget, ReceiverCallAbi, UnaryOpIr,
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
    capabilities::{ExecutionControl, RuntimeNativeConfigCapabilityContext},
    env::{check_cancelled, Env, Flow},
    exceptions::{catch_err, catch_ok, exception_envelope_for_catch},
    flow_completion::FlowCompletionPolicy,
    invocation::EvalProgramProjection,
    native_capability::project_runtime_native_capability_context,
    native_invocation::{resolve_config_builtin_type_arg_plan, resolve_runtime_native_invocation},
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
    runtime_ops::{runtime_from_wire, runtime_object_from_fields, runtime_to_wire_required_plan},
    spawn_ops,
    type_projection::EvalTypeProjection,
    *,
};
use crate::error::RuntimeError;
use promoted_runtime::dispatch::NativeDispatch;
use skiff_runtime_boundary::stream::is_stream_value;
use skiff_runtime_native as promoted_runtime;
use skiff_runtime_native_contract::{native_target_binding_key, native_target_name};

pub struct EvalContext<'a> {
    pub interpreter: &'a Interpreter,
    program: EvalProgramProjection<'a>,
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
    ) -> Self {
        let program = interpreter.program.projection();
        let execution = context.execution();
        Self {
            interpreter,
            program,
            context,
            execution,
            heap,
            env,
            addr,
            file,
            executable,
        }
    }

    fn type_projection(&self) -> EvalTypeProjection<'a> {
        EvalTypeProjection::new(self.program)
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
                let sink = self.env.stream_sink.as_ref().ok_or_else(|| {
                    RuntimeError::Decode("emit used outside a stream output context".to_string())
                })?;
                let value = runtime_to_wire_required_plan(
                    &value,
                    self.env.current_stream_item_type.as_ref(),
                    "stream emit item",
                    self.heap,
                )?;
                sink.send_with_cancel(value, &[self.execution.cancel_flag()])
                    .await?;
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
                self.interpreter
                    .eval_program_db_operation(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        operation,
                    )
                    .await
            }
            LinkedExprIr::DbQuery {
                target,
                query,
                projection,
                ..
            } => {
                self.interpreter
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
                    .await
            }
            LinkedExprIr::DbTransaction { transaction } => {
                self.interpreter
                    .eval_program_explicit_db_transaction(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        transaction,
                    )
                    .await
            }
            LinkedExprIr::DbLeaseClaim { claim } => {
                self.interpreter
                    .eval_program_db_lease_claim(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        claim,
                    )
                    .await
            }
            LinkedExprIr::DbLeaseRead { read } => {
                self.interpreter
                    .eval_program_db_lease_read(
                        self.context.clone(),
                        self.heap,
                        self.env,
                        self.addr,
                        self.file,
                        self.executable,
                        read,
                    )
                    .await
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
                self.program,
                self.addr,
                self.heap,
                self.env,
                self.executable,
                iterable_expr,
            )? {
                return self
                    .interpreter
                    .exec_program_stream_producer_for_in(
                        self.program,
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
                super::service_dispatch::call_outbound_service_operation(
                    self.interpreter,
                    &outbound_context,
                    self.heap,
                    self.env,
                    self.addr,
                    dependency_ref,
                    &operation_abi_id,
                    args.to_vec(),
                )
                .await
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
            self.program,
            self.addr,
            self.heap,
            self.env,
            self.executable,
            call,
        )? {
            let value = self
                .interpreter
                .prepare_deferred_stream_producer(
                    self.program,
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
            LinkedCallTarget::LocalExecutable { .. } | LinkedCallTarget::PackageSymbol { .. } => {
                Err(RuntimeError::InvalidArtifact(format!(
                    "RuntimeProgram call target {} was not linked before execution",
                    program_call_target_kind(&call.target)
                )))
            }
            LinkedCallTarget::ExternalServiceSymbol { symbol } => {
                Err(RuntimeError::InvalidArtifact(format!(
                    "RuntimeProgram external service call {} must use service dependency symbols",
                    symbol.symbol_path()
                )))
            }
            LinkedCallTarget::ServiceDependencySymbol { symbol } => {
                let outbound_context = self.context.outbound_context();
                super::service_dispatch::call_outbound_service(
                    self.interpreter,
                    &outbound_context,
                    self.heap,
                    self.env,
                    self.addr,
                    call,
                    symbol,
                    values,
                )
                .await
            }
            LinkedCallTarget::Native { target } => {
                if is_db_builtin_op(&native_target_name(target)) {
                    return Err(RuntimeError::Unsupported(format!(
                        "old RuntimeProgram db builtin {} is not supported for object DB; use explicit DbOperation IR",
                        native_target_name(target)
                    )));
                }
                let native_dispatch = NativeDispatch::new();
                let invocation = resolve_runtime_native_invocation(
                    self.interpreter,
                    self.addr,
                    self.env,
                    call,
                    target,
                )?;
                let native_capability_context = project_runtime_native_capability_context(
                    &self.context,
                    self.env.stream_capability_context(),
                    invocation.required_context(),
                );
                native_dispatch
                    .dispatch_resolved_native_call(
                        native_capability_context,
                        invocation,
                        values,
                        self.heap,
                    )
                    .await
                    .map_err(RuntimeError::from)
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
                        self.program.type_view(),
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
        let mut prepared: Option<super::program_stream::PreparedNativeStreamProducer> = None;
        let mut values = Vec::with_capacity(call.args.len());
        for arg in &call.args {
            let expr = program_expression_ref(self.executable, *arg)?;
            let producer = self.interpreter.resolve_stream_producer_call(
                self.program,
                self.addr,
                self.heap,
                self.env,
                self.executable,
                expr,
            )?;
            if let Some(producer) = producer {
                if let Some(existing) = prepared.as_ref() {
                    self.interpreter
                        .cancel_prepared_native_stream_producer_arg(existing);
                    return Err(RuntimeError::Unsupported(
                        "multiple stream-producing executable call arguments are not supported"
                            .to_string(),
                    ));
                }
                let next_prepared = self
                    .interpreter
                    .prepare_native_stream_producer_arg(
                        self.program,
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

        let Some(prepared) = prepared else {
            return Ok(None);
        };
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
            self.program,
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
        let invocation =
            resolve_runtime_native_invocation(self.interpreter, self.addr, self.env, call, target)?;
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
                self.program,
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
        let native_capability_context = project_runtime_native_capability_context(
            &self.context,
            self.env.stream_capability_context(),
            invocation.required_context(),
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
