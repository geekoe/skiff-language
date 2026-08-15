use std::collections::{BTreeMap, BTreeSet};

use skiff_artifact_model::bytecode::dto::{
    DbOperandRole, DbOperationKind, DbOperationReference, TaskSubmitReference, TaskSubmitTargetRef,
    TaskSubmitTimingRef,
};
use skiff_artifact_model::{
    bytecode::encode_instruction, bytecode::limits, contract_for_opcode, descriptor_for_opcode,
    AssignTargetIr, BoxSourceIr, BytecodeFunctionOrigin, BytecodeIntrinsicRef, BytecodeRelocation,
    BytecodeSpecialization, CallLoanLayout, CallTargetIr, CallableMayEffects, CatchMatcher,
    DbBodyIr, DbOpKindIr, DbTargetIr, ExceptionRegion, ExprIr, ExprRefIr, FrameLayout,
    FunctionTypeParamIr, HostEffectReference, HostEffectSignature, InstructionSourceSite,
    InterfaceInstantiationRef, InterfaceMethodSlotSignatureIr, InterfaceRequirementMethod,
    IntrinsicReference, LiteralIr, LocalInterfaceMethod, LocalInterfaceRef, MetadataValue,
    NativeTarget, Opcode, ParamModeIr, ParameterSlotDecl, PatternIr, PrivilegedAffineFieldAccess,
    RelocatableBytecodeFunction, RemoteInterfaceMethod, RemoteInterfaceRef, ResumeDescriptor,
    ResumeErrorMode, ResumeResultMaterialization, ServiceBoundaryPlan, ServiceCallRef,
    SourceMapEntry, StatementAttributionId, StatementEntry, SyntheticInstructionSiteReason,
    TrapFailureKind, TypeRefIr, ValueDropPlan, ValueTransferPlan, WritablePathSegment,
};
use skiff_compiler_lowering::mir::{
    MirCallArgument, MirDirectCallFacts, MirEmissionAnchor, MirExpression, MirForInItemKind,
    MirFunction, MirIndexReceiverKind, MirParamMode, MirSlot, MirSlotKind, MirSourceEvent,
    MirStatementPlacement, MirStmtKind, MirUnit, MirWritablePathSegment, MirWritablePlace,
    MirWritableRoot,
};

use super::{
    admission::LocalInterfaceFacts,
    carriers::{
        FunctionMachineCarrierFacts, MachineDefaultValueFact, MachineDefaultValueKind,
        MachineShapeCarrierFact, MachineWritableStepFact,
    },
    constants::{qualify_local_types, ConstantImage},
    inputs::ValidatedEmissionInputs,
    BytecodeEmissionError, FunctionValueTransferPlans,
};
use super::{inputs::is_void, intrinsics::static_intrinsic_canonical_key};

const TASK_SUBMIT_METADATA_KEY: &str = "dispatchSubmit";

pub(super) fn emit_functions(
    inputs: &ValidatedEmissionInputs<'_>,
    image: &mut ConstantImage,
    source_attribution: SourceAttributionMode,
    local_interface_tables: &LocalInterfaceFacts,
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
        let emitter = FunctionEmitter::new(
            unit,
            function,
            function_key,
            plans,
            image,
            inputs,
            inputs.service_boundary_plans,
            local_interface_tables,
            source_attribution,
        )?;
        functions.insert(function_key.clone(), emitter.emit()?);
    }
    Ok(functions)
}

#[derive(Clone, Copy)]
pub(super) enum SourceAttributionMode {
    AdmittedPhase1,
    PrivateBackend,
}

struct FunctionEmitter<'a> {
    unit: &'a MirUnit,
    function: &'a MirFunction,
    key: String,
    plans: &'a FunctionValueTransferPlans,
    machine_carriers: &'a FunctionMachineCarrierFacts,
    image: &'a mut ConstantImage,
    inputs: &'a ValidatedEmissionInputs<'a>,
    service_boundary_plans: &'a BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
    local_interface_tables: &'a LocalInterfaceFacts,
    source_attribution: SourceAttributionMode,
    instructions: Vec<RawInstruction>,
    relocations: Vec<BytecodeRelocation>,
    pending_branches: Vec<PendingBranch>,
    pending_pc_branches: Vec<PendingPcBranch>,
    pending_resumes: Vec<PendingResume>,
    pending_exception_regions: Vec<PendingExceptionRegion>,
    call_loan_layouts: Vec<CallLoanLayout>,
    block_starts: Vec<Option<usize>>,
    events: Vec<MirSourceEvent>,
    event_mapping: Vec<Option<usize>>,
    expression_emissions: BTreeMap<u32, u32>,
    current_block: u32,
    generated_slots: Vec<MirSlot>,
    loop_backedges: BTreeMap<u32, LoopBackedge>,
    stream_loop_item_states: BTreeMap<(u32, u32), EmittedSlotState>,
    value_block_body_blocks: BTreeSet<u32>,
    throw_source_sites: Vec<(usize, InstructionSourceSite)>,
    stream_source_sites: Vec<(usize, InstructionSourceSite)>,
    generated_source_sites: Vec<(usize, InstructionSourceSite)>,
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

#[derive(Clone)]
struct PendingResume {
    instruction: usize,
    operand: usize,
    expected_stack_height_before_result: u32,
    result_ty: Option<TypeRefIr>,
    result_expression: Option<u32>,
    result_materialization: Option<ResumeResultMaterialization>,
    emit_stream_item_shape_ref: Option<u32>,
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
    header_block: u32,
    continuation_block: u32,
    header_instruction: usize,
    iterable_slot: u32,
    index_slot: u32,
    item_slot: u32,
    value_slot: Option<u32>,
    array: bool,
    stream: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EmittedSlotState {
    Live,
    Empty,
}

impl<'a> FunctionEmitter<'a> {
    fn new(
        unit: &'a MirUnit,
        function: &'a MirFunction,
        key: &str,
        plans: &'a FunctionValueTransferPlans,
        image: &'a mut ConstantImage,
        inputs: &'a ValidatedEmissionInputs<'a>,
        service_boundary_plans: &'a BTreeMap<ServiceCallRef, ServiceBoundaryPlan>,
        local_interface_tables: &'a LocalInterfaceFacts,
        source_attribution: SourceAttributionMode,
    ) -> Result<Self, BytecodeEmissionError> {
        let events = function
            .source_event_plan
            .events()
            .map(<[MirSourceEvent]>::to_vec)
            .unwrap_or_default();
        let event_count = events.len();
        let value_block_body_blocks = value_block_body_blocks(function)?;
        let machine_carriers = inputs.machine_carriers.function(key).ok_or_else(|| {
            unsupported(
                key,
                "exact machine carrier facts",
                "function carrier row is absent",
            )
        })?;
        Ok(Self {
            unit,
            function,
            key: key.to_string(),
            plans,
            machine_carriers,
            image,
            inputs,
            service_boundary_plans,
            local_interface_tables,
            source_attribution,
            instructions: Vec::new(),
            relocations: Vec::new(),
            pending_branches: Vec::new(),
            pending_pc_branches: Vec::new(),
            pending_resumes: Vec::new(),
            pending_exception_regions: Vec::new(),
            call_loan_layouts: Vec::new(),
            block_starts: vec![None; function.blocks.len()],
            events,
            event_mapping: vec![None; event_count],
            expression_emissions: BTreeMap::new(),
            current_block: 0,
            generated_slots: Vec::new(),
            loop_backedges: BTreeMap::new(),
            stream_loop_item_states: BTreeMap::new(),
            value_block_body_blocks,
            throw_source_sites: Vec::new(),
            stream_source_sites: Vec::new(),
            generated_source_sites: Vec::new(),
            operand_depth: 0,
        })
    }

    fn expression_carrier(&self, expression: u32) -> Result<&TypeRefIr, BytecodeEmissionError> {
        self.machine_carriers
            .expression(expression)
            .map(|carrier| carrier.ty())
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact machine carrier facts",
                    &format!("expression {expression} carrier is absent"),
                )
            })
    }

    fn slot_carrier(&self, slot: u32) -> Result<&TypeRefIr, BytecodeEmissionError> {
        self.machine_carriers
            .slot(slot)
            .map(|carrier| carrier.ty())
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact machine carrier facts",
                    &format!("slot {slot} carrier is absent"),
                )
            })
    }

    fn emitted_slot_carrier(&self, slot: u32) -> Result<TypeRefIr, BytecodeEmissionError> {
        if let Some(carrier) = self.machine_carriers.slot(slot) {
            return Ok(carrier.ty().clone());
        }
        let generated = usize::try_from(slot)
            .ok()
            .and_then(|slot| slot.checked_sub(self.function.slots.len()))
            .and_then(|slot| self.generated_slots.get(slot))
            .and_then(|slot| slot.ty.clone());
        generated.ok_or_else(|| {
            unsupported(
                &self.key,
                "exact generated slot carrier",
                &format!("slot {slot} carrier is absent"),
            )
        })
    }

    fn result_carrier(&self) -> Result<&TypeRefIr, BytecodeEmissionError> {
        self.machine_carriers
            .result()
            .map(|carrier| carrier.ty())
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact machine carrier facts",
                    "non-void function result carrier is absent",
                )
            })
    }

    fn machine_expression_shape_fields(
        &self,
        expression: u32,
        owner: &TypeRefIr,
        context: &'static str,
    ) -> Result<BTreeMap<String, TypeRefIr>, BytecodeEmissionError> {
        self.machine_carriers
            .expression_shape(expression)
            .filter(|shape| shape.owner() == owner)
            .map(machine_shape_fields)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact expression producer shape",
                    &format!(
                        "{context} expression {expression} owner {owner:?} has no analyzed field layout"
                    ),
                )
            })
    }

    fn machine_slot_shape_fields(
        &self,
        slot: u32,
        owner: &TypeRefIr,
        context: &'static str,
    ) -> Result<BTreeMap<String, TypeRefIr>, BytecodeEmissionError> {
        self.machine_carriers
            .slot_shape(slot)
            .filter(|shape| shape.owner() == owner)
            .map(machine_shape_fields)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact slot producer shape",
                    &format!("{context} slot {slot} owner {owner:?} has no analyzed field layout"),
                )
            })
    }

    fn machine_construct_shape_fields(
        &self,
        expression: u32,
        owner: &TypeRefIr,
        context: &'static str,
    ) -> Result<BTreeMap<String, TypeRefIr>, BytecodeEmissionError> {
        self.machine_carriers
            .construct_shape(expression)
            .filter(|shape| shape.owner() == owner)
            .map(machine_shape_fields)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact construct carrier shape",
                    &format!(
                        "{context} expression {expression} owner {owner:?} has no analyzed field layout"
                    ),
                )
            })
    }

    fn emit(mut self) -> Result<RelocatableBytecodeFunction, BytecodeEmissionError> {
        for block in &self.function.blocks {
            if self.value_block_body_blocks.contains(&block.id) {
                continue;
            }
            let ordinal = usize::try_from(block.id)
                .map_err(|_| arithmetic(self.key.as_str(), "block id to usize conversion"))?;
            let start = self.instructions.len();
            self.block_starts[ordinal] = Some(start);
            self.current_block = block.id;
            self.emit_block(block)?;
        }
        if let Some(last) = self.instructions.last() {
            if matches!(
                contract_for_opcode(last.opcode).control,
                skiff_artifact_model::ControlContract::Fallthrough
            ) && (is_void(&self.function.return_type) || self.function.stream_result.is_some())
            {
                self.emit_op(Opcode::Return, Vec::new())?;
            }
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
        let source_map = self.build_source_map(&instruction_pcs, words.len())?;
        let frame = self.build_frame()?;
        let self_type_ref = self
            .function
            .self_type
            .as_ref()
            .map(|ty| {
                self.image.type_index(
                    self.unit.module_path.as_str(),
                    ty,
                    &format!("function `{key}` self type", key = self.key),
                )
            })
            .transpose()?;
        let origin = BytecodeFunctionOrigin::Executable {
            executable: self.function.origin.clone(),
        };
        Ok(RelocatableBytecodeFunction {
            function_key: self.key.clone(),
            origin,
            type_parameters: self.function.type_params.clone(),
            self_type_ref,
            words,
            relocations: self.relocations,
            call_loan_layouts: self.call_loan_layouts,
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
        self.emit_value_block_block(block)
    }

    fn emit_value_block(
        &mut self,
        expression_index: u32,
        result: ExprRefIr,
    ) -> Result<(), BytecodeEmissionError> {
        let fact = self
            .function
            .expression_blocks
            .get(&expression_index)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "ValueBlock",
                    &format!("expression {expression_index} has no exact completion facts"),
                )
            })?;
        let completion_targets = fact
            .completion_targets
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if completion_targets.is_empty() {
            return Err(unsupported(
                &self.key,
                "ValueBlock",
                "expression completion target set is empty",
            ));
        }
        let body_ids = value_block_body_ids(self.function, fact)?;
        let body_depth = self.operand_depth;
        let mut completion_edges = Vec::new();
        for &block_id in &body_ids {
            let ordinal = usize::try_from(block_id)
                .map_err(|_| arithmetic(self.key.as_str(), "ValueBlock block id conversion"))?;
            let block = self.function.blocks.get(ordinal).ok_or_else(|| {
                unsupported(
                    &self.key,
                    "ValueBlock",
                    &format!("body block {block_id} is absent"),
                )
            })?;
            let start = self.instructions.len();
            self.block_starts[ordinal] = Some(start);
            self.current_block = block_id;
            self.operand_depth = body_depth;
            let branch_start = self.pending_branches.len();
            self.emit_value_block_block(block)?;
            self.redirect_value_block_completions(
                block_id,
                fact,
                branch_start,
                &body_ids,
                &mut completion_edges,
            )?;
        }
        self.operand_depth = body_depth;
        let resume_instruction = self.instructions.len();
        for edge in &mut completion_edges {
            edge.target_instruction = resume_instruction;
        }
        self.pending_pc_branches.extend(completion_edges);
        self.emit_expression(result)
    }

    fn redirect_value_block_completions(
        &mut self,
        block_id: u32,
        fact: &skiff_compiler_lowering::mir::MirExpressionBlockFact,
        start: usize,
        body_blocks: &BTreeSet<u32>,
        completion_edges: &mut Vec<PendingPcBranch>,
    ) -> Result<(), BytecodeEmissionError> {
        let mut retained = Vec::new();
        for branch in self.pending_branches.drain(start..) {
            if fact.completion_targets.contains(&block_id)
                && branch.block == fact.continuation_block
            {
                completion_edges.push(PendingPcBranch {
                    instruction: branch.instruction,
                    operand: branch.operand,
                    target_instruction: 0,
                });
            } else if body_blocks.contains(&branch.block) {
                retained.push(branch);
            } else {
                return Err(unsupported(
                    &self.key,
                    "ValueBlock CFG",
                    &format!(
                        "body block {block_id} escapes to unexpected block {}",
                        branch.block
                    ),
                ));
            }
        }
        self.pending_branches.extend(retained);
        Ok(())
    }

    fn emit_db_operation(
        &mut self,
        expression: &MirExpression,
        operation: &skiff_artifact_model::DbOperationIr,
    ) -> Result<(), BytecodeEmissionError> {
        if operation.op != DbOpKindIr::Insert || operation.many {
            return Err(unsupported(
                &self.key,
                "DbOperation",
                "bytecode F6 facts currently admit single-object db insert only",
            ));
        }
        if operation.selector.is_some()
            || operation.query.is_some()
            || operation.change.is_some()
            || operation.projection.is_some()
        {
            return Err(unsupported(
                &self.key,
                "DbOperation",
                "single-object db insert must not carry selector/query/change facts",
            ));
        }
        let body = operation
            .body
            .as_ref()
            .or(operation.insert_body.as_ref())
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "DbOperation",
                    "single-object db insert has no object body",
                )
            })?;
        let DbBodyIr::ObjectFields { fields } = body else {
            return Err(unsupported(
                &self.key,
                "DbOperation",
                "bytecode F6 facts currently admit object-field insert only",
            ));
        };
        let target_type = self.db_object_publication_type(&operation.target.type_ref);
        let construct_fields =
            self.record_construct_fields(&operation.target.type_ref, fields, "db insert object")?;
        let shape = self.image.intern_shape(
            self.unit.module_path.as_str(),
            &target_type,
            &construct_fields,
            &format!("db insert object shape in `{}`", self.key),
        )?;
        for name in construct_fields.keys() {
            self.emit_expression(
                *fields
                    .get(name)
                    .expect("record_construct_fields checked the field set"),
            )?;
        }
        let field_count = u32::try_from(construct_fields.len())
            .map_err(|_| arithmetic(&self.key, "db insert field count conversion"))?;
        self.emit_op(Opcode::NewRecord, vec![shape, field_count])?;

        let intrinsic = self.db_intrinsic_reference(operation, &target_type)?;
        let relocation_index = u32::try_from(self.relocations.len())
            .map_err(|_| arithmetic(&self.key, "db relocation index conversion"))?;
        self.relocations
            .push(BytecodeRelocation::IntrinsicRef { intrinsic });
        let operands = vec![relocation_index, 1, 1];
        let instruction = self.emit_op(Opcode::InvokeIntrinsic, operands)?;
        self.generated_source_sites.push((
            instruction,
            InstructionSourceSite::Synthetic {
                reason: SyntheticInstructionSiteReason::CompilerDesugaring,
            },
        ));
        self.emit_number_constant(0)?;
        let site_instruction = self.emit_op(Opcode::Pop, Vec::new())?;
        for (index, event) in self.events.iter().enumerate() {
            if matches!(
                event.anchor,
                MirEmissionAnchor::Expression {
                    expression_index: anchored,
                    ..
                } if anchored == expression.index
            ) {
                self.event_mapping[index].get_or_insert(site_instruction);
            }
        }
        self.map_call_event(expression.index);
        Ok(())
    }

    fn db_intrinsic_reference(
        &mut self,
        operation: &skiff_artifact_model::DbOperationIr,
        target_type: &TypeRefIr,
    ) -> Result<IntrinsicReference, BytecodeEmissionError> {
        let mut effects = skiff_artifact_model::host_effect_registry()
            .entries()
            .iter()
            .find(|entry| entry.binding_key == "std.db.operation")
            .map(|entry| entry.signature.effects.clone())
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "db operation",
                    "std.db.operation is absent from the frozen host registry",
                )
            })?;
        effects.may_pending = false;
        effects.pending_effect_categories.clear();
        let parameter_plan = self.image.exact_type_plan(
            self.unit.module_path.as_str(),
            target_type,
            &format!("db insert parameter plan in `{}`", self.key),
        )?;
        let result_type = self.db_object_publication_type(&operation.result_type);
        let result_plan = self.image.exact_type_plan(
            self.unit.module_path.as_str(),
            &result_type,
            &format!("db insert result plan in `{}`", self.key),
        )?;
        let signature = HostEffectSignature {
            parameter_types: vec![target_type.clone()],
            parameter_modes: vec![ParamModeIr::Value],
            parameter_plans: vec![parameter_plan.clone()],
            result_types: vec![result_type.clone()],
            result_plans: vec![result_plan.clone()],
            effects,
        };
        for ty in signature
            .parameter_types
            .iter()
            .chain(&signature.result_types)
        {
            self.image
                .intern_type(self.unit.module_path.as_str(), ty, "db insert")?;
        }
        Ok(IntrinsicReference {
            target: BytecodeIntrinsicRef::Static {
                canonical_key: "std.db.operation".to_string(),
                signature_version: 1,
            },
            signature,
            db_operation: Some(Box::new(DbOperationReference {
                op: DbOperationKind::Insert,
                target: DbTargetIr {
                    type_ref: target_type.clone(),
                    type_name: operation.target.type_name.clone(),
                },
                operand_roles: vec![DbOperandRole::ObjectFields],
                result_type,
                result_plans: vec![result_plan],
            })),
        })
    }

    fn db_object_publication_type(&self, ty: &TypeRefIr) -> TypeRefIr {
        let TypeRefIr::DbObjectSymbol { symbol } = ty else {
            return super::constants::qualify_local_types(self.unit.module_path.as_str(), ty);
        };
        if symbol.module_path != self.unit.module_path {
            return ty.clone();
        }
        self.unit
            .type_table
            .iter()
            .enumerate()
            .find(|(_, declaration)| declaration.name == symbol.symbol)
            .map(|(type_index, _)| TypeRefIr::PublicationType {
                module_path: symbol.module_path.clone(),
                type_index: type_index as u32,
            })
            .unwrap_or_else(|| ty.clone())
    }

    fn emit_db_transaction(
        &mut self,
        expression: &MirExpression,
        transaction: &skiff_artifact_model::DbTransactionIr,
    ) -> Result<(), BytecodeEmissionError> {
        let mut body_blocks = self
            .function
            .blocks
            .iter()
            .filter(|block| block.label == transaction.body);
        let mut body_block = body_blocks.next().cloned().ok_or_else(|| {
            unsupported(
                &self.key,
                "DbTransaction",
                &format!("transaction body `{}` has no MIR block", transaction.body),
            )
        })?;
        if body_blocks.next().is_some() {
            return Err(unsupported(
                &self.key,
                "DbTransaction",
                "transaction body maps to multiple MIR blocks",
            ));
        }
        body_block.successors.clear();
        self.emit_value_block_block(&body_block)?;
        self.emit_expression(transaction.result)?;
        self.map_completed_expression_events(expression.index)?;
        Ok(())
    }

    fn emit_value_block_block(
        &mut self,
        block: &skiff_compiler_lowering::mir::MirBlock,
    ) -> Result<(), BytecodeEmissionError> {
        self.emit_stream_continuation_cleanup(block.id)?;
        let instruction_start = self.instructions.len();
        for statement in &block.statements {
            self.map_statement_events(statement.statement_index);
            self.emit_statement(statement)?;
        }

        if self.instructions[instruction_start..]
            .last()
            .is_some_and(|instruction| {
                matches!(
                    contract_for_opcode(instruction.opcode).control,
                    skiff_artifact_model::ControlContract::Return
                        | skiff_artifact_model::ControlContract::TailCall
                        | skiff_artifact_model::ControlContract::Raise
                        | skiff_artifact_model::ControlContract::Rethrow
                )
            })
        {
            return Ok(());
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

        let mut stream_item_states =
            self.stream_item_states_after_block(block.id, instruction_start)?;
        if !block.successors.is_empty() {
            if let [successor] = block.successors.as_slice() {
                let backedge = self
                    .loop_backedges
                    .values()
                    .find(|backedge| {
                        backedge.header_block == *successor
                            && if backedge.stream {
                                self.stream_loop_item_states
                                    .contains_key(&(backedge.header_block, block.id))
                            } else {
                                self.loop_backedges
                                    .get(&block.id)
                                    .is_some_and(|registered| {
                                        registered.header_block == backedge.header_block
                                    })
                            }
                    })
                    .cloned();
                if let Some(backedge) = backedge {
                    let stream_item_state = if backedge.stream {
                        let state = stream_item_states
                            .iter_mut()
                            .find(|(header, _, _)| *header == backedge.header_block)
                            .ok_or_else(|| {
                                unsupported(
                                    &self.key,
                                    "stream loop item lifecycle",
                                    &format!(
                                        "block {} has no propagated state for stream header {}",
                                        block.id, backedge.header_block
                                    ),
                                )
                            })?;
                        if self
                            .function
                            .liveness
                            .blocks
                            .get(&block.id)
                            .is_none_or(|liveness| {
                                liveness.live_out.binary_search(&backedge.item_slot).is_ok()
                            })
                        {
                            return Err(unsupported(
                                &self.key,
                                "stream loop item lifecycle",
                                &format!(
                                    "block {} retains item slot {} across its redefining backedge",
                                    block.id, backedge.item_slot
                                ),
                            ));
                        }
                        let item_state = state.2;
                        state.2 = EmittedSlotState::Empty;
                        Some(item_state)
                    } else {
                        None
                    };
                    self.emit_loop_backedge(block.id, &backedge, stream_item_state)?;
                    self.propagate_stream_item_states(block, &stream_item_states)?;
                    return Ok(());
                }
            }
        }
        {
            match block.successors.as_slice() {
                [] => {}
                [successor] => {
                    self.emit_stream_exit_item_cleanup(*successor, &mut stream_item_states)?;
                    self.emit_jump_to_block(*successor)?;
                }
                _ => {}
            }
        }
        self.propagate_stream_item_states(block, &stream_item_states)?;
        if block.successors.is_empty() {
            let terminal = self.instructions.last().is_some_and(|instruction| {
                matches!(
                    instruction.opcode,
                    Opcode::Return | Opcode::Throw | Opcode::Rethrow | Opcode::Jump
                )
            });
            let needs_return = self.function.stream_result.is_some()
                && self
                    .instructions
                    .last()
                    .is_some_and(|instruction| instruction.opcode == Opcode::EmitStream)
                || (is_void(&self.function.return_type) && !terminal);
            if needs_return {
                self.emit_op(Opcode::Return, Vec::new())?;
            }
        }
        Ok(())
    }

    fn emit_statement(
        &mut self,
        statement: &skiff_compiler_lowering::mir::MirStmt,
    ) -> Result<(), BytecodeEmissionError> {
        match &statement.kind {
            MirStmtKind::InitSlot { slot, value } => {
                self.emit_slot_value(*slot, *value)?;
            }
            MirStmtKind::Assign {
                target: AssignTargetIr::Slot { slot },
                value,
                ..
            } => {
                self.anchor_extra_value_events(value.expression)?;
                self.emit_slot_value(*slot, *value)?;
            }
            MirStmtKind::Assign {
                target,
                place,
                value,
                ..
            } => {
                let target_object = match target {
                    AssignTargetIr::Field { object, .. } | AssignTargetIr::Index { object, .. } => {
                        Some(*object)
                    }
                    AssignTargetIr::Slot { .. } | AssignTargetIr::ActorSelfField { .. } => None,
                };
                self.emit_writable_assign(statement.statement_index, place, *value, target_object)?;
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
                let instruction = self.emit_stream(*value)?;
                let site = self.required_statement_site(statement.statement_index)?;
                self.stream_source_sites.push((instruction, site));
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
                self.emit_stream_next(
                    Some(statement.statement_index),
                    *endpoint_slot,
                    item_type,
                    Some(end_block),
                )?;
            }
            MirStmtKind::TestEffectRegister {
                expect,
                step_expect,
                outcome,
                ..
            } => {
                self.emit_number_constant(0)?;
                self.emit_op(Opcode::Pop, Vec::new())?;
                for expected in expect.iter().chain(step_expect.iter()) {
                    self.map_completed_expression_events(expected.value.expression)?;
                }
                match outcome {
                    skiff_artifact_model::TestEffectOutcomeIr::Respond { value, .. } => {
                        self.map_completed_expression_events(value.expression)?;
                    }
                    skiff_artifact_model::TestEffectOutcomeIr::Throw { value, .. } => {
                        self.map_completed_expression_events(value.expression)?;
                    }
                    skiff_artifact_model::TestEffectOutcomeIr::Stream { values, .. } => {
                        for value in values {
                            self.map_completed_expression_events(value.expression)?;
                        }
                    }
                }
            }
            MirStmtKind::Expr { value } | MirStmtKind::Dispatch { call: value } => {
                let expression = self.function.expression(*value)?;
                self.emit_expression(*value)?;
                if !is_void(&expression.ty) {
                    self.emit_op(Opcode::Pop, Vec::new())?;
                }
            }
            MirStmtKind::Throw {
                value,
                payload_type,
                site,
            } => {
                // Phase 3: the MIR `payload_type` fact is not the exception
                // identity. The instruction operand carries only the thrown
                // value's structural transfer-plan type; the runtime captures
                // the actual concrete catch identity from the popped value.
                // A diverging payload fact fails closed instead of leaking a
                // stale static type into the envelope path.
                let value_type = &self.function.expression(*value)?.ty;
                if value_type != payload_type {
                    return Err(unsupported(
                        &self.key,
                        "throw payload type",
                        "static payload_type diverges from the thrown value's type",
                    ));
                }
                self.emit_expression(*value)?;
                let carrier = self.expression_carrier(value.expression)?;
                let type_ref = self.image.type_index(
                    self.unit.module_path.as_str(),
                    carrier,
                    &format!("statement throw value type in `{}`", self.key),
                )?;
                // The throw instruction itself is the raise site: the
                // statement placement event belongs after the payload
                // expression, on the raising instruction, so the linked
                // statement schedule carries the throw source site.
                let throw_instruction = self.instructions.len();
                self.reanchor_statement_events(statement.statement_index, throw_instruction);
                self.emit_op(Opcode::Throw, vec![type_ref])?;
                self.throw_source_sites
                    .push((throw_instruction, site.clone()));
            }
            MirStmtKind::Rethrow { exception_slot } => {
                self.emit_op(Opcode::Rethrow, vec![*exception_slot])?;
            }
            MirStmtKind::Return { value } => {
                if self.function.stream_result.is_some() {
                    if let Some(value) = value {
                        self.emit_expression(*value)?;
                        if !self
                            .instructions
                            .last()
                            .is_some_and(|instruction| instruction.opcode == Opcode::Trap)
                        {
                            self.emit_op(Opcode::Pop, Vec::new())?;
                        }
                    }
                    self.emit_op(Opcode::Return, Vec::new())?;
                    return Ok(());
                }
                if let Some(value) = value {
                    let expression = self.function.expression(*value)?;
                    if self.try_emit_tail_call(expression)? {
                        return Ok(());
                    }
                    self.emit_expression(*value)?;
                    if self.instructions.last().is_some_and(|instruction| {
                        matches!(
                            instruction.opcode,
                            Opcode::Trap | Opcode::Throw | Opcode::Rethrow
                        )
                    }) {
                        return Ok(());
                    }
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
                let value_type = &self.function.expression(*value)?.ty;
                if value_type != payload_type {
                    return Err(unsupported(
                        &self.key,
                        "throw payload type",
                        "static payload_type diverges from the thrown value's type",
                    ));
                }
                self.emit_expression(*value)?;
                let carrier = self.expression_carrier(value.expression)?;
                let type_ref = self.image.type_index(
                    self.unit.module_path.as_str(),
                    carrier,
                    &format!("expression throw value type in `{}`", self.key),
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
            ExprIr::ValueBlock { result, .. } => {
                self.emit_value_block(expression.index, *result)?;
            }
            ExprIr::DbOperation { operation } => {
                self.emit_db_operation(expression, operation)?;
            }
            ExprIr::DbTransaction { transaction } => {
                self.emit_db_transaction(expression, transaction)?;
            }
            other => {
                return Err(unsupported(
                    &self.key,
                    "MIR expression",
                    &format!("{other:?} is outside the scalar emitter subset"),
                ));
            }
        }
        self.map_completed_expression_events(expression.index)
    }

    fn emit_call_expression(
        &mut self,
        expression: &MirExpression,
    ) -> Result<(), BytecodeEmissionError> {
        if let ExprIr::Call { call } = &expression.expression {
            if is_duration_milliseconds_target(call) {
                self.emit_duration_milliseconds_constructor(expression)?;
                return Ok(());
            }
        }
        if self.try_emit_ordinary_call(expression)? {
            self.anchor_extra_call_expression_events(expression.index)?;
            return Ok(());
        }
        let ExprIr::Call { call } = &expression.expression else {
            return Err(unsupported(
                &self.key,
                "call expression",
                "expression is not a call",
            ));
        };
        if matches!(&call.target, CallTargetIr::Builtin { op } if op == "db.transaction") {
            if call.args.len() != 1 || !call.inout_args.is_empty() || !call.type_args.is_empty() {
                return Err(unsupported(
                    &self.key,
                    "db transaction",
                    "db transaction statement facts must carry exactly one body value",
                ));
            }
            self.emit_expression(call.args[0])?;
            self.map_call_event(expression.index);
            return Ok(());
        }
        if !call.inout_args.is_empty() {
            return Err(BytecodeEmissionError::InOutEmissionPending {
                function_key: self.key.clone(),
                expression: expression.index,
            });
        }
        if !call.type_args.is_empty()
            && !matches!(
                &call.target,
                CallTargetIr::Native { .. }
                    | CallTargetIr::Builtin { .. }
                    | CallTargetIr::ReceiverBuiltin { .. }
            )
        {
            return Err(unsupported(
                &self.key,
                "generic non-direct call",
                "non-direct calls with type arguments are outside the emitted core",
            ));
        }
        if self.function.native
            || self.unit.module_path == "std"
            || self.unit.module_path.starts_with("std.")
        {
            if let CallTargetIr::Native { target } = &call.target {
                if !native_binding_registered(target)
                    || (self.function.native && is_stream_type(&self.function.return_type))
                {
                    self.emit_native_wrapper_trap()?;
                    self.map_all_unmapped_expression_events_to_last();
                    return Ok(());
                }
            }
        }
        let is_array_push = matches!(
            (&call.target, call.args.as_slice()),
            (
                CallTargetIr::Builtin { op },
                [_, _],
            ) if op == "Array.push"
                || op == "core.array.push"
                || op == "receiver:Array.push@1"
        ) || matches!(
            &call.target,
            CallTargetIr::ReceiverBuiltin { op } if op.canonical_key == "receiver:Array.push@1"
        );
        if is_array_push {
            self.emit_array_push(call, expression)?;
            return Ok(());
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
                let boundary_plan = self
                    .service_boundary_plans
                    .get(&service_call)
                    .ok_or_else(|| {
                        unsupported(
                            &self.key,
                            "service call target",
                            &format!(
                                "service call ref {} has no compiler-emitted boundary plan",
                                service_call_ref_index.index()
                            ),
                        )
                    })?
                    .clone();
                self.emit_pending_call(
                    expression,
                    Opcode::CallService,
                    BytecodeRelocation::ServiceOperationRef {
                        service_call: skiff_artifact_model::ServiceCallBoundaryFacts::new(
                            service_call,
                            boundary_plan,
                        ),
                    },
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
                let table = super::admission::resolve_local_interface_table_for_call(
                    self.unit,
                    self.function,
                    call,
                    self.local_interface_tables,
                )
                .map_err(|error| {
                    unsupported(&self.key, "interface call requirement", &error.to_string())
                })?;
                let methods = table
                    .methods
                    .iter()
                    .into_iter()
                    .map(|method| InterfaceRequirementMethod {
                        slot: method.slot,
                        method_abi_id: method.method_abi_id.clone(),
                        signature: qualified_interface_signature(
                            self.unit.module_path.as_str(),
                            &method.signature,
                        ),
                        effects: method.effects.clone(),
                    })
                    .collect::<Vec<_>>();
                let relocation = BytecodeRelocation::InterfaceRequirementRef {
                    interface: qualified_interface_instantiation(
                        self.unit.module_path.as_str(),
                        interface,
                    ),
                    methods,
                };
                self.emit_pending_call(
                    expression,
                    Opcode::CallInterface,
                    relocation,
                    Some(*slot),
                    true,
                )
            }
            CallTargetIr::Native { target }
                if static_intrinsic_canonical_key(
                    target.binding_key.as_deref().unwrap_or_default(),
                )
                .is_some() =>
            {
                let binding_key = target.binding_key.as_deref().unwrap_or_default();
                let canonical_key =
                    static_intrinsic_canonical_key(binding_key).expect("guard checked key");
                let relocation = BytecodeRelocation::IntrinsicRef {
                    intrinsic: self.intrinsic_reference(call, expression, canonical_key)?,
                };
                self.emit_pending_call(expression, Opcode::InvokeIntrinsic, relocation, None, false)
            }
            CallTargetIr::Native { target } => {
                let relocation = BytecodeRelocation::HostEffectRef(
                    self.host_effect_reference(call, expression, target)?,
                );
                self.emit_pending_call(expression, Opcode::InvokeHost, relocation, None, true)
            }
            CallTargetIr::Builtin { op } => {
                if matches!(
                    op.as_str(),
                    "Array.push" | "core.array.push" | "receiver:Array.push@1"
                ) {
                    self.emit_array_push(call, expression)?;
                    return Ok(());
                }
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
        self.anchor_extra_call_expression_events(expression.index)?;
        Ok(())
    }

    fn emit_duration_milliseconds_constructor(
        &mut self,
        expression: &MirExpression,
    ) -> Result<(), BytecodeEmissionError> {
        let ExprIr::Call { call } = &expression.expression else {
            return Err(unsupported(
                &self.key,
                "Duration.milliseconds constructor",
                "expression is not a call",
            ));
        };
        if call.args.len() != 1 {
            return Err(unsupported(
                &self.key,
                "Duration.milliseconds constructor",
                "constructor requires exactly one literal integer argument",
            ));
        }
        let argument = self.function.expression(call.args[0])?;
        let ExprIr::Literal {
            value: LiteralIr::Number { value },
        } = &argument.expression
        else {
            return Err(unsupported(
                &self.key,
                "Duration.milliseconds constructor",
                "constructor argument is not a literal number",
            ));
        };
        let pool = self.image.add_literal_constant(
            self.unit.module_path.as_str(),
            &LiteralIr::Number {
                value: value.clone(),
            },
            &expression.ty,
            &format!(
                "function `{key}` Duration.milliseconds expression {index}",
                key = self.key,
                index = expression.index
            ),
        )?;
        self.emit_op(Opcode::Const, vec![pool])?;
        self.map_call_event(call.args[0].expression);
        // The constructor is an identity alias. A second typed constant plus
        // Pop gives the constructor source event its own exact program point
        // while the operand stack remains one Duration value.
        self.emit_op(Opcode::Const, vec![pool])?;
        self.emit_op(Opcode::Pop, Vec::new())?;
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
        let result_materialization = self.host_result_materialization(expression, &relocation)?;
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
                    Some(self.expression_carrier(expression.index)?.clone())
                },
                result_expression: (result_count != 0).then_some(expression.index),
                result_materialization,
                emit_stream_item_shape_ref: None,
                end_block: None,
            });
        }
        self.map_call_event(expression.index);
        Ok(())
    }

    fn host_result_materialization(
        &mut self,
        expression: &MirExpression,
        relocation: &BytecodeRelocation,
    ) -> Result<Option<ResumeResultMaterialization>, BytecodeEmissionError> {
        let BytecodeRelocation::HostEffectRef(effect) = relocation else {
            return Ok(None);
        };
        if effect.target.binding_key.as_deref() != Some("std.http.client.request") {
            return Ok(None);
        }
        if is_void(&expression.ty) {
            return Err(unsupported(
                &self.key,
                "std.http.request result materialization",
                "the canonical HTTP client request result must not be void",
            ));
        }
        let result_carrier = self.expression_carrier(expression.index)?.clone();
        let fields = self.machine_expression_shape_fields(
            expression.index,
            &result_carrier,
            "std.http.request result materialization",
        )?;
        let shape_ref = self.image.intern_shape(
            self.unit.module_path.as_str(),
            &result_carrier,
            &fields,
            &format!("std.http.request result materialization in `{}`", self.key),
        )?;
        Ok(Some(ResumeResultMaterialization::DenseRecord { shape_ref }))
    }

    fn intern_signature_types(
        &mut self,
        signature: &HostEffectSignature,
        context: &str,
    ) -> Result<(), BytecodeEmissionError> {
        for ty in signature
            .parameter_types
            .iter()
            .chain(signature.result_types.iter())
        {
            self.intern_type_tree(ty, context)?;
        }
        Ok(())
    }

    fn normalize_signature_type_tree(&self, ty: &TypeRefIr) -> TypeRefIr {
        match ty {
            TypeRefIr::Builtin { name, args } => TypeRefIr::Builtin {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| self.normalize_signature_type_tree(arg))
                    .collect(),
            },
            TypeRefIr::Nullable { inner } => TypeRefIr::Nullable {
                inner: Box::new(self.normalize_signature_type_tree(inner)),
            },
            TypeRefIr::Union { items } => TypeRefIr::Union {
                items: items
                    .iter()
                    .map(|item| self.normalize_signature_type_tree(item))
                    .collect(),
            },
            TypeRefIr::AppliedNominal { base, arguments } => TypeRefIr::AppliedNominal {
                base: base.clone(),
                arguments: arguments
                    .iter()
                    .map(|argument| self.normalize_signature_type_tree(argument))
                    .collect(),
            },
            TypeRefIr::Record { fields } => TypeRefIr::Record {
                fields: fields
                    .iter()
                    .map(|(name, field)| (name.clone(), self.normalize_signature_type_tree(field)))
                    .collect(),
            },
            TypeRefIr::Function {
                params,
                return_type,
            } => TypeRefIr::Function {
                params: params
                    .iter()
                    .map(|param| skiff_artifact_model::FunctionTypeParamIr {
                        name: param.name.clone(),
                        ty: self.normalize_signature_type_tree(&param.ty),
                    })
                    .collect(),
                return_type: Box::new(self.normalize_signature_type_tree(return_type)),
            },
            _ => self.normalize_host_signature_type(ty),
        }
    }

    fn intern_type_tree(
        &mut self,
        ty: &TypeRefIr,
        context: &str,
    ) -> Result<(), BytecodeEmissionError> {
        let ty = self.normalize_signature_type_tree(ty);
        self.image
            .intern_type(self.unit.module_path.as_str(), &ty, context)?;
        match &ty {
            TypeRefIr::Builtin { args, .. }
            | TypeRefIr::AppliedNominal {
                arguments: args, ..
            } => {
                for arg in args {
                    self.intern_type_tree(arg, context)?;
                }
            }
            TypeRefIr::Nullable { inner } => self.intern_type_tree(inner, context)?,
            TypeRefIr::Union { items } => {
                for item in items {
                    self.intern_type_tree(item, context)?;
                }
            }
            TypeRefIr::Record { fields } => {
                for field in fields.values() {
                    self.intern_type_tree(field, context)?;
                }
            }
            TypeRefIr::Function {
                params,
                return_type,
            } => {
                for param in params {
                    self.intern_type_tree(&param.ty, context)?;
                }
                self.intern_type_tree(return_type, context)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn host_effect_reference(
        &mut self,
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
        let signature = self.call_signature(call, expression, effects)?;
        self.intern_signature_types(&signature, &format!("host effect `{binding_key}`"))?;
        Ok(HostEffectReference {
            target: target.clone(),
            signature,
            db_operation: None,
        })
    }

    fn intrinsic_reference(
        &mut self,
        call: &skiff_artifact_model::CallIr,
        expression: &MirExpression,
        canonical_key: &str,
    ) -> Result<IntrinsicReference, BytecodeEmissionError> {
        let canonical_key = static_intrinsic_canonical_key(canonical_key).unwrap_or(canonical_key);
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
        let signature = self.call_signature(call, expression, effects)?;
        self.intern_signature_types(&signature, &format!("intrinsic `{canonical_key}`"))?;
        Ok(IntrinsicReference {
            target,
            signature,
            db_operation: None,
        })
    }

    fn receiver_intrinsic_reference(
        &mut self,
        call: &skiff_artifact_model::CallIr,
        expression: &MirExpression,
        op: &skiff_artifact_model::BuiltinReceiverOp,
    ) -> Result<IntrinsicReference, BytecodeEmissionError> {
        let target = BytecodeIntrinsicRef::Receiver { op: *op };
        let effects = skiff_artifact_model::intrinsic_registry()
            .entries()
            .iter()
            .find(|entry| entry.target == target)
            .map(|entry| entry.signature.effects.clone())
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "receiver intrinsic target",
                    &format!(
                        "receiver key `{}` is absent from the intrinsic registry",
                        op.canonical_key
                    ),
                )
            })?;
        let signature = self.call_signature(call, expression, effects)?;
        self.intern_signature_types(
            &signature,
            &format!("receiver intrinsic `{}`", op.canonical_key),
        )?;
        Ok(IntrinsicReference {
            target,
            signature,
            db_operation: None,
        })
    }

    fn normalize_host_signature_type(&self, ty: &TypeRefIr) -> TypeRefIr {
        let symbol_path = match ty {
            TypeRefIr::PublicationType {
                module_path,
                type_index,
            } if module_path == &self.unit.module_path => self
                .unit
                .type_table
                .get(*type_index as usize)
                .map(|declaration| format!("{module_path}.{}", declaration.name)),
            TypeRefIr::LocalType { type_index } => self
                .unit
                .type_table
                .get(*type_index as usize)
                .map(|declaration| format!("{}.{}", self.unit.module_path, declaration.name)),
            _ => None,
        };
        match symbol_path {
            Some(symbol_path) => {
                let abi_expectation = self
                    .unit
                    .external_refs
                    .package_symbols
                    .iter()
                    .find(|symbol| symbol.symbol_path == symbol_path)
                    .and_then(|symbol| symbol.abi_expectation.clone())
                    .or_else(|| {
                        self.unit
                            .external_refs
                            .package_symbols
                            .iter()
                            .find(|symbol| {
                                matches!(
                                    &symbol.package,
                                    skiff_artifact_model::PackageRefIr::PackageId { package_id }
                                        if package_id == "skiff.run/std"
                                )
                            })
                            .and_then(|symbol| symbol.abi_expectation.clone())
                    });
                TypeRefIr::PackageSymbol {
                    symbol: skiff_artifact_model::PackageSymbolRef {
                        package: skiff_artifact_model::PackageRefIr::PackageId {
                            package_id: "skiff.run/std".to_string(),
                        },
                        symbol_path,
                        abi_expectation,
                    },
                }
            }
            None => ty.clone(),
        }
    }

    fn call_signature(
        &mut self,
        call: &skiff_artifact_model::CallIr,
        expression: &MirExpression,
        effects: skiff_artifact_model::CallableMayEffects,
    ) -> Result<HostEffectSignature, BytecodeEmissionError> {
        let parameter_types = call
            .args
            .iter()
            .map(|argument| {
                let ty = self.expression_carrier(argument.expression)?;
                Ok(super::constants::qualify_local_types(
                    self.unit.module_path.as_str(),
                    &self.normalize_host_signature_type(ty),
                ))
            })
            .collect::<Result<Vec<_>, BytecodeEmissionError>>()?;
        let mut parameter_plans = Vec::with_capacity(call.args.len());
        for argument in &call.args {
            let ty = self.expression_carrier(argument.expression)?.clone();
            let shape = self
                .machine_carriers
                .expression_shape(argument.expression)
                .cloned();
            let source_plan = self.image.exact_type_plan(
                self.unit.module_path.as_str(),
                &ty,
                &format!("host call parameter plan in `{}`", self.key),
            )?;
            parameter_plans.push(self.bind_privileged_plan(&ty, &source_plan, shape.as_ref())?);
        }
        let result_types = if is_void(&expression.ty) {
            Vec::new()
        } else {
            let carrier = self.expression_carrier(expression.index)?;
            vec![super::constants::qualify_local_types(
                self.unit.module_path.as_str(),
                &self.normalize_host_signature_type(carrier),
            )]
        };
        let result_plans = if result_types.is_empty() {
            Vec::new()
        } else {
            let carrier = self.expression_carrier(expression.index)?.clone();
            let shape = self
                .machine_carriers
                .expression_shape(expression.index)
                .cloned();
            let source_plan = self.image.exact_type_plan(
                self.unit.module_path.as_str(),
                &carrier,
                &format!("host call result plan in `{}`", self.key),
            )?;
            vec![self.bind_privileged_plan(&carrier, &source_plan, shape.as_ref())?]
        };
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

    fn emit_slot_value(
        &mut self,
        slot: u32,
        value: ExprRefIr,
    ) -> Result<(), BytecodeEmissionError> {
        let expression = self.function.expression(value)?;
        if is_never_type(&expression.ty) {
            self.emit_expression(value)?;
            return Ok(());
        }
        if is_stream_type(&expression.ty) {
            if let ExprIr::LoadSlot { slot: source } = &expression.expression {
                self.begin_expression(expression.index);
                self.emit_op(Opcode::MoveSlot, vec![*source, slot])?;
                self.map_completed_expression_events(expression.index)?;
                return Ok(());
            }
        }
        self.emit_expression(value)?;
        self.emit_op(Opcode::StoreSlot, vec![slot])?;
        Ok(())
    }

    fn emit_stream(&mut self, value: ExprRefIr) -> Result<usize, BytecodeEmissionError> {
        let stream_item_type = self
            .function
            .stream_result
            .as_ref()
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "EmitStream",
                    "function has no exact Stream<T> result facts",
                )
            })?
            .item_type
            .clone();
        let value_expression = self.function.expression(value)?;
        let (construct_type, construct_fields) = match &value_expression.expression {
            ExprIr::Construct { type_ref, fields } => (type_ref.clone(), fields.clone()),
            _ => {
                return Err(unsupported(
                    &self.key,
                    "EmitStream item shape",
                    "stream item is not an exact record construction",
                ));
            }
        };
        let admitted_nominal_branch = matches!(
            self.source_attribution,
            SourceAttributionMode::AdmittedPhase1
        ) && construct_type == stream_item_type;
        if construct_type != stream_item_type
            || (!stream_item_type_matches(&value_expression.ty, &stream_item_type)
                && !admitted_nominal_branch)
        {
            return Err(unsupported(
                &self.key,
                "EmitStream",
                &format!(
                    "emitted value type `{:?}` does not match stream item type `{:?}`",
                    value_expression.ty, stream_item_type
                ),
            ));
        }
        let carriers = self.record_construct_carrier_fields(
            value_expression.index,
            &construct_type,
            &construct_fields,
            "EmitStream item shape",
        )?;
        let emit_stream_item_shape_ref = self.image.intern_shape(
            self.unit.module_path.as_str(),
            &construct_type,
            &carriers,
            &format!("EmitStream item shape in `{}`", self.key),
        )?;
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
            result_expression: None,
            result_materialization: None,
            emit_stream_item_shape_ref: Some(emit_stream_item_shape_ref),
            end_block: None,
        });
        Ok(instruction)
    }

    fn required_statement_site(
        &self,
        statement_index: u32,
    ) -> Result<InstructionSourceSite, BytecodeEmissionError> {
        let sites = self
            .events
            .iter()
            .filter_map(|event| {
                matches!(
                    event.anchor,
                    MirEmissionAnchor::Statement {
                        statement_index: anchored,
                        ..
                    } | MirEmissionAnchor::GeneratedStatement {
                        statement_index: anchored,
                        ..
                    } if anchored == statement_index
                )
                .then_some(&event.site)
            })
            .collect::<Vec<_>>();
        let [site] = sites.as_slice() else {
            return Err(unsupported(
                &self.key,
                "EmitStream source attribution",
                "statement lacks one exact source or synthetic site",
            ));
        };
        Ok((*site).clone())
    }

    fn emit_stream_next(
        &mut self,
        statement_index: Option<u32>,
        endpoint_slot: u32,
        _item_type: &TypeRefIr,
        end_block: Option<u32>,
    ) -> Result<(), BytecodeEmissionError> {
        let endpoint_type = self.emitted_slot_carrier(endpoint_slot)?;
        let TypeRefIr::Builtin { name, args } = endpoint_type else {
            return Err(unsupported(
                &self.key,
                "StreamNext",
                "endpoint slot is not Stream<T>",
            ));
        };
        let item_type = match statement_index {
            Some(statement_index) => self
                .machine_carriers
                .stream_next_item(statement_index)
                .map(|carrier| carrier.ty())
                .ok_or_else(|| {
                    unsupported(
                        &self.key,
                        "exact StreamNext carrier",
                        &format!("statement {statement_index} item carrier is absent"),
                    )
                })?
                .clone(),
            None => args[0].clone(),
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
            result_ty: Some(item_type),
            result_expression: None,
            result_materialization: None,
            emit_stream_item_shape_ref: None,
            end_block,
        });
        Ok(())
    }

    fn emit_array_push(
        &mut self,
        call: &skiff_artifact_model::CallIr,
        expression: &MirExpression,
    ) -> Result<(), BytecodeEmissionError> {
        if call.args.len() != 2 {
            return Err(unsupported(
                &self.key,
                "receiver builtin",
                &format!(
                    "Array.push requires a receiver and one value, found {}",
                    call.args.len()
                ),
            ));
        }
        let receiver = self.function.expression(call.args[0])?;
        let ExprIr::LoadSlot { slot } = receiver.expression else {
            return Err(unsupported(
                &self.key,
                "receiver builtin",
                "Array.push receiver is not a writable local slot",
            ));
        };
        self.emit_expression(call.args[0])?;
        self.emit_expression(call.args[1])?;
        self.emit_op(Opcode::ArrayPushOwned, vec![slot])?;
        self.map_call_event(expression.index);
        Ok(())
    }

    fn emit_receiver_builtin(
        &mut self,
        call: &skiff_artifact_model::CallIr,
        expression: &MirExpression,
        op: &skiff_artifact_model::BuiltinReceiverOp,
    ) -> Result<(), BytecodeEmissionError> {
        if op.canonical_key == "receiver:Array.push@1" {
            self.emit_array_push(call, expression)?;
            return Ok(());
        }
        match (op.receiver, op.method) {
            (
                skiff_artifact_model::BuiltinReceiverRoot::Array,
                skiff_artifact_model::BuiltinReceiverMethod::Length,
            ) => {
                if call.args.len() != 1 {
                    return Err(unsupported(
                        &self.key,
                        "receiver builtin",
                        &format!(
                            "`{}` requires exactly one receiver argument, found {}",
                            op.canonical_key,
                            call.args.len()
                        ),
                    ));
                }
                self.emit_op(Opcode::ArrayLen, Vec::new())?;
                self.map_call_event(expression.index);
                return Ok(());
            }
            (
                skiff_artifact_model::BuiltinReceiverRoot::Map,
                skiff_artifact_model::BuiltinReceiverMethod::Length,
            ) => {
                if call.args.len() != 1 {
                    return Err(unsupported(
                        &self.key,
                        "receiver builtin",
                        &format!(
                            "`{}` requires exactly one receiver argument, found {}",
                            op.canonical_key,
                            call.args.len()
                        ),
                    ));
                }
                self.emit_op(Opcode::MapLen, Vec::new())?;
                self.map_call_event(expression.index);
                return Ok(());
            }
            (
                skiff_artifact_model::BuiltinReceiverRoot::Array,
                skiff_artifact_model::BuiltinReceiverMethod::Push,
            ) => {
                self.emit_array_push(call, expression)?;
                return Ok(());
            }
            (
                skiff_artifact_model::BuiltinReceiverRoot::StringText,
                skiff_artifact_model::BuiltinReceiverMethod::Length,
            ) => {
                if call.args.len() != 1 {
                    return Err(unsupported(
                        &self.key,
                        "receiver builtin",
                        &format!(
                            "`{}` requires exactly one receiver argument, found {}",
                            op.canonical_key,
                            call.args.len()
                        ),
                    ));
                }
                let effects = CallableMayEffects {
                    escapes_caller_value: false,
                    requires_same_heap_identity: false,
                    invokes_unknown_target: false,
                    may_pending: false,
                    pending_effect_categories: Vec::new(),
                    inout_path_effects: Vec::new(),
                };
                // Exact local interface string results carry their compiler
                // signature as a static intrinsic row. VM dispatch still
                // fails closed until the kernel lane lands string length.
                let signature = self.call_signature(call, expression, effects)?;
                let relocation = BytecodeRelocation::IntrinsicRef {
                    intrinsic: IntrinsicReference {
                        target: BytecodeIntrinsicRef::Static {
                            canonical_key: "std.string.length".to_string(),
                            signature_version: 1,
                        },
                        signature,
                        db_operation: None,
                    },
                };
                self.emit_pending_call(
                    expression,
                    Opcode::InvokeIntrinsic,
                    relocation,
                    None,
                    false,
                )?;
                return Ok(());
            }
            _ => {}
        }
        let relocation = BytecodeRelocation::IntrinsicRef {
            intrinsic: self.receiver_intrinsic_reference(call, expression, op)?,
        };
        self.emit_pending_call(expression, Opcode::InvokeIntrinsic, relocation, None, false)
    }

    fn try_emit_tail_call(
        &mut self,
        expression: &MirExpression,
    ) -> Result<bool, BytecodeEmissionError> {
        let ExprIr::Call { call } = &expression.expression else {
            return Ok(false);
        };
        if expression.direct_call.is_none() {
            return Ok(false);
        }
        if call.metadata.contains_key(TASK_SUBMIT_METADATA_KEY) {
            return Ok(false);
        }
        if self.has_extra_expression_events(expression.index) {
            return Ok(false);
        }
        self.emit_direct_call(expression, true)
    }

    fn try_emit_ordinary_call(
        &mut self,
        expression: &MirExpression,
    ) -> Result<bool, BytecodeEmissionError> {
        let ExprIr::Call { call } = &expression.expression else {
            return Ok(false);
        };
        if expression.direct_call.is_none() {
            return Ok(false);
        }
        if call.metadata.contains_key(TASK_SUBMIT_METADATA_KEY) {
            self.emit_task_submit_call(expression, call)?;
            return Ok(true);
        }
        self.emit_direct_call(expression, false)
    }

    fn emit_task_submit_call(
        &mut self,
        expression: &MirExpression,
        call: &skiff_artifact_model::CallIr,
    ) -> Result<(), BytecodeEmissionError> {
        if !call.inout_args.is_empty() || !call.type_args.is_empty() {
            return Err(unsupported(
                &self.key,
                "task submit",
                "task dispatch must not carry inout or type arguments",
            ));
        }
        let task = self.task_submit_reference(call)?;
        for argument in &call.args {
            self.emit_expression(*argument)?;
        }
        let mut task_expression = expression.clone();
        task_expression.ty = TypeRefIr::builtin("TaskRef");
        self.image.intern_type(
            self.unit.module_path.as_str(),
            &task_expression.ty,
            &format!("task submit result type in `{}`", self.key),
        )?;
        self.emit_pending_call(
            &task_expression,
            Opcode::InvokeIntrinsic,
            BytecodeRelocation::TaskSubmitRef { task },
            None,
            false,
        )?;
        if !matches!(
            &expression.ty,
            TypeRefIr::Builtin { name, args } if name == "TaskRef" && args.is_empty()
        ) {
            self.emit_op(Opcode::Pop, Vec::new())?;
        }
        Ok(())
    }

    fn task_submit_reference(
        &self,
        call: &skiff_artifact_model::CallIr,
    ) -> Result<TaskSubmitReference, BytecodeEmissionError> {
        let metadata = call.metadata.get(TASK_SUBMIT_METADATA_KEY).ok_or_else(|| {
            unsupported(
                &self.key,
                "task submit",
                "dispatch call has no task metadata",
            )
        })?;
        let MetadataValue::Object(metadata) = metadata else {
            return Err(unsupported(
                &self.key,
                "task submit",
                "dispatchSubmit metadata must be an object",
            ));
        };
        let target_kind = metadata
            .get("targetKind")
            .and_then(|value| match value {
                skiff_artifact_model::MetadataValue::String(value) => Some(value.as_str()),
                _ => None,
            })
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "task submit",
                    "dispatchSubmit metadata lacks targetKind",
                )
            })?;
        let target_identity = metadata
            .get("target")
            .and_then(|value| match value {
                skiff_artifact_model::MetadataValue::String(value) => Some(value.clone()),
                _ => None,
            })
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "task submit",
                    "dispatchSubmit metadata lacks target identity",
                )
            })?;
        let timing = task_submit_timing(metadata, &self.key)?;
        let target = match target_kind {
            "function" => {
                let function_key = match &call.target {
                    CallTargetIr::LocalExecutable { executable_index } => {
                        let target = self.unit.function_by_executable_index(*executable_index)?;
                        super::inputs::canonical_function_key(
                            &self.unit.module_path,
                            &target.symbol,
                        )?
                    }
                    CallTargetIr::PublicationExecutable {
                        module_path,
                        executable_index,
                    } => {
                        let target_unit =
                            self.inputs.units.get(module_path.as_str()).ok_or_else(|| {
                                unsupported(
                                    &self.key,
                                    "task submit",
                                    &format!("task target module `{module_path}` is absent"),
                                )
                            })?;
                        let target = target_unit.function_by_executable_index(*executable_index)?;
                        super::inputs::canonical_function_key(module_path, &target.symbol)?
                    }
                    _ => {
                        return Err(unsupported(
                            &self.key,
                            "task submit",
                            "function task target must be an executable function",
                        ));
                    }
                };
                TaskSubmitTargetRef::Function { function_key }
            }
            "actorMethod" => {
                let CallTargetIr::ActorMethod {
                    actor,
                    actor_abi_identity,
                    actor_implementation_identity,
                    method_identity,
                } = &call.target
                else {
                    return Err(unsupported(
                        &self.key,
                        "task submit",
                        "actor task target must be an actor method",
                    ));
                };
                TaskSubmitTargetRef::ActorMethod {
                    actor: actor.clone(),
                    actor_abi_identity: actor_abi_identity.clone(),
                    actor_implementation_identity: actor_implementation_identity.clone(),
                    method_identity: method_identity.clone(),
                }
            }
            other => {
                return Err(unsupported(
                    &self.key,
                    "task submit",
                    &format!("dispatch target kind {other} is unsupported"),
                ));
            }
        };
        Ok(TaskSubmitReference {
            target,
            target_identity,
            timing,
        })
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
        let has_inout = facts
            .arguments
            .iter()
            .any(|argument| matches!(argument, MirCallArgument::InOut { .. }));
        if tail && has_inout {
            return Ok(false);
        }
        let values = facts
            .arguments
            .iter()
            .filter_map(|argument| match argument {
                MirCallArgument::Value { value } => Some(*value),
                MirCallArgument::InOut { .. } => None,
            })
            .collect::<Vec<_>>();
        let loans = facts
            .arguments
            .iter()
            .filter_map(|argument| match argument {
                MirCallArgument::InOut { loan } => Some(loan.clone()),
                MirCallArgument::Value { .. } => None,
            })
            .collect::<Vec<_>>();
        let mut selectors = Vec::new();
        let loan_layout = if loans.is_empty() {
            None
        } else {
            Some(self.emit_inout_loan_layout(&loans, &mut selectors)?)
        };
        for value in &values {
            self.emit_expression(*value)?;
        }
        for selector in &selectors {
            self.emit_expression(*selector)?;
        }
        let relocation = self.direct_relocation(expression, facts)?;
        let relocation_index = u32::try_from(self.relocations.len())
            .map_err(|_| arithmetic(self.key.as_str(), "relocation index conversion"))?;
        self.relocations.push(relocation);
        let input_count = u32::try_from(values.len() + selectors.len())
            .map_err(|_| arithmetic(self.key.as_str(), "inout input count conversion"))?;
        let mut operands = vec![relocation_index, input_count];
        let opcode = if let Some(loan_layout) = loan_layout {
            operands.push(u32::from(!is_void(&expression.ty)));
            operands.push(loan_layout);
            Opcode::CallLocalInOut
        } else if tail {
            Opcode::TailCallLocal
        } else {
            operands.push(u32::from(!is_void(&expression.ty)));
            Opcode::CallLocal
        };
        self.emit_op(opcode, operands)?;
        self.map_call_event(expression.index);
        Ok(true)
    }

    fn emit_inout_loan_layout(
        &mut self,
        _loans: &[skiff_compiler_lowering::mir::MirInOutLoan],
        _selectors: &mut Vec<ExprRefIr>,
    ) -> Result<u32, BytecodeEmissionError> {
        Err(unsupported(
            &self.key,
            "inout call",
            "inout loans have no admitted Phase 5 machine-carrier boundary",
        ))
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
        if facts.concrete_receiver.is_some() {
            return Err(unsupported(
                &self.key,
                "receiver-bound call",
                "receiver-bound local calls are outside the emitted core",
            ));
        }
        let specialization = BytecodeSpecialization {
            type_arguments: call.type_args.values().cloned().collect(),
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
                package_callable_id: canonical_exact_package_callable_id(package_callable_id),
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
        statement_index: u32,
        place: &MirWritablePlace,
        value: ExprRefIr,
        target_object: Option<ExprRefIr>,
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
        let fact = self
            .machine_carriers
            .writable_path(statement_index)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact writable carrier facts",
                    &format!("statement {statement_index} has no analyzed path"),
                )
            })?
            .clone();
        let root_ty = self.emitted_slot_carrier(slot)?;
        if fact.root().ty() != &root_ty {
            return Err(unsupported(
                &self.key,
                "exact writable carrier facts",
                &format!(
                    "statement {statement_index} root {:?} differs from slot {slot} carrier {root_ty:?}",
                    fact.root().ty()
                ),
            ));
        }
        let leaf_ty = self.expression_carrier(value.expression)?.clone();
        if fact.leaf().ty() != &leaf_ty {
            return Err(unsupported(
                &self.key,
                "exact writable carrier facts",
                &format!(
                    "statement {statement_index} leaf {:?} differs from expression {} carrier {leaf_ty:?}",
                    fact.leaf().ty(),
                    value.expression
                ),
            ));
        }
        if fact.steps().len() != place.path.len() {
            return Err(unsupported(
                &self.key,
                "exact writable carrier facts",
                &format!(
                    "statement {statement_index} analyzed path length {} differs from MIR length {}",
                    fact.steps().len(),
                    place.path.len()
                ),
            ));
        }
        let mut current_ty = root_ty.clone();
        let mut selector_expressions = Vec::new();
        let mut segments = Vec::new();
        let mut next_selector_ordinal = 0u32;
        for (ordinal, (segment, step)) in place.path.iter().zip(fact.steps()).enumerate() {
            match (segment, step) {
                (
                    MirWritablePathSegment::Field { name },
                    MachineWritableStepFact::DenseField {
                        name: exact_name,
                        shape,
                    },
                ) if name == exact_name && shape.owner() == &current_ty => {
                    let fields = machine_shape_fields(shape);
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
                (
                    MirWritablePathSegment::Index { index, access, .. },
                    MachineWritableStepFact::ArrayIndex {
                        selector_expression,
                        selector,
                        element,
                    },
                ) if access.receiver_kind == MirIndexReceiverKind::Array
                    && *selector_expression == index.expression =>
                {
                    let selector_ordinal = next_selector_ordinal;
                    next_selector_ordinal = next_selector_ordinal.saturating_add(1);
                    selector_expressions.push(*index);
                    let selector_ty = self.expression_carrier(index.expression)?;
                    if selector.ty() != selector_ty
                        || selector.ty() != &TypeRefIr::builtin("number")
                    {
                        return Err(unsupported(
                            &self.key,
                            "exact writable carrier facts",
                            &format!(
                                "statement {statement_index} Array selector {ordinal} has drifted carrier"
                            ),
                        ));
                    }
                    let TypeRefIr::Builtin { name, args } = &current_ty else {
                        return Err(unsupported(
                            &self.key,
                            "exact writable carrier facts",
                            &format!(
                                "statement {statement_index} Array segment {ordinal} root is not an exact Array carrier"
                            ),
                        ));
                    };
                    if name != "Array" || args.len() != 1 || &args[0] != element.ty() {
                        return Err(unsupported(
                            &self.key,
                            "exact writable carrier facts",
                            &format!(
                                "statement {statement_index} Array segment {ordinal} element carrier differs from its container"
                            ),
                        ));
                    }
                    let element_type_ref = self.image.type_index(
                        self.unit.module_path.as_str(),
                        element.ty(),
                        &format!("writable array path in `{}`", self.key),
                    )?;
                    segments.push(WritablePathSegment::ArrayIndex {
                        selector_ordinal,
                        element_type_ref,
                    });
                    current_ty = element.ty().clone();
                }
                (
                    MirWritablePathSegment::Index { index, access, .. },
                    MachineWritableStepFact::MapKey {
                        selector_expression,
                        selector,
                        key,
                        value,
                    },
                ) if access.receiver_kind == MirIndexReceiverKind::Map
                    && *selector_expression == index.expression =>
                {
                    let selector_ordinal = next_selector_ordinal;
                    next_selector_ordinal = next_selector_ordinal.saturating_add(1);
                    selector_expressions.push(*index);
                    if selector.ty() != self.expression_carrier(index.expression)?
                        || selector.ty() != key.ty()
                    {
                        return Err(unsupported(
                            &self.key,
                            "exact writable carrier facts",
                            &format!(
                                "statement {statement_index} Map selector {ordinal} has drifted carrier"
                            ),
                        ));
                    }
                    let TypeRefIr::Builtin { name, args } = &current_ty else {
                        return Err(unsupported(
                            &self.key,
                            "exact writable carrier facts",
                            &format!(
                                "statement {statement_index} Map segment {ordinal} root is not an exact Map carrier"
                            ),
                        ));
                    };
                    if name != "Map"
                        || args.len() != 2
                        || &args[0] != key.ty()
                        || &args[1] != value.ty()
                    {
                        return Err(unsupported(
                            &self.key,
                            "exact writable carrier facts",
                            &format!(
                                "statement {statement_index} Map segment {ordinal} key/value carriers differ from its container"
                            ),
                        ));
                    }
                    let key_type_ref = self.image.type_index(
                        self.unit.module_path.as_str(),
                        key.ty(),
                        &format!("writable map key path in `{}`", self.key),
                    )?;
                    let value_type_ref = self.image.type_index(
                        self.unit.module_path.as_str(),
                        value.ty(),
                        &format!("writable map value path in `{}`", self.key),
                    )?;
                    segments.push(WritablePathSegment::MapKey {
                        selector_ordinal,
                        key_type_ref,
                        value_type_ref,
                    });
                    current_ty = value.ty().clone();
                }
                _ => {
                    return Err(unsupported(
                        &self.key,
                        "exact writable carrier facts",
                        &format!(
                            "statement {statement_index} path segment {ordinal} differs from analyzed producer facts"
                        ),
                    ));
                }
            }
        }
        if is_never_type(&self.function.expression(value)?.ty) {
            for selector in &selector_expressions {
                self.emit_expression(*selector)?;
            }
            self.emit_expression(value)?;
            return Ok(());
        }
        if leaf_ty != current_ty {
            return Err(unsupported(
                &self.key,
                "exact writable carrier facts",
                &format!(
                    "statement {statement_index} leaf carrier {leaf_ty:?} differs from analyzed path carrier {current_ty:?}"
                ),
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
        let writable_pc = self.emit_op(
            Opcode::SetWritablePath,
            vec![slot, path_ref, selector_count],
        )?;
        if let Some(object) = target_object {
            // The assign target object expression is never evaluated as a
            // read: the collapsed target-key event covers the writable-path
            // opcode, while the object chain's own events anchor to distinct
            // trailing synthetic instructions.
            self.anchor_writable_target_chain(object, writable_pc)?;
        }
        Ok(())
    }

    fn anchor_extra_value_events(
        &mut self,
        expression_index: u32,
    ) -> Result<(), BytecodeEmissionError> {
        let indices = self
            .events
            .iter()
            .enumerate()
            .filter(|(index, event)| {
                self.event_mapping[*index].is_none()
                    && matches!(
                        event.anchor,
                        MirEmissionAnchor::Expression {
                            expression_index: anchored,
                            occurrence_ordinal,
                        } if anchored == expression_index && occurrence_ordinal > 0
                    )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for index in indices {
            let instruction = self.instructions.len();
            self.emit_number_constant(0)?;
            self.emit_op(Opcode::Pop, Vec::new())?;
            self.event_mapping[index] = Some(instruction);
        }
        Ok(())
    }

    fn anchor_writable_target_chain(
        &mut self,
        object: ExprRefIr,
        writable_pc: usize,
    ) -> Result<(), BytecodeEmissionError> {
        if let Some((index, _)) = self.events.iter().enumerate().find(|(index, event)| {
            self.event_mapping[*index].is_none()
                && matches!(
                    event.anchor,
                    MirEmissionAnchor::Expression {
                        expression_index: anchored,
                        occurrence_ordinal,
                    } if anchored == object.expression && occurrence_ordinal > 0
                )
        }) {
            self.event_mapping[index] = Some(writable_pc);
        }
        let mut indices = Vec::new();
        self.collect_writable_target_chain(object, &mut indices)?;
        for expression_index in indices {
            self.anchor_all_expression_events(expression_index)?;
        }
        Ok(())
    }

    fn collect_writable_target_chain(
        &mut self,
        object: ExprRefIr,
        indices: &mut Vec<u32>,
    ) -> Result<(), BytecodeEmissionError> {
        indices.push(object.expression);
        let expression = self.function.expression(object)?;
        match &expression.expression {
            ExprIr::Field { object, .. } => self.collect_writable_target_chain(*object, indices)?,
            ExprIr::Index { object, .. } => self.collect_writable_target_chain(*object, indices)?,
            _ => {}
        }
        Ok(())
    }

    fn anchor_all_expression_events(
        &mut self,
        expression_index: u32,
    ) -> Result<(), BytecodeEmissionError> {
        loop {
            let Some(index) = self
                .events
                .iter()
                .enumerate()
                .find(|(index, event)| {
                    self.event_mapping[*index].is_none()
                        && matches!(
                            event.anchor,
                            MirEmissionAnchor::Expression {
                                expression_index: anchored,
                                ..
                            } if anchored == expression_index
                        )
                })
                .map(|(index, _)| index)
            else {
                return Ok(());
            };
            let instruction = self.instructions.len();
            self.emit_number_constant(0)?;
            self.emit_op(Opcode::Pop, Vec::new())?;
            self.event_mapping[index] = Some(instruction);
        }
    }

    fn emit_field_read(
        &mut self,
        object: ExprRefIr,
        field: &str,
    ) -> Result<(), BytecodeEmissionError> {
        let object_expression = self.function.expression(object)?;
        let object_index = object_expression.index;
        let object_ty = self.expression_carrier(object_expression.index)?.clone();
        let object_slot = match &object_expression.expression {
            ExprIr::LoadSlot { slot } => Some(*slot),
            _ => None,
        };
        let fields = self.machine_expression_shape_fields(
            object_expression.index,
            &object_ty,
            "field read",
        )?;
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
            &object_ty,
            &fields,
            &format!("field read `{field}` in `{}`", self.key),
        )?;
        let privileged_access = super::constants::privileged_affine_identity(&object_ty)
            .and_then(|identity| {
                skiff_artifact_model::native_value_lifecycle_registry()
                    .privileged_affine_composite(identity)
            })
            .and_then(|schema| schema.fields.get(ordinal))
            .map(|field| field.access);
        if privileged_access == Some(PrivilegedAffineFieldAccess::AffineTake) {
            let Some(slot) = object_slot else {
                return Err(unsupported(
                    &self.key,
                    "privileged affine field take",
                    "the exact aggregate owner is not a source slot",
                ));
            };
            self.begin_expression(object_index);
            self.emit_op(Opcode::TakeSlot, vec![slot])?;
            self.map_completed_expression_events(object_index)?;
            self.emit_op(Opcode::TakeDenseField, vec![shape, ordinal as u32])?;
        } else {
            self.emit_expression(object)?;
            self.emit_op(Opcode::GetDenseField, vec![shape, ordinal as u32])?;
        }
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

    fn emit_default_fact(
        &mut self,
        fact: &MachineDefaultValueFact,
        context: &'static str,
    ) -> Result<(), BytecodeEmissionError> {
        match fact.kind() {
            MachineDefaultValueKind::Literal { value } => {
                let pool = self.image.add_literal_constant(
                    self.unit.module_path.as_str(),
                    value,
                    fact.carrier().ty(),
                    context,
                )?;
                self.emit_op(Opcode::Const, vec![pool])?;
                Ok(())
            }
            MachineDefaultValueKind::EmptyArray { element } => {
                let element_ref =
                    self.image
                        .type_index(self.unit.module_path.as_str(), element.ty(), context)?;
                self.emit_op(Opcode::NewArrayBuilder, vec![element_ref])?;
                self.emit_op(Opcode::FreezeArray, Vec::new())?;
                Ok(())
            }
            MachineDefaultValueKind::Record { shape, fields } => {
                if shape.owner() != fact.carrier().ty() || shape.fields().keys().ne(fields.keys()) {
                    return Err(unsupported(
                        &self.key,
                        "exact catch default facts",
                        "record default shape differs from its exact producer",
                    ));
                }
                let shape_fields = machine_shape_fields(shape);
                for (name, field) in fields {
                    if shape
                        .fields()
                        .get(name)
                        .is_none_or(|carrier| carrier.ty() != field.carrier().ty())
                    {
                        return Err(unsupported(
                            &self.key,
                            "exact catch default facts",
                            &format!("record default field `{name}` has drifted carrier"),
                        ));
                    }
                    self.emit_default_fact(field, context)?;
                }
                let shape_ref = self.image.intern_shape(
                    self.unit.module_path.as_str(),
                    fact.carrier().ty(),
                    &shape_fields,
                    context,
                )?;
                let field_count = u32::try_from(fields.len())
                    .map_err(|_| arithmetic(&self.key, "default record field count conversion"))?;
                self.emit_op(Opcode::NewRecord, vec![shape_ref, field_count])?;
                Ok(())
            }
        }
    }

    fn emit_record_construct(
        &mut self,
        expression: &MirExpression,
        type_ref: &TypeRefIr,
        fields: &BTreeMap<String, ExprRefIr>,
    ) -> Result<(), BytecodeEmissionError> {
        let carriers = self.record_construct_carrier_fields(
            expression.index,
            type_ref,
            fields,
            "record construct",
        )?;
        // The runtime tag comes from the constructed nominal leaf, not the
        // surrounding static context: a union-typed constructor must still
        // carry its concrete leaf identity so throw/catch match the actual
        // branch. Slots/parameters/returns keep the union static type.
        let shape = self.image.intern_shape(
            self.unit.module_path.as_str(),
            type_ref,
            &carriers,
            &format!("record construct in `{}`", self.key),
        )?;
        for name in carriers.keys() {
            self.emit_expression(*fields.get(name).expect("field set was checked"))?;
        }
        let field_count = u32::try_from(carriers.len())
            .map_err(|_| arithmetic(&self.key, "record field count conversion"))?;
        self.emit_op(Opcode::NewRecord, vec![shape, field_count])?;
        Ok(())
    }

    fn record_construct_carrier_fields(
        &self,
        expression: u32,
        type_ref: &TypeRefIr,
        fields: &BTreeMap<String, ExprRefIr>,
        context: &'static str,
    ) -> Result<BTreeMap<String, TypeRefIr>, BytecodeEmissionError> {
        self.record_construct_fields(type_ref, fields, context)?;
        let carriers = self.machine_construct_shape_fields(expression, type_ref, context)?;
        if carriers.len() != fields.len() || carriers.keys().any(|name| !fields.contains_key(name))
        {
            return Err(unsupported(
                &self.key,
                context,
                "analyzed machine shape differs from the checked construct field set",
            ));
        }
        Ok(carriers)
    }

    fn record_construct_fields(
        &self,
        type_ref: &TypeRefIr,
        fields: &BTreeMap<String, ExprRefIr>,
        context: &'static str,
    ) -> Result<BTreeMap<String, TypeRefIr>, BytecodeEmissionError> {
        let declared = match self.record_shape_fields(type_ref, context) {
            Ok(declared) => declared,
            Err(_) if is_package_symbol_type(type_ref) => fields
                .iter()
                .map(|(name, value)| {
                    Ok((name.clone(), self.function.expression(*value)?.ty.clone()))
                })
                .collect::<Result<BTreeMap<_, _>, BytecodeEmissionError>>()?,
            Err(error) => return Err(error),
        };
        if declared.len() != fields.len() || declared.keys().any(|name| !fields.contains_key(name))
        {
            return Err(unsupported(
                &self.key,
                context,
                "construct field set does not exactly match the declared shape",
            ));
        }
        Ok(declared)
    }

    fn emit_array_literal(
        &mut self,
        expression: &MirExpression,
        items: &[ExprRefIr],
    ) -> Result<(), BytecodeEmissionError> {
        let carrier = self.expression_carrier(expression.index)?.clone();
        let element_ty = self.array_element_type(&carrier, "array literal")?;
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
        let carrier = self.expression_carrier(expression.index)?.clone();
        let (key_ty, value_ty) = self.map_key_value_types(&carrier, "map literal")?;
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
        let facts = self
            .local_interface_tables
            .table(&method_table.interface, concrete_type)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "interface box",
                    "exact local interface table is absent",
                )
            })?;
        let methods = facts
            .methods
            .iter()
            .map(|method| LocalInterfaceMethod {
                slot: method.slot,
                method_name: method.method_name.clone(),
                method_abi_id: method.method_abi_id.clone(),
                signature: qualified_interface_signature(
                    self.unit.module_path.as_str(),
                    &method.signature,
                ),
                effects: method.effects.clone(),
                function_key: method.function_key.clone(),
                receiver_call_abi: method.receiver_call_abi,
            })
            .collect::<Vec<_>>();
        let relocation = BytecodeRelocation::LocalInterfaceRef {
            interface: LocalInterfaceRef {
                interface: qualified_interface_instantiation(
                    self.unit.module_path.as_str(),
                    &method_table.interface,
                ),
                concrete_type: qualify_local_types(self.unit.module_path.as_str(), concrete_type),
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
            TypeRefIr::Builtin { name, args } if name == "Exception" && args.len() == 1 => {
                Ok(BTreeMap::from([("error".to_string(), args[0].clone())]))
            }
            TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
                let exception_ty = TypeRefIr::Builtin {
                    name: "Exception".to_string(),
                    args: vec![args[1].clone()],
                };
                Ok(BTreeMap::from([
                    ("exception".to_string(), exception_ty),
                    ("tag".to_string(), TypeRefIr::builtin("string")),
                ]))
            }
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
            TypeRefIr::ServiceSymbol { symbol } | TypeRefIr::DbObjectSymbol { symbol } => {
                let unit = self
                    .inputs
                    .units
                    .get(symbol.module_path.as_str())
                    .ok_or_else(|| {
                        unsupported(
                            &self.key,
                            context,
                            &format!("symbol module `{}` is absent", symbol.module_path),
                        )
                    })?;
                let declaration = unit
                    .type_table
                    .iter()
                    .find(|declaration| declaration.name == symbol.symbol)
                    .ok_or_else(|| {
                        unsupported(
                            &self.key,
                            context,
                            &format!("symbol `{}` is absent", symbol.symbol_path()),
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
                        &format!("symbol `{}` is not a record", symbol.symbol_path()),
                    )),
                }
            }
            TypeRefIr::PackageSymbol { symbol } => self.package_record_fields(symbol, context),
            TypeRefIr::AppliedNominal {
                base: skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol { symbol },
                arguments,
            } if arguments.is_empty() => self.package_record_fields(symbol, context),
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

    fn package_record_fields(
        &self,
        symbol: &skiff_artifact_model::PackageSymbolRef,
        context: &'static str,
    ) -> Result<BTreeMap<String, TypeRefIr>, BytecodeEmissionError> {
        let lookup_key = match &symbol.package {
            skiff_artifact_model::PackageRefIr::Dependency { dependency_ref } => {
                (dependency_ref.clone(), symbol.symbol_path.clone())
            }
            skiff_artifact_model::PackageRefIr::PackageId { package_id } => {
                (package_id.clone(), symbol.symbol_path.clone())
            }
        };
        self.unit
            .package_type_records
            .get(&lookup_key)
            .cloned()
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    context,
                    &format!(
                        "package symbol `{}` has no resolved record shape",
                        symbol.symbol_path
                    ),
                )
            })
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
        let iterable_ty = self.expression_carrier(iterable.expression)?.clone();
        let (array, stream, item_slot, value_slot, item_ty) = match &facts.binding {
            skiff_compiler_lowering::mir::MirForInBinding::Item { slot, kind, .. } => match kind {
                MirForInItemKind::ArrayItem => (
                    true,
                    false,
                    *slot,
                    None,
                    self.array_element_type(&iterable_ty, "for-in Array item")?,
                ),
                MirForInItemKind::MapKey => {
                    let (key, _) = self.map_key_value_types(&iterable_ty, "for-in Map key")?;
                    (false, false, *slot, None, key)
                }
                MirForInItemKind::StreamItem => {
                    let TypeRefIr::Builtin { name, args } = &iterable_ty else {
                        return Err(unsupported(
                            &self.key,
                            "for-in Stream item",
                            "iterable carrier is not Stream<T>",
                        ));
                    };
                    if name != "Stream" || args.len() != 1 {
                        return Err(unsupported(
                            &self.key,
                            "for-in Stream item",
                            "iterable carrier is not exact Stream<T>",
                        ));
                    }
                    (false, true, *slot, None, args[0].clone())
                }
            },
            skiff_compiler_lowering::mir::MirForInBinding::MapEntry {
                key_slot,
                value_slot,
                ..
            } => {
                let (key, _) = self.map_key_value_types(&iterable_ty, "for-in Map entry")?;
                (false, false, *key_slot, Some(*value_slot), key)
            }
        };
        let iterable_slot = self.push_generated_slot(&iterable_ty, "$forIterable")?;
        if stream {
            let index_slot = 0;
            let iterable_expression = self.function.expression(iterable)?;
            if let ExprIr::LoadSlot { slot } = &iterable_expression.expression {
                self.begin_expression(iterable_expression.index);
                self.emit_op(Opcode::MoveSlot, vec![*slot, iterable_slot])?;
                self.map_completed_expression_events(iterable_expression.index)?;
            } else {
                self.emit_expression(iterable)?;
                self.emit_op(Opcode::StoreSlot, vec![iterable_slot])?;
            }
            let next_generated = self
                .events
                .iter()
                .filter_map(|event| match event.attribution_id {
                    StatementAttributionId::Generated { ordinal } => Some(ordinal),
                    _ => None,
                })
                .max()
                .map_or(0, |ordinal| ordinal + 1);
            let event_index = self.events.len();
            self.events.push(MirSourceEvent {
                attribution_id: StatementAttributionId::Generated {
                    ordinal: next_generated,
                },
                site: InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                },
                anchor: MirEmissionAnchor::GeneratedStatement {
                    statement_index: 0,
                    placement: MirStatementPlacement::BeforeStatement,
                },
            });
            self.event_mapping.push(None);
            let header_instruction = self.emit_op(Opcode::BudgetCheckpoint, Vec::new())?;
            self.event_mapping[event_index] = Some(header_instruction);
            let next_event_index = self.events.len();
            self.events.push(MirSourceEvent {
                attribution_id: StatementAttributionId::Generated {
                    ordinal: next_generated
                        .checked_add(1)
                        .ok_or_else(|| arithmetic(&self.key, "stream event ordinal"))?,
                },
                site: InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                },
                anchor: MirEmissionAnchor::GeneratedStatement {
                    statement_index: 0,
                    placement: MirStatementPlacement::BeforeStatement,
                },
            });
            self.event_mapping.push(Some(self.instructions.len()));
            debug_assert_eq!(next_event_index, self.event_mapping.len() - 1);
            self.emit_stream_next(None, iterable_slot, &item_ty, Some(continuation))?;
            self.emit_op(Opcode::StoreSlot, vec![item_slot])?;
            self.emit_jump_to_block(body)?;
            self.stream_loop_item_states
                .insert((self.current_block, body), EmittedSlotState::Live);
            self.loop_backedges.insert(
                body,
                LoopBackedge {
                    header_block: self.current_block,
                    continuation_block: continuation,
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
                header_block: self.current_block,
                continuation_block: continuation,
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

    fn stream_item_states_after_block(
        &self,
        block: u32,
        instruction_start: usize,
    ) -> Result<Vec<(u32, u32, EmittedSlotState)>, BytecodeEmissionError> {
        let mut states = Vec::new();
        for backedge in self
            .loop_backedges
            .values()
            .filter(|backedge| backedge.stream)
        {
            let Some(mut state) = self
                .stream_loop_item_states
                .get(&(backedge.header_block, block))
                .copied()
            else {
                continue;
            };
            for instruction in &self.instructions[instruction_start..] {
                state = apply_emitted_slot_state(
                    state,
                    backedge.item_slot,
                    instruction,
                    &self.key,
                    block,
                )?;
            }
            states.push((backedge.header_block, backedge.item_slot, state));
        }
        Ok(states)
    }

    fn propagate_stream_item_states(
        &mut self,
        block: &skiff_compiler_lowering::mir::MirBlock,
        states: &[(u32, u32, EmittedSlotState)],
    ) -> Result<(), BytecodeEmissionError> {
        for &(header, _slot, state) in states {
            let backedge = self
                .loop_backedges
                .values()
                .find(|backedge| backedge.header_block == header && backedge.stream)
                .ok_or_else(|| {
                    unsupported(
                        &self.key,
                        "stream loop item lifecycle",
                        &format!("stream header {header} lost its exact loop facts"),
                    )
                })?;
            for &successor in &block.successors {
                if successor == header || successor == backedge.continuation_block {
                    continue;
                }
                match self.stream_loop_item_states.entry((header, successor)) {
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        entry.insert(state);
                    }
                    std::collections::btree_map::Entry::Occupied(entry)
                        if *entry.get() == state => {}
                    std::collections::btree_map::Entry::Occupied(_) => {
                        return Err(unsupported(
                            &self.key,
                            "stream loop item lifecycle",
                            &format!(
                                "block {successor} merges live and consumed item states from stream header {header}"
                            ),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn emit_stream_exit_item_cleanup(
        &mut self,
        successor: u32,
        states: &mut [(u32, u32, EmittedSlotState)],
    ) -> Result<(), BytecodeEmissionError> {
        for (header, item_slot, state) in states {
            let Some(backedge) = self
                .loop_backedges
                .values()
                .find(|backedge| backedge.header_block == *header && backedge.stream)
            else {
                return Err(unsupported(
                    &self.key,
                    "stream loop item lifecycle",
                    &format!("stream header {header} lost its exact loop facts"),
                ));
            };
            if backedge.continuation_block != successor {
                continue;
            }
            if *state == EmittedSlotState::Live {
                self.emit_op(Opcode::Drop, vec![*item_slot])?;
            }
            *state = EmittedSlotState::Empty;
        }
        Ok(())
    }

    fn emit_stream_continuation_cleanup(
        &mut self,
        block: u32,
    ) -> Result<(), BytecodeEmissionError> {
        let endpoints = self
            .loop_backedges
            .values()
            .filter(|backedge| backedge.stream && backedge.continuation_block == block)
            .map(|backedge| backedge.iterable_slot)
            .collect::<BTreeSet<_>>();
        for endpoint in endpoints {
            self.emit_op(Opcode::Drop, vec![endpoint])?;
        }
        Ok(())
    }

    fn emit_loop_backedge(
        &mut self,
        _block: u32,
        backedge: &LoopBackedge,
        stream_item_state: Option<EmittedSlotState>,
    ) -> Result<(), BytecodeEmissionError> {
        if backedge.stream {
            match stream_item_state {
                Some(EmittedSlotState::Live) => {
                    self.emit_op(Opcode::Drop, vec![backedge.item_slot])?;
                }
                Some(EmittedSlotState::Empty) => {}
                None => {
                    return Err(unsupported(
                        &self.key,
                        "stream loop item lifecycle",
                        "stream backedge has no exact emitted item state",
                    ));
                }
            }
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
        let value_carrier = self.expression_carrier(value_expression.index)?.clone();
        let temp_slot = self.push_generated_slot(&value_carrier, "$matchValue")?;
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
        let default = self
            .machine_carriers
            .catch_default(catch_expression_index)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact catch default facts",
                    &format!(
                        "catch expression {catch_expression_index} has no analyzed default producer"
                    ),
                )
            })?
            .clone();
        let slot_carrier = self.emitted_slot_carrier(catch_slot)?;
        if default.carrier().ty() != &slot_carrier {
            return Err(unsupported(
                &self.key,
                "exact catch default facts",
                &format!(
                    "catch expression {catch_expression_index} default carrier {:?} differs from slot {catch_slot} carrier {slot_carrier:?}",
                    default.carrier().ty()
                ),
            ));
        }
        self.emit_default_fact(&default, "catch slot default")?;
        self.emit_op(Opcode::StoreSlot, vec![catch_slot])?;
        let try_ty = self.function.expression(try_expression)?.ty.clone();
        let start_instruction = self.instructions.len();
        self.emit_expression(try_expression)?;
        // A throw/rethrow try expression is typed `never`: the raise already
        // consumed its payload and never falls through, so there is no try
        // value to pop.
        if !is_void(&try_ty) && !is_never_type(&try_ty) {
            self.emit_op(Opcode::Pop, Vec::new())?;
        }
        let handler_instruction = self.instructions.len();
        self.pending_exception_regions.push(PendingExceptionRegion {
            start_instruction,
            handler_instruction,
            catch_slot,
            catch_type: catch_type.clone(),
            cleanup_depth: region.cleanup_depth,
        });
        self.emit_expression(body)?;
        let exception_ty = TypeRefIr::Builtin {
            name: "Exception".to_string(),
            args: vec![catch_type.clone()],
        };
        let exception_fact = self
            .machine_carriers
            .catch_exception_shape(catch_expression_index)
            .filter(|shape| shape.owner() == &exception_ty)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact catch exception facts",
                    &format!(
                        "catch expression {catch_expression_index} has no analyzed exception producer"
                    ),
                )
            })?
            .clone();
        let exception_fields = machine_shape_fields(&exception_fact);
        let body_carrier = self.expression_carrier(body.expression)?;
        if exception_fields.get("error") != Some(body_carrier) {
            return Err(unsupported(
                &self.key,
                "exact catch exception facts",
                &format!(
                    "catch expression {catch_expression_index} body carrier differs from exception payload"
                ),
            ));
        }
        let exception_shape = self.image.intern_shape(
            self.unit.module_path.as_str(),
            &exception_ty,
            &exception_fields,
            &format!("Exception construction in `{}`", self.key),
        )?;
        self.emit_op(Opcode::NewRecord, vec![exception_shape, 1])?;
        let result_ty = self.expression_carrier(catch_expression_index)?.clone();
        let result_fact = self
            .machine_carriers
            .expression_shape(catch_expression_index)
            .filter(|shape| shape.owner() == &result_ty)
            .ok_or_else(|| {
                unsupported(
                    &self.key,
                    "exact catch result facts",
                    &format!(
                        "catch expression {catch_expression_index} has no analyzed result producer"
                    ),
                )
            })?
            .clone();
        let fields = machine_shape_fields(&result_fact);
        if fields.get("exception") != Some(&exception_ty)
            || fields.get("tag") != Some(&TypeRefIr::builtin("string"))
        {
            return Err(unsupported(
                &self.key,
                "exact catch result facts",
                &format!(
                    "catch expression {catch_expression_index} result fields differ from exact generated producers"
                ),
            ));
        }
        let tag_pool = self.image.add_literal_constant(
            self.unit.module_path.as_str(),
            &skiff_artifact_model::LiteralIr::String {
                value: "err".to_string(),
            },
            &TypeRefIr::builtin("string"),
            &format!("CatchResult tag in `{}`", self.key),
        )?;
        self.emit_op(Opcode::Const, vec![tag_pool])?;
        let expected_result_ty = TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![try_ty, catch_type.clone()],
        };
        if result_ty != expected_result_ty {
            return Err(unsupported(
                &self.key,
                "exact catch result facts",
                &format!(
                    "catch expression {catch_expression_index} carrier {result_ty:?} differs from generated owner {expected_result_ty:?}"
                ),
            ));
        }
        let shape = self.image.intern_shape(
            self.unit.module_path.as_str(),
            &result_ty,
            &fields,
            &format!("CatchResult construction in `{}`", self.key),
        )?;
        self.emit_op(Opcode::NewRecord, vec![shape, 2])?;
        Ok(())
    }

    fn generated_slot_plan(
        &self,
        ty: &TypeRefIr,
    ) -> Result<skiff_artifact_model::ValueTransferPlan, BytecodeEmissionError> {
        self.image.exact_type_plan(
            self.unit.module_path.as_str(),
            ty,
            &format!("generated slot plan in `{}`", self.key),
        )
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

    fn map_all_unmapped_expression_events_to_last(&mut self) {
        let Some(instruction) = self.instructions.len().checked_sub(1) else {
            return;
        };
        for (index, event) in self.events.iter().enumerate() {
            if matches!(
                event.anchor,
                MirEmissionAnchor::Expression { .. }
                    | MirEmissionAnchor::LocalCall { .. }
                    | MirEmissionAnchor::TailLocalCallCandidate { .. }
            ) && self.event_mapping[index].is_none()
            {
                self.event_mapping[index] = Some(instruction);
            }
        }
    }

    fn emit_native_wrapper_trap(&mut self) -> Result<(), BytecodeEmissionError> {
        let pool = self.image.add_literal_constant(
            self.unit.module_path.as_str(),
            &LiteralIr::Bool { value: false },
            &TypeRefIr::builtin("bool"),
            &format!("native wrapper trap in `{}`", self.key),
        )?;
        self.emit_op(Opcode::Const, vec![pool])?;
        self.emit_op(Opcode::Trap, vec![TrapFailureKind::Assertion as u32])?;
        Ok(())
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
        *self
            .expression_emissions
            .entry(expression_index)
            .or_insert(0) += 1;
    }

    fn map_completed_expression_events(
        &mut self,
        expression_index: u32,
    ) -> Result<(), BytecodeEmissionError> {
        let instruction = self.instructions.len().checked_sub(1).ok_or_else(|| {
            unsupported(
                &self.key,
                "expression source event",
                "expression produced no emitted instruction",
            )
        })?;
        for (index, event) in self.events.iter().enumerate() {
            if let MirEmissionAnchor::Expression {
                expression_index: anchored,
                ..
            } = event.anchor
            {
                if anchored == expression_index {
                    self.event_mapping[index].get_or_insert(instruction);
                }
            }
        }
        Ok(())
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
                }
                | MirEmissionAnchor::Expression {
                    expression_index: anchored,
                    occurrence_ordinal: 0,
                } if anchored == expression_index
            );
            if matches {
                self.event_mapping[index].get_or_insert(self.instructions.len() - 1);
            }
        }
    }

    fn has_extra_expression_events(&self, expression_index: u32) -> bool {
        self.events.iter().any(|event| {
            matches!(
                event.anchor,
                MirEmissionAnchor::Expression {
                    expression_index: anchored,
                    occurrence_ordinal,
                } if anchored == expression_index && occurrence_ordinal > 0
            )
        })
    }

    fn anchor_extra_call_expression_events(
        &mut self,
        expression_index: u32,
    ) -> Result<(), BytecodeEmissionError> {
        let has_extra = self.has_extra_expression_events(expression_index);
        if !has_extra {
            return Ok(());
        }
        let instruction = self.instructions.len();
        self.emit_number_constant(0)?;
        self.emit_op(Opcode::Pop, Vec::new())?;
        for (index, event) in self.events.iter().enumerate() {
            if let MirEmissionAnchor::Expression {
                expression_index: anchored,
                ..
            } = event.anchor
            {
                if anchored == expression_index {
                    self.event_mapping[index].get_or_insert(instruction);
                }
            }
        }
        Ok(())
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

    /// Moves one statement's placement events onto a later emitted
    /// instruction. Statement forms of `throw` evaluate their payload before
    /// the raising instruction, so the raise-site event must leave the
    /// payload's first instruction and land on the `Throw` instruction.
    fn reanchor_statement_events(&mut self, statement_index: u32, instruction: usize) {
        for (index, event) in self.events.iter().enumerate() {
            if matches!(
                event.anchor,
                MirEmissionAnchor::Statement {
                    statement_index: anchored,
                    ..
                } if anchored == statement_index
            ) {
                self.event_mapping[index] = Some(instruction);
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
                    &format!(
                        "instruction {opcode:?} underflows the emitted stack at depth {} after {:?}",
                        self.operand_depth,
                        self.instructions.iter().map(|i| i.opcode).collect::<Vec<_>>()
                    ),
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
        let pending_resumes = self.pending_resumes.clone();
        for pending in &pending_resumes {
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
            let mut result_plans = Vec::with_capacity(usize::from(pending.result_ty.is_some()));
            if let Some(ty) = &pending.result_ty {
                let shape = pending
                    .result_expression
                    .and_then(|expression| self.machine_carriers.expression_shape(expression))
                    .cloned();
                let source_plan = self.image.exact_type_plan(
                    self.unit.module_path.as_str(),
                    ty,
                    &format!("resume result plan in `{}`", self.key),
                )?;
                result_plans.push(self.bind_privileged_plan(ty, &source_plan, shape.as_ref())?);
            }
            let result_materializations = match &pending.result_ty {
                Some(_) => vec![pending.result_materialization],
                None => {
                    if pending.result_materialization.is_some() {
                        return Err(unsupported(
                            &self.key,
                            "resume result materialization",
                            "a zero-result resume cannot carry a materialization fact",
                        ));
                    }
                    Vec::new()
                }
            };
            let emit_stream_item_shape_ref =
                match (descriptor.kind, pending.emit_stream_item_shape_ref) {
                    (Opcode::EmitStream, Some(shape_ref)) => Some(shape_ref),
                    (Opcode::EmitStream, None) => {
                        return Err(unsupported(
                            &self.key,
                            "EmitStream item shape",
                            "EmitStream lacks its exact constructed item shape",
                        ));
                    }
                    (_, None) => None,
                    (_, Some(_)) => {
                        return Err(unsupported(
                            &self.key,
                            "EmitStream item shape",
                            "a non-EmitStream resume carries an item shape",
                        ));
                    }
                };
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
                Some(
                    *pcs.get(start)
                        .ok_or_else(|| arithmetic(&self.key, "resume end pc lookup"))?,
                )
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
                result_materializations,
                emit_stream_item_shape_ref,
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
                    &format!(
                        "instruction {:?} underflows the emitted stack",
                        instruction.opcode
                    ),
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
        pcs: &[u32],
        word_count: usize,
    ) -> Result<Vec<SourceMapEntry>, BytecodeEmissionError> {
        if word_count == 0 {
            return Ok(Vec::new());
        }
        let word_count = u32::try_from(word_count)
            .map_err(|_| arithmetic(&self.key, "source map word count"))?;
        if matches!(
            self.source_attribution,
            SourceAttributionMode::PrivateBackend
        ) {
            return Ok(vec![SourceMapEntry {
                start_pc: 0,
                end_pc: word_count,
                site: InstructionSourceSite::Synthetic {
                    reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                },
            }]);
        }

        let mut covered_instructions = BTreeSet::new();
        let mut rows = Vec::new();
        for (event_index, event) in self.events.iter().enumerate() {
            if !matches!(
                event.anchor,
                MirEmissionAnchor::Expression { .. }
                    | MirEmissionAnchor::LocalCall { .. }
                    | MirEmissionAnchor::TailLocalCallCandidate { .. }
                    | MirEmissionAnchor::BudgetCheckpoint { .. }
                    | MirEmissionAnchor::GeneratedStatement { .. }
            ) {
                continue;
            }
            let instruction_index = self.event_mapping[event_index].ok_or_else(|| {
                unsupported(
                    &self.key,
                    "Phase 1 source attribution",
                    &format!("source event {event_index} was not anchored to emitted code"),
                )
            })?;
            if self.instructions.get(instruction_index).is_none() {
                return Err(arithmetic(
                    &self.key,
                    "Phase 1 source event instruction lookup",
                ));
            }
            if !covered_instructions.insert(instruction_index) {
                // Collapsed source keys (a rethrow identifier lowered
                // directly into its host node) share the host expression's
                // instruction. The first-recorded event wins the source map
                // entry; the collapsed key still contributes its statement
                // charge through the statement schedule.
                let collapsed_duplicate = matches!(
                    event.anchor,
                    MirEmissionAnchor::Expression {
                        occurrence_ordinal,
                        ..
                    } if occurrence_ordinal > 0
                );
                if !collapsed_duplicate {
                    return Err(unsupported(
                        &self.key,
                        "Phase 1 source attribution",
                        &format!(
                            "source event {event_index} does not uniquely anchor an instruction"
                        ),
                    ));
                }
                continue;
            }
            let start_pc = *pcs
                .get(instruction_index)
                .ok_or_else(|| arithmetic(&self.key, "Phase 1 source event pc lookup"))?;
            let end_pc = pcs
                .get(instruction_index + 1)
                .copied()
                .unwrap_or(word_count);
            rows.push(SourceMapEntry {
                start_pc,
                end_pc,
                site: event.site.clone(),
            });
        }
        for (instruction_index, site) in self
            .throw_source_sites
            .iter()
            .chain(&self.stream_source_sites)
            .chain(&self.generated_source_sites)
        {
            let start_pc = *pcs
                .get(*instruction_index)
                .ok_or_else(|| arithmetic(&self.key, "throw source site instruction pc lookup"))?;
            let end_pc = pcs
                .get(instruction_index + 1)
                .copied()
                .unwrap_or(word_count);
            rows.push(SourceMapEntry {
                start_pc,
                end_pc,
                site: site.clone(),
            });
        }
        rows.sort_by_key(|entry| entry.start_pc);
        Ok(rows)
    }

    fn build_frame(&mut self) -> Result<FrameLayout, BytecodeEmissionError> {
        let slot_count = self.function.slots.len() + self.generated_slots.len();
        let mut slot_type_refs = Vec::with_capacity(slot_count);
        for slot in &self.function.slots {
            let ty = self.slot_carrier(slot.slot)?.clone();
            slot_type_refs.push(self.image.type_index(
                self.unit.module_path.as_str(),
                &ty,
                &format!(
                    "function `{key}` slot `{name}` type",
                    key = self.key,
                    name = slot.name
                ),
            )?);
        }
        let source_slot_plans = self
            .function
            .slots
            .iter()
            .zip(&self.plans.slot_plans)
            .map(|(slot, plan)| {
                Ok((
                    self.slot_carrier(slot.slot)?.clone(),
                    plan.clone(),
                    self.machine_carriers.slot_shape(slot.slot).cloned(),
                ))
            })
            .collect::<Result<Vec<_>, BytecodeEmissionError>>()?;
        let mut slot_plans = Vec::with_capacity(slot_count);
        for (ty, plan, shape) in source_slot_plans {
            slot_plans.push(self.bind_privileged_plan(&ty, &plan, shape.as_ref())?);
        }
        let generated = self
            .generated_slots
            .iter()
            .map(|slot| {
                Ok((
                    slot.name.clone(),
                    slot.ty.clone().ok_or_else(|| {
                        unsupported(
                            &self.key,
                            "generated frame slot type",
                            &format!("slot `{}` has no exact type", slot.name),
                        )
                    })?,
                ))
            })
            .collect::<Result<Vec<_>, BytecodeEmissionError>>()?;
        for (name, ty) in generated {
            slot_type_refs.push(self.image.type_index(
                self.unit.module_path.as_str(),
                &ty,
                &format!(
                    "function `{key}` generated slot `{name}` type",
                    key = self.key,
                ),
            )?);
            let plan = self.generated_slot_plan(&ty)?;
            slot_plans.push(self.bind_privileged_plan(&ty, &plan, None)?);
        }
        let mut parameter_slots = Vec::new();
        for parameter in &self.function.params {
            let slot = parameter.slot as usize;
            let plan = slot_plans.get(slot).cloned().ok_or_else(|| {
                unsupported(
                    &self.key,
                    "parameter transfer plan",
                    &format!("parameter `{name}` has no slot plan", name = parameter.name),
                )
            })?;
            let dense_record_shape_ref = self
                .inputs
                .dense_parameter_materializations
                .get(&self.key)
                .filter(|fact| fact.slot == parameter.slot)
                .map(|fact| {
                    if fact.ty != parameter.ty {
                        return Err(unsupported(
                            &self.key,
                            "dense parameter materialization",
                            &format!(
                                "parameter `{}` type differs from admitted gateway fact",
                                parameter.name
                            ),
                        ));
                    }
                    let carrier = self.slot_carrier(parameter.slot)?.clone();
                    if carrier != fact.ty {
                        return Err(unsupported(
                            &self.key,
                            "dense parameter materialization",
                            &format!(
                                "parameter `{}` machine carrier differs from its nominal gateway type",
                                parameter.name
                            ),
                        ));
                    }
                    let fields = self.machine_slot_shape_fields(
                        parameter.slot,
                        &carrier,
                        "dense parameter materialization",
                    )?;
                    if fields != fact.fields {
                        return Err(unsupported(
                            &self.key,
                            "dense parameter materialization",
                            &format!(
                                "parameter `{}` machine field layout differs from admitted gateway fact",
                                parameter.name
                            ),
                        ));
                    }
                    self.image.intern_shape(
                        self.unit.module_path.as_str(),
                        &carrier,
                        &fields,
                        &format!(
                            "rawHttp gateway parameter `{}` in `{}`",
                            parameter.name, self.key
                        ),
                    )
                })
                .transpose()?;
            parameter_slots.push(ParameterSlotDecl {
                slot: parameter.slot,
                mode: match parameter.mode {
                    MirParamMode::Value => ParamModeIr::Value,
                    MirParamMode::InOut => ParamModeIr::InOut,
                },
                plan,
                dense_record_shape_ref,
            });
        }
        if let Some(receiver) = &self.function.receiver {
            if !parameter_slots
                .iter()
                .any(|parameter| parameter.slot == receiver.slot)
            {
                let slot = receiver.slot as usize;
                let plan = slot_plans.get(slot).cloned().ok_or_else(|| {
                    unsupported(
                        &self.key,
                        "receiver transfer plan",
                        &format!("receiver slot {slot} has no slot plan"),
                    )
                })?;
                parameter_slots.push(ParameterSlotDecl {
                    slot: receiver.slot,
                    mode: ParamModeIr::Value,
                    plan,
                    dense_record_shape_ref: None,
                });
            }
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
            let result_ty = self.result_carrier()?.clone();
            vec![self.image.type_index(
                self.unit.module_path.as_str(),
                &result_ty,
                &format!("function `{key}` return type", key = self.key),
            )?]
        };
        let stream_result_type_ref = if is_stream_producer {
            let stream_ty = self
                .machine_carriers
                .stream_result()
                .map(|carrier| carrier.ty().clone())
                .ok_or_else(|| {
                    unsupported(
                        &self.key,
                        "exact stream result carrier",
                        "stream producer frame row is absent",
                    )
                })?;
            Some(self.image.type_index(
                self.unit.module_path.as_str(),
                &stream_ty,
                &format!("function `{key}` stream authority type", key = self.key),
            )?)
        } else {
            None
        };
        let result_plans = if is_stream_producer {
            Vec::new()
        } else {
            let source_plans = self.plans.result_plans.clone();
            let return_type = if result_count == 0 {
                self.function.return_type.clone()
            } else {
                self.result_carrier()?.clone()
            };
            let result_shape = self.machine_carriers.result_shape().cloned();
            let mut result_plans = Vec::with_capacity(source_plans.len());
            for plan in source_plans {
                result_plans.push(self.bind_privileged_plan(
                    &return_type,
                    &plan,
                    result_shape.as_ref(),
                )?);
            }
            result_plans
        };
        let writable_local_slots = self
            .function
            .slots
            .iter()
            .filter(|slot| slot.writable_local && slot.kind == MirSlotKind::Local)
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

    fn bind_privileged_plan(
        &mut self,
        ty: &TypeRefIr,
        source_plan: &ValueTransferPlan,
        shape: Option<&MachineShapeCarrierFact>,
    ) -> Result<ValueTransferPlan, BytecodeEmissionError> {
        let Some(identity) = super::constants::privileged_affine_identity(ty) else {
            return Ok(source_plan.clone());
        };
        let ValueTransferPlan::FromType { ty: planned_ty } = source_plan else {
            return Err(unsupported(
                &self.key,
                "privileged affine slot plan",
                "source authority must defer the pool-local recursive shape binding",
            ));
        };
        if !same_privileged_package_symbol(ty, planned_ty)
            || super::constants::privileged_affine_identity(planned_ty) != Some(identity)
        {
            return Err(unsupported(
                &self.key,
                "privileged affine slot plan",
                "FromType authority differs from the exact privileged slot type",
            ));
        }
        let shape = shape.filter(|shape| shape.owner() == ty).ok_or_else(|| {
            unsupported(
                &self.key,
                "privileged affine occurrence plan",
                "the exact value occurrence has no compiler-owned shape fact",
            )
        })?;
        let fields = machine_shape_fields(shape);
        let shape_ref = self.image.intern_shape(
            self.unit.module_path.as_str(),
            ty,
            &fields,
            &format!("privileged affine slot plan in `{}`", self.key),
        )?;
        Ok(ValueTransferPlan::MoveOnly {
            drop: ValueDropPlan::RecursiveShape { shape_ref },
        })
    }
}

fn same_privileged_package_symbol(left: &TypeRefIr, right: &TypeRefIr) -> bool {
    fn symbol(ty: &TypeRefIr) -> Option<&skiff_artifact_model::PackageSymbolRef> {
        match ty {
            TypeRefIr::PackageSymbol { symbol } => Some(symbol),
            TypeRefIr::AppliedNominal {
                base: skiff_artifact_model::NominalTypeRefBaseIr::PackageSymbol { symbol },
                arguments,
            } if arguments.is_empty() => Some(symbol),
            _ => None,
        }
    }
    matches!((symbol(left), symbol(right)), (Some(left), Some(right)) if left == right)
}

fn apply_emitted_slot_state(
    mut state: EmittedSlotState,
    slot: u32,
    instruction: &RawInstruction,
    function_key: &str,
    block: u32,
) -> Result<EmittedSlotState, BytecodeEmissionError> {
    let require_live = |state| {
        if state == EmittedSlotState::Live {
            Ok(())
        } else {
            Err(unsupported(
                function_key,
                "stream loop item lifecycle",
                &format!(
                    "block {block} applies {:?} to consumed item slot {slot}",
                    instruction.opcode
                ),
            ))
        }
    };
    match instruction.opcode {
        Opcode::Drop | Opcode::TakeSlot if instruction.operands.first() == Some(&slot) => {
            require_live(state)?;
            state = EmittedSlotState::Empty;
        }
        Opcode::MoveSlot => {
            if instruction.operands.first() == Some(&slot) {
                require_live(state)?;
                state = EmittedSlotState::Empty;
            }
            if instruction.operands.get(1) == Some(&slot) {
                if state == EmittedSlotState::Live {
                    return Err(unsupported(
                        function_key,
                        "stream loop item lifecycle",
                        &format!("block {block} overwrites live item slot {slot}"),
                    ));
                }
                state = EmittedSlotState::Live;
            }
        }
        Opcode::CopySlot if instruction.operands.get(1) == Some(&slot) => {
            if state == EmittedSlotState::Live {
                return Err(unsupported(
                    function_key,
                    "stream loop item lifecycle",
                    &format!("block {block} overwrites live item slot {slot}"),
                ));
            }
            state = EmittedSlotState::Live;
        }
        Opcode::StoreSlot if instruction.operands.first() == Some(&slot) => {
            if state == EmittedSlotState::Live {
                return Err(unsupported(
                    function_key,
                    "stream loop item lifecycle",
                    &format!("block {block} overwrites live item slot {slot}"),
                ));
            }
            state = EmittedSlotState::Live;
        }
        _ => {}
    }
    Ok(state)
}

fn stack_effect(
    instruction: &RawInstruction,
    function: &MirFunction,
) -> Result<(usize, usize), BytecodeEmissionError> {
    Ok(match instruction.opcode {
        Opcode::Const | Opcode::LoadSlot | Opcode::TakeSlot => (0, 1),
        Opcode::StoreSlot | Opcode::Pop => (1, 0),
        Opcode::Drop => (0, 0),
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
        Opcode::CallLocalInOut => {
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
        Opcode::GetDenseField | Opcode::TakeDenseField | Opcode::RepresentationWrap => (1, 1),
        Opcode::SetWritablePath => (instruction.operands[2] as usize + 1, 0),
        Opcode::NewArrayBuilder | Opcode::NewMapBuilder => (0, 1),
        Opcode::ArrayBuilderPush => (2, 1),
        Opcode::MapBuilderPut => (3, 1),
        Opcode::FreezeArray | Opcode::FreezeMap => (1, 1),
        Opcode::ArrayGet | Opcode::MapGet => (2, 1),
        Opcode::ArrayLen | Opcode::MapLen => (1, 1),
        Opcode::MapEntryAt => (2, 2),
        Opcode::ArrayPushOwned => (1, 0),
        Opcode::MapPutOwned => (2, 0),
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

fn canonical_exact_package_callable_id(
    callable_id: &skiff_artifact_model::PackageCallableId,
) -> skiff_artifact_model::PackageCallableId {
    let value = callable_id.as_str();
    let Some(rest) = value.strip_prefix("pkg-callable:") else {
        return callable_id.clone();
    };
    let Some((package_id, public_path)) = rest.split_once(':') else {
        return callable_id.clone();
    };
    if public_path.starts_with("top-level:") {
        return callable_id.clone();
    }
    skiff_artifact_model::PackageCallableId::new(format!(
        "pkg-callable:{package_id}:top-level:{public_path}"
    ))
}

fn native_binding_registered(target: &NativeTarget) -> bool {
    let Some(binding_key) = target.binding_key.as_deref() else {
        return false;
    };
    skiff_artifact_model::host_effect_registry()
        .entries()
        .iter()
        .any(|entry| entry.binding_key == binding_key)
}

fn is_duration_milliseconds_target(call: &skiff_artifact_model::CallIr) -> bool {
    matches!(
        &call.target,
        CallTargetIr::Native { target }
            if target.binding_key.as_deref() == Some("core.duration.milliseconds")
    )
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

fn machine_shape_fields(shape: &MachineShapeCarrierFact) -> BTreeMap<String, TypeRefIr> {
    shape
        .fields()
        .iter()
        .map(|(name, carrier)| (name.clone(), carrier.ty().clone()))
        .collect()
}

fn qualified_interface_instantiation(
    module_path: &str,
    interface: &InterfaceInstantiationRef,
) -> InterfaceInstantiationRef {
    InterfaceInstantiationRef {
        interface_abi_id: interface.interface_abi_id.clone(),
        canonical_type_args: interface
            .canonical_type_args
            .iter()
            .map(|ty| qualify_local_types(module_path, ty))
            .collect(),
    }
}

fn qualified_interface_signature(
    module_path: &str,
    signature: &InterfaceMethodSlotSignatureIr,
) -> InterfaceMethodSlotSignatureIr {
    InterfaceMethodSlotSignatureIr {
        params: signature
            .params
            .iter()
            .map(|parameter| FunctionTypeParamIr {
                name: parameter.name.clone(),
                ty: qualify_local_types(module_path, &parameter.ty),
            })
            .collect(),
        return_type: qualify_local_types(module_path, &signature.return_type),
    }
}

fn stream_item_type_matches(actual: &TypeRefIr, expected: &TypeRefIr) -> bool {
    actual == expected
        || package_symbol_type_matches(actual, expected)
        || matches!(
            (actual, expected),
            (
                TypeRefIr::Literal {
                    value: skiff_artifact_model::LiteralIr::String { .. },
                },
                TypeRefIr::Builtin { name, args },
            ) if name == "string" && args.is_empty()
        )
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

fn package_symbol_type_matches(actual: &TypeRefIr, expected: &TypeRefIr) -> bool {
    let (
        TypeRefIr::PackageSymbol { symbol: actual },
        TypeRefIr::PackageSymbol { symbol: expected },
    ) = (actual, expected)
    else {
        return false;
    };
    if actual == expected {
        return true;
    }
    if actual.symbol_path != expected.symbol_path
        || actual.abi_expectation != expected.abi_expectation
    {
        return false;
    }
    matches!(
        (&actual.package, &expected.package),
        (
            skiff_artifact_model::PackageRefIr::Dependency { dependency_ref },
            skiff_artifact_model::PackageRefIr::PackageId { package_id },
        ) if dependency_ref == "std" && package_id == "skiff.run/std"
    ) || matches!(
        (&actual.package, &expected.package),
        (
            skiff_artifact_model::PackageRefIr::PackageId { package_id },
            skiff_artifact_model::PackageRefIr::Dependency { dependency_ref },
        ) if dependency_ref == "std" && package_id == "skiff.run/std"
    )
}

fn is_stream_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if name == "Stream" && args.len() == 1
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
    if target.self_type.is_some() {
        return Err(unsupported(
            caller,
            "receiver-bound target",
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

fn value_block_body_blocks(function: &MirFunction) -> Result<BTreeSet<u32>, BytecodeEmissionError> {
    let mut bodies = BTreeSet::new();
    for fact in function.expression_blocks.values() {
        bodies.extend(value_block_body_ids(function, fact)?);
    }
    for expression in &function.expressions {
        let ExprIr::DbTransaction { transaction } = &expression.expression else {
            continue;
        };
        for block in &function.blocks {
            if block.label == transaction.body {
                bodies.insert(block.id);
            }
        }
    }
    Ok(bodies)
}

fn value_block_body_ids(
    function: &MirFunction,
    fact: &skiff_compiler_lowering::mir::MirExpressionBlockFact,
) -> Result<BTreeSet<u32>, BytecodeEmissionError> {
    let body_block = fact.body_block;
    let body_ordinal = usize::try_from(body_block)
        .map_err(|_| arithmetic("scalar emitter", "ValueBlock body block id conversion"))?;
    function.blocks.get(body_ordinal).ok_or_else(|| {
        unsupported(
            "scalar emitter",
            "ValueBlock CFG",
            &format!("body block {body_block} is absent"),
        )
    })?;
    let mut pending = vec![body_block];
    let mut seen = BTreeSet::new();
    while let Some(block_id) = pending.pop() {
        let ordinal = usize::try_from(block_id)
            .map_err(|_| arithmetic("scalar emitter", "ValueBlock block id conversion"))?;
        let block = function.blocks.get(ordinal).ok_or_else(|| {
            unsupported(
                "scalar emitter",
                "ValueBlock CFG",
                &format!("body block {block_id} is absent"),
            )
        })?;
        if block.id != block_id {
            return Err(unsupported(
                "scalar emitter",
                "ValueBlock CFG",
                &format!("body block {block_id} is non-canonical"),
            ));
        }
        if !seen.insert(block_id) {
            continue;
        }
        pending.extend(
            block
                .successors
                .iter()
                .copied()
                .filter(|successor| *successor != fact.continuation_block),
        );
    }
    for completion in &fact.completion_targets {
        if !seen.contains(completion) {
            return Err(unsupported(
                "scalar emitter",
                "ValueBlock CFG",
                &format!("completion block {completion} is outside the ValueBlock body"),
            ));
        }
    }
    Ok(seen)
}

fn task_submit_timing(
    metadata: &BTreeMap<String, MetadataValue>,
    function_key: &str,
) -> Result<TaskSubmitTimingRef, BytecodeEmissionError> {
    let Some(timing) = metadata.get("timing") else {
        return Ok(TaskSubmitTimingRef::Immediate);
    };
    let MetadataValue::Object(timing) = timing else {
        return Err(unsupported(
            function_key,
            "task submit",
            "dispatchSubmit timing must be an object",
        ));
    };
    let kind = timing
        .get("kind")
        .and_then(|value| match value {
            MetadataValue::String(value) => Some(value.as_str()),
            _ => None,
        })
        .ok_or_else(|| {
            unsupported(
                function_key,
                "task submit",
                "dispatchSubmit timing kind is missing",
            )
        })?;
    match kind {
        "immediate" => Ok(TaskSubmitTimingRef::Immediate),
        "after" | "at" => {
            let expression = timing
                .get("expr")
                .and_then(|value| match value {
                    MetadataValue::Number(value) => value.as_u64(),
                    _ => None,
                })
                .ok_or_else(|| {
                    unsupported(
                        function_key,
                        "task submit",
                        "dispatchSubmit timing expression is missing",
                    )
                })?;
            let expression = u32::try_from(expression).map_err(|_| {
                unsupported(
                    function_key,
                    "task submit",
                    "dispatchSubmit timing expression does not fit u32",
                )
            })?;
            if kind == "after" {
                Ok(TaskSubmitTimingRef::After { expression })
            } else {
                Ok(TaskSubmitTimingRef::At { expression })
            }
        }
        other => Err(unsupported(
            function_key,
            "task submit",
            &format!("dispatch timing kind {other} is unsupported"),
        )),
    }
}

fn return_count(function: &MirFunction) -> usize {
    usize::from(!is_void(&function.return_type) && function.stream_result.is_none())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        BinaryOpIr, CallableEffectSummary, ExprIr, ExprRefIr, FileIrUnit, InstructionSourceSite,
        LiteralIr, PackageCallableId, PackageRefIr, PackageSymbolRef, PendingEffectCategory,
        SyntheticInstructionSiteReason, TypeRefIr, ValueDropPlan, ValueTransferPlan,
    };
    use skiff_compiler_lowering::{
        mir::{
            liveness::compute_liveness, MirBlock, MirExecutableKind, MirExpression, MirFunction,
            MirLiveness, MirSlot, MirSlotKind, MirSourceEventPlan, MirSourceEventUnavailableReason,
            MirStatementEntry, MirStmt, MirStmtKind, MirUnit,
        },
        Bounds, ConstEvaluator,
    };

    use super::*;
    use crate::bytecode::constants::build_constant_image;
    use crate::bytecode::plans::{
        derive_bytecode_value_transfer_plans_unchecked, derive_test_bytecode_value_transfer_plans,
    };

    #[test]
    fn stream_backedge_state_distinguishes_shared_and_consumed_items() {
        let shared = apply_emitted_slot_state(
            EmittedSlotState::Live,
            7,
            &RawInstruction {
                opcode: Opcode::LoadSlot,
                operands: vec![7],
            },
            "main::shared",
            2,
        )
        .expect("a shared item remains live for backedge cleanup");
        assert_eq!(shared, EmittedSlotState::Live);

        let consumed = apply_emitted_slot_state(
            EmittedSlotState::Live,
            7,
            &RawInstruction {
                opcode: Opcode::MoveSlot,
                operands: vec![7, 8],
            },
            "main::consumed",
            2,
        )
        .expect("an exact MoveSlot consumes the iteration item");
        assert_eq!(consumed, EmittedSlotState::Empty);
    }

    /// The statement form of `throw` emits its payload first; the raise
    /// instruction must still carry exactly one source/synthetic site in the
    /// function source map, taken from the retained MIR `site`.
    #[test]
    fn statement_throw_places_its_site_on_the_raise_instruction() {
        let throw_site = InstructionSourceSite::Synthetic {
            reason: SyntheticInstructionSiteReason::CompilerDesugaring,
        };
        let mut function = MirFunction {
            executable_index: 0,
            origin: skiff_artifact_model::PackageExecutableCoordinate {
                file_ir_identity: "file:main".to_string(),
                module_path: "main".to_string(),
                executable_index: 0,
            },
            symbol: "main.boom".to_string(),
            kind: MirExecutableKind::Function,
            native: false,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: None,
            receiver: None,
            slots: Vec::new(),
            index_accesses: BTreeMap::new(),
            expression_blocks: BTreeMap::new(),
            expressions: vec![MirExpression {
                index: 0,
                expression: ExprIr::Literal {
                    value: LiteralIr::Number {
                        value: serde_json::Number::from(1_u64),
                    },
                },
                ty: TypeRefIr::builtin("number"),
                writable: None,
                direct_call: None,
                stream_result: None,
                remote_interface: None,
            }],
            blocks: vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![MirStmt {
                    statement_index: 0,
                    span: None,
                    kind: MirStmtKind::Throw {
                        value: ExprRefIr { expression: 0 },
                        payload_type: TypeRefIr::builtin("number"),
                        site: throw_site.clone(),
                    },
                }],
                successors: Vec::new(),
            }],
            regions: Vec::new(),
            statements: vec![MirStatementEntry {
                statement_index: 0,
                span: None,
            }],
            stream_result: None,
            liveness: MirLiveness::default(),
            effect_summary_ref: PackageCallableId::new("callable:main:boom".to_string()),
            effect_summary: CallableEffectSummary::analysis_pending(),
            source_span: None,
            source_event_plan: MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ),
        };
        function.liveness = compute_liveness(&function).expect("test liveness computes");

        let mut file_ir = FileIrUnit::empty("main", "source-hash");
        file_ir.file_ir_identity = "file:main".to_string();
        let bundle = ConstEvaluator::new(Bounds::default())
            .evaluate_unit(&file_ir)
            .expect("test bundle evaluates");
        let unit = MirUnit {
            file_ir_identity: file_ir.file_ir_identity.clone(),
            module_path: file_ir.module_path.clone(),
            actor_declarations: file_ir.actor_declarations.clone(),
            external_refs: file_ir.external_refs.clone(),
            source_map: file_ir.source_map.clone(),
            type_table: file_ir.type_table.clone(),
            package_type_records: BTreeMap::new(),
            link_targets: file_ir.link_targets.clone(),
            constants: Vec::new(),
            functions: vec![function],
        };
        let units = [unit];
        let plans = derive_test_bytecode_value_transfer_plans(&units)
            .expect("the source classifier covers the test MIR");
        let bundles = [bundle];
        let dense_parameter_materializations = BTreeMap::new();
        let machine_carriers = super::super::carriers::analyze_machine_carriers(&units)
            .expect("test machine carriers analyze");
        let service_boundary_plans = BTreeMap::new();
        let inputs = ValidatedEmissionInputs::validate(
            &units,
            &bundles,
            &plans,
            &dense_parameter_materializations,
            &machine_carriers,
            &[],
            &service_boundary_plans,
        )
        .expect("test inputs validate");
        let mut image = build_constant_image(&inputs).expect("test image builds");
        let unit = inputs.units.get("main").expect("test unit is present");
        let function = unit
            .functions
            .iter()
            .find(|function| function.symbol == "main.boom")
            .expect("test function is present");
        let function_plans = inputs
            .function_plans
            .get("main::boom")
            .expect("test function plans are present");
        let local_interface_tables = LocalInterfaceFacts::empty();
        let emitter = FunctionEmitter::new(
            unit,
            function,
            "main::boom",
            function_plans,
            &mut image,
            &inputs,
            &service_boundary_plans,
            &local_interface_tables,
            SourceAttributionMode::AdmittedPhase1,
        )
        .expect("test emitter constructs");
        let emitted = emitter.emit().expect("test function emits");

        assert_eq!(
            emitted.source_map.len(),
            1,
            "only the raise instruction needs a source site: {emitted:?}"
        );
        let row = &emitted.source_map[0];
        assert_eq!(row.site, throw_site, "the MIR throw site must be retained");
        assert!(
            row.start_pc > 0,
            "the raise site must sit after the payload expression, got pc {}",
            row.start_pc
        );
    }

    /// A raw `CatchResult` slot is not a producer. Without the exact Catch
    /// expression shape, a field read must fail instead of synthesizing a
    /// semantic owner layout for `.tag`.
    #[test]
    fn bare_tag_discriminator_read_without_a_producer_shape_fails_closed() {
        let catch_result = TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![TypeRefIr::builtin("void"), TypeRefIr::builtin("number")],
        };
        let tag_type = TypeRefIr::Union {
            items: vec![
                TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: "err".to_string(),
                    },
                },
                TypeRefIr::Literal {
                    value: LiteralIr::String {
                        value: "ok".to_string(),
                    },
                },
            ],
        };
        let mut function = MirFunction {
            executable_index: 0,
            origin: skiff_artifact_model::PackageExecutableCoordinate {
                file_ir_identity: "file:main".to_string(),
                module_path: "main".to_string(),
                executable_index: 0,
            },
            symbol: "main.run".to_string(),
            kind: MirExecutableKind::Function,
            native: false,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: None,
            receiver: None,
            slots: vec![MirSlot {
                slot: 0,
                name: "attempt".to_string(),
                kind: MirSlotKind::Local,
                writable_local: false,
                ty: Some(catch_result.clone()),
            }],
            index_accesses: BTreeMap::new(),
            expression_blocks: BTreeMap::new(),
            expressions: vec![
                MirExpression {
                    index: 0,
                    expression: ExprIr::LoadSlot { slot: 0 },
                    ty: catch_result.clone(),
                    writable: None,
                    direct_call: None,
                    stream_result: None,
                    remote_interface: None,
                },
                MirExpression {
                    index: 1,
                    expression: ExprIr::Field {
                        object: ExprRefIr { expression: 0 },
                        field: "tag".to_string(),
                    },
                    ty: tag_type,
                    writable: None,
                    direct_call: None,
                    stream_result: None,
                    remote_interface: None,
                },
                MirExpression {
                    index: 2,
                    expression: ExprIr::Literal {
                        value: LiteralIr::String {
                            value: "ok".to_string(),
                        },
                    },
                    ty: TypeRefIr::Literal {
                        value: LiteralIr::String {
                            value: "ok".to_string(),
                        },
                    },
                    writable: None,
                    direct_call: None,
                    stream_result: None,
                    remote_interface: None,
                },
                MirExpression {
                    index: 3,
                    expression: ExprIr::Binary {
                        op: BinaryOpIr::Equal,
                        left: ExprRefIr { expression: 1 },
                        right: ExprRefIr { expression: 2 },
                    },
                    ty: TypeRefIr::builtin("bool"),
                    writable: None,
                    direct_call: None,
                    stream_result: None,
                    remote_interface: None,
                },
            ],
            blocks: vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![MirStmt {
                    statement_index: 0,
                    span: None,
                    kind: MirStmtKind::Expr {
                        value: ExprRefIr { expression: 3 },
                    },
                }],
                successors: Vec::new(),
            }],
            regions: Vec::new(),
            statements: vec![MirStatementEntry {
                statement_index: 0,
                span: None,
            }],
            stream_result: None,
            liveness: MirLiveness::default(),
            effect_summary_ref: PackageCallableId::new("callable:main:run".to_string()),
            effect_summary: CallableEffectSummary::analysis_pending(),
            source_span: None,
            source_event_plan: MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ),
        };
        function.liveness = compute_liveness(&function).expect("test liveness computes");

        let mut file_ir = FileIrUnit::empty("main", "source-hash");
        file_ir.file_ir_identity = "file:main".to_string();
        let bundle = ConstEvaluator::new(Bounds::default())
            .evaluate_unit(&file_ir)
            .expect("test bundle evaluates");
        let unit = MirUnit {
            file_ir_identity: file_ir.file_ir_identity.clone(),
            module_path: file_ir.module_path.clone(),
            actor_declarations: file_ir.actor_declarations.clone(),
            external_refs: file_ir.external_refs.clone(),
            source_map: file_ir.source_map.clone(),
            type_table: file_ir.type_table.clone(),
            package_type_records: BTreeMap::new(),
            link_targets: file_ir.link_targets.clone(),
            constants: Vec::new(),
            functions: vec![function],
        };
        let plans = derive_test_bytecode_value_transfer_plans(std::slice::from_ref(&unit))
            .expect("the source classifier covers the test MIR");
        let error =
            crate::bytecode::emitter::emit_bytecode_artifact_unchecked(&[unit], &[bundle], &plans)
                .expect_err("a raw CatchResult slot cannot mint a semantic shape");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedConstruct {
                function_key,
                construct: "exact expression producer shape",
                location,
            } if function_key == "main::run"
                && location.contains("field read expression 0")
                && location.contains("has no analyzed field layout")
        ));
    }

    /// A union-typed record constructor still tags the runtime value with the
    /// constructed nominal leaf: the NewRecord shape carries the leaf type so
    /// throw/catch matches the actual branch identity, while the expression's
    /// static union type stays a slot/parameter context fact.
    #[test]
    fn union_typed_construct_emits_the_nominal_leaf_shape() {
        let leaf = TypeRefIr::LocalType { type_index: 0 };
        let union_ty = TypeRefIr::Union {
            items: vec![leaf.clone(), TypeRefIr::LocalType { type_index: 1 }],
        };
        let mut function = MirFunction {
            executable_index: 0,
            origin: skiff_artifact_model::PackageExecutableCoordinate {
                file_ir_identity: "file:main".to_string(),
                module_path: "main".to_string(),
                executable_index: 0,
            },
            symbol: "main.run".to_string(),
            kind: MirExecutableKind::Function,
            native: false,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: None,
            receiver: None,
            slots: Vec::new(),
            index_accesses: BTreeMap::new(),
            expression_blocks: BTreeMap::new(),
            expressions: vec![
                MirExpression {
                    index: 0,
                    expression: ExprIr::Literal {
                        value: LiteralIr::Number {
                            value: serde_json::Number::from(1_u64),
                        },
                    },
                    ty: TypeRefIr::builtin("number"),
                    writable: None,
                    direct_call: None,
                    stream_result: None,
                    remote_interface: None,
                },
                MirExpression {
                    index: 1,
                    expression: ExprIr::Construct {
                        type_ref: leaf.clone(),
                        fields: BTreeMap::from([(
                            "marker".to_string(),
                            ExprRefIr { expression: 0 },
                        )]),
                    },
                    ty: union_ty.clone(),
                    writable: None,
                    direct_call: None,
                    stream_result: None,
                    remote_interface: None,
                },
            ],
            blocks: vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![MirStmt {
                    statement_index: 0,
                    span: None,
                    kind: MirStmtKind::Throw {
                        value: ExprRefIr { expression: 1 },
                        payload_type: union_ty,
                        site: InstructionSourceSite::Synthetic {
                            reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                        },
                    },
                }],
                successors: Vec::new(),
            }],
            regions: Vec::new(),
            statements: vec![MirStatementEntry {
                statement_index: 0,
                span: None,
            }],
            stream_result: None,
            liveness: MirLiveness::default(),
            effect_summary_ref: PackageCallableId::new("callable:main:run".to_string()),
            effect_summary: CallableEffectSummary::analysis_pending(),
            source_span: None,
            source_event_plan: MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ),
        };
        function.liveness = compute_liveness(&function).expect("test liveness computes");

        let mut file_ir = FileIrUnit::empty("main", "source-hash");
        file_ir.file_ir_identity = "file:main".to_string();
        file_ir.type_table.push(skiff_artifact_model::TypeDeclIr {
            name: "LeafA".to_string(),
            descriptor: skiff_artifact_model::TypeDescriptorIr::Record {
                fields: BTreeMap::from([("marker".to_string(), TypeRefIr::builtin("number"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        file_ir.type_table.push(skiff_artifact_model::TypeDeclIr {
            name: "LeafB".to_string(),
            descriptor: skiff_artifact_model::TypeDescriptorIr::Record {
                fields: BTreeMap::from([("marker".to_string(), TypeRefIr::builtin("number"))]),
            },
            type_params: Vec::new(),
            implements: Vec::new(),
            source_span: None,
        });
        let bundle = ConstEvaluator::new(Bounds::default())
            .evaluate_unit(&file_ir)
            .expect("test bundle evaluates");
        let unit = MirUnit {
            file_ir_identity: file_ir.file_ir_identity.clone(),
            module_path: file_ir.module_path.clone(),
            actor_declarations: file_ir.actor_declarations.clone(),
            external_refs: file_ir.external_refs.clone(),
            source_map: file_ir.source_map.clone(),
            type_table: file_ir.type_table.clone(),
            package_type_records: BTreeMap::new(),
            link_targets: file_ir.link_targets.clone(),
            constants: Vec::new(),
            functions: vec![function],
        };
        let plans = derive_test_bytecode_value_transfer_plans(std::slice::from_ref(&unit))
            .expect("the source classifier covers the test MIR");
        let artifact =
            crate::bytecode::emitter::emit_bytecode_artifact_unchecked(&[unit], &[bundle], &plans)
                .expect("union-typed construct emission succeeds");
        let leaf_shape = artifact
            .image
            .pools
            .shapes
            .iter()
            .filter_map(|entry| match entry {
                skiff_artifact_model::BytecodePoolEntry::ShapeRef { shape } => Some(shape),
                _ => None,
            })
            .find(|shape| {
                shape.fields.len() == 1
                    && shape
                        .fields
                        .first()
                        .is_some_and(|field| field.name == "marker")
            })
            .expect("the construct shape is interned");
        let tagged_type = &artifact.image.pools.types[leaf_shape.type_ref as usize];
        assert_eq!(
            tagged_type,
            &skiff_artifact_model::BytecodePoolEntry::TypeRef {
                ty: TypeRefIr::PublicationType {
                    module_path: "main".to_string(),
                    type_index: 0,
                },
                representation_carrier: None,
                plan: ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::SnapshotRelease,
                },
            },
            "the runtime tag must be the nominal leaf, not the union context"
        );
    }

    /// Phase 4 gate 1: a canonical `std.time.sleep` invocation emits one
    /// `HostEffectRef` carrying the canonical binding id and the pinned
    /// registry signature facts unchanged. The retired `NativeCall` →
    /// `HostEffect` effect rewrite must not reappear in the artifact.
    #[test]
    fn canonical_sleep_emits_pinned_registry_signature_without_effect_rewrite() {
        let duration = TypeRefIr::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::PackageId {
                    package_id: "skiff.run/std".to_string(),
                },
                symbol_path: "std.time.Duration".to_string(),
                abi_expectation: None,
            },
        };
        let mut function = MirFunction {
            executable_index: 0,
            origin: skiff_artifact_model::PackageExecutableCoordinate {
                file_ir_identity: "file:main".to_string(),
                module_path: "main".to_string(),
                executable_index: 0,
            },
            symbol: "main.run".to_string(),
            kind: MirExecutableKind::Function,
            native: false,
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: TypeRefIr::builtin("void"),
            self_type: None,
            receiver: None,
            slots: vec![MirSlot {
                slot: 0,
                name: "delay".to_string(),
                kind: MirSlotKind::Local,
                writable_local: false,
                ty: Some(duration.clone()),
            }],
            index_accesses: BTreeMap::new(),
            expression_blocks: BTreeMap::new(),
            expressions: vec![
                MirExpression {
                    index: 0,
                    expression: ExprIr::LoadSlot { slot: 0 },
                    ty: duration.clone(),
                    writable: None,
                    direct_call: None,
                    stream_result: None,
                    remote_interface: None,
                },
                MirExpression {
                    index: 1,
                    expression: ExprIr::Call {
                        call: skiff_artifact_model::CallIr {
                            target: skiff_artifact_model::CallTargetIr::Native {
                                target: skiff_artifact_model::NativeTarget {
                                    namespace: "std.time".to_string(),
                                    symbol: "sleep".to_string(),
                                    binding_key: Some("std.time.sleep".to_string()),
                                    metadata: BTreeMap::new(),
                                },
                            },
                            concrete_receiver: None,
                            site: InstructionSourceSite::Synthetic {
                                reason: SyntheticInstructionSiteReason::CompilerDesugaring,
                            },
                            args: vec![ExprRefIr { expression: 0 }],
                            inout_args: Vec::new(),
                            type_args: BTreeMap::new(),
                            metadata: BTreeMap::new(),
                        },
                    },
                    ty: TypeRefIr::builtin("void"),
                    writable: None,
                    direct_call: None,
                    stream_result: None,
                    remote_interface: None,
                },
            ],
            blocks: vec![MirBlock {
                id: 0,
                label: "entry".to_string(),
                statements: vec![MirStmt {
                    statement_index: 0,
                    span: None,
                    kind: MirStmtKind::Expr {
                        value: ExprRefIr { expression: 1 },
                    },
                }],
                successors: Vec::new(),
            }],
            regions: Vec::new(),
            statements: vec![MirStatementEntry {
                statement_index: 0,
                span: None,
            }],
            stream_result: None,
            liveness: MirLiveness::default(),
            effect_summary_ref: PackageCallableId::new("callable:main:run".to_string()),
            effect_summary: CallableEffectSummary::analysis_pending(),
            source_span: None,
            source_event_plan: MirSourceEventPlan::unavailable(
                MirSourceEventUnavailableReason::SourceFactsNotProvided,
            ),
        };
        function.liveness = compute_liveness(&function).expect("test liveness computes");

        let mut file_ir = FileIrUnit::empty("main", "source-hash");
        file_ir.file_ir_identity = "file:main".to_string();
        let bundle = ConstEvaluator::new(Bounds::default())
            .evaluate_unit(&file_ir)
            .expect("test bundle evaluates");
        let unit = MirUnit {
            file_ir_identity: file_ir.file_ir_identity.clone(),
            module_path: file_ir.module_path.clone(),
            actor_declarations: file_ir.actor_declarations.clone(),
            external_refs: file_ir.external_refs.clone(),
            source_map: file_ir.source_map.clone(),
            type_table: file_ir.type_table.clone(),
            package_type_records: BTreeMap::new(),
            link_targets: file_ir.link_targets.clone(),
            constants: Vec::new(),
            functions: vec![function],
        };
        let plans = derive_bytecode_value_transfer_plans_unchecked(
            std::slice::from_ref(&unit),
            |_module_path, _ty| {
                Ok(ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::Trivial,
                })
            },
        )
        .expect("the explicit raw-backend fixture plan covers the test MIR");
        let artifact =
            crate::bytecode::emitter::emit_bytecode_artifact_unchecked(&[unit], &[bundle], &plans)
                .expect("canonical sleep emission succeeds");
        let relocation = artifact.image.functions["main::run"]
            .relocations
            .iter()
            .find_map(|relocation| match relocation {
                BytecodeRelocation::HostEffectRef(effect) => Some(effect),
                _ => None,
            })
            .expect("sleep emits exactly one HostEffectRef");

        assert_eq!(
            relocation.target.binding_key.as_deref(),
            Some("std.time.sleep"),
            "the artifact carries the canonical binding id"
        );
        assert_eq!(
            relocation.signature.parameter_types,
            vec![duration.clone()],
            "the pinned Duration parameter type is carried exactly"
        );
        assert_eq!(
            relocation.signature.parameter_modes,
            vec![skiff_artifact_model::ParamModeIr::Value]
        );
        assert_eq!(
            relocation.signature.parameter_plans,
            vec![ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial,
            }]
        );
        assert!(relocation.signature.result_types.is_empty());
        assert!(relocation.signature.result_plans.is_empty());
        assert!(
            relocation.signature.effects.may_pending,
            "the pinned mayPending fact is carried exactly"
        );
        assert_eq!(
            relocation.signature.effects.pending_effect_categories,
            vec![PendingEffectCategory::NativeCall],
            "the registry NativeCall trace is carried without the retired rewrite"
        );
    }
}
