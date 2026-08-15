use super::*;

/// One owner temporarily outside frame storage while an ownership-changing
/// instruction crosses a fallible lifecycle boundary. The exact linked plan
/// travels with the owner; successful operations drain the escrow through an
/// infallible commit, while terminal cleanup removes an entry only after its
/// physical release succeeds.
#[derive(Clone)]
pub(super) struct EscrowedOwner {
    pub(super) value: ValueSlot,
    pub(super) plan: LinkedValueTransferPlan,
    pub(super) site: VmLifecycleSite,
}

impl EscrowedOwner {
    pub(super) fn new(
        value: ValueSlot,
        plan: LinkedValueTransferPlan,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    ) -> Self {
        Self {
            value,
            plan,
            site: VmLifecycleSite {
                function,
                instruction,
                opcode,
            },
        }
    }
}

/// A fully checked frame-slot overwrite. The old destination remains in its
/// live cell until its release succeeds. The incoming owner must already be
/// rooted in a source cell or instruction escrow while this reservation is in
/// flight.
#[derive(Clone)]
pub(super) struct SlotOverwriteReservation {
    destination_index: usize,
    destination_plan: LinkedValueTransferPlan,
    destination_live: bool,
}

#[derive(Clone, Copy)]
pub(super) struct InstructionAdvanceReservation {
    frame_ordinal: usize,
    next_instruction: InstructionIndex,
}

impl InstructionAdvanceReservation {
    pub(super) const fn next_instruction(&self) -> InstructionIndex {
        self.next_instruction
    }
}

#[derive(Clone)]
pub(super) struct StoreSlotReservation {
    frame_ordinal: usize,
    pub(super) operand_index: usize,
    remaining_height: usize,
    pub(super) destination: SlotOverwriteReservation,
    advance: InstructionAdvanceReservation,
}

#[derive(Clone)]
pub(super) struct CopySlotReservation {
    pub(super) destination: SlotOverwriteReservation,
    advance: InstructionAdvanceReservation,
}

#[derive(Clone)]
pub(super) struct MoveSlotReservation {
    pub(super) source_index: usize,
    pub(super) destination: SlotOverwriteReservation,
    advance: InstructionAdvanceReservation,
}

/// Caller-side result destinations validated before a child return starts
/// moving owners out of its frame.
#[derive(Clone, Copy)]
pub(super) struct OperandPushWindowReservation {
    frame_ordinal: usize,
    start: usize,
    next_height: usize,
}

#[derive(Clone, Copy)]
pub(super) struct ChildReturnReservation {
    pub(super) child_frame_ordinal: usize,
    pub(super) caller_frame_ordinal: usize,
    pub(super) caller_destination: OperandPushWindowReservation,
    pub(super) resume_instruction: InstructionIndex,
    pub(super) caller_end: usize,
    pub(super) caller_region_depth: usize,
}

#[derive(Clone, Copy)]
pub(super) struct IntrinsicResultReservation {
    frame_ordinal: usize,
    argument_start: usize,
    argument_count: usize,
    remaining_height: usize,
    result_height: usize,
    next_instruction: InstructionIndex,
}

/// One fully checked top-of-stack consumption transaction.
///
/// Source owners remain in their original operand cells throughout every
/// fallible read, share, transfer, allocation, or heap adoption. Exact source
/// and result plans, the result window, and the next instruction are all
/// resolved before the first heap call. Commits below are therefore heap-free
/// and infallible.
#[derive(Clone)]
pub(super) struct OperandConsumeReservation {
    frame_ordinal: usize,
    source_start: usize,
    source_count: usize,
    remaining_height: usize,
    result_height: usize,
    next_instruction: InstructionIndex,
    source_plans: Box<[LinkedValueTransferPlan]>,
    result_plans: Box<[LinkedValueTransferPlan]>,
    function: FunctionIndex,
    instruction: InstructionIndex,
    opcode: Opcode,
}

impl VmFiber {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn reserve_operand_consume(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
        source_count: usize,
        result_count: usize,
    ) -> Result<(OperandConsumeReservation, Vec<ValueSlot>), VmError> {
        let (frame, source_start, values) = self.borrow_operands(source_count)?;
        if frame.function() != function || frame.instruction() != instruction {
            return Err(VmError::FiberNotRunnable { state: self.state });
        }
        let remaining_height = frame.operand_height().checked_sub(source_count).ok_or(
            VmError::OperandStackUnderflow {
                function,
                needed: source_count,
                available: frame.operand_height(),
            },
        )?;
        let result_height =
            remaining_height
                .checked_add(result_count)
                .ok_or(VmError::OperandStackOverflow {
                    function,
                    capacity: frame.operand_capacity(),
                })?;
        if result_height > frame.operand_capacity() {
            return Err(VmError::OperandStackOverflow {
                function,
                capacity: frame.operand_capacity(),
            });
        }
        let source_end = source_start
            .checked_add(source_count)
            .filter(|end| *end <= self.values.len() && *end <= self.live_values.len());
        let result_end = source_start
            .checked_add(result_count)
            .filter(|end| *end <= self.values.len() && *end <= self.live_values.len());
        if source_end.is_none() || result_end.is_none() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        for ordinal in source_count..result_count {
            let index = source_start + ordinal;
            if self.live_values[index] {
                return Err(VmError::LiveDestination {
                    function,
                    instruction,
                    location: VmValueLocation::Operand(remaining_height + ordinal),
                });
            }
        }
        let advance = self.reserve_instruction_advance(&frame, function, instruction)?;
        let source_plans = (0..source_count)
            .map(|ordinal| {
                self.stack_map_operand_plan(function, instruction, remaining_height + ordinal)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let result_plans = (0..result_count)
            .map(|ordinal| {
                self.stack_map_operand_plan(
                    function,
                    advance.next_instruction,
                    remaining_height + ordinal,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if !source_plans
            .iter()
            .chain(&result_plans)
            .all(LifecycleExecutor::supports_release)
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode,
            });
        }
        Ok((
            OperandConsumeReservation {
                frame_ordinal: advance.frame_ordinal,
                source_start,
                source_count,
                remaining_height,
                result_height,
                next_instruction: advance.next_instruction,
                source_plans: source_plans.into_boxed_slice(),
                result_plans: result_plans.into_boxed_slice(),
                function,
                instruction,
                opcode,
            },
            values,
        ))
    }

    pub(super) fn reserved_source_plan<'reservation>(
        &self,
        reservation: &'reservation OperandConsumeReservation,
        ordinal: usize,
    ) -> &'reservation LinkedValueTransferPlan {
        &reservation.source_plans[ordinal]
    }

    pub(super) fn reserved_result_plan<'reservation>(
        &self,
        reservation: &'reservation OperandConsumeReservation,
        ordinal: usize,
    ) -> &'reservation LinkedValueTransferPlan {
        &reservation.result_plans[ordinal]
    }

    pub(super) fn reanchor_reserved_source(
        &mut self,
        reservation: &OperandConsumeReservation,
        ordinal: usize,
        value: ValueSlot,
    ) {
        let index = reservation.source_start + ordinal;
        debug_assert!(self.live_values[index]);
        self.values[index] = value;
    }

    /// Marks a reserved source as adopted by an aggregate heap operation.
    ///
    /// The adopting heap call must have completed successfully before this
    /// helper is used. Until then the source cell remains the unique VM root,
    /// including any flags returned by a preceding owner transfer.
    pub(super) fn adopt_reserved_source(
        &mut self,
        reservation: &OperandConsumeReservation,
        ordinal: usize,
    ) {
        let index = reservation.source_start + ordinal;
        debug_assert!(ordinal < reservation.source_count);
        debug_assert!(self.live_values[index]);
        self.clear_value(index);
    }

    pub(super) fn release_reserved_sources_reverse(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        reservation: &OperandConsumeReservation,
        start: usize,
        end: usize,
    ) -> Result<(), VmError> {
        if start > end || end > reservation.source_count {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function: reservation.function,
                instruction: reservation.instruction,
                opcode: reservation.opcode,
            });
        }
        for ordinal in (start..end).rev() {
            let index = reservation.source_start + ordinal;
            if !self.live_values.get(index).copied().unwrap_or(false) {
                self.state = VmFiberState::Terminal;
                return Err(VmError::DeadValueRead {
                    location: VmValueLocation::Operand(reservation.remaining_height + ordinal),
                });
            }
            let value = self.values[index];
            if !is_discardable_root(&value) {
                if let Err(error) = executor.release(&value, &reservation.source_plans[ordinal]) {
                    self.state = VmFiberState::Terminal;
                    return Err(error.into_vm_error(
                        reservation.function,
                        reservation.instruction,
                        reservation.opcode,
                    ));
                }
            }
            self.clear_value(index);
        }
        Ok(())
    }

    pub(super) fn commit_consumed_operands(&mut self, reservation: OperandConsumeReservation) {
        debug_assert!(reservation.result_plans.is_empty());
        for index in reservation.source_start..reservation.source_start + reservation.source_count {
            debug_assert!(!self.live_values[index]);
            self.values[index] = ValueSlot::null();
        }
        let frame = &mut self.frames[reservation.frame_ordinal];
        frame.set_operand_height(reservation.remaining_height);
        frame.resume_to(reservation.next_instruction);
    }

    pub(super) fn commit_operand_result(
        &mut self,
        reservation: OperandConsumeReservation,
        result: ValueSlot,
    ) {
        debug_assert_eq!(reservation.result_plans.len(), 1);
        for index in reservation.source_start..reservation.source_start + reservation.source_count {
            self.values[index] = ValueSlot::null();
            self.live_values[index] = false;
        }
        self.values[reservation.source_start] = result;
        self.live_values[reservation.source_start] = true;
        let frame = &mut self.frames[reservation.frame_ordinal];
        frame.set_operand_height(reservation.result_height);
        frame.resume_to(reservation.next_instruction);
    }

    pub(super) fn reserve_instruction_advance(
        &self,
        frame: &VmFrame,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<InstructionAdvanceReservation, VmError> {
        let frame_ordinal = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        if self.frames.get(frame_ordinal).is_none_or(|current| {
            current.function() != frame.function()
                || current.instruction() != frame.instruction()
                || function != frame.function()
                || instruction != frame.instruction()
        }) {
            return Err(VmError::FiberNotRunnable { state: self.state });
        }
        let next =
            instruction
                .get()
                .checked_add(1)
                .ok_or(VmError::InstructionPointerOutOfBounds {
                    function,
                    instruction,
                })?;
        if next as usize >= self.function(function)?.instructions().len() {
            return Err(VmError::InstructionPointerOutOfBounds {
                function,
                instruction,
            });
        }
        Ok(InstructionAdvanceReservation {
            frame_ordinal,
            next_instruction: InstructionIndex::new(next),
        })
    }

    pub(super) fn reserve_slot_overwrite(
        &self,
        frame: &VmFrame,
        slot_count: usize,
        destination: FrameSlotIndex,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<SlotOverwriteReservation, VmError> {
        let destination_index = Self::slot_index(frame, slot_count, destination, frame.function())?;
        let destination_live = self.live_values.get(destination_index).copied().ok_or(
            VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            },
        )?;
        self.values
            .get(destination_index)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        if destination_live
            && self
                .function(frame.function())?
                .frame()
                .writable_local_slots()
                .binary_search(&destination)
                .is_err()
        {
            return Err(VmError::LiveDestination {
                function,
                instruction,
                location: VmValueLocation::FrameSlot(destination),
            });
        }
        Ok(SlotOverwriteReservation {
            destination_index,
            destination_plan: self.slot_plan(frame.function(), destination)?,
            destination_live,
        })
    }

    pub(super) fn reserve_move_slot(
        &self,
        frame: &VmFrame,
        slot_count: usize,
        source: FrameSlotIndex,
        destination: FrameSlotIndex,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<MoveSlotReservation, VmError> {
        let source_index = Self::slot_index(frame, slot_count, source, frame.function())?;
        if !self.live_values.get(source_index).copied().unwrap_or(false) {
            return Err(VmError::DeadValueRead {
                location: VmValueLocation::FrameSlot(source),
            });
        }
        self.values
            .get(source_index)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        if source == destination {
            return Err(VmError::LiveDestination {
                function,
                instruction,
                location: VmValueLocation::FrameSlot(destination),
            });
        }
        Ok(MoveSlotReservation {
            source_index,
            destination: self.reserve_slot_overwrite(
                frame,
                slot_count,
                destination,
                function,
                instruction,
            )?,
            advance: self.reserve_instruction_advance(frame, function, instruction)?,
        })
    }

    pub(super) fn reserve_copy_slot(
        &self,
        frame: &VmFrame,
        slot_count: usize,
        destination: FrameSlotIndex,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<CopySlotReservation, VmError> {
        Ok(CopySlotReservation {
            destination: self.reserve_slot_overwrite(
                frame,
                slot_count,
                destination,
                function,
                instruction,
            )?,
            advance: self.reserve_instruction_advance(frame, function, instruction)?,
        })
    }

    pub(super) fn reserve_store_slot(
        &self,
        frame: &VmFrame,
        slot_count: usize,
        destination: FrameSlotIndex,
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<StoreSlotReservation, VmError> {
        let remaining_height =
            frame
                .operand_height()
                .checked_sub(1)
                .ok_or(VmError::OperandStackUnderflow {
                    function,
                    needed: 1,
                    available: frame.operand_height(),
                })?;
        let operand_index = frame.operand_base().checked_add(remaining_height).ok_or(
            VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            },
        )?;
        if !self
            .live_values
            .get(operand_index)
            .copied()
            .unwrap_or(false)
        {
            return Err(VmError::DeadValueRead {
                location: VmValueLocation::Operand(remaining_height),
            });
        }
        self.values
            .get(operand_index)
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        Ok(StoreSlotReservation {
            frame_ordinal: self
                .frames
                .len()
                .checked_sub(1)
                .ok_or(VmError::FiberNotRunnable { state: self.state })?,
            operand_index,
            remaining_height,
            destination: self.reserve_slot_overwrite(
                frame,
                slot_count,
                destination,
                function,
                instruction,
            )?,
            advance: self.reserve_instruction_advance(frame, function, instruction)?,
        })
    }

    pub(super) fn release_reserved_destination(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        reservation: &SlotOverwriteReservation,
        function: FunctionIndex,
        instruction: InstructionIndex,
        opcode: Opcode,
    ) -> Result<(), VmError> {
        if reservation.destination_live {
            let old = self.values[reservation.destination_index];
            executor
                .release(&old, &reservation.destination_plan)
                .map_err(|error| error.into_vm_error(function, instruction, opcode))?;
            self.clear_value(reservation.destination_index);
        }
        Ok(())
    }

    pub(super) fn commit_move_slot(&mut self, reservation: MoveSlotReservation) {
        let value = self.values[reservation.source_index];
        self.values[reservation.destination.destination_index] = value;
        self.live_values[reservation.destination.destination_index] = true;
        self.clear_value(reservation.source_index);
        self.frames[reservation.advance.frame_ordinal]
            .resume_to(reservation.advance.next_instruction);
    }

    pub(super) fn commit_copy_slot(&mut self, reservation: CopySlotReservation, shared: ValueSlot) {
        let adopted = self
            .terminal_escrow
            .pop()
            .expect("copy-slot shared owner is escrowed until infallible adoption");
        debug_assert!(adopted.value == shared);
        self.values[reservation.destination.destination_index] = adopted.value;
        self.live_values[reservation.destination.destination_index] = true;
        self.frames[reservation.advance.frame_ordinal]
            .resume_to(reservation.advance.next_instruction);
    }

    pub(super) fn commit_store_slot(&mut self, reservation: StoreSlotReservation) {
        debug_assert_eq!(reservation.frame_ordinal, reservation.advance.frame_ordinal);
        let value = self.values[reservation.operand_index];
        self.values[reservation.destination.destination_index] = value;
        self.live_values[reservation.destination.destination_index] = true;
        self.clear_value(reservation.operand_index);
        let frame = &mut self.frames[reservation.frame_ordinal];
        frame.set_operand_height(reservation.remaining_height);
        frame.resume_to(reservation.advance.next_instruction);
    }

    pub(super) fn reserve_intrinsic_result(
        &self,
        function: FunctionIndex,
        instruction: InstructionIndex,
        argument_count: usize,
    ) -> Result<(IntrinsicResultReservation, Vec<ValueSlot>), VmError> {
        let (frame, argument_start, values) = self.borrow_operands(argument_count)?;
        let remaining_height = frame.operand_height().checked_sub(argument_count).ok_or(
            VmError::OperandStackUnderflow {
                function,
                needed: argument_count,
                available: frame.operand_height(),
            },
        )?;
        let result_height =
            remaining_height
                .checked_add(1)
                .ok_or(VmError::OperandStackOverflow {
                    function,
                    capacity: frame.operand_capacity(),
                })?;
        if result_height > frame.operand_capacity() {
            return Err(VmError::OperandStackOverflow {
                function,
                capacity: frame.operand_capacity(),
            });
        }
        let argument_end = argument_start
            .checked_add(argument_count)
            .filter(|end| *end <= self.values.len() && *end <= self.live_values.len());
        if argument_end.is_none() {
            return Err(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            });
        }
        if argument_count == 0
            && self
                .live_values
                .get(argument_start)
                .copied()
                .unwrap_or(true)
        {
            return Err(VmError::LiveDestination {
                function,
                instruction,
                location: VmValueLocation::Operand(frame.operand_height()),
            });
        }
        let advance = self.reserve_instruction_advance(&frame, function, instruction)?;
        Ok((
            IntrinsicResultReservation {
                frame_ordinal: advance.frame_ordinal,
                argument_start,
                argument_count,
                remaining_height,
                result_height,
                next_instruction: advance.next_instruction,
            },
            values,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn release_intrinsic_argument_window(
        &mut self,
        executor: &mut LifecycleExecutor<'_>,
        reservation: &IntrinsicResultReservation,
        values: &[ValueSlot],
        plans: &[LinkedValueTransferPlan],
        function: FunctionIndex,
        instruction: InstructionIndex,
    ) -> Result<(), VmError> {
        if values.len() != reservation.argument_count
            || plans.len() != reservation.argument_count
            || self
                .frames
                .get(reservation.frame_ordinal)
                .is_none_or(|frame| {
                    frame.function() != function
                        || frame.instruction() != instruction
                        || frame.operand_height()
                            != reservation.remaining_height + reservation.argument_count
                })
        {
            return Err(VmError::FullValueLifecyclePlanUnavailable {
                function,
                instruction,
                opcode: Opcode::InvokeIntrinsic,
            });
        }
        for (ordinal, value) in values.iter().enumerate() {
            let index = reservation.argument_start + ordinal;
            if !self.live_values[index] || self.values[index] != *value {
                return Err(VmError::DeadValueRead {
                    location: VmValueLocation::Operand(reservation.remaining_height + ordinal),
                });
            }
        }
        for ordinal in (0..reservation.argument_count).rev() {
            let index = reservation.argument_start + ordinal;
            let value = self.values[index];
            if !matches!(value.kind(), Some(ValueKind::ConstRef)) {
                if let Err(error) = executor.release(&value, &plans[ordinal]) {
                    // A released suffix cannot be read twice. Leave the
                    // continuous prefix in fiber storage for terminal cleanup.
                    self.state = VmFiberState::Terminal;
                    return Err(error.into_vm_error(
                        function,
                        instruction,
                        Opcode::InvokeIntrinsic,
                    ));
                }
            }
            self.clear_value(index);
            self.frames[reservation.frame_ordinal]
                .set_operand_height(reservation.remaining_height + ordinal);
        }
        Ok(())
    }

    pub(super) fn commit_intrinsic_result(
        &mut self,
        reservation: IntrinsicResultReservation,
        result: ValueSlot,
    ) {
        debug_assert_eq!(
            self.frames[reservation.frame_ordinal].operand_height(),
            reservation.remaining_height
        );
        debug_assert!(!self.live_values[reservation.argument_start]);
        self.values[reservation.argument_start] = result;
        self.live_values[reservation.argument_start] = true;
        let frame = &mut self.frames[reservation.frame_ordinal];
        frame.set_operand_height(reservation.result_height);
        frame.resume_to(reservation.next_instruction);
    }

    pub(super) fn reserve_operand_push_window(
        &self,
        frame_ordinal: usize,
        count: usize,
    ) -> Result<OperandPushWindowReservation, VmError> {
        let frame = self
            .frames
            .get(frame_ordinal)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let next_height =
            frame
                .operand_height()
                .checked_add(count)
                .ok_or(VmError::OperandStackOverflow {
                    function: frame.function(),
                    capacity: frame.operand_capacity(),
                })?;
        if next_height > frame.operand_capacity() {
            return Err(VmError::OperandStackOverflow {
                function: frame.function(),
                capacity: frame.operand_capacity(),
            });
        }
        let start = frame
            .operand_base()
            .checked_add(frame.operand_height())
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        let end = start
            .checked_add(count)
            .filter(|end| *end <= self.values.len() && *end <= self.live_values.len())
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        if let Some(offset) = self.live_values[start..end].iter().position(|live| *live) {
            return Err(VmError::LiveDestination {
                function: frame.function(),
                instruction: frame.instruction(),
                location: VmValueLocation::Operand(frame.operand_height() + offset),
            });
        }
        Ok(OperandPushWindowReservation {
            frame_ordinal,
            start,
            next_height,
        })
    }

    pub(super) fn reserve_child_return(
        &self,
        child: &VmFrame,
        result_count: usize,
    ) -> Result<ChildReturnReservation, VmError> {
        let child_frame_ordinal = self
            .frames
            .len()
            .checked_sub(1)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let caller_frame_ordinal = child_frame_ordinal
            .checked_sub(1)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        if self.frames.get(child_frame_ordinal).is_none_or(|current| {
            current.function() != child.function()
                || current.instruction() != child.instruction()
                || current.slot_base() != child.slot_base()
        }) {
            return Err(VmError::FiberNotRunnable { state: self.state });
        }
        let resume_instruction =
            child
                .resume_instruction()
                .ok_or(VmError::VerifiedEntryInvariant {
                    invariant: VmVerifiedInvariant::ChildFrameResumeMissing,
                })?;
        let caller = self
            .frames
            .get(caller_frame_ordinal)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        let caller_end = caller
            .segment_end()
            .filter(|end| *end <= self.values.len() && *end <= self.live_values.len())
            .ok_or(VmError::VerifiedEntryInvariant {
                invariant: VmVerifiedInvariant::FrameLayoutOverflow,
            })?;
        let caller_region_depth = *self
            .region_depths
            .get(caller_frame_ordinal)
            .ok_or(VmError::FiberNotRunnable { state: self.state })?;
        if self.region_depths.len() != self.frames.len() {
            return Err(VmError::FiberNotRunnable { state: self.state });
        }
        Ok(ChildReturnReservation {
            child_frame_ordinal,
            caller_frame_ordinal,
            caller_destination: self
                .reserve_operand_push_window(caller_frame_ordinal, result_count)?,
            resume_instruction,
            caller_end,
            caller_region_depth,
        })
    }

    pub(super) fn commit_operand_push_window(
        &mut self,
        reservation: OperandPushWindowReservation,
        values: &[ValueSlot],
    ) {
        debug_assert_eq!(
            reservation.next_height - self.frames[reservation.frame_ordinal].operand_height(),
            values.len()
        );
        for (offset, value) in values.iter().copied().enumerate() {
            let index = reservation.start + offset;
            self.values[index] = value;
            self.live_values[index] = true;
        }
        self.frames[reservation.frame_ordinal].set_operand_height(reservation.next_height);
    }
}
