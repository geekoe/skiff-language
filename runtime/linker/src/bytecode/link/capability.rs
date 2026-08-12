use skiff_artifact_model::{
    CallableEffectSummary, GatewayProtocolSurface, LiteralIr, Opcode, ParamModeIr, TypeRefIr,
};
use skiff_runtime_linked_bytecode::{
    LinkedBytecodeCandidate, LinkedCallableSignature, LinkedConstantReference,
    LinkedFrozenConstantValue, LinkedGatewayCallableRole, LinkedInstructionTarget, LinkedSlotState,
    LinkedValueDropPlan, LinkedValueTransferPlan, TypeIndex,
};

use crate::bytecode::{BytecodeLinkError, BytecodeLinkLocation, Phase1LinkedCapability};

use super::DeploymentLinker;

impl DeploymentLinker<'_> {
    /// Closed Phase 1 allowlist over the fully linked reachable publication
    /// closure. This runs after relocation and table resolution but before the
    /// raw candidate can leave the production linker.
    pub(super) fn admit_phase_1_capabilities(
        &self,
        candidate: &LinkedBytecodeCandidate,
    ) -> Result<(), BytecodeLinkError> {
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
                for resolved in instruction.resolved_operands() {
                    admit_resolved_target(resolved.target(), location.clone())?;
                }
                admit_opcode(instruction.opcode(), location)?;
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
            if !frame.writable_local_slots().is_empty() {
                return rejected(Phase1LinkedCapability::Writable, function_location);
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
                admit_trivial_plan(plan, function_location.clone())?;
            }
            for ty in frame.slot_types().iter().chain(frame.result_types()) {
                admit_type_index(candidate, *ty, false, function_location.clone())?;
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
                    admit_stack_value(candidate, value.ty(), value.plan(), location.clone())?;
                }
                for slot in state.slots_before() {
                    if let LinkedSlotState::Live(value) = slot {
                        admit_stack_value(candidate, value.ty(), value.plan(), location.clone())?;
                    }
                }
            }

            admit_effect_summary(function.declarative_effect_summary(), function_location)?;
        }

        // Entry signatures are untrusted linked facts as well, but direct
        // instructions above retain precedence for their exact PC-owned
        // capability diagnostic.
        for entry in candidate.operation_entries() {
            admit_signature(candidate, entry.signature(), self.deployment_location())?;
        }
        for entry in candidate.gateway_entries() {
            for callable in entry.callables() {
                admit_signature(candidate, callable.signature(), self.deployment_location())?;
            }
        }
        self.admit_global_tables(candidate)
    }

    fn admit_public_roots(
        &self,
        candidate: &LinkedBytecodeCandidate,
    ) -> Result<(), BytecodeLinkError> {
        let location = self.deployment_location();
        for entry in candidate.gateway_entries() {
            if !matches!(
                &entry.protocol_surface().protocol,
                GatewayProtocolSurface::Http(_)
            ) {
                return rejected(Phase1LinkedCapability::WebSocket, location);
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
    ) -> Result<(), BytecodeLinkError> {
        let location = self.deployment_location();
        for (present, capability) in [
            (
                !candidate.service_operations().is_empty(),
                Phase1LinkedCapability::ServiceTarget,
            ),
            (
                !candidate.actor_creates().is_empty(),
                Phase1LinkedCapability::Actor,
            ),
            (
                !candidate.actor_methods().is_empty(),
                Phase1LinkedCapability::Actor,
            ),
            (
                !candidate.interface_tables().is_empty(),
                Phase1LinkedCapability::Interface,
            ),
            (
                !candidate.synthetic_callbacks().is_empty(),
                Phase1LinkedCapability::Callback,
            ),
            (
                !candidate.callback_capture_layouts().is_empty(),
                Phase1LinkedCapability::Callback,
            ),
            (
                !candidate.host_effect_adapters().is_empty(),
                Phase1LinkedCapability::HostTarget,
            ),
            (
                !candidate.intrinsics().is_empty(),
                Phase1LinkedCapability::IntrinsicTarget,
            ),
            (
                !candidate.resume_sites().is_empty(),
                Phase1LinkedCapability::Stream,
            ),
            (
                !candidate.writable_paths().is_empty(),
                Phase1LinkedCapability::InOut,
            ),
            (
                !candidate.shapes().is_empty(),
                Phase1LinkedCapability::Aggregate,
            ),
            (
                !candidate.constant_roots().is_empty(),
                Phase1LinkedCapability::Constant,
            ),
        ] {
            if present {
                return rejected(capability, location);
            }
        }

        for ty in candidate.types() {
            if ty.container_layout().is_some() {
                return rejected(Phase1LinkedCapability::ValueShape, location);
            }
            admit_type(ty.type_ref(), true, location.clone())?;
        }
        for constant in candidate.constants() {
            if matches!(
                constant.reference(),
                LinkedConstantReference::PackageSymbol { .. }
            ) {
                return rejected(Phase1LinkedCapability::Constant, location);
            }
            admit_type_index(candidate, constant.ty(), false, location.clone())?;
            admit_trivial_plan(constant.plan(), location.clone())?;
        }
        for node in candidate.frozen_constant_nodes() {
            let LinkedFrozenConstantValue::Literal(value) = node.value() else {
                return rejected(Phase1LinkedCapability::Aggregate, location);
            };
            if !immediate_literal(value) {
                return rejected(Phase1LinkedCapability::ValueShape, location);
            }
        }
        Ok(())
    }
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
        Opcode::CallLocalInOut | Opcode::SetWritablePath => Phase1LinkedCapability::InOut,
        Opcode::Throw
        | Opcode::Rethrow
        | Opcode::EnterRegion
        | Opcode::LeaveRegion
        | Opcode::Trap => Phase1LinkedCapability::Exception,
        Opcode::SwitchTag
        | Opcode::NewRecord
        | Opcode::GetDenseField
        | Opcode::NewArrayBuilder
        | Opcode::ArrayBuilderPush
        | Opcode::FreezeArray
        | Opcode::NewMapBuilder
        | Opcode::MapBuilderPut
        | Opcode::FreezeMap
        | Opcode::ArrayGet
        | Opcode::MapGet
        | Opcode::ArrayLen
        | Opcode::MapLen
        | Opcode::MapEntryAt
        | Opcode::ArrayPushOwned
        | Opcode::MapPutOwned => Phase1LinkedCapability::Aggregate,
        Opcode::Drop => Phase1LinkedCapability::Resource,
        Opcode::RepresentationWrap => Phase1LinkedCapability::ValueShape,
        unsupported => Phase1LinkedCapability::UnsupportedOpcode(unsupported),
    };
    rejected(capability, location)
}

fn admit_resolved_target(
    target: LinkedInstructionTarget,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let capability = match target {
        LinkedInstructionTarget::FrameSlot(_)
        | LinkedInstructionTarget::Branch(_)
        | LinkedInstructionTarget::Function(_)
        | LinkedInstructionTarget::Constant(_) => return Ok(()),
        LinkedInstructionTarget::SwitchTable(_) | LinkedInstructionTarget::Shape(_) => {
            Phase1LinkedCapability::Aggregate
        }
        LinkedInstructionTarget::ActiveRegion(_) => Phase1LinkedCapability::Exception,
        LinkedInstructionTarget::CallLoanLayout(_) | LinkedInstructionTarget::WritablePath(_) => {
            Phase1LinkedCapability::InOut
        }
        LinkedInstructionTarget::ServiceOperation(_) => Phase1LinkedCapability::ServiceTarget,
        LinkedInstructionTarget::ActorMethod(_) => Phase1LinkedCapability::Actor,
        LinkedInstructionTarget::InterfaceTable(_) => Phase1LinkedCapability::Interface,
        LinkedInstructionTarget::SyntheticCallback(_)
        | LinkedInstructionTarget::CallbackCaptureLayout(_) => Phase1LinkedCapability::Callback,
        LinkedInstructionTarget::HostEffectAdapter(_) => Phase1LinkedCapability::HostTarget,
        LinkedInstructionTarget::Intrinsic(_) => Phase1LinkedCapability::IntrinsicTarget,
        LinkedInstructionTarget::Type(_) => Phase1LinkedCapability::ValueShape,
        LinkedInstructionTarget::ResumeSite(_) => Phase1LinkedCapability::PendingEffect(
            skiff_artifact_model::PendingEffectCategory::Unknown,
        ),
    };
    rejected(capability, location)
}

fn admit_trivial_plan(
    plan: &LinkedValueTransferPlan,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    if matches!(
        plan,
        LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial
        }
    ) {
        Ok(())
    } else {
        rejected(Phase1LinkedCapability::Resource, location)
    }
}

fn admit_signature(
    candidate: &LinkedBytecodeCandidate,
    signature: &LinkedCallableSignature,
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
        admit_type_index(candidate, *ty, false, location.clone())?;
    }
    for plan in signature
        .parameter_plans()
        .iter()
        .chain(signature.result_plans())
    {
        admit_trivial_plan(plan, location.clone())?;
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
    candidate: &LinkedBytecodeCandidate,
    ty: TypeIndex,
    plan: &LinkedValueTransferPlan,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    admit_type_index(candidate, ty, false, location.clone())?;
    admit_trivial_plan(plan, location)
}

fn admit_type_index(
    candidate: &LinkedBytecodeCandidate,
    index: TypeIndex,
    allow_void: bool,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let Some(ty) = candidate.types().get(index.get() as usize) else {
        return rejected(Phase1LinkedCapability::ValueShape, location);
    };
    admit_type(ty.type_ref(), allow_void, location)
}

fn admit_type(
    ty: &TypeRefIr,
    allow_void: bool,
    location: BytecodeLinkLocation,
) -> Result<(), BytecodeLinkError> {
    let capability = match ty {
        TypeRefIr::Builtin { name, args }
            if args.is_empty()
                && (matches!(name.as_str(), "integer" | "number" | "bool" | "null")
                    || (allow_void && name == "void")) =>
        {
            return Ok(());
        }
        TypeRefIr::Literal { value } if immediate_literal(value) => return Ok(()),
        TypeRefIr::TypeParam { .. } | TypeRefIr::AppliedNominal { .. } => {
            Phase1LinkedCapability::Generic
        }
        TypeRefIr::AnyInterface { .. } => Phase1LinkedCapability::Interface,
        TypeRefIr::Function { .. } => Phase1LinkedCapability::Callback,
        TypeRefIr::ServiceSymbol { .. } | TypeRefIr::DbObjectSymbol { .. } => {
            Phase1LinkedCapability::ServiceTarget
        }
        _ => Phase1LinkedCapability::ValueShape,
    };
    rejected(capability, location)
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
