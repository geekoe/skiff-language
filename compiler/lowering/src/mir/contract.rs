//! Fail-closed accessors and structured MIR construction errors.

use std::collections::BTreeSet;

use skiff_artifact_model::{
    BoxSourceIr, CallableEffectSummary, ContractOperationId, ExprIr, ExprRefIr,
    PackageExecutableCoordinate, ReceiverCallAbi, TypeRefIr,
};
use skiff_compiler_core::PackageCallableIdentityError;

use super::{
    abi::{direct_call_facts, is_direct_target},
    facts::call_writable_facts,
    MirConst, MirExpression, MirFunction, MirInOutPathSegment, MirIndexPolicy, MirParamMode,
    MirReceiverFacts, MirSlot, MirSlotKind, MirStmtKind, MirUnit, MirWritablePathSegment,
    MirWritablePlace,
};

impl MirUnit {
    /// Resolves one exact path-free package executable coordinate. Both owner
    /// components are checked before the executable-table index is used.
    pub fn function_by_origin(
        &self,
        origin: &PackageExecutableCoordinate,
    ) -> Result<&MirFunction, MirContractError> {
        if origin.file_ir_identity != self.file_ir_identity
            || origin.module_path != self.module_path
        {
            return Err(MirContractError::ExecutableOriginOwnerMismatch {
                expected_file_ir_identity: self.file_ir_identity.clone(),
                expected_module_path: self.module_path.clone(),
                actual_file_ir_identity: origin.file_ir_identity.clone(),
                actual_module_path: origin.module_path.clone(),
            });
        }
        let function = self.function_by_executable_index(origin.executable_index)?;
        if function.origin != *origin {
            return Err(MirContractError::FunctionOriginMismatch {
                function: function.symbol.clone(),
            });
        }
        Ok(function)
    }

    /// Resolves an exact executable-table index to its MIR function. MIR
    /// function order is declaration-name order, so consumers must not index
    /// `functions` directly with a File IR executable index.
    pub fn function_by_executable_index(
        &self,
        executable_index: u32,
    ) -> Result<&MirFunction, MirContractError> {
        let mut matches = self
            .functions
            .iter()
            .filter(|function| function.executable_index == executable_index);
        let function =
            matches
                .next()
                .ok_or_else(|| MirContractError::MissingExecutableFunction {
                    module_path: self.module_path.clone(),
                    executable_index,
                })?;
        if matches.next().is_some() {
            return Err(MirContractError::DuplicateExecutableFunction {
                module_path: self.module_path.clone(),
                executable_index,
            });
        }
        Ok(function)
    }

    /// Validates that executable indices are unique and dense even though the
    /// public function vector has a different deterministic ordering.
    pub fn validate_executable_indices(&self) -> Result<(), MirContractError> {
        let mut seen = BTreeSet::new();
        for function in &self.functions {
            if !seen.insert(function.executable_index) {
                return Err(MirContractError::DuplicateExecutableFunction {
                    module_path: self.module_path.clone(),
                    executable_index: function.executable_index,
                });
            }
            let expected = skiff_artifact_model::PackageExecutableCoordinate {
                file_ir_identity: self.file_ir_identity.clone(),
                module_path: self.module_path.clone(),
                executable_index: function.executable_index,
            };
            if function.origin != expected {
                return Err(MirContractError::FunctionOriginMismatch {
                    function: function.symbol.clone(),
                });
            }
        }
        for expected in 0..self.functions.len() {
            let expected =
                u32::try_from(expected).map_err(|_| MirContractError::ExecutableIndexOverflow {
                    module_path: self.module_path.clone(),
                })?;
            self.function_by_executable_index(expected)?;
        }
        Ok(())
    }

    /// Resolves a function-local `LoadConst` index to the exact graph key and
    /// type metadata owned by this MIR unit.
    pub fn constant(&self, const_index: u32) -> Result<&MirConst, MirContractError> {
        let constant = self.constants.get(const_index as usize).ok_or_else(|| {
            MirContractError::MissingConstant {
                module_path: self.module_path.clone(),
                const_index,
                constant_count: self.constants.len(),
            }
        })?;
        if constant.index != const_index {
            return Err(MirContractError::ConstantIndexMismatch {
                module_path: self.module_path.clone(),
                requested: const_index,
                stored: constant.index,
            });
        }
        Ok(constant)
    }

    /// Validates dense constant indices and unique ConstEvaluator graph keys.
    pub fn validate_constants(&self) -> Result<(), MirContractError> {
        let mut symbols = BTreeSet::new();
        for (expected, constant) in self.constants.iter().enumerate() {
            let expected =
                u32::try_from(expected).map_err(|_| MirContractError::ConstantIndexOverflow {
                    module_path: self.module_path.clone(),
                })?;
            if constant.index != expected {
                return Err(MirContractError::ConstantIndexMismatch {
                    module_path: self.module_path.clone(),
                    requested: expected,
                    stored: constant.index,
                });
            }
            if !symbols.insert(&constant.symbol) {
                return Err(MirContractError::DuplicateConstantSymbol {
                    module_path: self.module_path.clone(),
                    symbol: constant.symbol.clone(),
                });
            }
        }
        Ok(())
    }
}

impl MirFunction {
    /// Resolves a function-local expression reference without consulting File
    /// IR. A missing or non-canonical index is a structured contract failure.
    pub fn expression(
        &self,
        expression_ref: ExprRefIr,
    ) -> Result<&MirExpression, MirContractError> {
        let expression = self
            .expressions
            .get(expression_ref.expression as usize)
            .ok_or_else(|| MirContractError::MissingExpression {
                function: self.symbol.clone(),
                index: expression_ref.expression,
                expression_count: self.expressions.len(),
            })?;
        if expression.index != expression_ref.expression {
            return Err(MirContractError::ExpressionIndexMismatch {
                function: self.symbol.clone(),
                requested: expression_ref.expression,
                stored: expression.index,
            });
        }
        Ok(expression)
    }

    /// Validates the complete function-owned expression table, including
    /// entries not reached by the current CFG.
    pub fn validate_expression_indices(&self) -> Result<(), MirContractError> {
        for (expected, expression) in self.expressions.iter().enumerate() {
            let expected =
                u32::try_from(expected).map_err(|_| MirContractError::ExpressionIndexOverflow {
                    function: self.symbol.clone(),
                })?;
            if expression.index != expected {
                return Err(MirContractError::ExpressionIndexMismatch {
                    function: self.symbol.clone(),
                    requested: expected,
                    stored: expression.index,
                });
            }
        }
        Ok(())
    }

    /// Resolves one source-owned bracket segment by its function-owned,
    /// single-evaluation selector expression.
    pub fn index_access(
        &self,
        selector: ExprRefIr,
    ) -> Result<&super::MirIndexAccessFacts, MirContractError> {
        let selector_expression = self.expression(selector)?;
        let access = self
            .index_accesses
            .get(&selector.expression)
            .ok_or_else(|| MirContractError::MissingIndexAccessFacts {
                function: self.symbol.clone(),
                selector: selector.expression,
            })?;
        if selector_expression.ty != access.selector_type {
            return Err(MirContractError::InvalidIndexAccessFacts {
                function: self.symbol.clone(),
                selector: selector.expression,
                message: "selector type disagrees with retained source fact".to_string(),
            });
        }
        Ok(access)
    }

    /// Proves all-and-only coverage of source bracket segments without
    /// deriving receiver kind or read/write policy from File IR shape.
    pub fn validate_index_accesses(&self) -> Result<(), MirContractError> {
        let mut used = BTreeSet::new();
        for expression in &self.expressions {
            if let ExprIr::Index { object, index } = &expression.expression {
                let access = self.index_access(*index)?;
                if self.expression(*object)?.ty != access.receiver_type {
                    return Err(MirContractError::InvalidIndexAccessFacts {
                        function: self.symbol.clone(),
                        selector: index.expression,
                        message: "receiver type disagrees with retained source fact".to_string(),
                    });
                }
                if expression.ty != access.result_type {
                    return Err(MirContractError::InvalidIndexAccessFacts {
                        function: self.symbol.clone(),
                        selector: index.expression,
                        message: "result type disagrees with retained source fact".to_string(),
                    });
                }
                if !matches!(
                    access.policy,
                    MirIndexPolicy::StrictRead | MirIndexPolicy::IntermediateMustExist
                ) {
                    return Err(MirContractError::InvalidIndexAccessFacts {
                        function: self.symbol.clone(),
                        selector: index.expression,
                        message: "value expression has a write/loan-only index policy".to_string(),
                    });
                }
                used.insert(index.expression);
            }
            if let Some(writable) = &expression.writable {
                if let Some(place) = &writable.mutating_receiver {
                    self.validate_place_index_accesses(place, false, &mut used)?;
                }
                for loan in &writable.inout_loans {
                    for (segment_index, segment) in loan.path.iter().enumerate() {
                        if let MirInOutPathSegment::Index {
                            selector, access, ..
                        } = segment
                        {
                            self.validate_embedded_index_access(*selector, access, &mut used)?;
                            let expected = if segment_index + 1 == loan.path.len() {
                                MirIndexPolicy::LoanMustExist
                            } else {
                                MirIndexPolicy::IntermediateMustExist
                            };
                            if access.policy != expected {
                                return Err(MirContractError::InvalidIndexAccessFacts {
                                    function: self.symbol.clone(),
                                    selector: selector.expression,
                                    message: format!(
                                        "inout path requires {expected:?}, found {:?}",
                                        access.policy
                                    ),
                                });
                            }
                        }
                    }
                }
            }
        }
        for block in &self.blocks {
            for statement in &block.statements {
                if let MirStmtKind::Assign { place, .. } = &statement.kind {
                    self.validate_place_index_accesses(place, true, &mut used)?;
                }
            }
        }
        if let Some(selector) = self
            .index_accesses
            .keys()
            .find(|selector| !used.contains(selector))
        {
            return Err(MirContractError::UnusedIndexAccessFacts {
                function: self.symbol.clone(),
                selector: *selector,
            });
        }
        Ok(())
    }

    fn validate_place_index_accesses(
        &self,
        place: &MirWritablePlace,
        assignment: bool,
        used: &mut BTreeSet<u32>,
    ) -> Result<(), MirContractError> {
        for (segment_index, segment) in place.path.iter().enumerate() {
            if let MirWritablePathSegment::Index { index, access, .. } = segment {
                self.validate_embedded_index_access(*index, access, used)?;
                let policy_is_valid = if assignment {
                    if segment_index + 1 == place.path.len() {
                        matches!(
                            access.policy,
                            MirIndexPolicy::TerminalReplace | MirIndexPolicy::TerminalUpsert
                        )
                    } else {
                        access.policy == MirIndexPolicy::IntermediateMustExist
                    }
                } else {
                    matches!(
                        access.policy,
                        MirIndexPolicy::StrictRead | MirIndexPolicy::IntermediateMustExist
                    )
                };
                if !policy_is_valid {
                    return Err(MirContractError::InvalidIndexAccessFacts {
                        function: self.symbol.clone(),
                        selector: index.expression,
                        message: format!(
                            "writable path context rejects index policy {:?}",
                            access.policy
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    fn validate_embedded_index_access(
        &self,
        selector: ExprRefIr,
        embedded: &super::MirIndexAccessFacts,
        used: &mut BTreeSet<u32>,
    ) -> Result<(), MirContractError> {
        if self.index_access(selector)? != embedded {
            return Err(MirContractError::InvalidIndexAccessFacts {
                function: self.symbol.clone(),
                selector: selector.expression,
                message: "writable path copy disagrees with canonical function fact".to_string(),
            });
        }
        used.insert(selector.expression);
        Ok(())
    }

    /// Resolves an exact block id. Block vector order and stored ids are one
    /// contract; consumers must not silently accept a mismatched id.
    pub fn block(&self, block: u32) -> Result<&super::MirBlock, MirContractError> {
        let entry =
            self.blocks
                .get(block as usize)
                .ok_or_else(|| MirContractError::MissingBlock {
                    function: self.symbol.clone(),
                    block,
                    block_count: self.blocks.len(),
                })?;
        if entry.id != block {
            return Err(MirContractError::BlockIndexMismatch {
                function: self.symbol.clone(),
                expected: block,
                stored: entry.id,
            });
        }
        Ok(entry)
    }

    /// Returns checked mutating receiver/inout facts for one expression.
    /// The expected fact is recomputed solely from this function's owned MIR
    /// expressions and slots, never from File IR.
    pub fn call_writable_facts(
        &self,
        expression_ref: ExprRefIr,
    ) -> Result<Option<&super::MirCallWritableFacts>, MirContractError> {
        let expression = self.expression(expression_ref)?;
        let expected = call_writable_facts(
            expression.index,
            &self.expressions,
            &self.slots,
            &self.index_accesses,
        )
        .map_err(|message| MirContractError::InvalidWritableFacts {
            function: self.symbol.clone(),
            expression: expression.index,
            message,
        })?;
        if expression.writable != expected {
            return Err(MirContractError::WritableFactsMismatch {
                function: self.symbol.clone(),
                expression: expression.index,
            });
        }
        Ok(expression.writable.as_ref())
    }

    /// Validates all expression-owned writable facts.
    pub fn validate_writable_facts(&self) -> Result<(), MirContractError> {
        self.validate_expression_indices()?;
        self.validate_index_accesses()?;
        for expression in &self.expressions {
            let reference = ExprRefIr {
                expression: expression.index,
            };
            self.call_writable_facts(reference)?;
            self.direct_call_facts(reference)?;
        }
        self.validate_receiver_facts()?;
        self.writable_local_slots()?;
        Ok(())
    }

    /// Returns checked dense direct-call ABI facts for a function-owned call.
    pub fn direct_call_facts(
        &self,
        expression_ref: ExprRefIr,
    ) -> Result<Option<&super::MirDirectCallFacts>, MirContractError> {
        let expression = self.expression(expression_ref)?;
        let expected = match &expression.expression {
            ExprIr::Call { call } if is_direct_target(&call.target) => {
                let stored = expression.direct_call.as_ref().ok_or_else(|| {
                    MirContractError::MissingDirectCallFacts {
                        function: self.symbol.clone(),
                        expression: expression.index,
                    }
                })?;
                Some(
                    direct_call_facts(call, &stored.parameter_modes, expression.writable.as_ref())
                        .map_err(|message| MirContractError::InvalidDirectCallFacts {
                            function: self.symbol.clone(),
                            expression: expression.index,
                            message,
                        })?,
                )
            }
            ExprIr::Call { call } if call.concrete_receiver.is_some() => {
                return Err(MirContractError::InvalidDirectCallFacts {
                    function: self.symbol.clone(),
                    expression: expression.index,
                    message: "non-direct call carries concreteReceiver".to_string(),
                });
            }
            _ => None,
        };
        if expression.direct_call != expected {
            return Err(MirContractError::DirectCallFactsMismatch {
                function: self.symbol.clone(),
                expression: expression.index,
            });
        }
        Ok(expression.direct_call.as_ref())
    }

    /// Revalidates the unified receiver carrier without inferring from a
    /// function kind or parameter spelling at emission time.
    pub fn validate_receiver_facts(&self) -> Result<(), MirContractError> {
        let self_slots = self
            .slots
            .iter()
            .filter(|slot| slot.kind == MirSlotKind::SelfValue)
            .collect::<Vec<_>>();
        let expected = match self.self_type.as_ref() {
            None => {
                if !self_slots.is_empty() {
                    return Err(MirContractError::InvalidReceiverFacts {
                        function: self.symbol.clone(),
                        message: "selfType is null but a SelfValue slot exists".to_string(),
                    });
                }
                None
            }
            Some(self_type) => {
                let slot = self.slot(0)?;
                if slot.ty.as_ref() != Some(self_type) {
                    return Err(MirContractError::InvalidReceiverFacts {
                        function: self.symbol.clone(),
                        message: "receiver slot zero does not match selfType".to_string(),
                    });
                }
                match slot.kind {
                    MirSlotKind::SelfValue => {
                        if self_slots.len() != 1 || self.params.iter().any(|param| param.slot == 0)
                        {
                            return Err(MirContractError::InvalidReceiverFacts {
                                function: self.symbol.clone(),
                                message: "implicit receiver is not the unique SelfValue slot zero"
                                    .to_string(),
                            });
                        }
                    }
                    MirSlotKind::Param => {
                        let param = self.params.first().ok_or_else(|| {
                            MirContractError::InvalidReceiverFacts {
                                function: self.symbol.clone(),
                                message: "explicit receiver has no parameter zero".to_string(),
                            }
                        })?;
                        if param.name != "self"
                            || param.slot != 0
                            || param.mode != MirParamMode::Value
                            || &param.ty != self_type
                            || !self_slots.is_empty()
                        {
                            return Err(MirContractError::InvalidReceiverFacts {
                                function: self.symbol.clone(),
                                message: "explicit receiver is not exact Value parameter zero"
                                    .to_string(),
                            });
                        }
                    }
                    other => {
                        return Err(MirContractError::InvalidReceiverFacts {
                            function: self.symbol.clone(),
                            message: format!("receiver slot zero has invalid kind {other:?}"),
                        });
                    }
                }
                Some(MirReceiverFacts {
                    ty: self_type.clone(),
                    slot: 0,
                    parameter_ordinal: 0,
                    call_abi: ReceiverCallAbi::ExplicitSelfFirst,
                })
            }
        };
        if self.receiver != expected {
            return Err(MirContractError::ReceiverFactsMismatch {
                function: self.symbol.clone(),
            });
        }
        Ok(())
    }

    /// Resolves a slot by its function-local index. Slot vector order is part
    /// of the MIR contract and is checked rather than inferred.
    pub fn slot(&self, slot: u32) -> Result<&MirSlot, MirContractError> {
        let entry = self
            .slots
            .get(slot as usize)
            .ok_or_else(|| MirContractError::MissingSlot {
                function: self.symbol.clone(),
                slot,
                slot_count: self.slots.len(),
            })?;
        if entry.slot != slot {
            return Err(MirContractError::SlotIndexMismatch {
                function: self.symbol.clone(),
                requested: slot,
                stored: entry.slot,
            });
        }
        Ok(entry)
    }

    /// Returns the source-owned static slot type. Future emitters must use
    /// this checked surface and fail when lowering left a slot untyped; they
    /// must not infer a replacement type from expressions or File IR.
    pub fn slot_type(&self, slot: u32) -> Result<&TypeRefIr, MirContractError> {
        let entry = self.slot(slot)?;
        entry
            .ty
            .as_ref()
            .ok_or_else(|| MirContractError::MissingSlotType {
                function: self.symbol.clone(),
                slot,
                name: entry.name.clone(),
            })
    }

    /// Validates that every slot exposed to emission has an exact static type.
    pub fn validate_slot_types(&self) -> Result<(), MirContractError> {
        for (expected, slot) in self.slots.iter().enumerate() {
            let expected =
                u32::try_from(expected).map_err(|_| MirContractError::SlotIndexOverflow {
                    function: self.symbol.clone(),
                })?;
            if slot.slot != expected {
                return Err(MirContractError::SlotIndexMismatch {
                    function: self.symbol.clone(),
                    requested: expected,
                    stored: slot.slot,
                });
            }
            self.slot_type(expected)?;
        }
        Ok(())
    }

    /// Source-confirmed mutable local slots, in canonical slot order.
    pub fn writable_local_slots(&self) -> Result<Vec<u32>, MirContractError> {
        let mut writable = Vec::new();
        for slot in &self.slots {
            if !slot.writable_local {
                continue;
            }
            if slot.kind != MirSlotKind::Local {
                return Err(MirContractError::InvalidWritableLocalSlot {
                    function: self.symbol.clone(),
                    slot: slot.slot,
                    kind: format!("{:?}", slot.kind),
                });
            }
            self.slot_type(slot.slot)?;
            writable.push(slot.slot);
        }
        Ok(writable)
    }

    /// Validates the exact `Stream<T>` facts retained by a function and its
    /// stream-typed expressions.
    pub fn validate_stream_facts(&self) -> Result<(), MirContractError> {
        if let Some(item_type) = stream_item_type(&self.return_type) {
            if self
                .stream_result
                .as_ref()
                .is_none_or(|facts| &facts.item_type != item_type)
            {
                return Err(MirContractError::InvalidStreamResultFacts {
                    function: self.symbol.clone(),
                    expression: u32::MAX,
                    message: "function return_type Stream<T> has no exact MIR stream result"
                        .to_string(),
                });
            }
        } else if self.stream_result.is_some() {
            return Err(MirContractError::InvalidStreamResultFacts {
                function: self.symbol.clone(),
                expression: u32::MAX,
                message: "function stream result exists without return_type Stream<T>".to_string(),
            });
        }
        for expression in &self.expressions {
            if let Some(item_type) = stream_item_type(&expression.ty) {
                if expression
                    .stream_result
                    .as_ref()
                    .is_none_or(|facts| &facts.item_type != item_type)
                {
                    return Err(MirContractError::InvalidStreamResultFacts {
                        function: self.symbol.clone(),
                        expression: expression.index,
                        message: "expression type Stream<T> has no exact MIR stream result"
                            .to_string(),
                    });
                }
            } else if expression.stream_result.is_some() {
                return Err(MirContractError::InvalidStreamResultFacts {
                    function: self.symbol.clone(),
                    expression: expression.index,
                    message: "expression stream result exists without expression type Stream<T>"
                        .to_string(),
                });
            }
        }
        for block in &self.blocks {
            for statement in &block.statements {
                if let MirStmtKind::StreamNext {
                    endpoint_slot,
                    item_type,
                } = &statement.kind
                {
                    let slot_type = self.slot_type(*endpoint_slot)?;
                    if stream_item_type(slot_type) != Some(item_type) {
                        return Err(MirContractError::InvalidStreamResultFacts {
                            function: self.symbol.clone(),
                            expression: u32::MAX,
                            message: "StreamNext endpoint slot is not Stream<T> for item T"
                                .to_string(),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    /// Validates retained remote interface facts against their owning
    /// `ExprIr::InterfaceBox` source. Absent facts remain an emission-time
    /// fail-closed condition so an unresolved service slot does not turn into
    /// a guessed local table.
    pub fn validate_remote_interface_facts(&self) -> Result<(), MirContractError> {
        for expression in &self.expressions {
            let Some(facts) = &expression.remote_interface else {
                continue;
            };
            let ExprIr::InterfaceBox {
                interface, source, ..
            } = &expression.expression
            else {
                return Err(MirContractError::InvalidRemoteInterfaceFacts {
                    function: self.symbol.clone(),
                    expression: expression.index,
                    message: "remote interface facts are owned by a non-InterfaceBox expression"
                        .to_string(),
                });
            };
            let BoxSourceIr::Remote {
                public_instance_key,
                operations,
                callee_protocol_identity,
                ..
            } = source
            else {
                return Err(MirContractError::InvalidRemoteInterfaceFacts {
                    function: self.symbol.clone(),
                    expression: expression.index,
                    message: "remote interface facts are owned by a local InterfaceBox".to_string(),
                });
            };
            if interface != &facts.interface
                || public_instance_key != &facts.public_instance_key
                || callee_protocol_identity.as_str() != facts.callee_protocol_identity.as_str()
                || operations.slots.len() != facts.methods.len()
            {
                return Err(MirContractError::InvalidRemoteInterfaceFacts {
                    function: self.symbol.clone(),
                    expression: expression.index,
                    message: "remote interface source disagrees with retained MIR facts"
                        .to_string(),
                });
            }
            for (index, method) in facts.methods.iter().enumerate() {
                if method.slot != index as u32 {
                    return Err(MirContractError::InvalidRemoteInterfaceFacts {
                        function: self.symbol.clone(),
                        expression: expression.index,
                        message: "remote interface method rows are not dense from slot zero"
                            .to_string(),
                    });
                }
                let slot = operations.slots.get(index).ok_or_else(|| {
                    MirContractError::InvalidRemoteInterfaceFacts {
                        function: self.symbol.clone(),
                        expression: expression.index,
                        message: "remote interface method rows are not source-aligned".to_string(),
                    }
                })?;
                if method.slot != slot.slot
                    || method.method_abi_id != slot.method_abi_id
                    || method.signature != slot.signature
                    || method.contract_operation_id
                        != ContractOperationId::new(slot.operation_abi_id.clone())
                {
                    return Err(MirContractError::InvalidRemoteInterfaceFacts {
                        function: self.symbol.clone(),
                        expression: expression.index,
                        message: "remote interface method row disagrees with source row"
                            .to_string(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Conservative pending fact derived from the single owned effect
    /// summary. Unknown analysis can never grant a synchronous optimization.
    pub fn may_pending(&self) -> bool {
        match &self.effect_summary {
            CallableEffectSummary::Analyzed { effects } => effects.may_pending(),
            CallableEffectSummary::Unknown { .. } => true,
        }
    }
}

fn stream_item_type(ty: &TypeRefIr) -> Option<&TypeRefIr> {
    match ty {
        TypeRefIr::Builtin { name, args } if name == "Stream" && args.len() == 1 => args.first(),
        _ => None,
    }
}

/// A fail-closed lookup/validation failure in an already-built MIR function.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MirContractError {
    #[error(
        "MIR executable origin owner ({actual_file_ir_identity}, {actual_module_path}) does not match unit owner ({expected_file_ir_identity}, {expected_module_path})"
    )]
    ExecutableOriginOwnerMismatch {
        expected_file_ir_identity: String,
        expected_module_path: String,
        actual_file_ir_identity: String,
        actual_module_path: String,
    },
    #[error("MIR function `{function}` origin disagrees with its owning unit coordinate")]
    FunctionOriginMismatch { function: String },
    #[error("MIR unit `{module_path}` has no function for executable index {executable_index}")]
    MissingExecutableFunction {
        module_path: String,
        executable_index: u32,
    },
    #[error(
        "MIR unit `{module_path}` has multiple functions for executable index {executable_index}"
    )]
    DuplicateExecutableFunction {
        module_path: String,
        executable_index: u32,
    },
    #[error("MIR unit `{module_path}` has more than u32::MAX functions")]
    ExecutableIndexOverflow { module_path: String },
    #[error(
        "MIR unit `{module_path}` has no constant {const_index} (constant count {constant_count})"
    )]
    MissingConstant {
        module_path: String,
        const_index: u32,
        constant_count: usize,
    },
    #[error(
        "MIR unit `{module_path}` constant lookup {requested} found non-canonical stored index {stored}"
    )]
    ConstantIndexMismatch {
        module_path: String,
        requested: u32,
        stored: u32,
    },
    #[error("MIR unit `{module_path}` has more than u32::MAX constants")]
    ConstantIndexOverflow { module_path: String },
    #[error("MIR unit `{module_path}` repeats constant symbol `{symbol}`")]
    DuplicateConstantSymbol { module_path: String, symbol: String },
    #[error(
        "MIR function `{function}` has no expression {index} (expression count {expression_count})"
    )]
    MissingExpression {
        function: String,
        index: u32,
        expression_count: usize,
    },
    #[error(
        "MIR function `{function}` expression lookup {requested} found non-canonical stored index {stored}"
    )]
    ExpressionIndexMismatch {
        function: String,
        requested: u32,
        stored: u32,
    },
    #[error("MIR function `{function}` has more than u32::MAX expressions")]
    ExpressionIndexOverflow { function: String },
    #[error(
        "MIR function `{function}` index selector {selector} has no exact source-owned access facts"
    )]
    MissingIndexAccessFacts { function: String, selector: u32 },
    #[error(
        "MIR function `{function}` index selector {selector} has invalid source-owned access facts: {message}"
    )]
    InvalidIndexAccessFacts {
        function: String,
        selector: u32,
        message: String,
    },
    #[error(
        "MIR function `{function}` retains unused source-owned index facts for selector {selector}"
    )]
    UnusedIndexAccessFacts { function: String, selector: u32 },
    #[error(
        "MIR function `{function}` expression {expression} has invalid writable facts: {message}"
    )]
    InvalidWritableFacts {
        function: String,
        expression: u32,
        message: String,
    },
    #[error(
        "MIR function `{function}` expression {expression} writable facts disagree with its owned expression/slot facts"
    )]
    WritableFactsMismatch { function: String, expression: u32 },
    #[error("MIR function `{function}` expression {expression} has no direct-call ABI facts")]
    MissingDirectCallFacts { function: String, expression: u32 },
    #[error(
        "MIR function `{function}` expression {expression} has invalid direct-call facts: {message}"
    )]
    InvalidDirectCallFacts {
        function: String,
        expression: u32,
        message: String,
    },
    #[error(
        "MIR function `{function}` expression {expression} direct-call facts disagree with its owned call"
    )]
    DirectCallFactsMismatch { function: String, expression: u32 },
    #[error("MIR function `{function}` has invalid receiver facts: {message}")]
    InvalidReceiverFacts { function: String, message: String },
    #[error(
        "MIR function `{function}` receiver facts disagree with selfType/slot/parameter facts"
    )]
    ReceiverFactsMismatch { function: String },
    #[error("MIR function `{function}` has no slot {slot} (slot count {slot_count})")]
    MissingSlot {
        function: String,
        slot: u32,
        slot_count: usize,
    },
    #[error(
        "MIR function `{function}` slot lookup {requested} found non-canonical stored index {stored}"
    )]
    SlotIndexMismatch {
        function: String,
        requested: u32,
        stored: u32,
    },
    #[error("MIR function `{function}` has more than u32::MAX slots")]
    SlotIndexOverflow { function: String },
    #[error("MIR function `{function}` slot {slot} (`{name}`) has no static type")]
    MissingSlotType {
        function: String,
        slot: u32,
        name: String,
    },
    #[error(
        "MIR function `{function}` marks slot {slot} ({kind}) writable, but only Local slots may be writable"
    )]
    InvalidWritableLocalSlot {
        function: String,
        slot: u32,
        kind: String,
    },
    #[error(
        "MIR function `{function}` block at position {expected} stores non-canonical id {stored}"
    )]
    BlockIndexMismatch {
        function: String,
        expected: u32,
        stored: u32,
    },
    #[error("MIR function `{function}` has more than u32::MAX blocks")]
    BlockIndexOverflow { function: String },
    #[error("MIR function `{function}` has no block {block} (block count {block_count})")]
    MissingBlock {
        function: String,
        block: u32,
        block_count: usize,
    },
    #[error("MIR function `{function}` block {block} references missing successor {successor}")]
    MissingSuccessorBlock {
        function: String,
        block: u32,
        successor: u32,
    },
    #[error(
        "MIR function `{function}` stream facts are invalid at expression {expression}: {message}"
    )]
    InvalidStreamResultFacts {
        function: String,
        expression: u32,
        message: String,
    },
    #[error(
        "MIR function `{function}` remote interface facts are invalid at expression {expression}: {message}"
    )]
    InvalidRemoteInterfaceFacts {
        function: String,
        expression: u32,
        message: String,
    },
}

/// A structured failure while converting File IR plus source facts into MIR.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MirBuildError {
    #[error(
        "MIR source facts for `{module_path}` executable {executable_index} have an invalid owner: {message}"
    )]
    InvalidSourceFactOwner {
        module_path: String,
        executable_index: u32,
        message: String,
    },
    #[error("MIR unit `{module_path}` has no exact fileIrIdentity")]
    MissingFileIrIdentity { module_path: String },
    #[error(
        "MIR units `{first_module}` and `{duplicate_module}` repeat fileIrIdentity `{file_ir_identity}`"
    )]
    DuplicateFileIrIdentity {
        file_ir_identity: String,
        first_module: String,
        duplicate_module: String,
    },
    #[error("MIR input repeats module path `{module_path}`")]
    DuplicateModulePath { module_path: String },
    #[error("package callable `{package_callable_id}` has invalid exact ABI facts: {message}")]
    InvalidPackageCallableAbi {
        package_callable_id: skiff_artifact_model::PackageCallableId,
        message: String,
    },
    #[error("package callable `{package_callable_id}` has conflicting exact ABI facts")]
    ConflictingPackageCallableAbi {
        package_callable_id: skiff_artifact_model::PackageCallableId,
    },
    #[error("service requirement alias `{alias}` has invalid facts: {message}")]
    InvalidServiceRequirementFacts { alias: String, message: String },
    #[error(
        "MIR unit `{module_path}` has {declaration_count} executable declarations but {executable_count} executable bodies"
    )]
    ExecutableCountMismatch {
        module_path: String,
        declaration_count: usize,
        executable_count: usize,
    },
    #[error("MIR unit `{module_path}` has more than u32::MAX executable bodies")]
    ExecutableIndexOverflow { module_path: String },
    #[error(
        "MIR unit `{module_path}` executable index {executable_index} is owned by both `{first_declaration}` and `{duplicate_declaration}`"
    )]
    DuplicateExecutableIndex {
        module_path: String,
        executable_index: u32,
        first_declaration: String,
        duplicate_declaration: String,
    },
    #[error(
        "MIR build for {module_path}::{declaration_name} references missing executable index {executable_index}"
    )]
    MissingExecutable {
        module_path: String,
        declaration_name: String,
        executable_index: u32,
    },
    #[error(
        "MIR executable declaration `{declaration_name}` in `{module_path}` stores symbol `{stored_symbol}`, expected `{expected_symbol}`"
    )]
    ExecutableDeclarationSymbolMismatch {
        module_path: String,
        declaration_name: String,
        expected_symbol: String,
        stored_symbol: String,
    },
    #[error(
        "MIR executable declaration `{declaration_name}` in `{module_path}` names `{declaration_symbol}` but its body names `{executable_symbol}`"
    )]
    ExecutableSymbolMismatch {
        module_path: String,
        declaration_name: String,
        declaration_symbol: String,
        executable_symbol: String,
    },
    #[error(
        "MIR unit `{module_path}` has {declaration_count} constant declarations but {constant_count} constant bodies"
    )]
    ConstantCountMismatch {
        module_path: String,
        declaration_count: usize,
        constant_count: usize,
    },
    #[error("MIR unit `{module_path}` has more than u32::MAX constants")]
    ConstantIndexOverflow { module_path: String },
    #[error(
        "MIR constant declaration `{declaration_name}` in `{module_path}` references index {const_index}, but only {constant_count} bodies exist"
    )]
    ConstantIndexOutOfBounds {
        module_path: String,
        declaration_name: String,
        const_index: u32,
        constant_count: usize,
    },
    #[error(
        "MIR constant declaration `{duplicate_declaration}` in `{module_path}` duplicates constant index {const_index}"
    )]
    DuplicateConstantIndex {
        module_path: String,
        const_index: u32,
        duplicate_declaration: String,
    },
    #[error(
        "MIR constant declaration `{declaration_name}` in `{module_path}` points to body `{constant_name}` at index {const_index}"
    )]
    ConstantNameMismatch {
        module_path: String,
        declaration_name: String,
        constant_name: String,
        const_index: u32,
    },
    #[error(
        "MIR constant declaration `{declaration_name}` in `{module_path}` stores symbol `{stored_symbol}`, expected `{expected_symbol}`"
    )]
    ConstantSymbolMismatch {
        module_path: String,
        declaration_name: String,
        expected_symbol: String,
        stored_symbol: String,
    },
    #[error("MIR unit `{module_path}` repeats constant symbol `{symbol}`")]
    DuplicateConstantSymbol { module_path: String, symbol: String },
    #[error(
        "MIR constant declaration `{declaration_name}` in `{module_path}` disagrees with its body {fact}"
    )]
    ConstantFactMismatch {
        module_path: String,
        declaration_name: String,
        fact: &'static str,
    },
    #[error("MIR unit `{module_path}` has no declaration for dense constant index {const_index}")]
    MissingConstantIndex {
        module_path: String,
        const_index: u32,
    },
    #[error(
        "MIR build requires source-owned callable effect facts for {module_path}::{declaration_name}"
    )]
    MissingCallableEffect {
        module_path: String,
        declaration_name: String,
    },
    #[error(
        "MIR function `{symbol}` in `{module_path}` has {expression_count} expressions but {expression_type_count} expression types"
    )]
    ExpressionTypeCountMismatch {
        module_path: String,
        symbol: String,
        expression_count: usize,
        expression_type_count: usize,
    },
    #[error(
        "MIR function `{symbol}` in `{module_path}` has {statement_count} statements but {statement_span_count} statement span entries"
    )]
    StatementSpanCountMismatch {
        module_path: String,
        symbol: String,
        statement_count: usize,
        statement_span_count: usize,
    },
    #[error("MIR function `{symbol}` in `{module_path}` has more than u32::MAX expressions")]
    ExpressionIndexOverflow { module_path: String, symbol: String },
    #[error(
        "MIR function `{symbol}` in `{module_path}` expression {expression} has invalid writable facts: {message}"
    )]
    InvalidWritableFacts {
        module_path: String,
        symbol: String,
        expression: u32,
        message: String,
    },
    #[error(
        "MIR function `{symbol}` in `{module_path}` expression {expression} has invalid direct-call facts: {message}"
    )]
    InvalidDirectCallFacts {
        module_path: String,
        symbol: String,
        expression: u32,
        message: String,
    },
    #[error("MIR function `{symbol}` in `{module_path}` has invalid receiver facts: {message}")]
    InvalidReceiverFacts {
        module_path: String,
        symbol: String,
        message: String,
    },
    #[error("MIR actor `{actor}` in `{module_path}` is invalid: {message}")]
    InvalidActorDeclaration {
        module_path: String,
        actor: String,
        message: String,
    },
    #[error(
        "failed to construct package callable identity for MIR function `{symbol}` in `{module_path}` (package `{package_id}`): {source}"
    )]
    CallableIdentity {
        package_id: String,
        module_path: String,
        symbol: String,
        #[source]
        source: PackageCallableIdentityError,
    },
    #[error("invalid MIR control flow for `{symbol}` in `{module_path}`: {message}")]
    InvalidControlFlow {
        module_path: String,
        symbol: String,
        message: String,
    },
    #[error("invalid MIR liveness input for `{symbol}` in `{module_path}`: {source}")]
    Liveness {
        module_path: String,
        symbol: String,
        #[source]
        source: Box<MirContractError>,
    },
    #[error("invalid owned MIR function contract for `{symbol}` in `{module_path}`: {source}")]
    InvalidFunctionContract {
        module_path: String,
        symbol: String,
        #[source]
        source: Box<MirContractError>,
    },
    #[error("invalid owned MIR unit contract for `{module_path}`: {source}")]
    InvalidUnitContract {
        module_path: String,
        #[source]
        source: Box<MirContractError>,
    },
}
