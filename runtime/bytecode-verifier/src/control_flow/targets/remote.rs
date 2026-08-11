use skiff_artifact_model::{
    contract_for_opcode, CallableEffectSummary, Opcode, OperandRole, PackageCallableId,
    PendingContract,
};
use skiff_runtime_linked_bytecode::{
    FunctionIndex, InstructionIndex, LinkedBytecodeCandidate, LinkedCallableSignature,
    LinkedInstruction, LinkedInstructionTarget, LinkedInterfaceTableKind,
    LinkedNativeCallableSignature,
};

use super::{
    call_plan,
    facts::{
        ExactCallPlan, ExactEffectFacts, ExactTargetCoordinate, PendingPlan, ResumeCoordinate,
    },
    ControlFlowFacts,
};
use crate::{
    concrete_values::ConcreteValueFacts, VerificationError, VerificationLocation,
    VerificationObligation,
};

struct RemoteTarget {
    coordinate: ExactTargetCoordinate,
    signature: LinkedCallableSignature,
    effect: ExactEffectFacts,
    pending: PendingPlan,
    resume: Option<ResumeCoordinate>,
}

pub(super) fn prove_remote_targets_and_call_plans(
    candidate: &LinkedBytecodeCandidate,
    concrete_values: &ConcreteValueFacts,
    control_flow: &ControlFlowFacts,
    dense: &mut [Vec<Option<ExactCallPlan>>],
) -> Result<(), VerificationError> {
    for function in candidate.functions() {
        for (ordinal, instruction) in function.instructions().iter().enumerate() {
            let site = u32::try_from(ordinal)
                .map(InstructionIndex::new)
                .map_err(|_| {
                    violation(
                        VerificationLocation::Image,
                        "instruction ordinal exceeds u32",
                    )
                })?;
            let location = VerificationLocation::Instruction {
                function: function.index(),
                instruction: site,
            };
            if !control_flow.proves_reachable_instruction(function.index(), site) {
                continue;
            }
            let Some(target) =
                remote_target(candidate, function.index(), site, instruction, location)?
            else {
                continue;
            };
            let plan = call_plan::prove_remote_call_plan(
                candidate,
                concrete_values,
                function.index(),
                site,
                target.coordinate,
                target.effect,
                &target.signature,
                target.pending,
                target.resume,
            )?;
            let slot = dense
                .get_mut(function.index().get() as usize)
                .and_then(|row| row.get_mut(site.get() as usize))
                .ok_or_else(|| violation(location, "remote call coordinate is out of bounds"))?;
            if slot.replace(plan).is_some() {
                return Err(violation(
                    location,
                    "remote call site already has an exact call plan",
                ));
            }
        }
    }
    Ok(())
}

fn remote_target(
    candidate: &LinkedBytecodeCandidate,
    caller: FunctionIndex,
    site: InstructionIndex,
    instruction: &LinkedInstruction,
    location: VerificationLocation,
) -> Result<Option<RemoteTarget>, VerificationError> {
    let contract = contract_for_opcode(instruction.opcode());
    let target = match instruction.opcode() {
        Opcode::CallService => Some(service_target(
            candidate,
            caller,
            site,
            instruction,
            contract,
            location,
        )?),
        Opcode::CallActor => Some(actor_target(
            candidate,
            caller,
            site,
            instruction,
            contract,
            location,
        )?),
        Opcode::CallInterface | Opcode::InvokeCallback => Some(interface_target(
            candidate,
            caller,
            site,
            instruction,
            contract,
            location,
        )?),
        Opcode::InvokeHost => Some(host_target(
            candidate,
            caller,
            site,
            instruction,
            contract,
            location,
        )?),
        Opcode::InvokeIntrinsic => Some(intrinsic_target(
            candidate,
            instruction,
            contract,
            location,
        )?),
        _ => None,
    };
    Ok(target)
}

#[allow(clippy::too_many_arguments)]
fn service_target(
    candidate: &LinkedBytecodeCandidate,
    caller: FunctionIndex,
    site: InstructionIndex,
    instruction: &LinkedInstruction,
    contract: &skiff_artifact_model::OpcodeContract,
    location: VerificationLocation,
) -> Result<RemoteTarget, VerificationError> {
    let index = resolve_index(instruction, contract, OperandRole::ServiceTarget, location)?;
    let row = candidate
        .service_operations()
        .get(index as usize)
        .filter(|row| row.index().get() == index)
        .ok_or_else(|| violation(location, "service operation target is out of bounds"))?;
    let summary = analyzed_summary(row.signature().effect_summary(), location)?;
    let canonical = PackageCallableId::new(format!(
        "service:{}:{}:{}",
        row.service_requirement_key().caller_package_build_id,
        row.service_requirement_key().service_requirement_slot,
        row.contract_operation_id()
    ));
    Ok(RemoteTarget {
        coordinate: ExactTargetCoordinate::ServiceOperation(row.index()),
        signature: row.signature().clone(),
        effect: ExactEffectFacts::new(canonical, summary),
        pending: pending_plan(contract, location)?,
        resume: resume_coordinate(candidate, caller, site, instruction, contract, location)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn actor_target(
    candidate: &LinkedBytecodeCandidate,
    caller: FunctionIndex,
    site: InstructionIndex,
    instruction: &LinkedInstruction,
    contract: &skiff_artifact_model::OpcodeContract,
    location: VerificationLocation,
) -> Result<RemoteTarget, VerificationError> {
    let index = resolve_index(instruction, contract, OperandRole::ActorTarget, location)?;
    let row = candidate
        .actor_methods()
        .get(index as usize)
        .filter(|row| row.index().get() == index)
        .ok_or_else(|| violation(location, "actor method target is out of bounds"))?;
    let summary = analyzed_summary(row.signature().effect_summary(), location)?;
    let canonical = PackageCallableId::new(format!(
        "actor:{}:{}:{}:{}",
        row.owner_package_build_id(),
        row.actor().symbol_path(),
        row.actor_abi_identity().as_str(),
        row.method_identity().as_str()
    ));
    Ok(RemoteTarget {
        coordinate: ExactTargetCoordinate::ActorMethod(row.index()),
        signature: row.signature().clone(),
        effect: ExactEffectFacts::new(canonical, summary),
        pending: pending_plan(contract, location)?,
        resume: resume_coordinate(candidate, caller, site, instruction, contract, location)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn interface_target(
    candidate: &LinkedBytecodeCandidate,
    caller: FunctionIndex,
    site: InstructionIndex,
    instruction: &LinkedInstruction,
    contract: &skiff_artifact_model::OpcodeContract,
    location: VerificationLocation,
) -> Result<RemoteTarget, VerificationError> {
    let index = resolve_index(
        instruction,
        contract,
        OperandRole::InterfaceTarget,
        location,
    )?;
    let table = candidate
        .interface_tables()
        .get(index as usize)
        .filter(|row| row.index().get() == index)
        .ok_or_else(|| violation(location, "interface target is out of bounds"))?;
    let method_ordinal = contract
        .operand_word(OperandRole::MethodOrdinal, instruction.operands())
        .ok_or_else(|| violation(location, "interface method ordinal is absent"))?;
    let signature = match table.kind() {
        LinkedInterfaceTableKind::Requirement(requirement) => requirement
            .methods()
            .get(method_ordinal as usize)
            .map(|method| method.signature())
            .ok_or_else(|| violation(location, "interface method ordinal is out of bounds"))?,
        LinkedInterfaceTableKind::Callback(callback) => callback
            .methods()
            .get(method_ordinal as usize)
            .map(|method| method.signature())
            .ok_or_else(|| violation(location, "interface callback ordinal is out of bounds"))?,
        LinkedInterfaceTableKind::Local(_) | LinkedInterfaceTableKind::Remote(_) => {
            return Err(unavailable(location));
        }
    };
    let summary = analyzed_summary(signature.effect_summary(), location)?;
    let canonical = PackageCallableId::new(format!(
        "interface:{}:{}",
        table.interface().artifact().interface_abi_id,
        method_ordinal
    ));
    Ok(RemoteTarget {
        coordinate: ExactTargetCoordinate::InterfaceMethod {
            table: table.index(),
            method_ordinal,
        },
        signature: signature.clone(),
        effect: ExactEffectFacts::new(canonical, summary),
        pending: pending_plan(contract, location)?,
        resume: resume_coordinate(candidate, caller, site, instruction, contract, location)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn host_target(
    candidate: &LinkedBytecodeCandidate,
    caller: FunctionIndex,
    site: InstructionIndex,
    instruction: &LinkedInstruction,
    contract: &skiff_artifact_model::OpcodeContract,
    location: VerificationLocation,
) -> Result<RemoteTarget, VerificationError> {
    let index = resolve_index(instruction, contract, OperandRole::HostTarget, location)?;
    let row = candidate
        .host_effect_adapters()
        .get(index as usize)
        .filter(|row| row.index().get() == index)
        .ok_or_else(|| violation(location, "host effect adapter target is out of bounds"))?;
    let signature = native_signature(row.signature(), location)?;
    let canonical = PackageCallableId::new(format!(
        "host:{}:{}:{}",
        row.namespace(),
        row.symbol(),
        row.binding_key().as_str()
    ));
    let may_pending = row.signature().effects().may_pending();
    Ok(RemoteTarget {
        coordinate: ExactTargetCoordinate::HostEffectAdapter(row.index()),
        signature,
        effect: ExactEffectFacts::new(
            canonical,
            CallableEffectSummary::Analyzed {
                effects: row.signature().effects().clone(),
            },
        ),
        pending: if may_pending {
            pending_plan(contract, location)?
        } else {
            PendingPlan::Never
        },
        resume: if may_pending {
            resume_coordinate(candidate, caller, site, instruction, contract, location)?
        } else {
            None
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn intrinsic_target(
    candidate: &LinkedBytecodeCandidate,
    instruction: &LinkedInstruction,
    contract: &skiff_artifact_model::OpcodeContract,
    location: VerificationLocation,
) -> Result<RemoteTarget, VerificationError> {
    let index = resolve_index(
        instruction,
        contract,
        OperandRole::IntrinsicTarget,
        location,
    )?;
    let row = candidate
        .intrinsics()
        .get(index as usize)
        .filter(|row| row.index().get() == index)
        .ok_or_else(|| violation(location, "intrinsic target is out of bounds"))?;
    let signature = native_signature(row.signature(), location)?;
    let canonical = PackageCallableId::new(format!("intrinsic:{}", row.index().get()));
    Ok(RemoteTarget {
        coordinate: ExactTargetCoordinate::Intrinsic(row.index()),
        signature,
        effect: ExactEffectFacts::new(
            canonical,
            CallableEffectSummary::Analyzed {
                effects: row.signature().effects().clone(),
            },
        ),
        pending: PendingPlan::Never,
        resume: None,
    })
}

fn analyzed_summary(
    summary: &CallableEffectSummary,
    location: VerificationLocation,
) -> Result<CallableEffectSummary, VerificationError> {
    match summary {
        CallableEffectSummary::Analyzed { .. } => Ok(summary.clone()),
        CallableEffectSummary::Unknown { .. } => Err(unavailable(location)),
    }
}

fn native_signature(
    native: &LinkedNativeCallableSignature,
    location: VerificationLocation,
) -> Result<LinkedCallableSignature, VerificationError> {
    LinkedCallableSignature::new(
        native.parameter_types().into(),
        native.parameter_modes().into(),
        native.parameter_plans().into(),
        native.result_types().into(),
        native.result_plans().into(),
        CallableEffectSummary::Analyzed {
            effects: native.effects().clone(),
        },
    )
    .map_err(|_| {
        violation(
            location,
            "native target signature is not internally coherent",
        )
    })
}

fn pending_plan(
    contract: &skiff_artifact_model::OpcodeContract,
    location: VerificationLocation,
) -> Result<PendingPlan, VerificationError> {
    match contract.pending {
        PendingContract::ActualWithResume { mode, .. } => Ok(PendingPlan::ActualWithResume(mode)),
        PendingContract::Never => Ok(PendingPlan::Never),
        _ => Err(unavailable(location)),
    }
}

fn resume_coordinate(
    candidate: &LinkedBytecodeCandidate,
    caller: FunctionIndex,
    site: InstructionIndex,
    instruction: &LinkedInstruction,
    contract: &skiff_artifact_model::OpcodeContract,
    location: VerificationLocation,
) -> Result<Option<ResumeCoordinate>, VerificationError> {
    let PendingContract::ActualWithResume { resume: role, .. } = contract.pending else {
        return Ok(None);
    };
    let descriptor = match resolved_target(instruction, contract, role, location)? {
        LinkedInstructionTarget::ResumeSite(descriptor) => descriptor,
        _ => {
            return Err(violation(
                location,
                "resume role has a non-resume typed target",
            ))
        }
    };
    let row = candidate
        .resume_sites()
        .get(descriptor.get() as usize)
        .filter(|row| row.index() == descriptor)
        .ok_or_else(|| violation(location, "resume descriptor is out of bounds"))?;
    if row.function() != caller || row.site() != site {
        return Err(violation(
            location,
            "resume descriptor is not bound to this exact call site",
        ));
    }
    Ok(Some(ResumeCoordinate::new(
        caller,
        descriptor,
        row.resume(),
    )))
}

fn resolve_index(
    instruction: &LinkedInstruction,
    contract: &skiff_artifact_model::OpcodeContract,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<u32, VerificationError> {
    let ordinal = contract
        .operand_position(role)
        .and_then(|ordinal| u32::try_from(ordinal).ok())
        .ok_or_else(|| violation(location, "call target role is absent"))?;
    let target = instruction
        .resolved_operands()
        .iter()
        .find(|operand| operand.operand_ordinal() == ordinal)
        .map(|operand| operand.target())
        .ok_or_else(|| violation(location, "call target typed operand is absent"))?;
    let index = match target {
        LinkedInstructionTarget::ServiceOperation(index) if role == OperandRole::ServiceTarget => {
            index.get()
        }
        LinkedInstructionTarget::ActorMethod(index) if role == OperandRole::ActorTarget => {
            index.get()
        }
        LinkedInstructionTarget::InterfaceTable(index) if role == OperandRole::InterfaceTarget => {
            index.get()
        }
        LinkedInstructionTarget::HostEffectAdapter(index) if role == OperandRole::HostTarget => {
            index.get()
        }
        LinkedInstructionTarget::Intrinsic(index) if role == OperandRole::IntrinsicTarget => {
            index.get()
        }
        _ => {
            return Err(violation(
                location,
                "call target operand kind differs from the canonical role",
            ));
        }
    };
    Ok(index)
}

fn resolved_target(
    instruction: &LinkedInstruction,
    contract: &skiff_artifact_model::OpcodeContract,
    role: OperandRole,
    location: VerificationLocation,
) -> Result<LinkedInstructionTarget, VerificationError> {
    let ordinal = contract
        .operand_position(role)
        .and_then(|ordinal| u32::try_from(ordinal).ok())
        .ok_or_else(|| violation(location, "resume operand role is absent"))?;
    instruction
        .resolved_operands()
        .iter()
        .find(|operand| operand.operand_ordinal() == ordinal)
        .map(|operand| operand.target())
        .ok_or_else(|| violation(location, "resume typed operand is absent"))
}

const fn unavailable(location: VerificationLocation) -> VerificationError {
    VerificationError::ProofUnavailable {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location,
    }
}

fn violation(location: VerificationLocation, detail: impl Into<String>) -> VerificationError {
    VerificationError::SemanticViolation {
        obligation: VerificationObligation::ExactTargetAndCallPlan,
        location,
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use skiff_artifact_model::InterfaceInstantiationRef;
    use skiff_artifact_model::{
        ActorAbiIdentity, ActorImplementationIdentity, ActorMethodIdentity, CallableMayEffects,
        ContractOperationId, PackageBuildId, PackageCallableId, PendingEffectCategory, PendingMode,
        ResumeErrorMode, ServiceProtocolIdentity, ServiceRequirementKey, ServiceSymbolRef,
    };
    use skiff_runtime_linked_bytecode::{
        ActorMethodIndex, ArtifactFunctionKey, FunctionIndex, HostEffectAdapterIndex,
        InstructionIndex, InterfaceTableIndex, IntrinsicIndex, LinkedActorImplementationRef,
        LinkedActorMethodTarget, LinkedBytecodeCandidate, LinkedCallableEffectDeclaration,
        LinkedCallableSignature, LinkedFrameLayout, LinkedFunction, LinkedFunctionTables,
        LinkedHostBindingKey, LinkedHostEffectAdapterTarget, LinkedInstruction,
        LinkedInstructionTarget, LinkedInterfaceInstantiation, LinkedInterfaceMethodAbiId,
        LinkedInterfaceRequirementMethod, LinkedInterfaceRequirementTable, LinkedInterfaceTable,
        LinkedInterfaceTableKind, LinkedIntrinsicKind, LinkedIntrinsicTarget,
        LinkedNativeCallableSignature, LinkedProgramPointState, LinkedResolvedOperand,
        LinkedServiceOperationTarget, LinkedStackMapCandidate, LinkedStaticIntrinsicTarget,
        ResumeSiteIndex, ServiceOperationIndex, SpecializationKey,
    };

    use super::*;
    use crate::{
        concrete_values::ConcreteValueFacts,
        control_flow::cfg,
        tests::fixtures::{candidate_parts, exact_hydration, generous_limits},
    };

    #[test]
    fn intrinsic_remote_target_mints_a_nopending_call_plan() {
        let (candidate, build) = candidate_with_intrinsic();
        let flow = cfg::prove_control_flow(&candidate, &generous_limits()).unwrap();
        let mut dense = vec![vec![None; candidate.functions()[0].instructions().len()]];
        prove_remote_targets_and_call_plans(
            &candidate,
            &ConcreteValueFacts::empty_for_test(),
            &flow,
            &mut dense,
        )
        .unwrap();
        let plan = dense[0][0].as_ref().unwrap();
        assert_eq!(
            plan.target(),
            ExactTargetCoordinate::Intrinsic(IntrinsicIndex::new(0))
        );
        assert_eq!(plan.pending(), PendingPlan::Never);
        let _ = build;
    }

    #[test]
    fn service_remote_target_mints_a_resume_boundary_plan() {
        let (candidate, build) = candidate_with_service();
        let flow = cfg::prove_control_flow(&candidate, &generous_limits()).unwrap();
        let mut dense = vec![vec![None; candidate.functions()[0].instructions().len()]];
        prove_remote_targets_and_call_plans(
            &candidate,
            &ConcreteValueFacts::empty_for_test(),
            &flow,
            &mut dense,
        )
        .unwrap();
        let plan = dense[0][0].as_ref().unwrap();
        assert_eq!(
            plan.target(),
            ExactTargetCoordinate::ServiceOperation(ServiceOperationIndex::new(0))
        );
        assert_eq!(
            plan.pending(),
            PendingPlan::ActualWithResume(PendingMode::ServiceBoundary)
        );
        assert!(plan.resume().is_some());
        let _ = build;
    }

    #[test]
    fn actor_remote_target_mints_a_boundary_plan() {
        let (candidate, build) = candidate_with_actor();
        let flow = cfg::prove_control_flow(&candidate, &generous_limits()).unwrap();
        let mut dense = vec![vec![None; candidate.functions()[0].instructions().len()]];
        prove_remote_targets_and_call_plans(
            &candidate,
            &ConcreteValueFacts::empty_for_test(),
            &flow,
            &mut dense,
        )
        .unwrap();
        let plan = dense[0][0].as_ref().unwrap();
        assert_eq!(
            plan.target(),
            ExactTargetCoordinate::ActorMethod(ActorMethodIndex::new(0))
        );
        assert_eq!(
            plan.pending(),
            PendingPlan::ActualWithResume(PendingMode::ActorBoundary)
        );
        let _ = build;
    }

    #[test]
    fn interface_remote_target_mints_a_boundary_plan() {
        let (candidate, build) = candidate_with_interface();
        let flow = cfg::prove_control_flow(&candidate, &generous_limits()).unwrap();
        let mut dense = vec![vec![None; candidate.functions()[0].instructions().len()]];
        prove_remote_targets_and_call_plans(
            &candidate,
            &ConcreteValueFacts::empty_for_test(),
            &flow,
            &mut dense,
        )
        .unwrap();
        let plan = dense[0][0].as_ref().unwrap();
        assert_eq!(
            plan.target(),
            ExactTargetCoordinate::InterfaceMethod {
                table: InterfaceTableIndex::new(0),
                method_ordinal: 0,
            }
        );
        assert_eq!(
            plan.pending(),
            PendingPlan::ActualWithResume(PendingMode::InterfaceBoundary)
        );
        let _ = build;
    }

    fn candidate_with_interface() -> (LinkedBytecodeCandidate, PackageBuildId) {
        let hydrated = exact_hydration();
        let mut parts = candidate_parts(&hydrated, None, None);
        let build = hydrated
            .packages()
            .values()
            .next()
            .unwrap()
            .reference()
            .package_build_id
            .clone();
        let mut effects = bottom();
        effects.may_pending = true;
        effects.pending_effect_categories = vec![PendingEffectCategory::InterfaceCall];
        let table = LinkedInterfaceTable::new(
            InterfaceTableIndex::new(0),
            LinkedInterfaceInstantiation::new(
                InterfaceInstantiationRef {
                    interface_abi_id: "interface:ping".to_string(),
                    canonical_type_args: Vec::new(),
                },
                Box::new([]),
            )
            .unwrap(),
            LinkedInterfaceTableKind::Requirement(
                LinkedInterfaceRequirementTable::new(Box::new([
                    LinkedInterfaceRequirementMethod::new(
                        0,
                        LinkedInterfaceMethodAbiId::parse("ping").unwrap(),
                        callable_signature(effects),
                    ),
                ]))
                .unwrap(),
            ),
        );
        parts.interface_tables = vec![table];
        parts.resume_sites = vec![resume_site()];
        parts.functions = vec![linked_function(
            &build,
            vec![
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::CallInterface,
                    Box::new([0, 0, 0, 0, 0]),
                    Box::new([
                        LinkedResolvedOperand::new(
                            0,
                            LinkedInstructionTarget::InterfaceTable(InterfaceTableIndex::new(0)),
                        ),
                        LinkedResolvedOperand::new(
                            4,
                            LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(0)),
                        ),
                    ]),
                    0,
                )
                .unwrap(),
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::Return,
                    Box::new([]),
                    Box::new([]),
                    1,
                )
                .unwrap(),
            ],
        )];
        (
            LinkedBytecodeCandidate::try_from_parts(parts).unwrap(),
            build,
        )
    }

    #[test]
    fn host_remote_target_mints_a_host_effect_boundary_plan() {
        let (candidate, build) = candidate_with_host();
        let flow = cfg::prove_control_flow(&candidate, &generous_limits()).unwrap();
        let mut dense = vec![vec![None; candidate.functions()[0].instructions().len()]];
        prove_remote_targets_and_call_plans(
            &candidate,
            &ConcreteValueFacts::empty_for_test(),
            &flow,
            &mut dense,
        )
        .unwrap();
        let plan = dense[0][0].as_ref().unwrap();
        assert_eq!(
            plan.target(),
            ExactTargetCoordinate::HostEffectAdapter(HostEffectAdapterIndex::new(0))
        );
        assert_eq!(
            plan.pending(),
            PendingPlan::ActualWithResume(PendingMode::HostEffect)
        );
        let _ = build;
    }

    fn candidate_with_host() -> (LinkedBytecodeCandidate, PackageBuildId) {
        let hydrated = exact_hydration();
        let mut parts = candidate_parts(&hydrated, None, None);
        let build = hydrated
            .packages()
            .values()
            .next()
            .unwrap()
            .reference()
            .package_build_id
            .clone();
        let mut effects = bottom();
        effects.may_pending = true;
        effects.pending_effect_categories = vec![PendingEffectCategory::HostEffect];
        let host = LinkedHostEffectAdapterTarget::new(
            HostEffectAdapterIndex::new(0),
            "std",
            "telemetry.emit",
            LinkedHostBindingKey::parse("std.telemetry.emit").unwrap(),
            std::collections::BTreeMap::new(),
            native_signature_effects(effects),
        )
        .unwrap();
        parts.host_effect_adapters = vec![host];
        parts.resume_sites = vec![resume_site()];
        parts.functions = vec![linked_function(
            &build,
            vec![
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::InvokeHost,
                    Box::new([0, 0, 0, 0]),
                    Box::new([
                        LinkedResolvedOperand::new(
                            0,
                            LinkedInstructionTarget::HostEffectAdapter(
                                HostEffectAdapterIndex::new(0),
                            ),
                        ),
                        LinkedResolvedOperand::new(
                            3,
                            LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(0)),
                        ),
                    ]),
                    0,
                )
                .unwrap(),
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::Return,
                    Box::new([]),
                    Box::new([]),
                    1,
                )
                .unwrap(),
            ],
        )];
        (
            LinkedBytecodeCandidate::try_from_parts(parts).unwrap(),
            build,
        )
    }

    fn native_signature_effects(effects: CallableMayEffects) -> LinkedNativeCallableSignature {
        LinkedNativeCallableSignature::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            effects,
        )
        .unwrap()
    }

    fn candidate_with_intrinsic() -> (LinkedBytecodeCandidate, PackageBuildId) {
        let hydrated = exact_hydration();
        let mut parts = candidate_parts(&hydrated, None, None);
        let build = hydrated
            .packages()
            .values()
            .next()
            .unwrap()
            .reference()
            .package_build_id
            .clone();
        let intrinsic = LinkedIntrinsicTarget::new(
            skiff_runtime_linked_bytecode::IntrinsicIndex::new(0),
            LinkedIntrinsicKind::Static(
                LinkedStaticIntrinsicTarget::new(
                    skiff_runtime_linked_bytecode::LinkedIntrinsicCanonicalKey::parse(
                        "core.array.empty",
                    )
                    .unwrap(),
                    1,
                )
                .unwrap(),
            ),
            native_bottom(),
        );
        parts.intrinsics = vec![intrinsic];
        parts.functions = vec![linked_function(
            &build,
            vec![
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::InvokeIntrinsic,
                    Box::new([0, 0, 0]),
                    Box::new([LinkedResolvedOperand::new(
                        0,
                        LinkedInstructionTarget::Intrinsic(
                            skiff_runtime_linked_bytecode::IntrinsicIndex::new(0),
                        ),
                    )]),
                    0,
                )
                .unwrap(),
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::Return,
                    Box::new([]),
                    Box::new([]),
                    1,
                )
                .unwrap(),
            ],
        )];
        (
            LinkedBytecodeCandidate::try_from_parts(parts).unwrap(),
            build,
        )
    }

    fn candidate_with_service() -> (LinkedBytecodeCandidate, PackageBuildId) {
        let hydrated = exact_hydration();
        let mut parts = candidate_parts(&hydrated, None, None);
        let build = hydrated
            .packages()
            .values()
            .next()
            .unwrap()
            .reference()
            .package_build_id
            .clone();
        let mut effects = bottom();
        effects.may_pending = true;
        effects.pending_effect_categories = vec![PendingEffectCategory::ServiceCall];
        let service = LinkedServiceOperationTarget::new(
            ServiceOperationIndex::new(0),
            ServiceRequirementKey {
                caller_package_build_id: build.clone(),
                service_requirement_slot: 0,
            },
            ContractOperationId::new("operation:ping"),
            ServiceProtocolIdentity::new("protocol:ping"),
            callable_signature(effects),
        );
        parts.service_operations = vec![service];
        parts.resume_sites = vec![resume_site()];
        parts.functions = vec![linked_function(
            &build,
            vec![
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::CallService,
                    Box::new([0, 0, 0, 0]),
                    Box::new([
                        LinkedResolvedOperand::new(
                            0,
                            LinkedInstructionTarget::ServiceOperation(ServiceOperationIndex::new(
                                0,
                            )),
                        ),
                        LinkedResolvedOperand::new(
                            3,
                            LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(0)),
                        ),
                    ]),
                    0,
                )
                .unwrap(),
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::Return,
                    Box::new([]),
                    Box::new([]),
                    1,
                )
                .unwrap(),
            ],
        )];
        (
            LinkedBytecodeCandidate::try_from_parts(parts).unwrap(),
            build,
        )
    }

    fn candidate_with_actor() -> (LinkedBytecodeCandidate, PackageBuildId) {
        let hydrated = exact_hydration();
        let mut parts = candidate_parts(&hydrated, None, None);
        let build = hydrated
            .packages()
            .values()
            .next()
            .unwrap()
            .reference()
            .package_build_id
            .clone();
        let mut effects = bottom();
        effects.may_pending = true;
        effects.pending_effect_categories = vec![PendingEffectCategory::ActorCall];
        let actor = LinkedActorMethodTarget::new(
            ActorMethodIndex::new(0),
            LinkedActorImplementationRef::new(
                build.clone(),
                ServiceSymbolRef {
                    module_path: "actor".to_string(),
                    symbol: "Worker".to_string(),
                },
                ActorAbiIdentity::new("actor-abi"),
                ActorImplementationIdentity::new("actor-impl"),
            ),
            ActorMethodIdentity::new("run"),
            FunctionIndex::new(0),
            callable_signature(effects),
        );
        parts.actor_methods = vec![actor];
        parts.resume_sites = vec![resume_site()];
        parts.functions = vec![linked_function(
            &build,
            vec![
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::CallActor,
                    Box::new([0, 0, 0, 0]),
                    Box::new([
                        LinkedResolvedOperand::new(
                            0,
                            LinkedInstructionTarget::ActorMethod(ActorMethodIndex::new(0)),
                        ),
                        LinkedResolvedOperand::new(
                            3,
                            LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(0)),
                        ),
                    ]),
                    0,
                )
                .unwrap(),
                LinkedInstruction::new(
                    skiff_artifact_model::Opcode::Return,
                    Box::new([]),
                    Box::new([]),
                    1,
                )
                .unwrap(),
            ],
        )];
        (
            LinkedBytecodeCandidate::try_from_parts(parts).unwrap(),
            build,
        )
    }

    fn linked_function(
        build: &PackageBuildId,
        instructions: Vec<LinkedInstruction>,
    ) -> LinkedFunction {
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
        let stack_map =
            LinkedStackMapCandidate::try_new(states.into_boxed_slice(), instructions.len(), 0, 0)
                .unwrap();
        LinkedFunction::new(
            FunctionIndex::new(0),
            SpecializationKey::new(
                build.clone(),
                ArtifactFunctionKey::parse("module::remote").unwrap(),
                PackageCallableId::new("remote"),
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
            .unwrap(),
            0,
            LinkedCallableEffectDeclaration::new(
                PackageCallableId::new("remote"),
                skiff_artifact_model::CallableEffectSummary::analysis_pending(),
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
        )
    }

    fn callable_signature(effects: CallableMayEffects) -> LinkedCallableSignature {
        LinkedCallableSignature::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            skiff_artifact_model::CallableEffectSummary::Analyzed { effects },
        )
        .unwrap()
    }

    fn native_bottom() -> LinkedNativeCallableSignature {
        LinkedNativeCallableSignature::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([]),
            bottom(),
        )
        .unwrap()
    }

    fn bottom() -> CallableMayEffects {
        CallableMayEffects {
            escapes_caller_value: false,
            requires_same_heap_identity: false,
            invokes_unknown_target: false,
            may_pending: false,
            pending_effect_categories: Vec::new(),
            inout_path_effects: Vec::new(),
        }
    }

    fn resume_site() -> skiff_runtime_linked_bytecode::LinkedResumeSite {
        skiff_runtime_linked_bytecode::LinkedResumeSite::new(
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
        .unwrap()
    }
}
