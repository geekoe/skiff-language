use std::collections::BTreeMap;

use skiff_artifact_model::{
    bytecode::encode_instruction, bytecode::limits, contract_for_opcode, descriptor_for_opcode,
    AssignTargetIr, BoxSourceIr, BytecodeFunctionOrigin, BytecodeIntrinsicRef, BytecodeRelocation,
    BytecodeSpecialization, CallTargetIr, CatchMatcher, ExceptionRegion, ExprIr, ExprRefIr,
    FrameLayout, HostEffectReference, HostEffectSignature, InstructionSourceSite,
    IntrinsicReference, LiteralIr, LocalInterfaceMethod, LocalInterfaceRef, Opcode, ParamModeIr,
    ParameterSlotDecl, PatternIr, RelocatableBytecodeFunction, RemoteInterfaceMethod,
    RemoteInterfaceRef, ResumeDescriptor, ResumeErrorMode, SourceMapEntry, StatementEntry,
    SyntheticInstructionSiteReason, TrapFailureKind, TypeRefIr, WritablePathSegment,
};
use skiff_compiler_lowering::mir::{
    MirCallArgument, MirDirectCallFacts, MirEmissionAnchor, MirExpression, MirForInItemKind,
    MirFunction, MirIndexReceiverKind, MirParamMode, MirSlot, MirSlotKind, MirSourceEvent,
    MirStmtKind, MirUnit, MirWritablePathSegment, MirWritablePlace, MirWritableRoot,
};

use super::inputs::is_void;
use super::{
    constants::ConstantImage, inputs::ValidatedEmissionInputs, BytecodeEmissionError,
    FunctionValueTransferPlans,
};

pub(crate) fn emit_functions(
    inputs: &ValidatedEmissionInputs<'_>,
    image: &mut ConstantImage,
) -> Result<BTreeMap<String, RelocatableBytecodeFunction>, BytecodeEmissionError> {
    let mut functions = BTreeMap::new();
    for (function_key, function) in &inputs.functions {
        let unit = inputs
            .units
            .get(function.origin.module_path.as_str())
            .ok_or_else(|| BytecodeEmissionError::CanonicalSerialization {
                context: format!("function `{function_key}` owner"),
                message: "MIR unit disappeared from validated inputs".to_string(),
            })?;
        let plans = inputs.function_plans.get(function_key).ok_or_else(|| {
            BytecodeEmissionError::MissingValueTransferPlans {
                function_key: function_key.clone(),
            }
        })?;
        let emitter = FunctionEmitter::new(unit, function, function_key, plans, image, inputs)?;
        functions.insert(function_key.clone(), emitter.emit()?);
    }
    Ok(functions)
}

struct FunctionEmitter<'a> {
    unit: &'a MirUnit,
    function: &'a MirFunction,
    key: String,
    plans: &'a FunctionValueTransferPlans,
    image: &'a mut ConstantImage,
    inputs: &'a ValidatedEmissionInputs<'a>,
    instructions: Vec<RawInstruction>,
    relocations: Vec<BytecodeRelocation>,
    pending_branches: Vec<PendingBranch>,
    pending_pc_branches: Vec<PendingPcBranch>,
    pending_resumes: Vec<PendingResume>,
    pending_exception_regions: Vec<PendingExceptionRegion>,
    block_starts: Vec<Option<usize>>,
    events: Vec<MirSourceEvent>,
    event_mapping: Vec<Option<usize>>,
    expression_emissions: BTreeMap<u32, u32>,
    current_block: u32,
    generated_slots: Vec<MirSlot>,
    loop_backedges: BTreeMap<u32, LoopBackedge>,
    operand_depth: usize,
}

struct RawInstruction {
    opcode: Opcode,
    operands: Vec<u32>,
}

struct PendingBranch {
    instruction: usize,
    operand: usize,
    block: u32,
}

struct PendingPcBranch {
    instruction: usize,
    operand: usize,
    target_instruction: usize,
}

struct PendingResume {
    instruction: usize,
    operand: usize,
    expected_stack_height_before_result: u32,
    result_ty: Option<TypeRefIr>,
    end_block: Option<u32>,
}

struct PendingExceptionRegion {
    start_instruction: usize,
    handler_instruction: usize,
    catch_slot: u32,
    catch_type: TypeRefIr,
    cleanup_depth: u32,
}

#[derive(Clone)]
struct LoopBackedge {
    header_instruction: usize,
    iterable_slot: u32,
    index_slot: u32,
    item_slot: u32,
    value_slot: Option<u32>,
    array: bool,
    stream: bool,
}

impl<'a> FunctionEmitter<'a> {
    fn new(
        unit: &'a MirUnit,
        function: &'a MirFunction,
        key: &str,
        plans: &'a FunctionValueTransferPlans,
        image: &'a mut ConstantImage,
        inputs: &'a ValidatedEmissionInputs<'a>,
    ) -> Result<Self, BytecodeEmissionError> {
        if !function.type_params.is_empty() || function.self_type.is_some() {
            return Err(unsupported(
                key,
                "generic or receiver-bound function emission",
                "non-generic scalar core only",
            ));
        }
        let events = function
            .source_event_plan
            .events()
            .map(<[MirSourceEvent]>::to_vec)
            .unwrap_or_default();
        let event_count = events.len();
        Ok(Self {
            unit,
            function,
            key: key.to_string(),
            plans,
            image,
            inputs,
            instructions: Vec::new(),
            relocations: Vec::new(),
            pending_branches: Vec::new(),
            pending_pc_branches: Vec::new(),
            pending_resumes: Vec::new(),
            pending_exception_regions: Vec::new(),
            block_starts: vec![None; function.blocks.len()],
            events,
            event_mapping: vec![None; event_count],
            expression_emissions: BTreeMap::new(),
            current_block: 0,
            generated_slots: Vec::new(),
            loop_backedges: BTreeMap::new(),
            operand_depth: 0,
        })
    }

    fn emit(mut self) -> Result<RelocatableBytecodeFunction, BytecodeEmissionError> {
        for block in &self.function.blocks {
            let ordinal = usize::try_from(block.id)
                .map_err(|_| arithmetic(self.key.as_str(), "block id to usize conversion"))?;
            let start = self.instructions.len();
            self.block_starts[ordinal] = Some(start);
            self.current_block = block.id;
            self.emit_block(block)?;
        }
        self.patch_branches()?;
        let instruction_pcs = self.instruction_pcs()?;
        self.patch_pc_branches(&instruction_pcs)?;
        self.patch_resumes(&instruction_pcs)?;
        let exception_regions = self.build_exception_regions(&instruction_pcs)?;
        let (words, instruction_pcs) = self.encode()?;
        let max_operand_depth = self.compute_max_operand_depth()?;
        check_limit(
            "MAX_WORDS_PER_FUNCTION",
            &format!("function `{key}` words", key = self.key),
            words.len(),
            limits::MAX_WORDS_PER_FUNCTION,
        )?;
        let statement_entries = self.build_statement_entries(&instruction_pcs)?;
        let source_map = self.build_source_map(words.len())?;
        let frame = self.build_frame()?;
        let origin = BytecodeFunctionOrigin::Executable {
            executable: self.function.origin.clone(),
        };
        Ok(RelocatableBytecodeFunction {
            function_key: self.key.clone(),
            origin,
            type_parameters: Vec::new(),
            self_type_ref: None,
            words,
            relocations: self.relocations,
            call_loan_layouts: Vec::new(),
            frame_layout: frame,
            max_operand_depth,
            effect_summary_ref: self.function.effect_summary_ref.clone(),
            exception_regions,
            active_regions: Vec::new(),
            switch_tables: Vec::new(),
            statement_entries,
            source_map,
        })
    }

    fn emit_block(
        &mut self,
        block: &skiff_compiler_lowering::mir::MirBlock,
    ) -> Result<(), BytecodeEmissionError> {
        for statement in &block.statements {
            self.map_statement_events(statement.statement_index);
            self.emit_statement(statement)?;
        }

        let has_budget_checkpoint = self.events.iter().any(|event| {
            matches!(
                event.anchor,
                MirEmissionAnchor::BudgetCheckpoint { edge, .. }
                    if edge.from_block() == block.id
            )
        });
        if has_budget_checkpoint {
            let instruction = self.emit_op(Opcode::BudgetCheckpoint, Vec::new())?;
            for (index, event) in self.events.iter().enumerate() {
                if let MirEmissionAnchor::BudgetCheckpoint { edge, .. } = event.anchor {
                    if edge.from_block() == block.id {
                        self.event_mapping[index].get_or_insert(instruction);
                    }
                }
            }
        }

        if !block.successors.is_empty() {
            if let Some(backedge) = self.loop_backedges.get(&block.id).cloned() {
                self.emit_loop_backedge(block.id, &backedge)?;
                return Ok(());
            }
        }
        {
            match block.successors.as_slice() {
                [] => {}
                [successor] => self.emit_jump_to_block(*successor)?,
                _ => {}
            }
        }
        if block.successors.is_empty()
            && self.function.stream_result.is_some()
            && self
                .instructions
                .last()
                .is_some_and(|instruction| instruction.opcode == Opcode::EmitStream)
        {
            self.emit_op(Opcode::Return, Vec::new())?;
        }
        Ok(())
    }

    fn emit_statement(
        &mut self,
        statement: &skiff_compiler_lowering::mir::MirStmt,
    ) -> Result<(), BytecodeEmissionError> {
        match &statement.kind {
            MirStmtKind::InitSlot { slot, value } => {
                self.emit_expression(*value)?;
                self.emit_op(Opcode::StoreSlot, vec![*slot])?;
            }
            MirStmtKind::Assign {
                target: AssignTargetIr::Slot { slot },
                value,
                ..
            } => {
                self.emit_expression(*value)?;
                self.emit_op(Opcode::StoreSlot, vec![*slot])?;
            }
            MirStmtKind::Assign { place, value, .. } => {
                self.emit_writable_assign(place, *value)?;
            }
            MirStmtKind::Assert { condition, .. } => {
                self.emit_expression(*condition)?;
                self.emit_op(Opcode::Trap, vec![TrapFailureKind::Assertion as u32])?;
            }
            MirStmtKind::Emit { operation, value } => {
                if !operation.is_empty() {
                    return Err(unsupported(
                        &self.key,
                        "EmitStream",
                        "non-stream emit operations are outside the emitted core",
                    ));
                }
                self.emit_stream(*value)?;
            }
            MirStmtKind::StreamNext {
                endpoint_slot,
                item_type,
            } => {
                let end_block = self
                    .function
                    .blocks
                    .get(self.current_block as usize)
                    .and_then(|block| block.successors.first().copied())
                    .ok_or_else(|| {
                        unsupported(
                            &self.key,
                            "StreamNext",
                            "statement has no natural end continuation",
                        )
                    })?;
                self.emit_stream_next(*endpoint_slot, item_type, Some(end_block))?;
            }
            MirStmtKind::Expr { value } => {
                let expression = self.function.expression(*value)?;
                self.emit_expression(*value)?;
                if !is_void(&expression.ty) {
                    self.emit_op(Opcode::Pop, Vec::new())?;
                }
            }
            MirStmtKind::Throw {
                value,
                payload_type,
                ..
            } => {
                self.emit_expression(*value)?;
                let type_ref = self.image.type_index(
                    self.unit.module_path.as_str(),
                    payload_type,
                    &format!("statement throw type in `{}`", self.key),
                )?;
                self.emit_op(Opcode::Throw, vec![type_ref])?;
            }
            MirStmtKind::Rethrow { exception_slot } => {
                self.emit_op(Opcode::Rethrow, vec![*exception_slot])?;
            }
            MirStmtKind::Return { value } => {
                if let Some(value) = value {
                    let expression = self.function.expression(*value)?;
                    if self.try_emit_tail_call(expression)? {
                        return Ok(());
                    }
                    self.emit_expression(*value)?;
                }
                self.emit_op(Opcode::Return, Vec::new())?;
            }
            MirStmtKind::If {
                condition,
                then_block,
                else_block,
            } => {
                self.emit_expression(*condition)?;
                let then_block = *then_block;
                let false_target = else_block.as_ref().copied().unwrap_or_else(|| {
                    self.function
                        .blocks
                        .get(self.current_block as usize)
                        .and_then(|block| {
                            block
                                .successors
                                .iter()
                                .copied()
                                .find(|successor| *successor != then_block)
                        })
                        .unwrap_or(then_block)
                });
                self.emit_branch(Opcode::JumpIfFalse, false_target)?;
                self.emit_branch(Opcode::Jump, then_block)?;
            }
            MirStmtKind::While { condition, body } => {
                self.emit_expression(*condition)?;
                let body = *body;
                let continuation = self
                    .function
                    .blocks
                    .get(self.current_block as usize)
                    .and_then(|block| {
                        block
                            .successors
                            .iter()
                            .copied()
                            .find(|successor| *successor != body)
                    })
                    .ok_or_else(|| {
                        unsupported(
                            &self.key,
                            "while without an explicit continuation",
                            "MIR successor set must include a loop exit",
                        )
                    })?;
                self.emit_branch(Opcode::JumpIfFalse, continuation)?;
                self.emit_branch(Opcode::Jump, body)?;
            }
            MirStmtKind::ForIn {
                iterable,
                facts,
                body,
                continuation,
            } => {
                self.emit_for_in(*iterable, facts, *body, *continuation)?;
            }
            MirStmtKind::Match { value, arms } => {
                self.emit_match(*value, arms)?;
            }
            MirStmtKind::Break | MirStmtKind::Continue => {}
            other => {
                return Err(unsupported(
                    &self.key,
                    "MIR statement",
                    &format!("{other:?} is outside the scalar emitter subset"),
                ));
            }
        }
        Ok(())
    }

    fn emit_expression(&mut self, expression_ref: ExprRefIr) -> Result<(), BytecodeEmissionError> {
        let expression = self.function.expression(expression_ref)?;
        self.begin_expression(expression.index);
        match &expression.expression {
            ExprIr::Literal { value } => {
                let ty = literal_type(value);
                let pool = self.image.add_literal_constant(
                    self.unit.module_path.as_str(),
                    value,
                    &ty,
                    &format!(
                        "function `{key}` literal expression {index}",
                        key = self.key,
                        index = expression.index
                    ),
                )?;
                self.emit_op(Opcode::Const, vec![pool])?;
            }
            ExprIr::LoadSlot { slot } => {
                self.emit_op(Opcode::LoadSlot, vec![*slot])?;
            }
            ExprIr::LoadConst { const_index } => {
                let constant = self.unit.constant(*const_index)?;
                let pool = self
                    .image
                    .roots
                    .get(&constant.symbol)
                    .copied()
                    .ok_or_else(|| {
                        unsupported(
                            &self.key,
                            "LoadConst",
                            &format!(
                                "constant `{symbol}` is absent from the constant image",
                                symbol = constant.symbol
                            ),
                        )
                    })?;
                self.emit_op(Opcode::Const, vec![pool])?;
            }
            ExprIr::Unary { op, value } => {
                self.emit_expression(*value)?;
                let opcode = match op {
                    skiff_artifact_model::UnaryOpIr::Not => Opcode::Not,
                    skiff_artifact_model::UnaryOpIr::Negate => Opcode::Negate,
                };
                self.emit_op(opcode, Vec::new())?;
            }
            ExprIr::Field { object, field } => {
                self.emit_field_read(*object, field)?;
            }
            ExprIr::Index { object, index } => {
                self.emit_index_read(*object, *index)?;
            }
            ExprIr::Construct { type_ref, fields } => {
                self.emit_record_construct(expression, type_ref, fields)?;
            }
            ExprIr::RepresentationWrap { value, type_ref } => {
                self.emit_expression(*value)?;
                let type_ref_index = self.image.type_index(
                    self.unit.module_path.as_str(),
                    type_ref,
                    &format!("representation wrap type in `{}`", self.key),
                )?;
                self.emit_op(Opcode::RepresentationWrap, vec![type_ref_index])?;
            }
            ExprIr::InterfaceBox { value, source, .. } => {
                self.emit_interface_box(expression, *value, source)?;
            }
            ExprIr::MapLiteral { entries } => {
                self.emit_map_literal(expression, entries)?;
            }
            ExprIr::ArrayLiteral { items } => {
                self.emit_array_literal(expression, items)?;
            }
            ExprIr::Binary { op, left, right } => {
                self.emit_expression(*left)?;
                self.emit_expression(*right)?;
                let opcode = binary_opcode(*op)?;
                self.emit_op(opcode, Vec::new())?;
            }
            ExprIr::Throw {
                value,
                payload_type,
                ..
            } => {
                self.emit_expression(*value)?;
                let type_ref = self.image.type_index(
                    self.unit.module_path.as_str(),
                    payload_type,
                    &format!("expression throw type in `{}`", self.key),
                )?;
                self.emit_op(Opcode::Throw, vec![type_ref])?;
            }
            ExprIr::Rethrow { exception_slot } => {
                self.emit_op(Opcode::Rethrow, vec![*exception_slot])?;
            }
            ExprIr::Catch {
                try_expression,
                catch_slot,
                catch_type,
                body,
            } => {
                self.emit_catch(
                    expression.index,
                    *try_expression,
                    *catch_slot,
                    catch_type,
                    *body,
                )?;
            }
            ExprIr::Call { .. } => {
                self.emit_call_expression(expression)?;
            }
            other => {
                return Err(unsupported(
                    &self.key,
                    "MIR expression",
                    &format!("{other:?} is outside the scalar emitter subset"),
                ));
            }
        }
        Ok(())
    }

    fn emit_call_expression(
        &mut self,
        expression: &MirExpression,
    ) -> Result<(), BytecodeEmissionError> {
        if self.try_emit_ordinary_call(expression)? {
            self.map_all_expression_events(expression.index);
            return Ok(());
        }
        let ExprIr::Call { call } = &expression.expression else {
            return Err(unsupported(
                &self.key,
                "call expression",
                "expression is not a call",
            ));
        };
        if !call.inout_args.is_empty() {
            return Err(BytecodeEmissionError::InOutEmissionPending {
                function_key: self.key.clone(),
                expression: expression.index,
            });
        }
        if !call.type_args.is_empty() {
            return Err(unsupported(
                &self.key,
                "generic non-direct call",
                "non-direct calls with type arguments are outside the emitted core",
            ));
        }
        for argument in &call.args {
            self.emit_expression(*argument)?;
        }
        let result = match &call.target {
            CallTargetIr::ServiceCall {
                service_call_ref_index,
            } => {
                let service_call = self
                    .unit
                    .external_refs
                    .service_call_ref(*service_call_ref_index)
                    .ok_or_else(|| {
                        unsupported(
                            &self.key,
                            "service call target",
                            &format!(
                                "service call ref {} is absent",
                                service_call_ref_index.index()
                            ),
                        )
                    })?
                    .clone();
                self.emit_pending_call(
                    expression,
                    Opcode::CallService,
                    BytecodeRelocation::ServiceOperationRef { service_call },
                    None,
                    true,
                )
            }
            CallTargetIr::ActorMethod {
                actor,
                actor_abi_identity,
                actor_implementation_identity,
                method_identity,
            } => self.emit_pending_call(
                expression,
                Opcode::CallActor,
                BytecodeRelocation::ActorMethodRef {
                    actor: actor.clone(),
                    actor_abi_identity: actor_abi_identity.clone(),
                    actor_implementation_identity: actor_implementation_identity.clone(),
                    method_identity: method_identity.clone(),
                },
                None,
                true,
            ),
            CallTargetIr::InterfaceMethod {
                interface, slot, ..
            } => {
                let relocation = BytecodeRelocation::InterfaceRequirementRef {
                    interface: interface.clone(),
                };
                self.emit_pending_call(
                    expression,
                    Opcode::CallInterface,
                    relocation,
                    Some(*slot),
                    true,
                )
            }
            CallTargetIr::Native { target } => {
                let relocation = BytecodeRelocation::HostEffectRef(
                    self.host_effect_reference(call, expression, target)?,
                );
                self.emit_pending_call(expression, Opcode::InvokeHost, relocation, None, true)
            }
            CallTargetIr::Builtin { op } => {
                let relocation = BytecodeRelocation::IntrinsicRef {
                    intrinsic: self.intrinsic_reference(call, expression, op)?,
                };
                self.emit_pending_call(expression, Opcode::InvokeIntrinsic, relocation, None, false)
            }
            CallTargetIr::ReceiverBuiltin { op } => {
                self.emit_receiver_builtin(call, expression, op)
            }
            other => Err(unsupported(
                &self.key,
                "non-direct call target",
                &format!("{other:?} is outside the emitted core"),
            )),
        };
        result?;
        self.map_all_expression_events(expression.index);
        Ok(())
    }

    fn emit_pending_call(
        &mut self,
        expression: &MirExpression,
        opcode: Opcode,
        relocation: BytecodeRelocation,
        method_ordinal: Option<u32>,
        requires_resume: bool,
    ) -> Result<(), BytecodeEmissionError> {
        let ExprIr::Call { call } = &expression.expression else {
            return Err(unsupported(
                &self.key,
                "pending call",
                "expression is not a call",
            ));
        };
        let relocation_index = u32::try_from(self.relocations.len())
            .map_err(|_| arithmetic(self.key.as_str(), "relocation index conversion"))?;
        self.relocations.push(relocation);

        let (arg_count, input_count) = if method_ordinal.is_some() {
            let args = call.args.len().checked_sub(1).ok_or_else(|| {
                unsupported(
                    &self.key,
                    "interface call",
                    "interface call has no carrier argument",
                )
            })?;
            (args, args + 1)
        } else {
            (call.args.len(), call.args.len())
        };
        let arg_count = u32::try_from(arg_count)
            .map_err(|_| arithmetic(self.key.as_str(), "argument count conversion"))?;
        let result_count = usize::from(!is_void(&expression.ty));
        let result_count = u32::try_from(result_count)
            .map_err(|_| arithmetic(self.key.as_str(), "result count conversion"))?;
        let mut operands = vec![relocation_index];
        if let Some(method_ordinal) = method_ordinal {
            operands.push(method_ordinal);
        }
        operands.push(arg_count);
        operands.push(result_count);
        let resume_operand = if requires_resume {
            let position = operands.len();
            operands.push(0);
            Some(position)
        } else {
            None
        };
        let expected_stack_height_before_result = if requires_resume {
            Some(
                self.operand_depth
                    .checked_sub(input_count)
                    .ok_or_else(|| {
                        unsupported(
                            &self.key,
                            "pending call stack",
                            &format!(
                                "call input underflows emitted operand stack (depth {}, input {input_count})",
                                self.operand_depth
                            ),
                        )
                    })?,
            )
        } else {
            None
        };
        let instruction = self.emit_op(opcode, operands)?;
        if let (Some(operand), Some(expected_stack_height_before_result)) =
            (resume_operand, expected_stack_height_before_result)
        {
            self.pending_resumes.push(PendingResume {
                instruction,
                operand,
                expected_stack_height_before_result: u32::try_from(
                    expected_stack_height_before_result,
                )
                .map_err(|_| arithmetic(&self.key, "resume stack height conversion"))?,
                result_ty: if result_count == 0 {
                    None
                } else {
                    Some(expression.ty.clone())
                },
                end_block: None,
            });
        }
        self.map_call_event(expression.index);
        Ok(())
    }

    fn host_effect_reference(
        &self,
        call: &skiff_artifact_model::CallIr,
        expression: &MirExpression,
        target: &skiff_artifact_model::NativeTarget,
    ) -> Result<HostEffectReference, BytecodeEmissionError> {
        let binding_key = target.binding_key.as_deref().ok_or_else(|| {
            unsupported(
                &self.key,
                "host effect target",
                "native target lacks an exact binding key",
            )
        })?;
        let effects = skiff_artifact_model::host_effect_registry()
            .entries()
            .iter()
            .find(|entry| entry.binding_key == binding_key)
            .map(|entry| entry.signature.effects.clone())
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "host effect target",
                    &format!("binding key `{binding_key}` is absent from the host registry"),
                )
            })?;
        Ok(HostEffectReference {
            target: target.clone(),
            signature: self.call_signature(call, expression, effects)?,
        })
    }

    fn intrinsic_reference(
        &self,
        call: &skiff_artifact_model::CallIr,
        expression: &MirExpression,
        canonical_key: &str,
    ) -> Result<IntrinsicReference, BytecodeEmissionError> {
        let target = BytecodeIntrinsicRef::Static {
            canonical_key: canonical_key.to_string(),
            signature_version: 1,
        };
        let effects = skiff_artifact_model::intrinsic_registry()
            .entries()
            .iter()
            .find(|entry| entry.target == target)
            .map(|entry| entry.signature.effects.clone())
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "intrinsic target",
                    &format!(
                        "canonical key `{canonical_key}` is absent from the intrinsic registry"
                    ),
                )
            })?;
        Ok(IntrinsicReference {
            target,
            signature: self.call_signature(call, expression, effects)?,
        })
    }

    fn call_signature(
        &self,
        call: &skiff_artifact_model::CallIr,
        expression: &MirExpression,
        effects: skiff_artifact_model::CallableMayEffects,
    ) -> Result<HostEffectSignature, BytecodeEmissionError> {
        let parameter_types = call
            .args
            .iter()
            .map(|argument| {
                let ty = &self.function.expression(*argument)?.ty;
                Ok(super::constants::qualify_local_types(
                    self.unit.module_path.as_str(),
                    ty,
                ))
            })
            .collect::<Result<Vec<_>, BytecodeEmissionError>>()?;
        let parameter_plans = parameter_types
            .iter()
            .cloned()
            .map(|ty| skiff_artifact_model::ValueTransferPlan::FromType { ty })
            .collect();
        let result_types = if is_void(&expression.ty) {
            Vec::new()
        } else {
            vec![super::constants::qualify_local_types(
                self.unit.module_path.as_str(),
                &expression.ty,
            )]
        };
        let result_plans = result_types
            .iter()
            .cloned()
            .map(|ty| skiff_artifact_model::ValueTransferPlan::FromType { ty })
            .collect();
        let parameter_count = parameter_types.len();
        Ok(HostEffectSignature {
            parameter_types,
            parameter_modes: vec![ParamModeIr::Value; parameter_count],
            parameter_plans,
            result_types,
            result_plans,
            effects,
        })
    }

    fn emit_stream(&mut self, value: ExprRefIr) -> Result<(), BytecodeEmissionError> {
        let stream = self.function.stream_result.as_ref().ok_or_else(|| {
            unsupported(
                &self.key,
                "EmitStream",
                "function has no exact Stream<T> result facts",
            )
        })?;
        let value_expression = self.function.expression(value)?;
        if !stream_item_type_matches(&value_expression.ty, &stream.item_type) {
            return Err(unsupported(
                &self.key,
                "EmitStream",
                &format!(
                    "emitted value type `{:?}` does not match stream item type `{:?}`",
                    value_expression.ty, stream.item_type
                ),
            ));
        }
        self.emit_expression(value)?;
        let expected_stack_height_before_result = self
            .operand_depth
            .checked_sub(1)
            .ok_or_else(|| unsupported(&self.key, "EmitStream", "stream item operand is absent"))?;
        let instruction = self.emit_op(Opcode::EmitStream, vec![0])?;
        self.pending_resumes.push(PendingResume {
            instruction,
            operand: 0,
            expected_stack_height_before_result: u32::try_from(expected_stack_height_before_result)
                .map_err(|_| arithmetic(&self.key, "EmitStream stack height conversion"))?,
            result_ty: None,
            end_block: None,
        });
        Ok(())
    }

    fn emit_stream_next(
        &mut self,
        endpoint_slot: u32,
        item_type: &TypeRefIr,
        end_block: Option<u32>,
    ) -> Result<(), BytecodeEmissionError> {
        let endpoint_type = self.slot_type(endpoint_slot)?;
        let TypeRefIr::Builtin { name, args } = endpoint_type else {
            return Err(unsupported(
                &self.key,
                "StreamNext",
                "endpoint slot is not Stream<T>",
            ));
        };
        if name != "Stream" || args.as_slice() != [item_type.clone()] {
            return Err(unsupported(
                &self.key,
                "StreamNext",
                "endpoint slot is not the exact Stream<T> authority",
            ));
        }
        let expected_stack_height_before_result = self.operand_depth;
        let instruction = self.emit_op(Opcode::StreamNext, vec![endpoint_slot, 0])?;
        self.pending_resumes.push(PendingResume {
            instruction,
            operand: 1,
            expected_stack_height_before_result: u32::try_from(expected_stack_height_before_result)
                .map_err(|_| arithmetic(&self.key, "StreamNext stack height conversion"))?,
            result_ty: Some(item_type.clone()),
            end_block,
        });
        Ok(())
    }

    fn emit_receiver_builtin(
        &mut self,
        call: &skiff_artifact_model::CallIr,
        expression: &MirExpression,
        op: &skiff_artifact_model::BuiltinReceiverOp,
    ) -> Result<(), BytecodeEmissionError> {
        if call.args.len() != 1 {
            return Err(unsupported(
                &self.key,
                "receiver builtin",
                "length builtin requires exactly one receiver argument",
            ));
        }
        match (op.receiver, op.method) {
            (
                skiff_artifact_model::BuiltinReceiverRoot::Array,
                skiff_artifact_model::BuiltinReceiverMethod::Length,
            ) => self.emit_op(Opcode::ArrayLen, Vec::new()),
            (
                skiff_artifact_model::BuiltinReceiverRoot::Map,
                skiff_artifact_model::BuiltinReceiverMethod::Length,
            ) => self.emit_op(Opcode::MapLen, Vec::new()),
            _ => Err(unsupported(
                &self.key,
                "receiver builtin",
                &format!(
                    "`{}` is outside the emitted collection core",
                    op.canonical_key
                ),
            )),
        }?;
        self.map_call_event(expression.index);
        Ok(())
    }

    fn try_emit_tail_call(
        &mut self,
        expression: &MirExpression,
    ) -> Result<bool, BytecodeEmissionError> {
        if !matches!(expression.expression, ExprIr::Call { .. }) || expression.direct_call.is_none()
        {
            return Ok(false);
        }
        self.map_all_expression_events(expression.index);
        self.emit_direct_call(expression, true)
    }

    fn try_emit_ordinary_call(
        &mut self,
        expression: &MirExpression,
    ) -> Result<bool, BytecodeEmissionError> {
        if !matches!(expression.expression, ExprIr::Call { .. }) || expression.direct_call.is_none()
        {
            return Ok(false);
        }
        self.emit_direct_call(expression, false)
    }

    fn emit_direct_call(
        &mut self,
        expression: &MirExpression,
        tail: bool,
    ) -> Result<bool, BytecodeEmissionError> {
        let facts = expression.direct_call.as_ref().ok_or_else(|| {
            unsupported(
                &self.key,
                "call expression",
                "direct call facts are absent from MIR",
            )
        })?;
        if facts
            .arguments
            .iter()
            .any(|argument| matches!(argument, MirCallArgument::InOut { .. }))
        {
            return Err(BytecodeEmissionError::InOutEmissionPending {
                function_key: self.key.clone(),
                expression: expression.index,
            });
        }
        for argument in &facts.arguments {
            let MirCallArgument::Value { value } = argument else {
                return Err(BytecodeEmissionError::InOutEmissionPending {
                    function_key: self.key.clone(),
                    expression: expression.index,
                });
            };
            self.emit_expression(*value)?;
        }
        let relocation = self.direct_relocation(expression, facts)?;
        let relocation_index = u32::try_from(self.relocations.len())
            .map_err(|_| arithmetic(self.key.as_str(), "relocation index conversion"))?;
        self.relocations.push(relocation);
        let arg_count = u32::try_from(facts.arguments.len())
            .map_err(|_| arithmetic(self.key.as_str(), "argument count conversion"))?;
        let mut operands = vec![relocation_index, arg_count];
        let opcode = if tail {
            Opcode::TailCallLocal
        } else {
            operands.push(u32::from(!is_void(&expression.ty)));
            Opcode::CallLocal
        };
        self.emit_op(opcode, operands)?;
        self.map_call_event(expression.index);
        Ok(true)
    }

    fn direct_relocation(
        &self,
        expression: &MirExpression,
        facts: &MirDirectCallFacts,
    ) -> Result<BytecodeRelocation, BytecodeEmissionError> {
        let ExprIr::Call { call } = &expression.expression else {
            return Err(unsupported(
                &self.key,
                "call relocation",
                "expression is not a call",
            ));
        };
        if !call.type_args.is_empty() || facts.concrete_receiver.is_some() {
            return Err(unsupported(
                &self.key,
                "generic or receiver-bound call",
                "non-generic scalar calls only",
            ));
        }
        let specialization = BytecodeSpecialization {
            type_arguments: Vec::new(),
            concrete_receiver: None,
        };
        match &call.target {
            CallTargetIr::LocalExecutable { executable_index } => {
                let target = self.unit.function_by_executable_index(*executable_index)?;
                require_narrow_target(&self.key, target)?;
                let function_key =
                    super::inputs::canonical_function_key(&self.unit.module_path, &target.symbol)?;
                Ok(BytecodeRelocation::LocalExecutableRef {
                    function_key,
                    specialization,
                })
            }
            CallTargetIr::PublicationExecutable {
                module_path,
                executable_index,
            } => {
                let target_unit = self.inputs.units.get(module_path.as_str()).ok_or_else(|| {
                    unsupported(
                        &self.key,
                        "publication-local call target",
                        &format!("module `{module_path}` is absent"),
                    )
                })?;
                let target = target_unit.function_by_executable_index(*executable_index)?;
                require_narrow_target(&self.key, target)?;
                let function_key =
                    super::inputs::canonical_function_key(module_path, &target.symbol)?;
                Ok(BytecodeRelocation::LocalExecutableRef {
                    function_key,
                    specialization,
                })
            }
            CallTargetIr::PackageCallable {
                package_ref,
                package_callable_id,
            } => Ok(BytecodeRelocation::PackageCallableRef {
                package_ref: package_ref.clone(),
                package_callable_id: package_callable_id.clone(),
                specialization,
            }),
            _ => Err(unsupported(
                &self.key,
                "call target",
                "only local and package-direct scalar calls are supported",
            )),
        }
    }

    fn emit_writable_assign(
        &mut self,
        place: &MirWritablePlace,
        value: ExprRefIr,
    ) -> Result<(), BytecodeEmissionError> {
        let MirWritableRoot::Slot { slot } = place.root else {
            return Err(unsupported(
                &self.key,
                "writable assignment",
                "actor durable roots are outside the emitted core",
            ));
        };
        let root =
            self.function.slots.get(slot as usize).ok_or_else(|| {
                unsupported(&self.key, "writable assignment", "root slot is absent")
            })?;
        if !root.writable_local {
            return Err(unsupported(
                &self.key,
                "writable assignment",
                "root slot is not a source-confirmed writable local",
            ));
        }
        let root_ty = root.ty.clone().ok_or_else(|| {
            unsupported(&self.key, "writable assignment", "root has no exact type")
        })?;
        let mut current_ty = root_ty.clone();
        let mut selector_expressions = Vec::new();
        let mut segments = Vec::new();
        let mut next_selector_ordinal = 0u32;
        for segment in &place.path {
            match segment {
                MirWritablePathSegment::Field { name } => {
                    let fields = self.record_shape_fields(&current_ty, "writable field path")?;
                    let field_ordinal = fields
                        .keys()
                        .position(|candidate| candidate == name)
                        .ok_or_else(|| {
                            unsupported(
                                &self.key,
                                "writable field path",
                                &format!("field `{name}` is absent from the record shape"),
                            )
                        })?;
                    let shape_ref = self.image.intern_shape(
                        self.unit.module_path.as_str(),
                        &current_ty,
                        &fields,
                        &format!("writable field path in `{}`", self.key),
                    )?;
                    segments.push(WritablePathSegment::DenseField {
                        shape_ref,
                        field_ordinal: field_ordinal as u32,
                    });
                    current_ty = fields.get(name).expect("ordinal was found").clone();
                }
                MirWritablePathSegment::Index { index, access, .. } => {
                    let selector_ordinal = next_selector_ordinal;
                    next_selector_ordinal = next_selector_ordinal.saturating_add(1);
                    selector_expressions.push(*index);
                    match access.receiver_kind {
                        MirIndexReceiverKind::Array => {
                            let element_ty = access.result_type.clone();
                            let element_type_ref = self.image.type_index(
                                self.unit.module_path.as_str(),
                                &element_ty,
                                &format!("writable array path in `{}`", self.key),
                            )?;
                            segments.push(WritablePathSegment::ArrayIndex {
                                selector_ordinal,
                                element_type_ref,
                            });
                            current_ty = element_ty;
                        }
                        MirIndexReceiverKind::Map => {
                            let (key_ty, value_ty) =
                                self.map_key_value_types(&current_ty, "writable map path")?;
                            let key_type_ref = self.image.type_index(
                                self.unit.module_path.as_str(),
                                &key_ty,
                                &format!("writable map key path in `{}`", self.key),
                            )?;
                            let value_type_ref = self.image.type_index(
                                self.unit.module_path.as_str(),
                                &value_ty,
                                &format!("writable map value path in `{}`", self.key),
                            )?;
                            segments.push(WritablePathSegment::MapKey {
                                selector_ordinal,
                                key_type_ref,
                                value_type_ref,
                            });
                            current_ty = value_ty;
                        }
                        MirIndexReceiverKind::JsonObject => {
                            return Err(unsupported(
                                &self.key,
                                "writable assignment",
                                "JsonObject writable paths are outside the emitted core",
                            ))
                        }
                    }
                }
            }
        }
        let leaf_ty = &self.function.expression(value)?.ty;
        if leaf_ty != &current_ty {
            return Err(unsupported(
                &self.key,
                "writable assignment",
                "assigned value type does not match writable path leaf type",
            ));
        }
        for selector in &selector_expressions {
            self.emit_expression(*selector)?;
        }
        self.emit_expression(value)?;
        let path_ref = self.image.intern_writable_path(
            self.unit.module_path.as_str(),
            &root_ty,
            &current_ty,
            segments,
            &format!("writable assignment in `{}`", self.key),
        )?;
        let selector_count = u32::try_from(selector_expressions.len())
            .map_err(|_| arithmetic(&self.key, "writable selector count conversion"))?;
        self.emit_op(
            Opcode::SetWritablePath,
            vec![slot, path_ref, selector_count],
        )?;
        Ok(())
    }

    fn emit_field_read(
        &mut self,
        object: ExprRefIr,
        field: &str,
    ) -> Result<(), BytecodeEmissionError> {
        let object_expression = self.function.expression(object)?;
        let fields = self.record_shape_fields(&object_expression.ty, "field read")?;
        let ordinal = fields
            .keys()
            .position(|name| name == field)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "record field read",
                    &format!("field `{field}` is absent from the record shape"),
                )
            })?;
        let shape = self.image.intern_shape(
            self.unit.module_path.as_str(),
            &object_expression.ty,
            &fields,
            &format!("field read `{field}` in `{}`", self.key),
        )?;
        self.emit_expression(object)?;
        self.emit_op(Opcode::GetDenseField, vec![shape, ordinal as u32])?;
        Ok(())
    }

    fn emit_index_read(
        &mut self,
        object: ExprRefIr,
        index: ExprRefIr,
    ) -> Result<(), BytecodeEmissionError> {
        let facts = self
            .function
            .index_accesses
            .get(&index.expression)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "index read",
                    &format!(
                        "selector expression {} has no source facts",
                        index.expression
                    ),
                )
            })?;
        self.emit_expression(object)?;
        self.emit_expression(index)?;
        match facts.receiver_kind {
            MirIndexReceiverKind::Array => {
                self.emit_op(Opcode::ArrayGet, Vec::new())?;
            }
            MirIndexReceiverKind::Map => {
                self.emit_op(Opcode::MapGet, Vec::new())?;
            }
            MirIndexReceiverKind::JsonObject => {
                return Err(unsupported(
                    &self.key,
                    "index read",
                    "JsonObject bracket reads are outside the emitted core",
                ))
            }
        }
        Ok(())
    }

    fn emit_record_construct(
        &mut self,
        expression: &MirExpression,
        type_ref: &TypeRefIr,
        fields: &BTreeMap<String, ExprRefIr>,
    ) -> Result<(), BytecodeEmissionError> {
        let declared = self.record_shape_fields(type_ref, "record construct")?;
        if declared.len() != fields.len() || declared.keys().any(|name| !fields.contains_key(name))
        {
            return Err(unsupported(
                &self.key,
                "record construct",
                "construct field set does not exactly match the declared shape",
            ));
        }
        let shape = self.image.intern_shape(
            self.unit.module_path.as_str(),
            &expression.ty,
            &declared,
            &format!("record construct in `{}`", self.key),
        )?;
        for name in declared.keys() {
            self.emit_expression(*fields.get(name).expect("field set was checked"))?;
        }
        let field_count = u32::try_from(declared.len())
            .map_err(|_| arithmetic(&self.key, "record field count conversion"))?;
        self.emit_op(Opcode::NewRecord, vec![shape, field_count])?;
        Ok(())
    }

    fn emit_array_literal(
        &mut self,
        expression: &MirExpression,
        items: &[ExprRefIr],
    ) -> Result<(), BytecodeEmissionError> {
        let element_ty = self.array_element_type(&expression.ty, "array literal")?;
        let element_ref = self.image.type_index(
            self.unit.module_path.as_str(),
            &element_ty,
            &format!("array literal element type in `{}`", self.key),
        )?;
        self.emit_op(Opcode::NewArrayBuilder, vec![element_ref])?;
        for item in items {
            self.emit_expression(*item)?;
            self.emit_op(Opcode::ArrayBuilderPush, Vec::new())?;
        }
        self.emit_op(Opcode::FreezeArray, Vec::new())?;
        Ok(())
    }

    fn emit_map_literal(
        &mut self,
        expression: &MirExpression,
        entries: &BTreeMap<String, ExprRefIr>,
    ) -> Result<(), BytecodeEmissionError> {
        let (key_ty, value_ty) = self.map_key_value_types(&expression.ty, "map literal")?;
        let key_ref = self.image.intern_type(
            self.unit.module_path.as_str(),
            &key_ty,
            &format!("map literal key type in `{}`", self.key),
        )?;
        let value_ref = self.image.intern_type(
            self.unit.module_path.as_str(),
            &value_ty,
            &format!("map literal value type in `{}`", self.key),
        )?;
        self.emit_op(Opcode::NewMapBuilder, vec![key_ref, value_ref])?;
        for (key, value) in entries {
            let key_pool = self.image.add_literal_constant(
                self.unit.module_path.as_str(),
                &skiff_artifact_model::LiteralIr::String { value: key.clone() },
                &TypeRefIr::builtin("string"),
                &format!("map literal key in `{}`", self.key),
            )?;
            self.emit_op(Opcode::Const, vec![key_pool])?;
            self.emit_expression(*value)?;
            self.emit_op(Opcode::MapBuilderPut, Vec::new())?;
        }
        self.emit_op(Opcode::FreezeMap, Vec::new())?;
        Ok(())
    }

    fn emit_interface_box(
        &mut self,
        expression: &MirExpression,
        value: ExprRefIr,
        source: &BoxSourceIr,
    ) -> Result<(), BytecodeEmissionError> {
        if let BoxSourceIr::Remote { .. } = source {
            let facts = expression.remote_interface.as_ref().ok_or_else(|| {
                unsupported(
                    &self.key,
                    "interface box",
                    "remote interface boxing lacks an exact service requirement slot in MIR",
                )
            })?;
            let methods = facts
                .methods
                .iter()
                .map(|method| RemoteInterfaceMethod {
                    slot: method.slot,
                    method_abi_id: method.method_abi_id.clone(),
                    signature: method.signature.clone(),
                    contract_operation_id: method.contract_operation_id.clone(),
                })
                .collect::<Vec<_>>();
            let relocation = BytecodeRelocation::RemoteInterfaceRef {
                interface: RemoteInterfaceRef {
                    service_requirement_slot: facts.service_requirement_slot,
                    public_instance_key: facts.public_instance_key.clone(),
                    interface: facts.interface.clone(),
                    methods,
                    callee_protocol_identity: facts.callee_protocol_identity.clone(),
                },
            };
            let relocation_index = u32::try_from(self.relocations.len()).map_err(|_| {
                arithmetic(&self.key, "remote interface relocation index conversion")
            })?;
            self.relocations.push(relocation);
            self.emit_op(Opcode::InterfaceBoxRemote, vec![relocation_index])?;
            return Ok(());
        }
        let BoxSourceIr::Local {
            concrete_type,
            method_table,
        } = source
        else {
            return Err(unsupported(
                &self.key,
                "interface box",
                "remote interface boxing lacks an exact service requirement slot in MIR",
            ));
        };
        let mut methods = Vec::with_capacity(method_table.slots.len());
        for slot in &method_table.slots {
            let target = self
                .unit
                .function_by_executable_index(slot.target.executable_index)?;
            let function_key =
                super::inputs::canonical_function_key(&self.unit.module_path, &target.symbol)?;
            methods.push(LocalInterfaceMethod {
                slot: slot.slot,
                method_name: slot.method_name.clone(),
                method_abi_id: slot.method_abi_id.clone(),
                signature: slot.signature.clone(),
                function_key,
                receiver_call_abi: slot.target.receiver_call_abi,
            });
        }
        let relocation = BytecodeRelocation::LocalInterfaceRef {
            interface: LocalInterfaceRef {
                interface: method_table.interface.clone(),
                concrete_type: concrete_type.clone(),
                methods,
            },
        };
        let relocation_index = u32::try_from(self.relocations.len())
            .map_err(|_| arithmetic(&self.key, "interface relocation index conversion"))?;
        self.relocations.push(relocation);
        self.emit_expression(value)?;
        let _ = expression;
        self.emit_op(Opcode::InterfaceBoxLocal, vec![relocation_index])?;
        Ok(())
    }

    fn record_shape_fields(
        &self,
        ty: &TypeRefIr,
        context: &'static str,
    ) -> Result<BTreeMap<String, TypeRefIr>, BytecodeEmissionError> {
        match ty {
            TypeRefIr::Record { fields } => Ok(fields.clone()),
            TypeRefIr::LocalType { type_index } => {
                let declaration =
                    self.unit
                        .type_table
                        .get(*type_index as usize)
                        .ok_or_else(|| {
                            unsupported(
                                &self.key,
                                context,
                                &format!("local type {type_index} is absent"),
                            )
                        })?;
                if !declaration.type_params.is_empty() {
                    return Err(unsupported(
                        &self.key,
                        context,
                        "generic nominal record shapes are outside the emitted core",
                    ));
                }
                match &declaration.descriptor {
                    skiff_artifact_model::TypeDescriptorIr::Record { fields } => Ok(fields.clone()),
                    _ => Err(unsupported(
                        &self.key,
                        context,
                        &format!("local type `{}` is not a record", declaration.name),
                    )),
                }
            }
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } => {
                let unit = self.inputs.units.get(module_path.as_str()).ok_or_else(|| {
                    unsupported(
                        &self.key,
                        context,
                        &format!("publication module `{module_path}` is absent"),
                    )
                })?;
                let declaration = unit.type_table.get(*type_index as usize).ok_or_else(|| {
                    unsupported(
                        &self.key,
                        context,
                        &format!("publication type {type_index} is absent"),
                    )
                })?;
                if !declaration.type_params.is_empty() {
                    return Err(unsupported(
                        &self.key,
                        context,
                        "generic nominal record shapes are outside the emitted core",
                    ));
                }
                match &declaration.descriptor {
                    skiff_artifact_model::TypeDescriptorIr::Record { fields } => Ok(fields.clone()),
                    _ => Err(unsupported(
                        &self.key,
                        context,
                        &format!("publication type `{}` is not a record", declaration.name),
                    )),
                }
            }
            TypeRefIr::AppliedNominal { .. } => Err(unsupported(
                &self.key,
                context,
                "applied nominal record shapes are outside the emitted core",
            )),
            other => Err(unsupported(
                &self.key,
                context,
                &format!("type `{other:?}` is not a record shape"),
            )),
        }
    }

    fn array_element_type(
        &self,
        ty: &TypeRefIr,
        context: &'static str,
    ) -> Result<TypeRefIr, BytecodeEmissionError> {
        match ty {
            TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => {
                Ok(args[0].clone())
            }
            other => Err(unsupported(
                &self.key,
                context,
                &format!("type `{other:?}` is not Array<T>"),
            )),
        }
    }

    fn map_key_value_types(
        &self,
        ty: &TypeRefIr,
        context: &'static str,
    ) -> Result<(TypeRefIr, TypeRefIr), BytecodeEmissionError> {
        match ty {
            TypeRefIr::Builtin { name, args } if name == "Map" && args.len() == 2 => {
                Ok((args[0].clone(), args[1].clone()))
            }
            other => Err(unsupported(
                &self.key,
                context,
                &format!("type `{other:?}` is not Map<K, V>"),
            )),
        }
    }

    fn emit_for_in(
        &mut self,
        iterable: ExprRefIr,
        facts: &skiff_compiler_lowering::mir::MirForInFacts,
        body: u32,
        continuation: u32,
    ) -> Result<(), BytecodeEmissionError> {
        let iterable_ty = facts.iterable_type.clone();
        let (array, stream, item_slot, value_slot, item_ty) = match &facts.binding {
            skiff_compiler_lowering::mir::MirForInBinding::Item { slot, ty, kind } => match kind {
                MirForInItemKind::ArrayItem => (true, false, *slot, None, ty.clone()),
                MirForInItemKind::MapKey => (false, false, *slot, None, ty.clone()),
                MirForInItemKind::StreamItem => (false, true, *slot, None, ty.clone()),
            },
            skiff_compiler_lowering::mir::MirForInBinding::MapEntry {
                key_slot,
                value_slot,
                ..
            } => (false, false, *key_slot, Some(*value_slot), iterable_ty.clone()),
        };
        let iterable_slot = self.push_generated_slot(&iterable_ty, "$forIterable")?;
        if stream {
            let index_slot = 0;
            if let ExprIr::LoadSlot { slot } = &self.function.expression(iterable)?.expression {
                self.emit_op(Opcode::MoveSlot, vec![*slot, iterable_slot])?;
            } else {
                self.emit_expression(iterable)?;
                self.emit_op(Opcode::StoreSlot, vec![iterable_slot])?;
            }
            let header_instruction = self.instructions.len();
            self.emit_stream_next(iterable_slot, &item_ty, Some(continuation))?;
            self.emit_op(Opcode::StoreSlot, vec![item_slot])?;
            self.emit_jump_to_block(body)?;
            self.loop_backedges.insert(
                body,
                LoopBackedge {
                    header_instruction,
                    iterable_slot,
                    index_slot,
                    item_slot,
                    value_slot,
                    array,
                    stream,
                },
            );
            return Ok(());
        }
        let index_slot = self.push_generated_slot(&TypeRefIr::builtin("number"), "$forIndex")?;
        self.emit_expression(iterable)?;
        self.emit_op(Opcode::StoreSlot, vec![iterable_slot])?;
        self.emit_number_constant(0)?;
        self.emit_op(Opcode::StoreSlot, vec![index_slot])?;
        let header_instruction = self.instructions.len();
        self.emit_op(Opcode::LoadSlot, vec![iterable_slot])?;
        self.emit_op(Opcode::LoadSlot, vec![index_slot])?;
        self.emit_op(
            if array {
                Opcode::ArrayLen
            } else {
                Opcode::MapLen
            },
            Vec::new(),
        )?;
        self.emit_op(Opcode::LoadSlot, vec![index_slot])?;
        self.emit_op(Opcode::LessThan, Vec::new())?;
        self.emit_branch(Opcode::JumpIfFalse, continuation)?;
        self.emit_branch(Opcode::Jump, body)?;
        self.loop_backedges.insert(
            body,
            LoopBackedge {
                header_instruction,
                iterable_slot,
                index_slot,
                item_slot,
                value_slot,
                array,
                stream,
            },
        );
        let _ = item_ty;
        Ok(())
    }

    fn emit_loop_backedge(
        &mut self,
        _block: u32,
        backedge: &LoopBackedge,
    ) -> Result<(), BytecodeEmissionError> {
        if backedge.stream {
            self.emit_jump_to_instruction(backedge.header_instruction)?;
            return Ok(());
        }
        self.emit_op(Opcode::LoadSlot, vec![backedge.iterable_slot])?;
        self.emit_op(Opcode::LoadSlot, vec![backedge.index_slot])?;
        if backedge.array {
            self.emit_op(Opcode::ArrayGet, Vec::new())?;
            self.emit_op(Opcode::StoreSlot, vec![backedge.item_slot])?;
        } else {
            self.emit_op(Opcode::MapEntryAt, Vec::new())?;
            if let Some(value_slot) = backedge.value_slot {
                self.emit_op(Opcode::StoreSlot, vec![value_slot])?;
            }
            self.emit_op(Opcode::StoreSlot, vec![backedge.item_slot])?;
        }
        self.emit_op(Opcode::LoadSlot, vec![backedge.index_slot])?;
        self.emit_number_constant(1)?;
        self.emit_op(Opcode::Add, Vec::new())?;
        self.emit_op(Opcode::StoreSlot, vec![backedge.index_slot])?;
        self.emit_jump_to_instruction(backedge.header_instruction)
    }

    fn emit_match(
        &mut self,
        value: ExprRefIr,
        arms: &[skiff_compiler_lowering::mir::MirMatchArmIr],
    ) -> Result<(), BytecodeEmissionError> {
        let value_expression = self.function.expression(value)?;
        let temp_slot = self.push_generated_slot(&value_expression.ty, "$matchValue")?;
        self.emit_expression(value)?;
        self.emit_op(Opcode::StoreSlot, vec![temp_slot])?;
        let current = self
            .function
            .blocks
            .get(self.current_block as usize)
            .ok_or_else(|| unsupported(&self.key, "match", "current block is absent"))?;
        let continuation = current.successors.last().copied().ok_or_else(|| {
            unsupported(&self.key, "match", "match has no continuation successor")
        })?;
        for (index, arm) in arms.iter().enumerate() {
            let next = arms
                .get(index + 1)
                .map(|next| next.body)
                .unwrap_or(continuation);
            match &arm.pattern {
                PatternIr::Wildcard => {
                    self.emit_jump_to_block(arm.body)?;
                    return Ok(());
                }
                PatternIr::Literal { value } => {
                    self.emit_op(Opcode::LoadSlot, vec![temp_slot])?;
                    let pool = self.image.add_literal_constant(
                        self.unit.module_path.as_str(),
                        value,
                        &literal_type(value),
                        &format!("match literal in `{}`", self.key),
                    )?;
                    self.emit_op(Opcode::Const, vec![pool])?;
                    self.emit_op(Opcode::Equal, Vec::new())?;
                    self.emit_branch(Opcode::JumpIfFalse, next)?;
                    self.emit_branch(Opcode::Jump, arm.body)?;
                }
                other => {
                    return Err(unsupported(
                        &self.key,
                        "match pattern",
                        &format!("{other:?} is outside the emitted core"),
                    ))
                }
            }
        }
        self.emit_jump_to_block(continuation)
    }

    fn emit_catch(
        &mut self,
        catch_expression_index: u32,
        try_expression: ExprRefIr,
        catch_slot: u32,
        catch_type: &TypeRefIr,
        body: ExprRefIr,
    ) -> Result<(), BytecodeEmissionError> {
        let region = self
            .function
            .regions
            .iter()
            .find(|region| region.catch_expr == catch_expression_index)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "catch expression",
                    "MIR exception region is absent",
                )
            })?;
        let start_instruction = self.instructions.len();
        self.emit_expression(try_expression)?;
        let handler_instruction = self.instructions.len();
        self.pending_exception_regions.push(PendingExceptionRegion {
            start_instruction,
            handler_instruction,
            catch_slot,
            catch_type: catch_type.clone(),
            cleanup_depth: region.cleanup_depth,
        });
        self.emit_expression(body)
    }

    fn generated_slot_plan(&self, ty: &TypeRefIr) -> skiff_artifact_model::ValueTransferPlan {
        if is_never_type(ty) {
            return skiff_artifact_model::ValueTransferPlan::SnapshotShare {
                drop: skiff_artifact_model::ValueDropPlan::Trivial,
            };
        }
        if is_package_symbol_type(ty) {
            return skiff_artifact_model::ValueTransferPlan::SnapshotShare {
                drop: skiff_artifact_model::ValueDropPlan::SnapshotRelease,
            };
        }
        skiff_artifact_model::ValueTransferPlan::FromType {
            ty: super::constants::qualify_local_types(self.unit.module_path.as_str(), ty),
        }
    }

    fn slot_type(&self, slot: u32) -> Result<&TypeRefIr, BytecodeEmissionError> {
        let index = slot as usize;
        let found = self
            .function
            .slots
            .get(index)
            .filter(|candidate| candidate.slot == slot)
            .or_else(|| {
                self.generated_slots
                    .get(index.saturating_sub(self.function.slots.len()))
                    .filter(|candidate| candidate.slot == slot)
            });
        found
            .and_then(|candidate| candidate.ty.as_ref())
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "slot type",
                    &format!("slot {slot} is absent"),
                )
            })
    }

    fn push_generated_slot(
        &mut self,
        ty: &TypeRefIr,
        name: &str,
    ) -> Result<u32, BytecodeEmissionError> {
        let slot = u32::try_from(self.function.slots.len() + self.generated_slots.len())
            .map_err(|_| arithmetic(&self.key, "generated slot count conversion"))?;
        self.generated_slots.push(MirSlot {
            slot,
            name: name.to_string(),
            kind: MirSlotKind::Local,
            writable_local: false,
            ty: Some(ty.clone()),
        });
        Ok(slot)
    }

    fn emit_number_constant(&mut self, value: u64) -> Result<(), BytecodeEmissionError> {
        let pool = self.image.add_literal_constant(
            self.unit.module_path.as_str(),
            &skiff_artifact_model::LiteralIr::Number {
                value: serde_json::Number::from(value),
            },
            &TypeRefIr::builtin("number"),
            &format!("generated number literal in `{}`", self.key),
        )?;
        self.emit_op(Opcode::Const, vec![pool])?;
        Ok(())
    }

    fn emit_jump_to_instruction(
        &mut self,
        target_instruction: usize,
    ) -> Result<(), BytecodeEmissionError> {
        let instruction = self.emit_op(Opcode::Jump, vec![0])?;
        self.pending_pc_branches.push(PendingPcBranch {
            instruction,
            operand: 0,
            target_instruction,
        });
        Ok(())
    }

    fn begin_expression(&mut self, expression_index: u32) {
        let emission = self
            .expression_emissions
            .entry(expression_index)
            .or_insert(0);
        let occurrence = *emission;
        for (index, event) in self.events.iter().enumerate() {
            if let MirEmissionAnchor::Expression {
                expression_index: anchored,
                occurrence_ordinal,
            } = event.anchor
            {
                if anchored == expression_index && occurrence_ordinal == occurrence {
                    self.event_mapping[index].get_or_insert(self.instructions.len());
                }
            }
        }
        *emission += 1;
    }

    fn map_call_event(&mut self, expression_index: u32) {
        for (index, event) in self.events.iter().enumerate() {
            let matches = matches!(
                event.anchor,
                MirEmissionAnchor::LocalCall {
                    expression_index: anchored,
                    ..
                }
                | MirEmissionAnchor::TailLocalCallCandidate {
                    expression_index: anchored,
                    ..
                } if anchored == expression_index
            );
            if matches {
                self.event_mapping[index].get_or_insert(self.instructions.len() - 1);
            }
        }
    }

    fn map_all_expression_events(&mut self, expression_index: u32) {
        for (index, event) in self.events.iter().enumerate() {
            if let MirEmissionAnchor::Expression {
                expression_index: anchored,
                ..
            } = event.anchor
            {
                if anchored == expression_index {
                    self.event_mapping[index].get_or_insert(self.instructions.len());
                }
            }
        }
    }

    fn map_statement_events(&mut self, statement_index: u32) {
        for (index, event) in self.events.iter().enumerate() {
            let matches = matches!(
                event.anchor,
                MirEmissionAnchor::Statement {
                    statement_index: anchored,
                    ..
                }
                | MirEmissionAnchor::GeneratedStatement {
                    statement_index: anchored,
                    ..
                } if anchored == statement_index
            );
            if matches {
                self.event_mapping[index].get_or_insert(self.instructions.len());
            }
        }
    }

    fn emit_op(
        &mut self,
        opcode: Opcode,
        operands: Vec<u32>,
    ) -> Result<usize, BytecodeEmissionError> {
        let contract = contract_for_opcode(opcode);
        if operands.len() != contract.operands.len() {
            return Err(unsupported(
                &self.key,
                "opcode operand count",
                &format!(
                    "{opcode:?} expected {}, got {}",
                    contract.operands.len(),
                    operands.len()
                ),
            ));
        }
        let instruction = RawInstruction { opcode, operands };
        let (input, output) = stack_effect(&instruction, self.function)?;
        self.operand_depth = self
            .operand_depth
            .checked_sub(input)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "operand stack",
                    "instruction underflows the emitted stack",
                )
            })?
            .checked_add(output)
            .ok_or_else(|| arithmetic(&self.key, "operand stack depth"))?;
        let index = self.instructions.len();
        self.instructions.push(instruction);
        Ok(index)
    }

    fn emit_branch(&mut self, opcode: Opcode, block: u32) -> Result<(), BytecodeEmissionError> {
        let instruction = self.emit_op(opcode, vec![0])?;
        self.pending_branches.push(PendingBranch {
            instruction,
            operand: 0,
            block,
        });
        Ok(())
    }

    fn emit_jump_to_block(&mut self, block: u32) -> Result<(), BytecodeEmissionError> {
        self.emit_branch(Opcode::Jump, block)
    }

    fn patch_branches(&mut self) -> Result<(), BytecodeEmissionError> {
        let pcs = self.instruction_pcs()?;
        for branch in &self.pending_branches {
            let start = self
                .block_starts
                .get(branch.block as usize)
                .copied()
                .flatten()
                .ok_or_else(|| {
                    unsupported(&self.key, "branch target", "target block was not emitted")
                })?;
            let target = pcs[start];
            let current = pcs[branch.instruction];
            let descriptor = descriptor_for_opcode(self.instructions[branch.instruction].opcode);
            let base = current
                .checked_add(1)
                .and_then(|value| value.checked_add(descriptor.operand_word_count()))
                .ok_or_else(|| arithmetic(&self.key, "branch base pc"))?;
            let delta = i64::from(target) - i64::from(base);
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&delta) {
                return Err(unsupported(
                    &self.key,
                    "branch delta",
                    "target is too far away",
                ));
            }
            self.instructions[branch.instruction].operands[branch.operand] = delta as u32;
        }
        Ok(())
    }

    fn patch_pc_branches(&mut self, pcs: &[u32]) -> Result<(), BytecodeEmissionError> {
        for branch in &self.pending_pc_branches {
            let target = *pcs.get(branch.target_instruction).ok_or_else(|| {
                unsupported(
                    &self.key,
                    "instruction branch target",
                    "target instruction was not emitted",
                )
            })?;
            let current = *pcs.get(branch.instruction).ok_or_else(|| {
                unsupported(
                    &self.key,
                    "instruction branch",
                    "branch instruction is absent",
                )
            })?;
            let descriptor = descriptor_for_opcode(self.instructions[branch.instruction].opcode);
            let base = current
                .checked_add(1)
                .and_then(|value| value.checked_add(descriptor.operand_word_count()))
                .ok_or_else(|| arithmetic(&self.key, "pc branch base pc"))?;
            let delta = i64::from(target) - i64::from(base);
            if !(i32::MIN as i64..=i32::MAX as i64).contains(&delta) {
                return Err(unsupported(
                    &self.key,
                    "instruction branch delta",
                    "target is too far away",
                ));
            }
            self.instructions[branch.instruction].operands[branch.operand] = delta as u32;
        }
        Ok(())
    }

    fn patch_resumes(&mut self, pcs: &[u32]) -> Result<(), BytecodeEmissionError> {
        for pending in &self.pending_resumes {
            let site_pc = *pcs.get(pending.instruction).ok_or_else(|| {
                unsupported(&self.key, "resume descriptor", "site instruction is absent")
            })?;
            let descriptor = descriptor_for_opcode(self.instructions[pending.instruction].opcode);
            let resume_pc = site_pc
                .checked_add(descriptor.instruction_word_count())
                .ok_or_else(|| arithmetic(&self.key, "resume pc overflow"))?;
            let result_type_refs = pending
                .result_ty
                .iter()
                .map(|ty| {
                    self.image.type_index(
                        self.unit.module_path.as_str(),
                        ty,
                        &format!("resume result type in `{}`", self.key),
                    )
                })
                .collect::<Result<Vec<_>, BytecodeEmissionError>>()?;
            let result_plans = pending
                .result_ty
                .iter()
                .map(|ty| skiff_artifact_model::ValueTransferPlan::FromType {
                    ty: super::constants::qualify_local_types(self.unit.module_path.as_str(), ty),
                })
                .collect();
            let end_resume_pc = if let Some(end_block) = pending.end_block {
                let ordinal = usize::try_from(end_block)
                    .map_err(|_| arithmetic(&self.key, "resume end block ordinal"))?;
                let start = self
                    .block_starts
                    .get(ordinal)
                    .copied()
                    .flatten()
                    .ok_or_else(|| {
                        unsupported(
                            &self.key,
                            "resume end block",
                            &format!("block {end_block} is absent"),
                        )
                    })?;
                Some(*pcs.get(start).ok_or_else(|| {
                    arithmetic(&self.key, "resume end pc lookup")
                })?)
            } else {
                None
            };
            let pool_index = self.image.add_resume_descriptor(ResumeDescriptor {
                function_key: self.key.clone(),
                site_pc,
                resume_pc,
                end_resume_pc,
                expected_stack_height_before_result: pending.expected_stack_height_before_result,
                result_type_refs,
                result_plans,
                error_mode: ResumeErrorMode::RaiseAtSite,
            })?;
            self.instructions[pending.instruction].operands[pending.operand] = pool_index;
        }
        Ok(())
    }

    fn build_exception_regions(
        &self,
        pcs: &[u32],
    ) -> Result<Vec<ExceptionRegion>, BytecodeEmissionError> {
        let mut regions = Vec::with_capacity(self.pending_exception_regions.len());
        for pending in &self.pending_exception_regions {
            let start_pc = *pcs.get(pending.start_instruction).ok_or_else(|| {
                unsupported(&self.key, "exception region", "start instruction is absent")
            })?;
            let handler_pc = *pcs.get(pending.handler_instruction).ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exception region",
                    "handler instruction is absent",
                )
            })?;
            let catch_slot_type_ref = self.image.type_index(
                self.unit.module_path.as_str(),
                &pending.catch_type,
                &format!("exception catch type in `{}`", self.key),
            )?;
            regions.push(ExceptionRegion {
                start_pc,
                end_pc: handler_pc,
                handler_pc,
                handler_stack_height: 0,
                catch_matchers: vec![CatchMatcher::TypeRef {
                    type_ref: catch_slot_type_ref,
                }],
                catch_slot: pending.catch_slot,
                catch_slot_type_ref,
                cleanup_depth: pending.cleanup_depth,
            });
        }
        Ok(regions)
    }

    fn encode(&self) -> Result<(Vec<u32>, Vec<u32>), BytecodeEmissionError> {
        let mut words = Vec::new();
        let mut pcs = Vec::with_capacity(self.instructions.len());
        for instruction in &self.instructions {
            pcs.push(
                u32::try_from(words.len())
                    .map_err(|_| arithmetic(&self.key, "function word pc conversion"))?,
            );
            let opcode = contract_for_opcode(instruction.opcode).opcode;
            words.extend(encode_instruction(opcode, &instruction.operands)?);
        }
        Ok((words, pcs))
    }

    fn instruction_pcs(&self) -> Result<Vec<u32>, BytecodeEmissionError> {
        let mut pcs = Vec::with_capacity(self.instructions.len());
        let mut pc = 0_u32;
        for instruction in &self.instructions {
            pcs.push(pc);
            let descriptor = descriptor_for_opcode(instruction.opcode);
            pc = pc
                .checked_add(descriptor.instruction_word_count())
                .ok_or_else(|| arithmetic(&self.key, "instruction pc overflow"))?;
        }
        Ok(pcs)
    }

    fn compute_max_operand_depth(&self) -> Result<u32, BytecodeEmissionError> {
        let mut depth = 0_usize;
        let mut max = 0_usize;
        for instruction in &self.instructions {
            let (input, output) = stack_effect(instruction, self.function)?;
            depth = depth.checked_sub(input).ok_or_else(|| {
                unsupported(
                    &self.key,
                    "operand stack",
                    "instruction underflows the emitted stack",
                )
            })?;
            depth = depth
                .checked_add(output)
                .ok_or_else(|| arithmetic(&self.key, "operand stack depth"))?;
            max = max.max(depth);
        }
        check_limit(
            "MAX_OPERAND_DEPTH",
            &format!("function `{key}` operand depth", key = self.key),
            max,
            limits::MAX_OPERAND_DEPTH,
        )?;
        u32::try_from(max).map_err(|_| arithmetic(&self.key, "operand depth conversion"))
    }

    fn build_statement_entries(
        &self,
        pcs: &[u32],
    ) -> Result<Vec<StatementEntry>, BytecodeEmissionError> {
        let mut rows = Vec::new();
        for (event_index, instruction) in self.event_mapping.iter().enumerate() {
            let instruction_index = instruction.ok_or_else(|| {
                unsupported(
                    &self.key,
                    "source event placement",
                    &format!(
                        "event {event_index} ({:?}) was not anchored to emitted code",
                        self.events[event_index].anchor
                    ),
                )
            })?;
            let pc = *pcs
                .get(instruction_index)
                .ok_or_else(|| arithmetic(&self.key, "source event instruction lookup"))?;
            rows.push((
                pc,
                event_index,
                self.events[event_index].attribution_id,
                self.events[event_index].site.clone(),
            ));
        }
        rows.sort_by_key(|row| (row.0, row.1));
        let mut entries = Vec::with_capacity(rows.len());
        let mut previous_pc = None;
        let mut sequence = 0_u32;
        for (pc, _, attribution_id, site) in rows {
            if previous_pc == Some(pc) {
                sequence = sequence
                    .checked_add(1)
                    .ok_or_else(|| arithmetic(&self.key, "statement sequence ordinal"))?;
            } else {
                previous_pc = Some(pc);
                sequence = 0;
            }
            entries.push(StatementEntry {
                pc,
                sequence_ordinal: sequence,
                attribution_id,
                site,
            });
        }
        Ok(entries)
    }

    fn build_source_map(
        &self,
        word_count: usize,
    ) -> Result<Vec<SourceMapEntry>, BytecodeEmissionError> {
        if word_count == 0 {
            return Err(unsupported(
                &self.key,
                "function source map",
                "function has no instructions",
            ));
        }
        Ok(vec![SourceMapEntry {
            start_pc: 0,
            end_pc: u32::try_from(word_count)
                .map_err(|_| arithmetic(&self.key, "source map word count"))?,
            site: InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerDesugaring,
            },
        }])
    }

    fn build_frame(&self) -> Result<FrameLayout, BytecodeEmissionError> {
        let slot_count = self.function.slots.len() + self.generated_slots.len();
        let mut slot_type_refs = Vec::with_capacity(slot_count);
        for slot in &self.function.slots {
            let ty = slot.ty.as_ref().ok_or_else(|| {
                unsupported(
                    &self.key,
                    "frame slot type",
                    &format!("slot `{name}` has no exact type", name = slot.name),
                )
            })?;
            slot_type_refs.push(self.image.type_index(
                self.unit.module_path.as_str(),
                ty,
                &format!(
                    "function `{key}` slot `{name}` type",
                    key = self.key,
                    name = slot.name
                ),
            )?);
        }
        let mut slot_plans = self.plans.slot_plans.clone();
        for generated in &self.generated_slots {
            let ty = generated.ty.as_ref().ok_or_else(|| {
                unsupported(
                    &self.key,
                    "generated frame slot type",
                    &format!("slot `{}` has no exact type", generated.name),
                )
            })?;
            slot_type_refs.push(self.image.type_index(
                self.unit.module_path.as_str(),
                ty,
                &format!(
                    "function `{key}` generated slot `{name}` type",
                    key = self.key,
                    name = generated.name
                ),
            )?);
            slot_plans.push(self.generated_slot_plan(ty));
        }
        let mut parameter_slots = Vec::new();
        for parameter in &self.function.params {
            let slot = parameter.slot as usize;
            let plan = self.plans.slot_plans.get(slot).cloned().ok_or_else(|| {
                unsupported(
                    &self.key,
                    "parameter transfer plan",
                    &format!("parameter `{name}` has no slot plan", name = parameter.name),
                )
            })?;
            parameter_slots.push(ParameterSlotDecl {
                slot: parameter.slot,
                mode: match parameter.mode {
                    MirParamMode::Value => ParamModeIr::Value,
                    MirParamMode::InOut => ParamModeIr::InOut,
                },
                plan,
            });
        }
        let is_stream_producer = self.function.stream_result.is_some();
        let result_count = if is_stream_producer {
            0
        } else {
            usize::from(!is_void(&self.function.return_type))
        };
        let result_type_refs = if result_count == 0 {
            Vec::new()
        } else {
            vec![self.image.type_index(
                self.unit.module_path.as_str(),
                &self.function.return_type,
                &format!("function `{key}` return type", key = self.key),
            )?]
        };
        let stream_result_type_ref = if is_stream_producer {
            Some(self.image.type_index(
                self.unit.module_path.as_str(),
                &self.function.return_type,
                &format!("function `{key}` stream authority type", key = self.key),
            )?)
        } else {
            None
        };
        let result_plans = if is_stream_producer {
            Vec::new()
        } else {
            self.plans.result_plans.clone()
        };
        let writable_local_slots = self
            .function
            .slots
            .iter()
            .filter(|slot| slot.writable_local)
            .map(|slot| slot.slot)
            .collect();
        Ok(FrameLayout {
            slot_count: u32::try_from(slot_count)
                .map_err(|_| arithmetic(&self.key, "frame slot count conversion"))?,
            slot_type_refs,
            parameter_slots,
            writable_local_slots,
            result_count: u32::try_from(result_count)
                .map_err(|_| arithmetic(&self.key, "frame result count conversion"))?,
            result_type_refs,
            result_plans,
            stream_result_type_ref,
            slot_plans,
        })
    }
}

fn stack_effect(
    instruction: &RawInstruction,
    function: &MirFunction,
) -> Result<(usize, usize), BytecodeEmissionError> {
    Ok(match instruction.opcode {
        Opcode::Const | Opcode::LoadSlot | Opcode::TakeSlot => (0, 1),
        Opcode::StoreSlot | Opcode::Pop | Opcode::Drop => (1, 0),
        Opcode::Dup => (1, 2),
        Opcode::CopySlot
        | Opcode::MoveSlot
        | Opcode::Jump
        | Opcode::BudgetCheckpoint
        | Opcode::Rethrow
        | Opcode::EnterRegion
        | Opcode::LeaveRegion => (0, 0),
        Opcode::JumpIfTrue | Opcode::JumpIfFalse => (1, 0),
        Opcode::CallLocal => {
            let arguments = instruction.operands[1] as usize;
            let results = instruction.operands[2] as usize;
            (arguments, results)
        }
        Opcode::TailCallLocal => (instruction.operands[1] as usize, 0),
        Opcode::CallService | Opcode::CallActor | Opcode::InvokeHost => {
            let arguments = instruction.operands[1] as usize;
            let results = instruction.operands[2] as usize;
            (arguments, results)
        }
        Opcode::CallInterface => {
            let arguments = instruction.operands[2] as usize;
            let results = instruction.operands[3] as usize;
            (arguments + 1, results)
        }
        Opcode::InvokeIntrinsic => {
            let arguments = instruction.operands[1] as usize;
            let results = instruction.operands[2] as usize;
            (arguments, results)
        }
        Opcode::Return => (return_count(function), 0),
        Opcode::Not | Opcode::Negate => (1, 1),
        Opcode::Add
        | Opcode::Subtract
        | Opcode::Multiply
        | Opcode::Divide
        | Opcode::Equal
        | Opcode::NotEqual
        | Opcode::LessThan
        | Opcode::LessOrEqual
        | Opcode::GreaterThan
        | Opcode::GreaterOrEqual => (2, 1),
        Opcode::Trap | Opcode::Throw => (1, 0),
        Opcode::NewRecord => (instruction.operands[1] as usize, 1),
        Opcode::GetDenseField | Opcode::RepresentationWrap => (1, 1),
        Opcode::SetWritablePath => (instruction.operands[2] as usize + 1, 0),
        Opcode::NewArrayBuilder | Opcode::NewMapBuilder => (0, 1),
        Opcode::ArrayBuilderPush => (2, 1),
        Opcode::MapBuilderPut => (3, 1),
        Opcode::FreezeArray | Opcode::FreezeMap => (1, 1),
        Opcode::ArrayGet | Opcode::MapGet => (2, 1),
        Opcode::ArrayLen | Opcode::MapLen => (1, 1),
        Opcode::MapEntryAt => (2, 2),
        Opcode::InterfaceBoxLocal => (1, 1),
        Opcode::InterfaceBoxRemote => (0, 1),
        Opcode::StreamNext => (0, 1),
        Opcode::EmitStream => (1, 0),
        _ => {
            return Err(unsupported(
                "scalar emitter",
                "opcode stack effect",
                "opcode is outside the scalar emitter subset",
            ));
        }
    })
}

fn binary_opcode(op: skiff_artifact_model::BinaryOpIr) -> Result<Opcode, BytecodeEmissionError> {
    Ok(match op {
        skiff_artifact_model::BinaryOpIr::Add => Opcode::Add,
        skiff_artifact_model::BinaryOpIr::Subtract => Opcode::Subtract,
        skiff_artifact_model::BinaryOpIr::Multiply => Opcode::Multiply,
        skiff_artifact_model::BinaryOpIr::Divide => Opcode::Divide,
        skiff_artifact_model::BinaryOpIr::Equal => Opcode::Equal,
        skiff_artifact_model::BinaryOpIr::NotEqual => Opcode::NotEqual,
        skiff_artifact_model::BinaryOpIr::LessThan => Opcode::LessThan,
        skiff_artifact_model::BinaryOpIr::LessThanOrEqual => Opcode::LessOrEqual,
        skiff_artifact_model::BinaryOpIr::GreaterThan => Opcode::GreaterThan,
        skiff_artifact_model::BinaryOpIr::GreaterThanOrEqual => Opcode::GreaterOrEqual,
        skiff_artifact_model::BinaryOpIr::And | skiff_artifact_model::BinaryOpIr::Or => {
            return Err(unsupported(
                "scalar emitter",
                "logical binary op",
                "And/Or are outside the scalar emitter subset",
            ));
        }
    })
}

fn stream_item_type_matches(actual: &TypeRefIr, expected: &TypeRefIr) -> bool {
    actual == expected
        || matches!(
            (actual, expected),
            (
                TypeRefIr::Builtin {
                    name: actual_name,
                    args: actual_args,
                },
                TypeRefIr::Builtin {
                    name: expected_name,
                    args: expected_args,
                },
            ) if actual_name == "integer"
                && expected_name == "number"
                && actual_args.is_empty()
                && expected_args.is_empty()
        )
}

fn is_never_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if name == "never" && args.is_empty()
    )
}

fn is_package_symbol_type(ty: &TypeRefIr) -> bool {
    matches!(ty, TypeRefIr::PackageSymbol { .. })
        || matches!(
            ty,
            TypeRefIr::AppliedNominal {
                base: skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol { .. },
                ..
            }
        )
}

fn literal_type(literal: &LiteralIr) -> TypeRefIr {
    TypeRefIr::builtin(match literal {
        LiteralIr::Null => "null",
        LiteralIr::Bool { .. } => "bool",
        LiteralIr::Number { .. } => "number",
        LiteralIr::String { .. } => "string",
    })
}

fn require_narrow_target(caller: &str, target: &MirFunction) -> Result<(), BytecodeEmissionError> {
    if !target.type_params.is_empty() || target.self_type.is_some() {
        return Err(unsupported(
            caller,
            "generic or receiver-bound target",
            &format!(
                "target `{symbol}` is not a narrow scalar function",
                symbol = target.symbol
            ),
        ));
    }
    Ok(())
}

fn unsupported(function_key: &str, construct: &'static str, detail: &str) -> BytecodeEmissionError {
    BytecodeEmissionError::UnsupportedConstruct {
        function_key: function_key.to_string(),
        construct,
        location: format!(" {detail}"),
    }
}

fn arithmetic(_function_key: &str, context: &'static str) -> BytecodeEmissionError {
    BytecodeEmissionError::ArithmeticOverflow { context }
}

fn check_limit(
    limit: &'static str,
    location: &str,
    actual: usize,
    max: u64,
) -> Result<(), BytecodeEmissionError> {
    if actual as u64 > max {
        return Err(BytecodeEmissionError::LimitExceeded {
            limit,
            location: location.to_string(),
            actual: actual as u64,
            max,
        });
    }
    Ok(())
}

fn return_count(function: &MirFunction) -> usize {
    usize::from(!is_void(&function.return_type) && function.stream_result.is_none())
}
