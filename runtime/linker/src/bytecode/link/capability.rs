use std::collections::BTreeSet;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryStreamContract, CallableEffectSummary, GatewayDispatchMode,
    GatewayProtocolSurface, LiteralIr, Opcode, PackageLocalAbiSymbol, PackageRefIr,
    PackageSymbolRef, ParamModeIr, PendingEffectCategory, TypeDescriptorIr, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    LinkedBytecodeCandidate, LinkedCallableSignature, LinkedCatchMatcher, LinkedConstantReference,
    LinkedFrozenConstantValue, LinkedGatewayCallableRole, LinkedInstruction,
    LinkedInstructionTarget, LinkedSlotState, LinkedValueDropPlan, LinkedValueTransferPlan,
    TypeIndex,
};
use skiff_runtime_loader::HydratedBytecodePackage;

use crate::bytecode::{
    types::normalize_type, BytecodeLinkError, BytecodeLinkLocation, Phase1LinkedCapability,
};

use super::DeploymentLinker;

impl DeploymentLinker<'_> {
    /// Closed Phase 1 allowlist over the fully linked reachable publication
    /// closure. This runs after relocation and table resolution but before the
    /// raw candidate can leave the production linker.
    pub(super) fn admit_phase_1_capabilities(
        &self,
        candidate: &LinkedBytecodeCandidate,
    ) -> Result<(), BytecodeLinkError> {
        let mut admitted_symbols = BTreeSet::new();
        self.admit_public_roots(candidate)?;

        for function in candidate.functions() {
            let (package, source) = self.source_function(function.key())?;
            let function_location = self.function_location(package, source);

            // The linked instruction and its resolved operand own the most
            // specific failure site. Inspect them before broader frame or
            // transfer facts so a forbidden target cannot be hidden by an
            // equally unsupported signature detail.
            for instruction in function.instructions() {
                let location =
                    self.instruction_location(package, source, instruction.artifact_pc());
                // The opcode owns the primary capability category at this
                // exact PC; resolved operands then close every target fact
                // for otherwise admitted opcodes.
                admit_opcode(instruction.opcode(), location.clone())?;
                if instruction.opcode() == Opcode::InvokeHost {
                    admit_pinned_host_call(candidate, instruction, location)?;
                } else {
                    for resolved in instruction.resolved_operands() {
                        admit_resolved_target(
                            self,
                            candidate,
                            resolved.target(),
                            &mut admitted_symbols,
                            location.clone(),
                        )?;
                    }
                }
            }

            if !function.key().concrete_type_arguments().is_empty()
                || function.key().concrete_receiver().is_some()
            {
                return rejected(Phase1LinkedCapability::Generic, function_location);
            }
            let frame = function.frame();
            if frame.stream_result_type_ref().is_some() {
                return rejected(Phase1LinkedCapability::Stream, function_location);
            }
            if frame
                .parameters()
                .iter()
                .any(|parameter| parameter.mode() != ParamModeIr::Value)
            {
                return rejected(Phase1LinkedCapability::InOut, function_location);
            }
            for plan in frame
                .slot_plans()
                .iter()
                .chain(frame.result_plans())
                .chain(frame.parameters().iter().map(|parameter| parameter.plan()))
            {
                admit_transfer_plan(plan, function_location.clone())?;
            }
            for ty in frame.slot_types().iter().chain(frame.result_types()) {
                admit_type_index(
                    self,
                    candidate,
                    *ty,
                    false,
                    &mut admitted_symbols,
                    function_location.clone(),
                )?;
            }
            if !function.active_regions().is_empty() {
                return rejected(Phase1LinkedCapability::Exception, function_location);
            }
            for region in function.exception_regions() {
                for matcher in region.catch_matchers() {
                    if let LinkedCatchMatcher::Type(index) = matcher {
                        admit_type_index(
                            self,
                            candidate,
                            *index,
                            false,
                            &mut admitted_symbols,
                            function_location.clone(),
                        )?;
                    }
                }
            }
            if !function.switch_tables().is_empty() {
                return rejected(Phase1LinkedCapability::Aggregate, function_location);
            }
            if !function.call_loan_layouts().is_empty() {
                return rejected(Phase1LinkedCapability::InOut, function_location);
            }

            for state in function.stack_map().entries() {
                let instruction = &function.instructions()[state.instruction().get() as usize];
                let location =
                    self.instruction_location(package, source, instruction.artifact_pc());
                if !state.active_regions().is_empty() {
                    return rejected(Phase1LinkedCapability::Exception, location);
                }
                for value in state.stack_before() {
                    admit_transient_stack_value(
                        self,
                        candidate,
                        value.ty(),
                        value.plan(),
                        &mut admitted_symbols,
                        location.clone(),
                    )?;
                }
                for slot in state.slots_before() {
                    if let LinkedSlotState::Live(value) = slot {
                        admit_stack_value(
                            self,
                            candidate,
                            value.ty(),
                            value.plan(),
                            &mut admitted_symbols,
                            location.clone(),
                        )?;
                    }
                }
            }

            admit_effect_summary(function.declarative_effect_summary(), function_location)?;
        }

        // Entry signatures are untrusted linked facts as well, but direct
        // instructions above retain precedence for their exact PC-owned
        // capability diagnostic.
        for entry in candidate.operation_entries() {
            admit_signature(
                self,
                candidate,
                entry.signature(),
                &mut admitted_symbols,
                BytecodeLinkLocation::OperationEntry {
                    deployment: Box::new(self.deployment.reference().clone()),
                    contract_operation_id: entry.contract_operation_id().clone(),
                },
            )?;
        }
        for entry in candidate.gateway_entries() {
            for callable in entry.callables() {
                admit_signature(
                    self,
                    candidate,
                    callable.signature(),
                    &mut admitted_symbols,
                    BytecodeLinkLocation::GatewayEntry {
                        deployment: Box::new(self.deployment.reference().clone()),
                        gateway_entry_key: entry.gateway_entry_key().clone(),
                    },
                )?;
            }
        }
        let referenced_constants = referenced_constant_indices(candidate);
        self.admit_global_tables(candidate, &mut admitted_symbols, &referenced_constants)
    }

    fn admit_public_roots(
        &self,
        candidate: &LinkedBytecodeCandidate,
    ) -> Result<(), BytecodeLinkError> {
        let contract = self
            .deployment
            .contract_store()
            .get(&self.deployment.deployment().contract)
            .expect("hydrated deployment retains its exact validated contract");
        for entry in candidate.operation_entries() {
            let location = BytecodeLinkLocation::OperationEntry {
                deployment: Box::new(self.deployment.reference().clone()),
                contract_operation_id: entry.contract_operation_id().clone(),
            };
            let Some(operation) = contract.operations.get(entry.contract_operation_id()) else {
                return rejected(Phase1LinkedCapability::ServiceTarget, location);
            };
            if !matches!(&operation.contract.stream, BoundaryStreamContract::Unary) {
                return rejected(Phase1LinkedCapability::Stream, location);
            }
            if !matches!(
                &operation.contract.callbacks,
                BoundaryCallbackContract::None
            ) {
                return rejected(Phase1LinkedCapability::Callback, location);
            }
        }

        for entry in candidate.gateway_entries() {
            let location = BytecodeLinkLocation::GatewayEntry {
                deployment: Box::new(self.deployment.reference().clone()),
                gateway_entry_key: entry.gateway_entry_key().clone(),
            };
            match &entry.protocol_surface().protocol {
                GatewayProtocolSurface::Http(http)
                    if http.dispatch_mode == GatewayDispatchMode::Unary => {}
                GatewayProtocolSurface::Http(_) => {
                    return rejected(Phase1LinkedCapability::Stream, location);
                }
                GatewayProtocolSurface::WebSocketConnect(_)
                | GatewayProtocolSurface::WebSocketJsonRpc(_) => {
                    return rejected(Phase1LinkedCapability::WebSocket, location);
                }
            }
            if entry.pre().is_some() || entry.guard().is_some() {
                return rejected(Phase1LinkedCapability::HttpGuardOrPre, location);
            }
            if entry.close_handler().is_some() || entry.close_adapter_plan().is_some() {
                return rejected(Phase1LinkedCapability::WebSocket, location);
            }
            if entry.handler().is_none()
                || entry
                    .callables()
                    .iter()
                    .any(|callable| callable.role() != LinkedGatewayCallableRole::Handler)
            {
                return rejected(Phase1LinkedCapability::Callback, location);
            }
        }
        Ok(())
    }

    fn admit_global_tables(
        &self,
        candidate: &LinkedBytecodeCandidate,
        admitted_symbols: &mut BTreeSet<String>,
        referenced_constants: &BTreeSet<u32>,
    ) -> Result<(), BytecodeLinkError> {
        let location = self.deployment_location();
        for constant in candidate.constants() {
            admit_constant(
                self,
                candidate,
                constant,
                admitted_symbols,
                referenced_constants.contains(&constant.index().get()),
            )?;
        }
        // §4a string-literal discriminator slice: a `string`-typed frozen
        // literal node is admissible only when an instruction-referenced
        // constant (the `tag == "<literal>"` operand) cites it. Package-global
        // and unreferenced string constants stay fail closed.
        let discriminator_nodes = discriminator_string_nodes(candidate, referenced_constants);
        for node in candidate.frozen_constant_nodes() {
            let location = frozen_node_location(self, node);
            let LinkedFrozenConstantValue::Literal(value) = node.value() else {
                return rejected(Phase1LinkedCapability::Aggregate, location);
            };
            if matches!(value, LiteralIr::String { .. }) {
                if !discriminator_nodes.contains(&node.index().get()) {
                    return rejected(Phase1LinkedCapability::ValueShape, location);
                }
            } else if !immediate_literal(value) {
                return rejected(Phase1LinkedCapability::ValueShape, location);
            }
        }
        for ty in candidate.types() {
            if let Some(layout) = ty.container_layout() {
                if !matches!(
                    layout.kind(),
                    skiff_runtime_linked_bytecode::LinkedContainerLayoutKind::Array
                ) {
                    return rejected(Phase1LinkedCapability::ValueShape, location);
                }
            }
            if is_string_type(ty.type_ref()) {
                if discriminator_nodes.is_empty() {
                    return rejected(Phase1LinkedCapability::ValueShape, location);
                }
                continue;
            }
            admit_type(
                self,
                ty.type_ref(),
                true,
                admitted_symbols,
                location.clone(),
            )?;
        }
        Ok(())
    }
}

/// Canonical binding ID of the single host effect admitted in Phase 4. Every
/// other host effect, and every resume descriptor without an exact pinned host
/// target, fails closed.
const PINNED_HOST_EFFECT_BINDING_KEY: &str = "std.time.sleep";

fn admit_pinned_host_call(
    candidate: &LinkedBytecodeCandidate,
    instruction: &LinkedInstruction,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let host_index = instruction
        .resolved_operands()
        .iter()
        .find_map(|resolved| match resolved.target() {
            LinkedInstructionTarget::HostEffectAdapter(index) => Some(index),
            _ => None,
        })
        .ok_or_else(|| host_target_error(location.clone()))?;
    let adapter = candidate
        .host_effect_adapters()
        .get(host_index.get() as usize)
        .filter(|row| row.index() == host_index)
        .ok_or_else(|| host_target_error(location.clone()))?;
    if adapter.binding_key().as_str() != PINNED_HOST_EFFECT_BINDING_KEY {
        return Err(host_target_error(location.clone()));
    }
    // Candidate shape validation already bounds every resolved operand, so
    // this presence check is defense in depth: a pinned host call must retain
    // an exact resume descriptor or it is rejected as an unresolved pending
    // effect.
    let resume_ok = instruction
        .resolved_operands()
        .iter()
        .any(|resolved| match resolved.target() {
            LinkedInstructionTarget::ResumeSite(index) => candidate
                .resume_sites()
                .get(index.get() as usize)
                .is_some_and(|row| row.index() == index),
            _ => false,
        });
    if !resume_ok {
        return rejected(
            Phase1LinkedCapability::PendingEffect(PendingEffectCategory::Unknown),
            location,
        );
    }
    Ok(())
}

fn host_target_error(location: BytecodeLinkLocation) -> BytecodeLinkError {
    BytecodeLinkError::UnsupportedPhase1Capability {
        capability: Phase1LinkedCapability::HostTarget,
        location,
    }
}

fn constant_location(
    linker: &DeploymentLinker<'_>,
    candidate: &LinkedBytecodeCandidate,
    constant: &skiff_runtime_linked_bytecode::LinkedConstantEntry,
) -> BytecodeLinkLocation {
    candidate
        .frozen_constant_nodes()
        .get(constant.reference().node().get() as usize)
        .map_or_else(
            || BytecodeLinkLocation::Package {
                package: Box::new(
                    linker
                        .deployment
                        .packages()
                        .get(constant.origin().package_build_id())
                        .expect("linked constant retains its hydrated package")
                        .reference()
                        .clone(),
                ),
            },
            |node| frozen_node_location(linker, node),
        )
}

fn frozen_node_location(
    linker: &DeploymentLinker<'_>,
    node: &skiff_runtime_linked_bytecode::LinkedFrozenConstantNode,
) -> BytecodeLinkLocation {
    let package = linker
        .deployment
        .packages()
        .get(node.origin().package_build_id())
        .expect("linked constant node retains its hydrated package");
    BytecodeLinkLocation::Constant {
        package: Box::new(package.reference().clone()),
        node_index: node.origin().artifact_index().get(),
    }
}

fn admit_constant(
    linker: &DeploymentLinker<'_>,
    candidate: &LinkedBytecodeCandidate,
    constant: &skiff_runtime_linked_bytecode::LinkedConstantEntry,
    admitted_symbols: &mut BTreeSet<String>,
    referenced: bool,
) -> Result<(), BytecodeLinkError> {
    let location = constant_location(linker, candidate, constant);
    if matches!(
        constant.reference(),
        LinkedConstantReference::PackageSymbol { .. }
    ) {
        return rejected(Phase1LinkedCapability::Constant, location);
    }
    // §4a string-literal discriminator slice: the `string`-typed frozen
    // literal constant bypasses the generic type gate; every other constant
    // still admits its exact linked type.
    let constant_type = candidate
        .types()
        .get(constant.ty().get() as usize)
        .filter(|row| row.index() == constant.ty())
        .map(|row| row.type_ref());
    let discriminator_string = referenced
        && candidate
            .frozen_constant_nodes()
            .get(constant.reference().node().get() as usize)
            .is_some_and(|node| is_discriminator_string_constant(node.value(), constant_type));
    if !discriminator_string {
        admit_type_index(
            linker,
            candidate,
            constant.ty(),
            false,
            admitted_symbols,
            location.clone(),
        )?;
    }
    admit_transfer_plan(constant.plan(), location)
}

/// The set of constant indexes cited by at least one `Const` instruction.
fn referenced_constant_indices(candidate: &LinkedBytecodeCandidate) -> BTreeSet<u32> {
    candidate
        .functions()
        .iter()
        .flat_map(|function| function.instructions())
        .flat_map(|instruction| instruction.resolved_operands())
        .filter_map(|resolved| match resolved.target() {
            LinkedInstructionTarget::Constant(index) => Some(index.get()),
            _ => None,
        })
        .collect()
}

/// The frozen node indexes of the §4a discriminator string constants: exact
/// `String` literals whose `string`-typed constant is cited by an instruction.
fn discriminator_string_nodes(
    candidate: &LinkedBytecodeCandidate,
    referenced_constants: &BTreeSet<u32>,
) -> BTreeSet<u32> {
    candidate
        .constants()
        .iter()
        .filter(|constant| referenced_constants.contains(&constant.index().get()))
        .filter_map(|constant| {
            let node = candidate
                .frozen_constant_nodes()
                .get(constant.reference().node().get() as usize)?;
            let ty = candidate
                .types()
                .get(constant.ty().get() as usize)
                .filter(|row| row.index() == constant.ty())
                .map(|row| row.type_ref());
            is_discriminator_string_constant(node.value(), ty).then_some(node.index().get())
        })
        .collect()
}

fn admit_opcode(opcode: Opcode, location: BytecodeLinkLocation) -> Result<(), BytecodeLinkError> {
    if matches!(
        opcode,
        Opcode::Const
            | Opcode::LoadSlot
            | Opcode::StoreSlot
            | Opcode::CopySlot
            | Opcode::MoveSlot
            | Opcode::TakeSlot
            | Opcode::Pop
            | Opcode::Dup
            | Opcode::Jump
            | Opcode::JumpIfTrue
            | Opcode::JumpIfFalse
            | Opcode::BudgetCheckpoint
            | Opcode::CallLocal
            | Opcode::Return
            | Opcode::Drop
            | Opcode::Not
            | Opcode::Negate
            | Opcode::Add
            | Opcode::Subtract
            | Opcode::Multiply
            | Opcode::Divide
            | Opcode::Equal
            | Opcode::NotEqual
            | Opcode::LessThan
            | Opcode::LessOrEqual
            | Opcode::GreaterThan
            | Opcode::GreaterOrEqual
            | Opcode::NewRecord
            | Opcode::GetDenseField
            | Opcode::SetWritablePath
            | Opcode::NewArrayBuilder
            | Opcode::ArrayBuilderPush
            | Opcode::FreezeArray
            | Opcode::ArrayGet
            | Opcode::ArrayLen
            | Opcode::ArrayPushOwned
            | Opcode::Throw
            | Opcode::Rethrow
    ) {
        return Ok(());
    }
    // InvokeHost is admitted only through the pinned-host-call proof, which
    // binds the exact host adapter and resume descriptor; every other binding
    // fails closed there.
    if opcode == Opcode::InvokeHost {
        return Ok(());
    }
    let capability = match opcode {
        Opcode::InvokeIntrinsic => Phase1LinkedCapability::IntrinsicTarget,
        Opcode::CallService => Phase1LinkedCapability::ServiceTarget,
        Opcode::CallActor => Phase1LinkedCapability::Actor,
        Opcode::CallInterface | Opcode::InterfaceBoxLocal | Opcode::InterfaceBoxRemote => {
            Phase1LinkedCapability::Interface
        }
        Opcode::InvokeCallback | Opcode::MakeCallback => Phase1LinkedCapability::Callback,
        Opcode::StreamNext | Opcode::EmitStream => Phase1LinkedCapability::Stream,
        Opcode::TailCallLocal => Phase1LinkedCapability::TailCall,
        Opcode::CallLocalInOut => Phase1LinkedCapability::InOut,
        Opcode::EnterRegion | Opcode::LeaveRegion | Opcode::Trap => {
            Phase1LinkedCapability::Exception
        }
        Opcode::SwitchTag
        | Opcode::NewMapBuilder
        | Opcode::MapBuilderPut
        | Opcode::FreezeMap
        | Opcode::MapGet
        | Opcode::MapLen
        | Opcode::MapEntryAt
        | Opcode::MapPutOwned => Phase1LinkedCapability::Aggregate,
        Opcode::RepresentationWrap => Phase1LinkedCapability::ValueShape,
        unsupported => Phase1LinkedCapability::UnsupportedOpcode(unsupported),
    };
    rejected(capability, location)
}

fn admit_resolved_target(
    linker: &DeploymentLinker<'_>,
    candidate: &LinkedBytecodeCandidate,
    target: LinkedInstructionTarget,
    admitted_symbols: &mut BTreeSet<String>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let capability = match target {
        LinkedInstructionTarget::FrameSlot(_)
        | LinkedInstructionTarget::Branch(_)
        | LinkedInstructionTarget::Function(_) => return Ok(()),
        LinkedInstructionTarget::Constant(index) => {
            let Some(constant) = candidate.constants().get(index.get() as usize) else {
                return rejected(Phase1LinkedCapability::Constant, location);
            };
            return admit_constant_reference(
                linker,
                candidate,
                constant,
                admitted_symbols,
                location,
            );
        }
        LinkedInstructionTarget::Shape(_) => return Ok(()),
        LinkedInstructionTarget::SwitchTable(_) => Phase1LinkedCapability::Aggregate,
        LinkedInstructionTarget::ActiveRegion(_) => Phase1LinkedCapability::Exception,
        LinkedInstructionTarget::WritablePath(_) => return Ok(()),
        LinkedInstructionTarget::CallLoanLayout(_) => Phase1LinkedCapability::InOut,
        LinkedInstructionTarget::ServiceOperation(_) => Phase1LinkedCapability::ServiceTarget,
        LinkedInstructionTarget::ActorMethod(_) => Phase1LinkedCapability::Actor,
        LinkedInstructionTarget::InterfaceTable(_) => Phase1LinkedCapability::Interface,
        LinkedInstructionTarget::SyntheticCallback(_)
        | LinkedInstructionTarget::CallbackCaptureLayout(_) => Phase1LinkedCapability::Callback,
        LinkedInstructionTarget::HostEffectAdapter(_) => Phase1LinkedCapability::HostTarget,
        LinkedInstructionTarget::Intrinsic(_) => Phase1LinkedCapability::IntrinsicTarget,
        LinkedInstructionTarget::Type(index) => {
            return admit_type_index(linker, candidate, index, false, admitted_symbols, location);
        }
        LinkedInstructionTarget::ResumeSite(_) => Phase1LinkedCapability::PendingEffect(
            skiff_artifact_model::PendingEffectCategory::Unknown,
        ),
    };
    rejected(capability, location)
}

fn admit_constant_reference(
    linker: &DeploymentLinker<'_>,
    candidate: &LinkedBytecodeCandidate,
    constant: &skiff_runtime_linked_bytecode::LinkedConstantEntry,
    admitted_symbols: &mut BTreeSet<String>,
    fallback_location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let Some(node) = candidate
        .frozen_constant_nodes()
        .get(constant.reference().node().get() as usize)
    else {
        return rejected(Phase1LinkedCapability::Constant, fallback_location);
    };
    let location = frozen_node_location(linker, node);
    if matches!(
        constant.reference(),
        LinkedConstantReference::PackageSymbol { .. }
    ) {
        return rejected(Phase1LinkedCapability::Constant, location);
    }
    // Phase 3 string-literal discriminator slice (§4a Amendment 1): a
    // compile-time string literal may enter the frozen constant heap only as
    // a `string`-typed constant. It is the operand of the `tag == "<literal>"`
    // discriminator comparison; generic string values never enter this slice.
    let constant_type = candidate
        .types()
        .get(constant.ty().get() as usize)
        .filter(|row| row.index() == constant.ty())
        .map(|row| row.type_ref());
    let discriminator_string = is_discriminator_string_constant(node.value(), constant_type);
    if !discriminator_string {
        admit_type_index(
            linker,
            candidate,
            constant.ty(),
            false,
            admitted_symbols,
            location.clone(),
        )?;
    }
    admit_transfer_plan(constant.plan(), location.clone())?;
    match node.value() {
        LinkedFrozenConstantValue::Literal(value)
            if immediate_literal(value) || matches!(value, LiteralIr::String { .. }) =>
        {
            Ok(())
        }
        LinkedFrozenConstantValue::Literal(_) => {
            rejected(Phase1LinkedCapability::ValueShape, location)
        }
        _ => rejected(Phase1LinkedCapability::Aggregate, location),
    }
}

/// The narrow frozen-constant slice for the string-literal discriminator
/// (§4a Amendment 1): a `String` literal whose exact linked type is the
/// unparameterized `string` builtin. Anything else is a generic value shape
/// and must fail closed.
fn is_discriminator_string_constant(
    literal: &LinkedFrozenConstantValue,
    ty: Option<&TypeRefIr>,
) -> bool {
    matches!(
        literal,
        LinkedFrozenConstantValue::Literal(LiteralIr::String { .. })
    ) && ty.is_some_and(is_string_type)
}

fn admit_transfer_plan(
    plan: &LinkedValueTransferPlan,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    match plan {
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial | LinkedValueDropPlan::SnapshotRelease,
        } => Ok(()),
        _ => rejected(Phase1LinkedCapability::Resource, location),
    }
}

fn admit_signature(
    linker: &DeploymentLinker<'_>,
    candidate: &LinkedBytecodeCandidate,
    signature: &LinkedCallableSignature,
    admitted_symbols: &mut BTreeSet<String>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if signature
        .parameter_modes()
        .iter()
        .any(|mode| *mode != ParamModeIr::Value)
    {
        return rejected(Phase1LinkedCapability::InOut, location);
    }
    for ty in signature
        .parameter_types()
        .iter()
        .chain(signature.result_types())
    {
        admit_type_index(
            linker,
            candidate,
            *ty,
            false,
            admitted_symbols,
            location.clone(),
        )?;
    }
    for plan in signature
        .parameter_plans()
        .iter()
        .chain(signature.result_plans())
    {
        admit_transfer_plan(plan, location.clone())?;
    }
    admit_effect_summary(signature.effect_summary(), location)
}

fn admit_effect_summary(
    summary: &CallableEffectSummary,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let effects = match summary {
        CallableEffectSummary::Unknown { .. } => {
            return rejected(Phase1LinkedCapability::Effect, location);
        }
        CallableEffectSummary::Analyzed { effects } => effects,
    };
    // Phase 4 admits exactly the pinned std.time.sleep pending authority.
    // Source analysis labels its pending category NativeCall; the exact
    // binding and resume descriptor are still pinned per instruction.
    let pinned_pending = effects.may_pending
        && !effects.pending_effect_categories.is_empty()
        && effects.pending_effect_categories.iter().all(|category| {
            matches!(
                category,
                PendingEffectCategory::NativeCall | PendingEffectCategory::HostEffect
            )
        });
    if (effects.may_pending || !effects.pending_effect_categories.is_empty()) && !pinned_pending {
        return rejected(
            Phase1LinkedCapability::PendingEffect(
                effects
                    .pending_effect_categories
                    .first()
                    .copied()
                    .unwrap_or(PendingEffectCategory::Unknown),
            ),
            location,
        );
    }
    if !effects.inout_path_effects.is_empty() {
        return rejected(Phase1LinkedCapability::InOut, location);
    }
    if effects.escapes_caller_value
        || effects.requires_same_heap_identity
        || effects.invokes_unknown_target
    {
        return rejected(Phase1LinkedCapability::Effect, location);
    }
    Ok(())
}

fn admit_stack_value(
    linker: &DeploymentLinker<'_>,
    candidate: &LinkedBytecodeCandidate,
    ty: TypeIndex,
    plan: &LinkedValueTransferPlan,
    admitted_symbols: &mut BTreeSet<String>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    admit_type_index(
        linker,
        candidate,
        ty,
        false,
        admitted_symbols,
        location.clone(),
    )?;
    admit_transfer_plan(plan, location)
}

/// Admits one transient operand-stack value. This is the only position where
/// the string-literal discriminator slice may put a `string`-typed value: the
/// union/`CatchResult` `tag` read and its `Equal` comparison keep the string
/// on the operand stack and never store it in a live slot. Every other
/// `string` position (slot types, signatures, results, aggregate fields)
/// remains fail closed through [`admit_type`].
fn admit_transient_stack_value(
    linker: &DeploymentLinker<'_>,
    candidate: &LinkedBytecodeCandidate,
    ty: TypeIndex,
    plan: &LinkedValueTransferPlan,
    admitted_symbols: &mut BTreeSet<String>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let string = candidate
        .types()
        .get(ty.get() as usize)
        .filter(|row| row.index() == ty)
        .is_some_and(|row| is_string_type(row.type_ref()));
    if !string {
        admit_type_index(
            linker,
            candidate,
            ty,
            false,
            admitted_symbols,
            location.clone(),
        )?;
    }
    admit_transfer_plan(plan, location)
}

fn admit_type_index(
    linker: &DeploymentLinker<'_>,
    candidate: &LinkedBytecodeCandidate,
    index: TypeIndex,
    allow_void: bool,
    admitted_symbols: &mut BTreeSet<String>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let Some(ty) = candidate.types().get(index.get() as usize) else {
        return rejected(Phase1LinkedCapability::ValueShape, location);
    };
    admit_type(
        linker,
        ty.type_ref(),
        allow_void,
        admitted_symbols,
        location,
    )
}

fn admit_type(
    linker: &DeploymentLinker<'_>,
    ty: &TypeRefIr,
    allow_void: bool,
    admitted_symbols: &mut BTreeSet<String>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    match ty {
        TypeRefIr::Record { fields } => {
            for field in fields.values() {
                admit_type(linker, field, false, admitted_symbols, location.clone())?;
            }
            return Ok(());
        }
        TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => {
            admit_type(linker, &args[0], false, admitted_symbols, location.clone())?;
            return Ok(());
        }
        TypeRefIr::Union { items } => {
            if items.is_empty() {
                return rejected(Phase1LinkedCapability::ValueShape, location);
            }
            for item in items {
                admit_type(linker, item, false, admitted_symbols, location.clone())?;
            }
            return Ok(());
        }
        // Phase 3 string-literal discriminator slice (§4a Amendment 1):
        // `CatchResult<T, E>` and `Exception<E>` are the canonical
        // discriminated-envelope record types whose `tag` field is a
        // compile-time string literal by construction. Their payload types
        // must still be admitted Phase 2 faces; a `never` try type names the
        // catch-over-throw divergence with no runtime value.
        TypeRefIr::Builtin { name, args } if name == "CatchResult" && args.len() == 2 => {
            admit_catch_result_try_argument(linker, &args[0], admitted_symbols, location.clone())?;
            admit_type(linker, &args[1], false, admitted_symbols, location.clone())?;
            return Ok(());
        }
        TypeRefIr::Builtin { name, args } if name == "Exception" && args.len() == 1 => {
            admit_type(linker, &args[0], false, admitted_symbols, location.clone())?;
            return Ok(());
        }
        TypeRefIr::PackageSymbol { symbol } => {
            return admit_package_symbol(linker, symbol, admitted_symbols, location);
        }
        other => admit_structural_leaf(other, allow_void, location),
    }
}

/// The `CatchResult` try argument may be the bottom `never` type (catch over a
/// throw expression) or any ordinary admitted payload face.
fn admit_catch_result_try_argument(
    linker: &DeploymentLinker<'_>,
    ty: &TypeRefIr,
    admitted_symbols: &mut BTreeSet<String>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if matches!(
        ty,
        TypeRefIr::Builtin { name, args }
            if args.is_empty() && (name == "never" || name == "void")
    ) {
        return Ok(());
    }
    admit_type(linker, ty, false, admitted_symbols, location)
}

fn is_canonical_sleep_duration_symbol(symbol: &PackageSymbolRef) -> bool {
    symbol.symbol_path == "std.time.Duration"
        && matches!(
            &symbol.package,
            PackageRefIr::PackageId { package_id }
                if package_id == "skiff.run/std"
        )
}

/// True for the exact unparameterized `string` builtin.
fn is_string_type(ty: &TypeRefIr) -> bool {
    matches!(
        ty,
        TypeRefIr::Builtin { name, args } if name == "string" && args.is_empty()
    )
}

/// Linker-free admission for the immediate leaf shapes. Anonymous record and
/// array recursion is handled by [`admit_type`] so nested package nominals can
/// be resolved; every other shape is rejected at this single boundary.
fn admit_structural_leaf(
    ty: &TypeRefIr,
    allow_void: bool,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    match ty {
        TypeRefIr::Builtin { name, args }
            if args.is_empty()
                && (matches!(name.as_str(), "integer" | "number" | "bool" | "null")
                    || (allow_void && name == "void")) =>
        {
            Ok(())
        }
        TypeRefIr::Literal { value }
            if immediate_literal(value) || matches!(value, LiteralIr::String { .. }) =>
        {
            Ok(())
        }
        TypeRefIr::TypeParam { .. } | TypeRefIr::AppliedNominal { .. } => {
            rejected(Phase1LinkedCapability::Generic, location)
        }
        TypeRefIr::AnyInterface { .. } => rejected(Phase1LinkedCapability::Interface, location),
        TypeRefIr::Function { .. } => rejected(Phase1LinkedCapability::Callback, location),
        TypeRefIr::ServiceSymbol { .. } | TypeRefIr::DbObjectSymbol { .. } => {
            rejected(Phase1LinkedCapability::ServiceTarget, location)
        }
        _ => rejected(Phase1LinkedCapability::ValueShape, location),
    }
}

fn admit_package_symbol(
    linker: &DeploymentLinker<'_>,
    symbol: &PackageSymbolRef,
    admitted_symbols: &mut BTreeSet<String>,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let PackageRefIr::PackageId { package_id } = &symbol.package else {
        return rejected(Phase1LinkedCapability::ValueShape, location);
    };
    let Some(owner) = linker
        .deployment
        .packages()
        .values()
        .find(|package| package.reference().package_id == *package_id)
    else {
        return rejected(Phase1LinkedCapability::ValueShape, location);
    };
    if symbol
        .abi_expectation
        .as_deref()
        .is_some_and(|expected| expected != owner.reference().package_local_abi_identity.as_str())
    {
        return rejected(Phase1LinkedCapability::ValueShape, location);
    }
    let path = format!("{package_id}::{}", symbol.symbol_path);
    if !admitted_symbols.insert(path.clone()) {
        // Recursive or self-referential record reference: fail closed, the
        // compiler admission already rejects it upstream.
        return rejected(Phase1LinkedCapability::ValueShape, location);
    }
    let resolved = owner
        .artifact()
        .package_local_abi
        .implementation_symbols
        .get(&symbol.symbol_path)
        .or_else(|| {
            owner
                .artifact()
                .package_local_abi
                .public_symbols
                .get(&symbol.symbol_path)
        });
    if is_canonical_sleep_duration_symbol(symbol) {
        let Some(PackageLocalAbiSymbol::Type { descriptor, .. }) = resolved else {
            return rejected(Phase1LinkedCapability::ValueShape, location);
        };
        let target = match descriptor {
            TypeDescriptorIr::Alias { target }
            | TypeDescriptorIr::Representation { representation: target } => target,
            _ => return rejected(Phase1LinkedCapability::ValueShape, location),
        };
        admitted_symbols.remove(&path);
        let concrete = normalize_type(linker.deployment, owner, target, &location)?;
        return admit_type(linker, &concrete, false, admitted_symbols, location);
    }
    let admission = match resolved {
        Some(PackageLocalAbiSymbol::Type {
            descriptor,
            type_params,
            is_alias,
            is_interface,
            ..
        }) if type_params.is_empty() && !*is_alias && !*is_interface => {
            admit_package_type_descriptor(linker, owner, descriptor, admitted_symbols)
        }
        _ => Err(BytecodeLinkError::UnsupportedPhase1Capability {
            capability: Phase1LinkedCapability::ValueShape,
            location: location.clone(),
        }),
    };
    admitted_symbols.remove(&path);
    admission
}

fn admit_package_type_descriptor(
    linker: &DeploymentLinker<'_>,
    owner: &HydratedBytecodePackage,
    descriptor: &TypeDescriptorIr,
    admitted_symbols: &mut BTreeSet<String>,
) -> Result<(), BytecodeLinkError> {
    let location = BytecodeLinkLocation::Package {
        package: Box::new(owner.reference().clone()),
    };
    match descriptor {
        TypeDescriptorIr::Record { fields } => {
            for field in fields.values() {
                let concrete = normalize_type(linker.deployment, owner, field, &location)?;
                admit_type(linker, &concrete, false, admitted_symbols, location.clone())?;
            }
            Ok(())
        }
        TypeDescriptorIr::Alias { target } => {
            let concrete = normalize_type(linker.deployment, owner, target, &location)?;
            admit_type(linker, &concrete, false, admitted_symbols, location)
        }
        _ => rejected(Phase1LinkedCapability::ValueShape, location),
    }
}

fn immediate_literal(value: &LiteralIr) -> bool {
    matches!(
        value,
        LiteralIr::Null | LiteralIr::Bool { .. } | LiteralIr::Number { .. }
    )
}

fn rejected<T>(
    capability: Phase1LinkedCapability,
    location: BytecodeLinkLocation,
) -> Result<T, BytecodeLinkError> {
    Err(BytecodeLinkError::UnsupportedPhase1Capability {
        capability,
        location,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        bytecode::opcodes::opcode_table_fingerprint, BytecodeArtifactRef, CallableEffectSummary,
        CallableMayEffects, LiteralIr, Opcode, PackageArtifactRef, PackageBuildId,
        PackageCallableId, PackageLocalAbiIdentity, PendingEffectCategory, ResumeErrorMode,
        TypeRefIr,
    };
    use skiff_runtime_linked_bytecode::{
        ArtifactFunctionKey, BytecodePackageIndex, FunctionIndex, HostEffectAdapterIndex,
        InstructionIndex, LinkedBytecodeAuthorityPins, LinkedBytecodeCandidate,
        LinkedBytecodeCandidateParts, LinkedCallableEffectDeclaration, LinkedFrameLayout,
        LinkedFunction, LinkedFunctionTables, LinkedHostBindingKey, LinkedHostEffectAdapterTarget,
        LinkedInstruction, LinkedInstructionTarget, LinkedNativeCallableSignature,
        LinkedPackageBytecodeProvenance, LinkedProgramPointState, LinkedResolvedOperand,
        LinkedResumeSite, LinkedStackMapCandidate, LinkedValueDropPlan, LinkedValueTransferPlan,
        ResumeSiteIndex, SpecializationKey,
    };

    use super::{
        admit_opcode, admit_pinned_host_call, admit_structural_leaf, admit_transfer_plan,
        is_discriminator_string_constant, is_string_type,
    };
    use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, Phase1LinkedCapability};

    fn location() -> BytecodeLinkLocation {
        BytecodeLinkLocation::Package {
            package: Box::new(PackageArtifactRef {
                package_id: "example.com/admission".to_string(),
                package_version: "0.1.0".to_string(),
                package_build_id: PackageBuildId::new("build"),
                package_local_abi_identity: PackageLocalAbiIdentity::new("abi"),
            }),
        }
    }

    #[test]
    fn capability_admission_admits_record_and_array_opcodes() {
        for opcode in [
            skiff_artifact_model::Opcode::NewRecord,
            skiff_artifact_model::Opcode::GetDenseField,
            skiff_artifact_model::Opcode::SetWritablePath,
            skiff_artifact_model::Opcode::NewArrayBuilder,
            skiff_artifact_model::Opcode::ArrayBuilderPush,
            skiff_artifact_model::Opcode::FreezeArray,
            skiff_artifact_model::Opcode::ArrayGet,
            skiff_artifact_model::Opcode::ArrayLen,
            skiff_artifact_model::Opcode::ArrayPushOwned,
            skiff_artifact_model::Opcode::Drop,
            skiff_artifact_model::Opcode::Throw,
            skiff_artifact_model::Opcode::Rethrow,
        ] {
            assert!(admit_opcode(opcode, location()).is_ok(), "{opcode:?}");
        }
    }

    #[test]
    fn capability_admission_keeps_timeout_regions_and_trap_fail_closed() {
        let expectations = [
            (
                skiff_artifact_model::Opcode::NewMapBuilder,
                Phase1LinkedCapability::Aggregate,
            ),
            (
                skiff_artifact_model::Opcode::MapPutOwned,
                Phase1LinkedCapability::Aggregate,
            ),
            (
                skiff_artifact_model::Opcode::RepresentationWrap,
                Phase1LinkedCapability::ValueShape,
            ),
            (
                skiff_artifact_model::Opcode::TailCallLocal,
                Phase1LinkedCapability::TailCall,
            ),
            (
                skiff_artifact_model::Opcode::EnterRegion,
                Phase1LinkedCapability::Exception,
            ),
            (
                skiff_artifact_model::Opcode::LeaveRegion,
                Phase1LinkedCapability::Exception,
            ),
            (
                skiff_artifact_model::Opcode::Trap,
                Phase1LinkedCapability::Exception,
            ),
        ];
        for (opcode, expected) in expectations {
            assert!(matches!(
                admit_opcode(opcode, location()),
                Err(BytecodeLinkError::UnsupportedPhase1Capability {
                    capability,
                    ..
                }) if capability == expected
            ));
        }
    }

    #[test]
    fn capability_admission_admits_immediate_scalar_leaves() {
        for (ty, allow_void) in [
            (TypeRefIr::builtin("number"), false),
            (TypeRefIr::builtin("integer"), false),
            (TypeRefIr::builtin("bool"), false),
            (TypeRefIr::builtin("null"), false),
            (TypeRefIr::builtin("void"), true),
            (
                TypeRefIr::Literal {
                    value: LiteralIr::Bool { value: true },
                },
                false,
            ),
        ] {
            assert!(
                admit_structural_leaf(&ty, allow_void, location()).is_ok(),
                "{ty:?}"
            );
        }
    }

    #[test]
    fn capability_admission_keeps_unsupported_value_shapes_fail_closed() {
        let cases = [
            (
                TypeRefIr::builtin("string"),
                Phase1LinkedCapability::ValueShape,
            ),
            (
                TypeRefIr::Builtin {
                    name: "Map".to_string(),
                    args: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
                },
                Phase1LinkedCapability::ValueShape,
            ),
            (
                TypeRefIr::Nullable {
                    inner: Box::new(TypeRefIr::builtin("number")),
                },
                Phase1LinkedCapability::ValueShape,
            ),
        ];
        for (ty, expected) in cases {
            assert!(matches!(
                admit_structural_leaf(&ty, false, location()),
                Err(BytecodeLinkError::UnsupportedPhase1Capability {
                    capability,
                    ..
                }) if capability == expected
            ));
        }
    }

    #[test]
    fn capability_admission_admits_exact_snapshot_plans_only() {
        assert!(admit_transfer_plan(
            &LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::Trivial,
            },
            location(),
        )
        .is_ok());
        assert!(admit_transfer_plan(
            &LinkedValueTransferPlan::SnapshotShare {
                drop: LinkedValueDropPlan::SnapshotRelease,
            },
            location(),
        )
        .is_ok());
        assert!(matches!(
            admit_transfer_plan(
                &LinkedValueTransferPlan::MoveOnly {
                    drop: LinkedValueDropPlan::SnapshotRelease,
                },
                location(),
            ),
            Err(BytecodeLinkError::UnsupportedPhase1Capability {
                capability: Phase1LinkedCapability::Resource,
                ..
            })
        ));
    }

    #[test]
    fn discriminator_string_constant_is_the_only_admitted_string_literal() {
        let string = TypeRefIr::builtin("string");
        assert!(is_string_type(&string));
        assert!(!is_string_type(&TypeRefIr::Builtin {
            name: "string".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        }));

        let literal =
            skiff_runtime_linked_bytecode::LinkedFrozenConstantValue::Literal(LiteralIr::String {
                value: "ok".to_string(),
            });
        assert!(is_discriminator_string_constant(&literal, Some(&string)));
        assert!(!is_discriminator_string_constant(&literal, None));
        assert!(!is_discriminator_string_constant(
            &literal,
            Some(&TypeRefIr::builtin("number")),
        ));
        assert!(!is_discriminator_string_constant(
            &skiff_runtime_linked_bytecode::LinkedFrozenConstantValue::Literal(LiteralIr::Number {
                value: serde_json::Number::from(1),
            }),
            Some(&string),
        ));
    }

    #[test]
    fn generic_string_values_stay_fail_closed_at_the_type_leaf() {
        assert!(matches!(
            admit_structural_leaf(&TypeRefIr::builtin("string"), false, location()),
            Err(BytecodeLinkError::UnsupportedPhase1Capability {
                capability: Phase1LinkedCapability::ValueShape,
                ..
            })
        ));
    }

    #[test]
    fn discriminator_envelope_builtins_require_the_exact_arity() {
        let catch_result = TypeRefIr::Builtin {
            name: "CatchResult".to_string(),
            args: vec![TypeRefIr::builtin("number")],
        };
        assert!(matches!(
            admit_structural_leaf(&catch_result, false, location()),
            Err(BytecodeLinkError::UnsupportedPhase1Capability {
                capability: Phase1LinkedCapability::ValueShape,
                ..
            })
        ));
    }

    fn native_signature() -> LinkedNativeCallableSignature {
        LinkedNativeCallableSignature::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            CallableMayEffects {
                escapes_caller_value: false,
                requires_same_heap_identity: false,
                invokes_unknown_target: false,
                may_pending: true,
                pending_effect_categories: vec![PendingEffectCategory::NativeCall],
                inout_path_effects: Vec::new(),
            },
        )
        .expect("empty native signature is valid")
    }

    fn host_candidate(binding_key: &str, with_resume: bool) -> LinkedBytecodeCandidate {
        let provenance = LinkedPackageBytecodeProvenance::new(
            BytecodePackageIndex::new(0),
            PackageBuildId::new("build"),
            BytecodeArtifactRef::new("test-identity"),
            "test-identity",
            "magic",
            "schema-v7",
            "isa-v1",
            opcode_table_fingerprint(),
            LinkedBytecodeAuthorityPins::new(
                skiff_artifact_model::native_value_lifecycle_registry_identity().clone(),
                skiff_artifact_model::value_lifecycle_policy_identity().clone(),
                skiff_artifact_model::host_effect_registry_identity().clone(),
                skiff_artifact_model::intrinsic_registry_identity().clone(),
                skiff_artifact_model::current_platform_error_projection_registry_ref().clone(),
            )
            .expect("authority pins are valid"),
        )
        .expect("provenance is valid");
        let adapter = LinkedHostEffectAdapterTarget::new(
            HostEffectAdapterIndex::new(0),
            "std",
            "time.sleep",
            LinkedHostBindingKey::parse(binding_key).expect("binding key parses"),
            BTreeMap::new(),
            native_signature(),
        )
        .expect("host adapter is valid");
        let instructions = vec![
            host_instruction(),
            LinkedInstruction::new(Opcode::Return, Box::new([]), Box::new([]), 1)
                .expect("return instruction is valid"),
        ];
        let states = (0..instructions.len())
            .map(|index| {
                LinkedProgramPointState::new(
                    InstructionIndex::new(index as u32),
                    Box::new([]),
                    Box::new([]),
                    Box::new([]),
                    Box::new([]),
                )
            })
            .collect::<Vec<_>>();
        let stack_map = LinkedStackMapCandidate::try_new(
            states.into_boxed_slice(),
            instructions.len(),
            0,
            0,
        )
        .expect("stack map is valid");
        let function = LinkedFunction::new(
            FunctionIndex::new(0),
            SpecializationKey::new(
                PackageBuildId::new("build"),
                ArtifactFunctionKey::parse("fixture::host").expect("function key parses"),
                PackageCallableId::new("host"),
                Box::new([]),
                None,
            ),
            instructions.into_boxed_slice(),
            LinkedFrameLayout::new(
                Box::new([]),
                Box::new([]),
                Box::new([]),
                Box::new([]),
                Box::new([]),
                Box::new([]),
                None,
            )
            .expect("frame is valid"),
            0,
            LinkedCallableEffectDeclaration::new(
                PackageCallableId::new("host"),
                CallableEffectSummary::analysis_pending(),
            ),
            LinkedFunctionTables::new(
                Box::new([]),
                Box::new([]),
                Box::new([]),
                Box::new([]),
                Box::new([]),
                Box::new([]),
            ),
            stack_map,
        );
        let resume_sites = with_resume
            .then(|| {
                LinkedResumeSite::new(
                    ResumeSiteIndex::new(0),
                    FunctionIndex::new(0),
                    InstructionIndex::new(0),
                    InstructionIndex::new(1),
                    None,
                    0,
                    Box::new([]),
                    Box::new([]),
                    ResumeErrorMode::RaiseAtSite,
                )
                .expect("resume site is valid")
            })
            .into_iter()
            .collect::<Vec<_>>();
        LinkedBytecodeCandidate::try_from_parts(LinkedBytecodeCandidateParts {
            packages: vec![provenance],
            functions: vec![function],
            operation_entries: Vec::new(),
            gateway_entries: Vec::new(),
            exact_local_targets: Vec::new(),
            service_operations: Vec::new(),
            actor_creates: Vec::new(),
            actor_methods: Vec::new(),
            interface_tables: Vec::new(),
            synthetic_callbacks: Vec::new(),
            callback_capture_layouts: Vec::new(),
            host_effect_adapters: vec![adapter],
            intrinsics: Vec::new(),
            types: Vec::new(),
            shapes: Vec::new(),
            constants: Vec::new(),
            constant_roots: Vec::new(),
            frozen_constant_nodes: Vec::new(),
            resume_sites,
            writable_paths: Vec::new(),
        })
        .expect("candidate parts are valid")
    }

    fn host_instruction() -> LinkedInstruction {
        let resolved = vec![
            LinkedResolvedOperand::new(
                0,
                LinkedInstructionTarget::HostEffectAdapter(HostEffectAdapterIndex::new(0)),
            ),
            LinkedResolvedOperand::new(
                3,
                LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(0)),
            ),
        ];
        LinkedInstruction::new(
            Opcode::InvokeHost,
            Box::new([0, 0, 0, 0]),
            resolved.into_boxed_slice(),
            0,
        )
        .expect("host instruction is valid")
    }

    #[test]
    fn pinned_sleep_host_call_with_resume_is_admitted() {
        let candidate = host_candidate("std.time.sleep", true);
        let instruction = host_instruction();
        admit_pinned_host_call(&candidate, &instruction, location())
            .expect("the pinned sleep call with an exact resume descriptor is admitted");
    }

    #[test]
    fn non_sleep_host_binding_fails_closed_at_the_pinned_gate() {
        let candidate = host_candidate("std.config.require", true);
        let instruction = host_instruction();
        assert!(matches!(
            admit_pinned_host_call(&candidate, &instruction, location()),
            Err(BytecodeLinkError::UnsupportedPhase1Capability {
                capability: Phase1LinkedCapability::HostTarget,
                ..
            })
        ));
    }

    #[test]
    fn invoke_host_opcode_is_delegated_to_the_pinned_call_gate() {
        assert!(admit_opcode(Opcode::InvokeHost, location()).is_ok());
    }
}
