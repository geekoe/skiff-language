use std::collections::BTreeSet;

use skiff_artifact_model::{
    BoundaryCallbackContract, BoundaryStreamContract, CallableEffectSummary, GatewayDispatchMode,
    GatewayProtocolSurface, LiteralIr, Opcode, PackageLocalAbiSymbol, PackageRefIr,
    PackageSymbolRef, ParamModeIr, TypeDescriptorIr, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    LinkedBytecodeCandidate, LinkedCallableSignature, LinkedConstantReference,
    LinkedFrozenConstantValue, LinkedGatewayCallableRole, LinkedInstructionTarget, LinkedSlotState,
    LinkedValueDropPlan, LinkedValueTransferPlan, TypeIndex,
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
            if !function.exception_regions().is_empty() || !function.active_regions().is_empty() {
                return rejected(Phase1LinkedCapability::Exception, function_location);
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
                if !state.writable_loans().is_empty() {
                    return rejected(Phase1LinkedCapability::InOut, location);
                }
                for value in state.stack_before() {
                    admit_stack_value(
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
        self.admit_global_tables(candidate, &mut admitted_symbols)
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
    ) -> Result<(), BytecodeLinkError> {
        let location = self.deployment_location();
        for constant in candidate.constants() {
            admit_constant(self, candidate, constant, admitted_symbols)?;
        }
        for node in candidate.frozen_constant_nodes() {
            let location = frozen_node_location(self, node);
            let LinkedFrozenConstantValue::Literal(value) = node.value() else {
                return rejected(Phase1LinkedCapability::Aggregate, location);
            };
            if !immediate_literal(value) {
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
) -> Result<(), BytecodeLinkError> {
    let location = constant_location(linker, candidate, constant);
    if matches!(
        constant.reference(),
        LinkedConstantReference::PackageSymbol { .. }
    ) {
        return rejected(Phase1LinkedCapability::Constant, location);
    }
    admit_type_index(
        linker,
        candidate,
        constant.ty(),
        false,
        admitted_symbols,
        location.clone(),
    )?;
    admit_transfer_plan(constant.plan(), location)
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
    ) {
        return Ok(());
    }
    let capability = match opcode {
        Opcode::InvokeHost => Phase1LinkedCapability::HostTarget,
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
        Opcode::Throw
        | Opcode::Rethrow
        | Opcode::EnterRegion
        | Opcode::LeaveRegion
        | Opcode::Trap => Phase1LinkedCapability::Exception,
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
            return admit_type_index(
                linker,
                candidate,
                index,
                false,
                admitted_symbols,
                location,
            );
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
    admit_type_index(
        linker,
        candidate,
        constant.ty(),
        false,
        admitted_symbols,
        location.clone(),
    )?;
    admit_transfer_plan(constant.plan(), location.clone())?;
    match node.value() {
        LinkedFrozenConstantValue::Literal(value) if immediate_literal(value) => Ok(()),
        LinkedFrozenConstantValue::Literal(_) => {
            rejected(Phase1LinkedCapability::ValueShape, location)
        }
        _ => rejected(Phase1LinkedCapability::Aggregate, location),
    }
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
    if let Some(category) = effects.pending_effect_categories.first().copied() {
        return rejected(Phase1LinkedCapability::PendingEffect(category), location);
    }
    if effects.may_pending {
        return rejected(
            Phase1LinkedCapability::PendingEffect(
                skiff_artifact_model::PendingEffectCategory::Unknown,
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
                admit_type(
                    linker,
                    field,
                    false,
                    admitted_symbols,
                    location.clone(),
                )?;
            }
            return Ok(());
        }
        TypeRefIr::Builtin { name, args } if name == "Array" && args.len() == 1 => {
            admit_type(
                linker,
                &args[0],
                false,
                admitted_symbols,
                location.clone(),
            )?;
            return Ok(());
        }
        TypeRefIr::PackageSymbol { symbol } => {
            return admit_package_symbol(linker, symbol, admitted_symbols, location);
        }
        other => admit_structural_leaf(other, allow_void, location),
    }
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
        TypeRefIr::Literal { value } if immediate_literal(value) => Ok(()),
        TypeRefIr::TypeParam { .. } | TypeRefIr::AppliedNominal { .. } => {
            rejected(Phase1LinkedCapability::Generic, location)
        }
        TypeRefIr::AnyInterface { .. } => {
            rejected(Phase1LinkedCapability::Interface, location)
        }
        TypeRefIr::Function { .. } => {
            rejected(Phase1LinkedCapability::Callback, location)
        }
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
    if symbol.abi_expectation.as_deref().is_some_and(|expected| {
        expected != owner.reference().package_local_abi_identity.as_str()
    }) {
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
                admit_type(
                    linker,
                    &concrete,
                    false,
                    admitted_symbols,
                    location.clone(),
                )?;
            }
            Ok(())
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
    use skiff_artifact_model::{
        LiteralIr, PackageArtifactRef, PackageBuildId, PackageLocalAbiIdentity, TypeRefIr,
    };
    use skiff_runtime_linked_bytecode::{LinkedValueDropPlan, LinkedValueTransferPlan};

    use super::{admit_opcode, admit_structural_leaf, admit_transfer_plan};
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
        ] {
            assert!(admit_opcode(opcode, location()).is_ok(), "{opcode:?}");
        }
    }

    #[test]
    fn capability_admission_keeps_other_aggregate_lanes_fail_closed() {
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
                skiff_artifact_model::Opcode::Throw,
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
            assert!(admit_structural_leaf(&ty, allow_void, location()).is_ok(), "{ty:?}");
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
}
