use std::collections::BTreeMap;

use skiff_artifact_model::{
    bytecode::encode_instruction, bytecode::limits, contract_for_opcode, descriptor_for_opcode,
    AssignTargetIr, BytecodeFunctionOrigin, BytecodeRelocation, BytecodeSpecialization,
    CallTargetIr, ExprIr, ExprRefIr, FrameLayout, InstructionSourceSite, LiteralIr, Opcode,
    ParamModeIr, ParameterSlotDecl, RelocatableBytecodeFunction, SourceMapEntry, StatementEntry,
    SyntheticInstructionSiteReason, TypeRefIr,
};
use skiff_compiler_lowering::mir::{
    MirCallArgument, MirDirectCallFacts, MirEmissionAnchor, MirExpression, MirFunction,
    MirParamMode, MirSourceEvent, MirStmtKind, MirUnit,
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
    block_starts: Vec<Option<usize>>,
    events: Vec<MirSourceEvent>,
    event_mapping: Vec<Option<usize>>,
    expression_emissions: BTreeMap<u32, u32>,
    current_block: u32,
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
            .ok_or_else(|| {
                unsupported(
                    key,
                    "function with unavailable source-event plan",
                    "emitter requires a finalized MIR source-event plan",
                )
            })?
            .to_vec();
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
            block_starts: vec![None; function.blocks.len()],
            events,
            event_mapping: vec![None; event_count],
            expression_emissions: BTreeMap::new(),
            current_block: 0,
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
            exception_regions: Vec::new(),
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

        match block.successors.as_slice() {
            [] => {}
            [successor] => self.emit_jump_to_block(*successor)?,
            _ => {}
        }
        Ok(())
    }

    fn emit_statement(
        &mut self,
        statement: &skiff_compiler_lowering::mir::MirStmt,
    ) -> Result<(), BytecodeEmissionError> {
        match &statement.kind {
            MirStmtKind::Let { slot, value } => {
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
            MirStmtKind::Expr { value } => {
                let expression = self.function.expression(*value)?;
                self.emit_expression(*value)?;
                if !is_void(&expression.ty) {
                    self.emit_op(Opcode::Pop, Vec::new())?;
                }
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
            ExprIr::Binary { op, left, right } => {
                self.emit_expression(*left)?;
                self.emit_expression(*right)?;
                let opcode = binary_opcode(*op)?;
                self.emit_op(opcode, Vec::new())?;
            }
            ExprIr::Call { .. } => {
                if !self.try_emit_ordinary_call(expression)? {
                    return Err(unsupported(
                        &self.key,
                        "call expression",
                        "non-direct calls are outside the scalar emitter subset",
                    ));
                }
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

    fn try_emit_tail_call(
        &mut self,
        expression: &MirExpression,
    ) -> Result<bool, BytecodeEmissionError> {
        if !matches!(expression.expression, ExprIr::Call { .. }) {
            return Ok(false);
        }
        self.map_all_expression_events(expression.index);
        self.emit_direct_call(expression, true)
    }

    fn try_emit_ordinary_call(
        &mut self,
        expression: &MirExpression,
    ) -> Result<bool, BytecodeEmissionError> {
        if !matches!(expression.expression, ExprIr::Call { .. }) {
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
        let index = self.instructions.len();
        self.instructions.push(RawInstruction { opcode, operands });
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
                    &format!("event {event_index} was not anchored to emitted code"),
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
        let slot_count = self.function.slots.len();
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
        let result_count = usize::from(!is_void(&self.function.return_type));
        let result_type_refs = if result_count == 0 {
            Vec::new()
        } else {
            vec![self.image.type_index(
                self.unit.module_path.as_str(),
                &self.function.return_type,
                &format!("function `{key}` return type", key = self.key),
            )?]
        };
        let result_plans = self.plans.result_plans.clone();
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
            slot_plans: self.plans.slot_plans.clone(),
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
        Opcode::CopySlot | Opcode::MoveSlot | Opcode::Jump | Opcode::BudgetCheckpoint => (0, 0),
        Opcode::JumpIfTrue | Opcode::JumpIfFalse => (1, 0),
        Opcode::CallLocal => {
            let arguments = instruction.operands[1] as usize;
            let results = instruction.operands[2] as usize;
            (arguments, results)
        }
        Opcode::TailCallLocal => (instruction.operands[1] as usize, 0),
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
    usize::from(!is_void(&function.return_type))
}
