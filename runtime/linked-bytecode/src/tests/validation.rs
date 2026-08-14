use skiff_artifact_model::{
    current_platform_error_projection_registry_ref, opcode_table_fingerprint, BytecodeArtifactRef,
    CallableEffectSummary, ContractOperationId, InstructionSourceSite, LinkedOperandKind,
    LiteralIr, NativeValueAdapterRole, NativeValueLifecycleAdapter, Opcode, PackageCallableId,
    PackageRefIr, PackageSymbolRef, ParamModeIr, ResumeErrorMode, StatementAttributionId,
    SyntheticInstructionSiteReason, TypeRefIr, OPCODE_CONTRACTS,
};

use crate::{
    ActiveRegionIndex, ActorMethodIndex, ArtifactCallbackCaptureIndex, ArtifactConstantIndex,
    ArtifactConstantNodeIndex, ArtifactShapeIndex, ArtifactWritablePathIndex, CallLoanLayoutIndex,
    CallbackCaptureLayoutIndex, CandidateLocation, CandidateReferenceKind, CandidateTable,
    ConstantIndex, FrameSlotIndex, FrozenConstantNodeIndex, FunctionIndex, HostEffectAdapterIndex,
    InstructionBoundaryIndex, InstructionIndex, InterfaceTableIndex, IntrinsicIndex,
    LinkedActiveRegion, LinkedActiveRegionKind, LinkedArtifactPoolOrigin, LinkedBytecodeCandidate,
    LinkedBytecodeCandidateError, LinkedBytecodeCandidateParts, LinkedBytecodeHeaderField,
    LinkedCallLoanBinding, LinkedCallLoanLayout, LinkedCallLoanLayoutError,
    LinkedCallableSignature, LinkedCallableSignatureError, LinkedCallbackCapture,
    LinkedCallbackCaptureLayout, LinkedCatchMatcher, LinkedConstantEntry, LinkedConstantReference,
    LinkedConstantRoot, LinkedConstantSymbolPath, LinkedContainerLayout, LinkedContainerLayoutKind,
    LinkedContainerPosition, LinkedContainerPositionKind, LinkedExceptionRegion, LinkedFrameLayout,
    LinkedFrameLayoutError, LinkedFrozenConstantNode, LinkedFrozenConstantValue,
    LinkedFunctionTables, LinkedInstruction, LinkedInstructionError, LinkedInstructionTarget,
    LinkedOperationEntry, LinkedPackageBytecodeProvenance, LinkedPackageBytecodeProvenanceError,
    LinkedParameterSlot, LinkedProgramPointState, LinkedResolvedOperand, LinkedResumeSite,
    LinkedShapeEntry, LinkedShapeField, LinkedSlotState, LinkedSourceMapEntry,
    LinkedStackMapCandidate, LinkedStackValue, LinkedStatementEntry, LinkedSwitchCase,
    LinkedSwitchTable, LinkedTypeEntry, LinkedValueDropPlan, LinkedValueTransferPlan,
    LinkedWritablePathEntry, LinkedWritablePathSegment, ResumeSiteIndex, ServiceOperationIndex,
    ShapeIndex, SwitchTableIndex, SyntheticCallbackIndex, TypeIndex, WritablePathIndex,
};

use super::fixtures::{
    authority_pins, authority_pins_with_platform_error_registry, build_id, function,
    function_with_key, historical_platform_error_projection_registry_ref, minimal_parts, package,
    package_with_authority_pins, signature, snapshot_plan, snapshot_release_plan,
    specialization_for, type_origin,
};

#[test]
fn package_provenance_retains_v10_header_identity_and_five_authority_pins() {
    let package = package(0, build_id());

    assert_eq!(package.package_build_id(), &build_id());
    assert_eq!(package.schema_version(), "skiff-bytecode-v10");
    assert_eq!(package.isa_version(), "skiff-bytecode-isa-v5");
    assert_eq!(
        package.opcode_table_fingerprint(),
        opcode_table_fingerprint()
    );
    assert_eq!(
        package.declared_bytecode_identity(),
        package.artifact_ref().bytecode_identity.as_str()
    );
    assert_eq!(
        package.authorities().native_value_lifecycle_registry(),
        skiff_artifact_model::native_value_lifecycle_registry_identity()
    );
    assert_eq!(
        package.authorities().value_lifecycle_policy(),
        skiff_artifact_model::value_lifecycle_policy_identity()
    );
    assert_eq!(
        package.authorities().host_effect_registry(),
        skiff_artifact_model::host_effect_registry_identity()
    );
    assert_eq!(
        package.authorities().intrinsic_registry(),
        skiff_artifact_model::intrinsic_registry_identity()
    );
    assert_eq!(
        package.authorities().platform_error_projection_registry(),
        current_platform_error_projection_registry_ref()
    );
}

#[test]
fn provenance_and_candidate_retain_historical_platform_error_registry_pin() {
    let historical = historical_platform_error_projection_registry_ref();
    let historical_package = package_with_authority_pins(
        0,
        build_id(),
        authority_pins_with_platform_error_registry(historical.clone()),
    );
    assert_eq!(
        historical_package
            .authorities()
            .platform_error_projection_registry(),
        &historical
    );
    assert_ne!(
        historical_package
            .authorities()
            .platform_error_projection_registry(),
        current_platform_error_projection_registry_ref()
    );

    let mut parts = minimal_parts(Vec::new());
    parts.packages[0] = historical_package;
    let candidate = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("base linked model admits any generally valid registry generation-v1 pin");

    assert_eq!(
        candidate.packages()[0]
            .authorities()
            .platform_error_projection_registry(),
        &historical
    );
    assert_ne!(
        candidate.packages()[0]
            .authorities()
            .platform_error_projection_registry(),
        current_platform_error_projection_registry_ref()
    );
}

#[test]
fn package_provenance_rejects_header_reference_identity_mismatch() {
    let error = LinkedPackageBytecodeProvenance::new(
        crate::BytecodePackageIndex::new(0),
        build_id(),
        BytecodeArtifactRef::new("bytecode:referenced"),
        "bytecode:declared",
        "skiff-bytecode",
        "skiff-bytecode-v10",
        "skiff-bytecode-isa-v5",
        "opcode-table-fingerprint:fixture",
        authority_pins(),
    )
    .expect_err("reference and declared header identities must agree");

    assert_eq!(
        error,
        LinkedPackageBytecodeProvenanceError::ArtifactIdentityMismatch {
            referenced: "bytecode:referenced".to_string(),
            declared: "bytecode:declared".to_string(),
        }
    );
    assert_eq!(
        LinkedBytecodeHeaderField::BytecodeIdentity.name(),
        "bytecode identity"
    );
}

#[test]
fn package_provenance_rejects_artifact_locator_paths() {
    let mut artifact_ref = BytecodeArtifactRef::new("bytecode:fixture");
    artifact_ref.artifact_path = Some("/tmp/bytecode.json".to_string());

    let error = LinkedPackageBytecodeProvenance::new(
        crate::BytecodePackageIndex::new(0),
        build_id(),
        artifact_ref,
        "bytecode:fixture",
        "skiff-bytecode",
        "skiff-bytecode-v10",
        "skiff-bytecode-isa-v5",
        "opcode-table-fingerprint:fixture",
        authority_pins(),
    )
    .expect_err("linked candidate provenance must remain path-free");

    assert_eq!(
        error,
        LinkedPackageBytecodeProvenanceError::ArtifactReferencePathNotAllowed
    );
}

#[test]
fn frame_rejects_plan_shape_and_parameter_plan_mismatch() {
    let shape_error = LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        Box::new([]),
        None,
    )
    .expect_err("one slot requires one concrete plan");
    assert_eq!(
        shape_error,
        LinkedFrameLayoutError::SlotPlanCountMismatch {
            slot_type_count: 1,
            slot_plan_count: 0,
        }
    );

    let mismatch = LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([LinkedParameterSlot::new(
            FrameSlotIndex::new(0),
            ParamModeIr::Value,
            LinkedValueTransferPlan::MoveOnly {
                drop: LinkedValueDropPlan::Trivial,
            },
        )]),
        Box::new([]),
        Box::new([]),
        Box::new([snapshot_plan()]),
        Box::new([]),
        None,
    )
    .expect_err("parameter and frame-slot plans must be identical");
    assert!(matches!(
        mismatch,
        LinkedFrameLayoutError::ParameterPlanMismatch { .. }
    ));

    let writable_parameter = LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([LinkedParameterSlot::new(
            FrameSlotIndex::new(0),
            ParamModeIr::Value,
            snapshot_plan(),
        )]),
        Box::new([FrameSlotIndex::new(0)]),
        Box::new([]),
        Box::new([snapshot_plan()]),
        Box::new([]),
        None,
    )
    .expect_err("incoming parameters cannot be caller-owned writable locals");
    assert_eq!(
        writable_parameter,
        LinkedFrameLayoutError::WritableLocalIsParameter {
            slot: FrameSlotIndex::new(0),
        }
    );
}

#[test]
fn call_loan_layout_rejects_empty_and_noncanonical_bindings() {
    assert_eq!(
        LinkedCallLoanLayout::try_new(CallLoanLayoutIndex::new(0), Box::new([]))
            .expect_err("an InOut call needs at least one loan"),
        LinkedCallLoanLayoutError::Empty
    );

    let error = LinkedCallLoanLayout::try_new(
        CallLoanLayoutIndex::new(0),
        Box::new([
            LinkedCallLoanBinding::new(2, FrameSlotIndex::new(1), WritablePathIndex::new(0)),
            LinkedCallLoanBinding::new(1, FrameSlotIndex::new(2), WritablePathIndex::new(1)),
        ]),
    )
    .expect_err("loan bindings must follow callee parameter order");
    assert_eq!(
        error,
        LinkedCallLoanLayoutError::NonCanonicalParameterOrder {
            previous: 2,
            current: 1,
        }
    );
}

#[test]
fn candidate_retains_specialization_bound_call_loans() {
    let base = function(0, "caller");
    let key = base.key().clone();
    let plan = snapshot_plan();
    let frame = LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0), TypeIndex::new(0)]),
        Box::new([LinkedParameterSlot::new(
            FrameSlotIndex::new(0),
            ParamModeIr::Value,
            plan.clone(),
        )]),
        Box::new([FrameSlotIndex::new(1)]),
        Box::new([TypeIndex::new(0)]),
        Box::new([plan.clone(), plan.clone()]),
        Box::new([plan.clone()]),
        None,
    )
    .expect("fixture frame has one writable non-parameter local");
    let call_loan_layout = LinkedCallLoanLayout::try_new(
        CallLoanLayoutIndex::new(0),
        Box::new([LinkedCallLoanBinding::new(
            0,
            FrameSlotIndex::new(1),
            WritablePathIndex::new(0),
        )]),
    )
    .expect("fixture has one canonical loan");
    let stack_map = LinkedStackMapCandidate::try_new(
        Box::new([LinkedProgramPointState::new(
            InstructionIndex::new(0),
            Box::new([]),
            Box::new([
                LinkedSlotState::Live(LinkedStackValue::new(TypeIndex::new(0), plan.clone())),
                LinkedSlotState::Live(LinkedStackValue::new(TypeIndex::new(0), plan.clone())),
            ]),
            Box::new([]),
            Box::new([]),
        )]),
        1,
        2,
        1,
    )
    .expect("fixture stack-map remains an untrusted locally-shaped claim");
    let function = crate::LinkedFunction::new(
        base.index(),
        key.clone(),
        Box::new([LinkedInstruction::new(
            Opcode::CallLocalInOut,
            Box::new([0, 0, 1, 0]),
            Box::new([
                LinkedResolvedOperand::new(
                    0,
                    LinkedInstructionTarget::Function(FunctionIndex::new(0)),
                ),
                LinkedResolvedOperand::new(
                    3,
                    LinkedInstructionTarget::CallLoanLayout(CallLoanLayoutIndex::new(0)),
                ),
            ]),
            0,
        )
        .expect("fixture inout call follows the canonical operand contract")]),
        frame,
        base.max_operand_depth(),
        base.effect().clone(),
        LinkedFunctionTables::new(
            Box::new([]),
            Box::new([]),
            Box::new([]),
            Box::new([call_loan_layout]),
            Box::new([]),
            Box::new([]),
        ),
        stack_map,
    );
    let mut parts = minimal_parts(vec![function]);
    parts.writable_paths.push(
        LinkedWritablePathEntry::new(
            WritablePathIndex::new(0),
            LinkedArtifactPoolOrigin::new(build_id(), ArtifactWritablePathIndex::new(0), Some(key))
                .expect("fixture path owner matches its caller specialization"),
            TypeIndex::new(0),
            TypeIndex::new(0),
            Box::new([]),
        )
        .expect("fixture empty writable path has no selectors"),
    );

    let candidate = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("candidate call loan is locally specialization-bound");

    assert_eq!(
        candidate.functions()[0].frame().writable_local_slots(),
        &[FrameSlotIndex::new(1)]
    );
    assert_eq!(
        candidate.functions()[0].call_loan_layouts()[0].loans()[0].writable_path(),
        WritablePathIndex::new(0)
    );
}

#[test]
fn callable_signature_rejects_shape_mismatch_and_preserves_in_out() {
    let error = LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([]),
        Box::new([snapshot_plan()]),
        Box::new([]),
        Box::new([]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect_err("one parameter type requires one mode");
    assert_eq!(
        error,
        LinkedCallableSignatureError::ParameterModeCountMismatch {
            parameter_type_count: 1,
            parameter_mode_count: 0,
        }
    );

    let signature = LinkedCallableSignature::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([ParamModeIr::InOut]),
        Box::new([snapshot_plan()]),
        Box::new([]),
        Box::new([]),
        CallableEffectSummary::analysis_pending(),
    )
    .expect("signature has one explicit mode and plan per parameter");
    assert_eq!(signature.parameter_modes(), [ParamModeIr::InOut]);
}

#[test]
fn candidate_rejects_non_dense_function_indices() {
    let error = LinkedBytecodeCandidate::try_from_parts(minimal_parts(vec![function(1, "one")]))
        .expect_err("function table must start at zero");

    assert_eq!(
        error,
        LinkedBytecodeCandidateError::NonDenseIndex {
            table: CandidateTable::Functions,
            position: 0,
            expected: 0,
            actual: 1,
        }
    );
}

#[test]
fn artifact_pool_origin_distinguishes_specialization_context() {
    let first_key = specialization_for(build_id(), "template", Box::new([TypeIndex::new(0)]), None);
    let second_key = specialization_for(
        build_id(),
        "template",
        Box::new([TypeIndex::new(0)]),
        Some(TypeIndex::new(0)),
    );
    let mut parts = minimal_parts(vec![
        function_with_key(0, first_key.clone(), "template"),
        function_with_key(1, second_key.clone(), "template"),
    ]);
    parts.types.extend([
        LinkedTypeEntry::new(
            TypeIndex::new(1),
            type_origin(1, Some(first_key)),
            TypeRefIr::builtin("string"),
            None,
        ),
        LinkedTypeEntry::new(
            TypeIndex::new(2),
            type_origin(1, Some(second_key)),
            TypeRefIr::builtin("string"),
            None,
        ),
    ]);

    let candidate = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("one artifact row may produce distinct specialization-owned rows");
    assert_eq!(
        candidate.types()[1].origin().artifact_index(),
        candidate.types()[2].origin().artifact_index()
    );
    assert_ne!(
        candidate.types()[1].origin().specialization(),
        candidate.types()[2].origin().specialization()
    );
}

#[test]
fn candidate_rejects_duplicate_artifact_origin() {
    let mut parts = minimal_parts(Vec::new());
    parts.types.push(LinkedTypeEntry::new(
        TypeIndex::new(1),
        type_origin(0, None),
        TypeRefIr::builtin("string"),
        None,
    ));

    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(parts),
        Err(LinkedBytecodeCandidateError::DuplicateArtifactOrigin {
            table: CandidateTable::Types,
            first_index: 0,
            duplicate_index: 1,
        })
    ));
}

#[test]
fn candidate_retains_exact_container_position_layouts() {
    let mut parts = minimal_parts(Vec::new());
    parts.types.extend([
        LinkedTypeEntry::new(
            TypeIndex::new(1),
            type_origin(1, None),
            TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            },
            Some(LinkedContainerLayout::array(LinkedContainerPosition::new(
                TypeIndex::new(0),
                snapshot_release_plan(),
            ))),
        ),
        LinkedTypeEntry::new(
            TypeIndex::new(2),
            type_origin(2, None),
            TypeRefIr::builtin("Json"),
            Some(LinkedContainerLayout::json(LinkedContainerPosition::new(
                TypeIndex::new(2),
                snapshot_release_plan(),
            ))),
        ),
        LinkedTypeEntry::new(
            TypeIndex::new(3),
            type_origin(3, None),
            TypeRefIr::builtin("JsonObject"),
            Some(LinkedContainerLayout::json_object(
                LinkedContainerPosition::new(TypeIndex::new(0), snapshot_release_plan()),
                LinkedContainerPosition::new(TypeIndex::new(2), snapshot_release_plan()),
            )),
        ),
        LinkedTypeEntry::new(
            TypeIndex::new(4),
            type_origin(4, None),
            TypeRefIr::Builtin {
                name: "Map".to_string(),
                args: vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("Json")],
            },
            Some(LinkedContainerLayout::map(
                LinkedContainerPosition::new(TypeIndex::new(0), snapshot_release_plan()),
                LinkedContainerPosition::new(TypeIndex::new(2), snapshot_release_plan()),
            )),
        ),
    ]);

    let candidate = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("all built-in containers carry their exact concrete positions");
    assert_eq!(
        candidate.types()[1]
            .container_layout()
            .expect("Array has a layout")
            .element()
            .expect("Array has one element position")
            .ty(),
        TypeIndex::new(0)
    );
    assert_eq!(
        candidate.types()[3]
            .container_layout()
            .expect("JsonObject has a layout")
            .value()
            .expect("JsonObject has a value position")
            .plan(),
        &snapshot_release_plan()
    );
}

#[test]
fn candidate_rejects_missing_or_wrong_container_layout_kind() {
    let array_type = TypeRefIr::Builtin {
        name: "Array".to_string(),
        args: vec![TypeRefIr::builtin("string")],
    };
    let mut missing = minimal_parts(Vec::new());
    missing.types.push(LinkedTypeEntry::new(
        TypeIndex::new(1),
        type_origin(1, None),
        array_type.clone(),
        None,
    ));
    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(missing),
        Err(LinkedBytecodeCandidateError::MissingContainerLayout {
            expected: LinkedContainerLayoutKind::Array,
            ..
        })
    ));

    let mut wrong = minimal_parts(Vec::new());
    wrong.types.push(LinkedTypeEntry::new(
        TypeIndex::new(1),
        type_origin(1, None),
        array_type,
        Some(LinkedContainerLayout::map(
            LinkedContainerPosition::new(TypeIndex::new(0), snapshot_release_plan()),
            LinkedContainerPosition::new(TypeIndex::new(0), snapshot_release_plan()),
        )),
    ));
    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(wrong),
        Err(LinkedBytecodeCandidateError::ContainerLayoutKindMismatch {
            expected: LinkedContainerLayoutKind::Array,
            actual: LinkedContainerLayoutKind::Map,
            ..
        })
    ));
}

#[test]
fn candidate_rejects_invalid_json_recursive_position() {
    let mut wrong_type = minimal_parts(Vec::new());
    wrong_type.types.push(LinkedTypeEntry::new(
        TypeIndex::new(1),
        type_origin(1, None),
        TypeRefIr::builtin("Json"),
        Some(LinkedContainerLayout::json(LinkedContainerPosition::new(
            TypeIndex::new(0),
            snapshot_release_plan(),
        ))),
    ));
    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(wrong_type),
        Err(
            LinkedBytecodeCandidateError::ContainerPositionTypeMismatch {
                position: LinkedContainerPositionKind::JsonRecursiveValue,
                ..
            }
        )
    ));

    let mut wrong_plan = minimal_parts(Vec::new());
    wrong_plan.types.push(LinkedTypeEntry::new(
        TypeIndex::new(1),
        type_origin(1, None),
        TypeRefIr::builtin("Json"),
        Some(LinkedContainerLayout::json(LinkedContainerPosition::new(
            TypeIndex::new(1),
            snapshot_plan(),
        ))),
    ));
    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(wrong_plan),
        Err(
            LinkedBytecodeCandidateError::ContainerPositionPlanMismatch {
                position: LinkedContainerPositionKind::JsonRecursiveValue,
                ..
            }
        )
    ));
}

fn target_for_operand_kind(kind: LinkedOperandKind) -> Option<LinkedInstructionTarget> {
    match kind {
        LinkedOperandKind::Immediate => None,
        LinkedOperandKind::Instruction => {
            Some(LinkedInstructionTarget::Branch(InstructionIndex::new(0)))
        }
        LinkedOperandKind::FrameSlot => {
            Some(LinkedInstructionTarget::FrameSlot(FrameSlotIndex::new(0)))
        }
        LinkedOperandKind::SwitchTable => Some(LinkedInstructionTarget::SwitchTable(
            SwitchTableIndex::new(0),
        )),
        LinkedOperandKind::ActiveRegion => Some(LinkedInstructionTarget::ActiveRegion(
            ActiveRegionIndex::new(0),
        )),
        LinkedOperandKind::CallLoanLayout => Some(LinkedInstructionTarget::CallLoanLayout(
            CallLoanLayoutIndex::new(0),
        )),
        LinkedOperandKind::Function => {
            Some(LinkedInstructionTarget::Function(FunctionIndex::new(0)))
        }
        LinkedOperandKind::ServiceOperation => Some(LinkedInstructionTarget::ServiceOperation(
            ServiceOperationIndex::new(0),
        )),
        LinkedOperandKind::ActorMethod => Some(LinkedInstructionTarget::ActorMethod(
            ActorMethodIndex::new(0),
        )),
        LinkedOperandKind::InterfaceTable => Some(LinkedInstructionTarget::InterfaceTable(
            InterfaceTableIndex::new(0),
        )),
        LinkedOperandKind::SyntheticCallback => Some(LinkedInstructionTarget::SyntheticCallback(
            SyntheticCallbackIndex::new(0),
        )),
        LinkedOperandKind::HostEffectAdapter => Some(LinkedInstructionTarget::HostEffectAdapter(
            HostEffectAdapterIndex::new(0),
        )),
        LinkedOperandKind::Intrinsic => {
            Some(LinkedInstructionTarget::Intrinsic(IntrinsicIndex::new(0)))
        }
        LinkedOperandKind::Constant => {
            Some(LinkedInstructionTarget::Constant(ConstantIndex::new(0)))
        }
        LinkedOperandKind::Type => Some(LinkedInstructionTarget::Type(TypeIndex::new(0))),
        LinkedOperandKind::Shape => Some(LinkedInstructionTarget::Shape(ShapeIndex::new(0))),
        LinkedOperandKind::WritablePath => Some(LinkedInstructionTarget::WritablePath(
            WritablePathIndex::new(0),
        )),
        LinkedOperandKind::CallbackCaptureLayout => Some(
            LinkedInstructionTarget::CallbackCaptureLayout(CallbackCaptureLayoutIndex::new(0)),
        ),
        LinkedOperandKind::ResumeSite => {
            Some(LinkedInstructionTarget::ResumeSite(ResumeSiteIndex::new(0)))
        }
    }
}

#[test]
fn every_opcode_contract_has_exact_linked_operand_targets() {
    for contract in OPCODE_CONTRACTS {
        let resolved = contract
            .operands
            .iter()
            .enumerate()
            .filter_map(|(ordinal, specification)| {
                target_for_operand_kind(specification.linked_kind)
                    .map(|target| LinkedResolvedOperand::new(ordinal as u32, target))
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let instruction = LinkedInstruction::new(
            contract.kind,
            vec![0; contract.operands.len()].into_boxed_slice(),
            resolved,
            17,
        )
        .expect("fixture targets are generated from the canonical operand contract");

        assert_eq!(instruction.artifact_pc(), 17);
        assert!(instruction.resolved_operands().iter().all(|resolved| {
            contract.operands[resolved.operand_ordinal() as usize].linked_kind
                == resolved.target().kind()
        }));
    }
}

#[test]
fn instruction_rejects_missing_unexpected_or_wrong_typed_targets() {
    assert!(matches!(
        LinkedInstruction::new(Opcode::CallLocal, Box::new([0, 1]), Box::new([]), 17),
        Err(LinkedInstructionError::OperandCountMismatch {
            expected: 3,
            actual: 2,
            ..
        })
    ));
    assert!(matches!(
        LinkedInstruction::new(Opcode::CallLocal, Box::new([0, 1, 1]), Box::new([]), 17),
        Err(LinkedInstructionError::MissingResolvedOperand {
            operand_ordinal: 0,
            expected: LinkedOperandKind::Function,
        })
    ));
    assert!(matches!(
        LinkedInstruction::new(
            Opcode::CallLocal,
            Box::new([0, 1, 1]),
            Box::new([LinkedResolvedOperand::new(
                0,
                LinkedInstructionTarget::Type(TypeIndex::new(0)),
            )]),
            17,
        ),
        Err(LinkedInstructionError::ResolvedOperandKindMismatch {
            operand_ordinal: 0,
            expected: LinkedOperandKind::Function,
            actual: LinkedOperandKind::Type,
        })
    ));
    assert!(matches!(
        LinkedInstruction::new(
            Opcode::CallLocal,
            Box::new([0, 1, 1]),
            Box::new([
                LinkedResolvedOperand::new(
                    0,
                    LinkedInstructionTarget::Function(FunctionIndex::new(0)),
                ),
                LinkedResolvedOperand::new(1, LinkedInstructionTarget::Type(TypeIndex::new(0))),
            ]),
            17,
        ),
        Err(LinkedInstructionError::UnexpectedResolvedOperand {
            operand_ordinal: 1,
            actual: LinkedOperandKind::Type,
        })
    ));
}

#[test]
fn instruction_rejects_noncanonical_or_out_of_bounds_target_ordinals() {
    assert!(matches!(
        LinkedInstruction::new(
            Opcode::CallLocalInOut,
            Box::new([0, 0, 1, 0]),
            Box::new([
                LinkedResolvedOperand::new(
                    3,
                    LinkedInstructionTarget::CallLoanLayout(CallLoanLayoutIndex::new(0)),
                ),
                LinkedResolvedOperand::new(
                    0,
                    LinkedInstructionTarget::Function(FunctionIndex::new(0)),
                ),
            ]),
            17,
        ),
        Err(LinkedInstructionError::NonCanonicalResolvedOperandOrder {
            previous: 3,
            current: 0,
        })
    ));
    assert!(matches!(
        LinkedInstruction::new(
            Opcode::CallLocal,
            Box::new([0, 1, 1]),
            Box::new([LinkedResolvedOperand::new(
                3,
                LinkedInstructionTarget::Function(FunctionIndex::new(0)),
            )]),
            17,
        ),
        Err(LinkedInstructionError::OperandOrdinalOutOfBounds {
            operand_ordinal: 3,
            operand_count: 3,
        })
    ));
}

#[test]
fn function_tables_preserve_regions_switch_attribution_and_source_sites() {
    let site = InstructionSourceSite::Synthetic {
        reason: SyntheticInstructionSiteReason::RuntimeControlFlow,
    };
    let exception = LinkedExceptionRegion::new(
        InstructionIndex::new(0),
        InstructionBoundaryIndex::new(1),
        InstructionIndex::new(0),
        0,
        Box::new([LinkedCatchMatcher::CatchAll]),
        FrameSlotIndex::new(0),
        TypeIndex::new(0),
        0,
    );
    let active = LinkedActiveRegion::new(
        ActiveRegionIndex::new(0),
        InstructionIndex::new(0),
        InstructionBoundaryIndex::new(1),
        LinkedActiveRegionKind::Timeout {
            duration_ms: 100,
            site: site.clone(),
        },
    );
    let switch = LinkedSwitchTable::try_new(
        Box::new([LinkedSwitchCase::new(
            TypeIndex::new(0),
            InstructionIndex::new(0),
        )]),
        InstructionIndex::new(0),
    )
    .expect("fixture switch tags are canonical");
    let statement = LinkedStatementEntry::new(
        InstructionIndex::new(0),
        0,
        StatementAttributionId::Generated { ordinal: 0 },
        site.clone(),
    );
    let tables = LinkedFunctionTables::new(
        Box::new([exception]),
        Box::new([active]),
        Box::new([switch]),
        Box::new([]),
        Box::new([statement]),
        Box::new([LinkedSourceMapEntry::new(
            InstructionIndex::new(0),
            InstructionBoundaryIndex::new(1),
            site.clone(),
        )]),
    );

    assert_eq!(
        tables.exception_regions()[0].catch_slot_type(),
        TypeIndex::new(0)
    );
    assert_eq!(
        tables.switch_tables()[0].default_target(),
        InstructionIndex::new(0)
    );
    assert_eq!(
        tables.statement_entries()[0].instruction(),
        InstructionIndex::new(0)
    );
    assert_eq!(tables.statement_entries()[0].sequence_ordinal(), 0);
    assert_eq!(
        tables.statement_entries()[0].attribution_id(),
        StatementAttributionId::Generated { ordinal: 0 }
    );
    assert_eq!(tables.statement_entries()[0].site(), &site);
    assert_eq!(
        tables.source_map()[0].end(),
        InstructionBoundaryIndex::new(1)
    );
}

fn candidate_with_nominal_data_resume_and_root_facts() -> LinkedBytecodeCandidate {
    let mut parts = minimal_parts(vec![function(0, "root")]);
    parts.callback_capture_layouts.push(
        LinkedCallbackCaptureLayout::try_new(
            CallbackCaptureLayoutIndex::new(0),
            LinkedArtifactPoolOrigin::new(
                build_id(),
                ArtifactCallbackCaptureIndex::new(0),
                Some(parts.functions[0].key().clone()),
            )
            .expect("fixture capture origin has exact specialization context"),
            parts.functions[0].key().artifact_function_key().clone(),
            FunctionIndex::new(0),
            Box::new([LinkedCallbackCapture::new(
                FrameSlotIndex::new(0),
                TypeIndex::new(0),
                snapshot_plan(),
            )]),
        )
        .expect("fixture capture slots are unique"),
    );
    parts.operation_entries.push(LinkedOperationEntry::new(
        ContractOperationId::new("operation:root"),
        FunctionIndex::new(0),
        signature(),
    ));
    parts.shapes.push(
        LinkedShapeEntry::new(
            ShapeIndex::new(0),
            LinkedArtifactPoolOrigin::new(build_id(), ArtifactShapeIndex::new(0), None)
                .expect("fixture shape origin is package-global"),
            TypeIndex::new(0),
            snapshot_plan(),
            None,
            Box::new([
                LinkedShapeField::new("value", TypeIndex::new(0), snapshot_plan())
                    .expect("fixture shape field name is non-empty"),
            ]),
        )
        .expect("fixture shape fields are canonical"),
    );
    parts
        .frozen_constant_nodes
        .push(LinkedFrozenConstantNode::new(
            FrozenConstantNodeIndex::new(0),
            LinkedArtifactPoolOrigin::new(build_id(), ArtifactConstantNodeIndex::new(0), None)
                .expect("fixture frozen-node origin is package-global"),
            LinkedFrozenConstantValue::Literal(LiteralIr::Null),
        ));
    parts.constants.push(LinkedConstantEntry::new(
        ConstantIndex::new(0),
        LinkedArtifactPoolOrigin::new(build_id(), ArtifactConstantIndex::new(0), None)
            .expect("fixture constant origin is package-global"),
        LinkedConstantReference::LocalNode {
            node: FrozenConstantNodeIndex::new(0),
        },
        TypeIndex::new(0),
        snapshot_plan(),
    ));
    parts.constant_roots.push(LinkedConstantRoot::new(
        build_id(),
        LinkedConstantSymbolPath::parse("constants.root")
            .expect("fixture constant root is canonical"),
        ConstantIndex::new(0),
    ));
    parts.resume_sites.push(
        LinkedResumeSite::new(
            ResumeSiteIndex::new(0),
            FunctionIndex::new(0),
            InstructionIndex::new(0),
            InstructionIndex::new(0),
            None,
            0,
            Box::new([TypeIndex::new(0)]),
            Box::new([snapshot_plan()]),
            Box::new([None]),
            ResumeErrorMode::RaiseAtSite,
        )
        .expect("fixture resume result types and plans align"),
    );
    parts.writable_paths.push(
        LinkedWritablePathEntry::new(
            WritablePathIndex::new(0),
            LinkedArtifactPoolOrigin::new(
                build_id(),
                ArtifactWritablePathIndex::new(0),
                Some(parts.functions[0].key().clone()),
            )
            .expect("fixture writable path has exact specialization context"),
            TypeIndex::new(0),
            TypeIndex::new(0),
            Box::new([LinkedWritablePathSegment::DenseField {
                shape: ShapeIndex::new(0),
                field_ordinal: 0,
            }]),
        )
        .expect("fixture writable path has dense selector ordinals"),
    );

    LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("fixture candidate references are locally in bounds")
}

#[test]
fn candidate_getters_retain_nominal_data_resume_and_root_facts() {
    let candidate = candidate_with_nominal_data_resume_and_root_facts();

    assert_eq!(
        candidate.packages()[0].schema_version(),
        "skiff-bytecode-v10"
    );
    assert_eq!(candidate.functions().len(), 1);
    assert_eq!(candidate.operation_entries().len(), 1);
    assert_eq!(candidate.callback_capture_layouts().len(), 1);
    assert_eq!(
        candidate.callback_capture_layouts()[0].captures()[0].plan(),
        &snapshot_plan()
    );
    assert_eq!(candidate.types().len(), 1);
    assert_eq!(candidate.shapes()[0].fields()[0].name(), "value");
    assert_eq!(candidate.constants()[0].plan(), &snapshot_plan());
    assert_eq!(
        candidate.constant_roots()[0].constant(),
        ConstantIndex::new(0)
    );
    assert!(matches!(
        candidate.frozen_constant_nodes()[0].value(),
        LinkedFrozenConstantValue::Literal(LiteralIr::Null)
    ));
    assert_eq!(
        candidate.resume_sites()[0].function(),
        FunctionIndex::new(0)
    );
    assert_eq!(candidate.resume_sites()[0].end_resume(), None);
    assert_eq!(candidate.functions()[0].stream_result_type_ref(), None);
    assert!(candidate.writable_paths()[0]
        .origin()
        .specialization()
        .is_some());
}

#[test]
fn callback_capture_layout_requires_its_exact_function_specialization() {
    let mut parts = minimal_parts(vec![function(0, "callback")]);
    parts.callback_capture_layouts.push(
        LinkedCallbackCaptureLayout::try_new(
            CallbackCaptureLayoutIndex::new(0),
            LinkedArtifactPoolOrigin::new(build_id(), ArtifactCallbackCaptureIndex::new(0), None)
                .expect("fixture package-global origin has a matching owner"),
            parts.functions[0].key().artifact_function_key().clone(),
            FunctionIndex::new(0),
            Box::new([LinkedCallbackCapture::new(
                FrameSlotIndex::new(0),
                TypeIndex::new(0),
                snapshot_plan(),
            )]),
        )
        .expect("fixture capture slots are unique"),
    );

    let error = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect_err("capture layout without exact specialization must fail closed");

    assert!(matches!(
        error,
        LinkedBytecodeCandidateError::CallbackCaptureOriginMismatch {
            layout,
            function
        } if layout == CallbackCaptureLayoutIndex::new(0)
            && function == FunctionIndex::new(0)
    ));
}

#[test]
fn package_symbol_constant_requires_exact_resolved_build_and_node_origin() {
    let resolved_origin = LinkedArtifactPoolOrigin::new(
        build_id(),
        ArtifactConstantNodeIndex::new(4),
        Some(specialization_for(
            build_id(),
            "constant-root",
            Box::new([]),
            None,
        )),
    )
    .expect("fixture resolution has exact owner and specialization");
    let reference = LinkedConstantReference::PackageSymbol {
        source: PackageSymbolRef {
            package: PackageRefIr::Dependency {
                dependency_ref: "models".to_string(),
            },
            symbol_path: "constants.root".to_string(),
            abi_expectation: None,
        },
        resolved_origin: resolved_origin.clone(),
        node: FrozenConstantNodeIndex::new(9),
    };

    let LinkedConstantReference::PackageSymbol {
        source,
        resolved_origin: retained,
        node,
    } = reference
    else {
        panic!("fixture must remain a package-symbol resolution");
    };
    assert_eq!(source.symbol_path, "constants.root");
    assert_eq!(retained, resolved_origin);
    assert_eq!(node, FrozenConstantNodeIndex::new(9));
}

#[test]
fn candidate_rejects_wrong_lifecycle_adapter_role() {
    let wrong_clone = NativeValueLifecycleAdapter {
        binding_key: "adapter.drop".to_string(),
        role: NativeValueAdapterRole::ResourceDrop,
        abi_version: 1,
    };
    let plan = LinkedValueTransferPlan::ExplicitCloneLease {
        clone_adapter: wrong_clone,
        drop: crate::LinkedResourceDropPlan::ResourceTableRelease,
    };
    let frame = LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([LinkedParameterSlot::new(
            FrameSlotIndex::new(0),
            ParamModeIr::Value,
            plan.clone(),
        )]),
        Box::new([]),
        Box::new([]),
        Box::new([plan]),
        Box::new([]),
        None,
    )
    .expect("wrong semantic role is still locally shape-valid");
    let base = function(0, "root");
    let function = crate::LinkedFunction::new(
        base.index(),
        base.key().clone(),
        base.instructions().to_vec().into_boxed_slice(),
        frame,
        base.max_operand_depth(),
        base.effect().clone(),
        base.tables().clone(),
        base.stack_map().clone(),
    );

    let error = LinkedBytecodeCandidate::try_from_parts(minimal_parts(vec![function]))
        .expect_err("clone adapter must have the CloneLease registry role");
    assert!(matches!(
        error,
        LinkedBytecodeCandidateError::LifecycleAdapterRoleMismatch {
            expected: NativeValueAdapterRole::CloneLease,
            actual: NativeValueAdapterRole::ResourceDrop,
            ..
        }
    ));
}

#[test]
fn candidate_reports_out_of_bounds_typed_reference() {
    let mut parts = minimal_parts(vec![function(0, "root")]);
    parts.operation_entries.push(LinkedOperationEntry::new(
        ContractOperationId::new("operation:bad"),
        FunctionIndex::new(1),
        signature(),
    ));

    assert!(matches!(
        LinkedBytecodeCandidate::try_from_parts(parts),
        Err(LinkedBytecodeCandidateError::ReferenceOutOfBounds {
            reference: CandidateReferenceKind::Function,
            index: 1,
            ..
        })
    ));
}

#[test]
fn effect_reference_remains_typed_package_callable_identity() {
    let function = function(0, "root");
    assert_eq!(
        function.effect_summary_ref(),
        &PackageCallableId::new("root")
    );
}

fn frame_with_stream_result(
    stream_result_type_ref: Option<TypeIndex>,
    result_types: Box<[TypeIndex]>,
) -> LinkedFrameLayout {
    let plan = snapshot_plan();
    let result_plans = result_types
        .iter()
        .map(|_| snapshot_plan())
        .collect::<Box<[_]>>();
    LinkedFrameLayout::new(
        Box::new([TypeIndex::new(0)]),
        Box::new([LinkedParameterSlot::new(
            FrameSlotIndex::new(0),
            ParamModeIr::Value,
            plan.clone(),
        )]),
        Box::new([]),
        result_types,
        Box::new([plan]),
        result_plans,
        stream_result_type_ref,
    )
    .expect("fixture stream frame has aligned slot and result plans")
}

fn function_with_frame(frame: LinkedFrameLayout) -> crate::LinkedFunction {
    let base = function(0, "streams");
    crate::LinkedFunction::new(
        base.index(),
        base.key().clone(),
        base.instructions().to_vec().into_boxed_slice(),
        frame,
        base.max_operand_depth(),
        base.effect().clone(),
        base.tables().clone(),
        base.stack_map().clone(),
    )
}

fn stream_next_instruction() -> LinkedInstruction {
    let contract = OPCODE_CONTRACTS
        .iter()
        .find(|contract| contract.kind == Opcode::StreamNext)
        .expect("StreamNext has a canonical opcode contract");
    let resolved = contract
        .operands
        .iter()
        .enumerate()
        .filter_map(|(ordinal, specification)| {
            target_for_operand_kind(specification.linked_kind)
                .map(|target| LinkedResolvedOperand::new(ordinal as u32, target))
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    LinkedInstruction::new(
        Opcode::StreamNext,
        vec![0; contract.operands.len()].into_boxed_slice(),
        resolved,
        0,
    )
    .expect("fixture StreamNext follows the canonical operand contract")
}

fn function_with_instructions(instructions: Box<[LinkedInstruction]>) -> crate::LinkedFunction {
    let base = function(0, "streams");
    let stack_map = LinkedStackMapCandidate::try_new(
        instructions
            .iter()
            .enumerate()
            .map(|(position, _)| {
                LinkedProgramPointState::new(
                    InstructionIndex::new(
                        u32::try_from(position).expect("fixture instruction position fits u32"),
                    ),
                    Box::new([]),
                    Box::new([LinkedSlotState::Live(LinkedStackValue::new(
                        TypeIndex::new(0),
                        snapshot_plan(),
                    ))]),
                    Box::new([]),
                    Box::new([]),
                )
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        instructions.len(),
        1,
        1,
    )
    .expect("fixture stack map has one state per instruction");
    crate::LinkedFunction::new(
        base.index(),
        base.key().clone(),
        instructions,
        base.frame().clone(),
        base.max_operand_depth(),
        base.effect().clone(),
        base.tables().clone(),
        stack_map,
    )
}

fn parts_with_stream_next_resume(
    end_resume: Option<InstructionIndex>,
) -> LinkedBytecodeCandidateParts {
    let budget = LinkedInstruction::new(Opcode::BudgetCheckpoint, Box::new([]), Box::new([]), 0)
        .expect("fixture budget checkpoint has no operands");
    let function = function_with_instructions(Box::new([
        stream_next_instruction(),
        budget.clone(),
        budget,
    ]));
    let mut parts = minimal_parts(vec![function]);
    parts.resume_sites.push(
        LinkedResumeSite::new(
            ResumeSiteIndex::new(0),
            FunctionIndex::new(0),
            InstructionIndex::new(0),
            InstructionIndex::new(1),
            end_resume,
            0,
            Box::new([TypeIndex::new(0)]),
            Box::new([snapshot_plan()]),
            Box::new([None]),
            ResumeErrorMode::RaiseAtSite,
        )
        .expect("fixture resume result types and plans align"),
    );
    parts
}

#[test]
fn linked_function_retains_explicit_stream_producer_authority() {
    let stream_type = TypeIndex::new(1);
    let frame = frame_with_stream_result(Some(stream_type), Box::new([]));
    let function = function_with_frame(frame);
    let mut parts = minimal_parts(vec![function]);
    parts.types.push(LinkedTypeEntry::new(
        TypeIndex::new(1),
        type_origin(1, None),
        TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        },
        None,
    ));

    let candidate = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("explicit stream producer authority is locally valid");
    assert_eq!(
        candidate.functions()[0].stream_result_type_ref(),
        Some(stream_type)
    );
    assert_eq!(
        candidate.functions()[0].frame().stream_result_type_ref(),
        Some(stream_type)
    );
    assert!(candidate.functions()[0].frame().result_types().is_empty());
}

#[test]
fn ordinary_result_frame_is_not_derived_as_stream_producer() {
    let stream_type = TypeIndex::new(1);
    let frame = frame_with_stream_result(None, Box::new([stream_type]));
    let mut parts = minimal_parts(vec![function_with_frame(frame)]);
    parts.types.push(LinkedTypeEntry::new(
        TypeIndex::new(1),
        type_origin(1, None),
        TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        },
        None,
    ));

    let candidate = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect("ordinary results never imply stream producer authority");
    assert_eq!(candidate.functions()[0].stream_result_type_ref(), None);
    assert_eq!(
        candidate.functions()[0].frame().result_types(),
        [stream_type]
    );
}

#[test]
fn candidate_rejects_stream_producer_with_ordinary_results() {
    let stream_type = TypeIndex::new(1);
    let frame = frame_with_stream_result(Some(stream_type), Box::new([TypeIndex::new(0)]));
    let mut parts = minimal_parts(vec![function_with_frame(frame)]);
    parts.types.push(LinkedTypeEntry::new(
        TypeIndex::new(1),
        type_origin(1, None),
        TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        },
        None,
    ));

    let error = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect_err("stream producers must not also carry ordinary results");
    assert!(matches!(
        error,
        LinkedBytecodeCandidateError::StreamProducerResultCountNotZero {
            function,
            result_count: 1,
        } if function == FunctionIndex::new(0)
    ));
}

#[test]
fn candidate_rejects_stream_producer_type_that_is_not_stream() {
    let frame = frame_with_stream_result(Some(TypeIndex::new(0)), Box::new([]));

    let error =
        LinkedBytecodeCandidate::try_from_parts(minimal_parts(vec![function_with_frame(frame)]))
            .expect_err("stream producer authority must select Stream<T>");
    assert!(matches!(
        error,
        LinkedBytecodeCandidateError::StreamProducerTypeMismatch {
            function,
            stream_type,
        } if function == FunctionIndex::new(0) && stream_type == TypeIndex::new(0)
    ));
}

#[test]
fn resume_site_retains_stream_next_end_resume_path() {
    let candidate = LinkedBytecodeCandidate::try_from_parts(parts_with_stream_next_resume(Some(
        InstructionIndex::new(2),
    )))
    .expect("StreamNext item and end resume paths are locally valid");

    assert_eq!(
        candidate.resume_sites()[0].end_resume(),
        Some(InstructionIndex::new(2))
    );
    assert_eq!(
        candidate.resume_sites()[0].resume(),
        InstructionIndex::new(1)
    );
}

#[test]
fn candidate_rejects_stream_next_without_end_resume() {
    let error = LinkedBytecodeCandidate::try_from_parts(parts_with_stream_next_resume(None))
        .expect_err("StreamNext requires its natural-end resume path");
    assert!(matches!(
        error,
        LinkedBytecodeCandidateError::StreamNextMissingEndResume {
            resume_site: 0,
            function,
            site,
        } if function == FunctionIndex::new(0) && site == InstructionIndex::new(0)
    ));
}

#[test]
fn candidate_rejects_end_resume_on_non_stream_resume_site() {
    let mut parts = minimal_parts(vec![function(0, "root")]);
    parts.resume_sites.push(
        LinkedResumeSite::new(
            ResumeSiteIndex::new(0),
            FunctionIndex::new(0),
            InstructionIndex::new(0),
            InstructionIndex::new(0),
            Some(InstructionIndex::new(0)),
            0,
            Box::new([]),
            Box::new([]),
            Box::new([]),
            ResumeErrorMode::RaiseAtSite,
        )
        .expect("fixture resume result types and plans align"),
    );

    let error = LinkedBytecodeCandidate::try_from_parts(parts)
        .expect_err("end resume pc is only valid for StreamNext");
    assert!(matches!(
        error,
        LinkedBytecodeCandidateError::EndResumeOnlyValidForStreamNext {
            resume_site: 0,
            function,
            site,
        } if function == FunctionIndex::new(0) && site == InstructionIndex::new(0)
    ));
}

#[test]
fn candidate_rejects_end_resume_outside_stream_next_function() {
    let error = LinkedBytecodeCandidate::try_from_parts(parts_with_stream_next_resume(Some(
        InstructionIndex::new(3),
    )))
    .expect_err("end resume pc must target the same linked function");
    assert!(matches!(
        error,
        LinkedBytecodeCandidateError::ReferenceOutOfBounds {
            location: CandidateLocation::TableRow {
                table: CandidateTable::ResumeSites,
                row: 0,
            },
            reference: CandidateReferenceKind::Instruction,
            index: 3,
            len: 3,
        }
    ));
}

#[test]
fn candidate_rejects_stream_next_end_resume_equal_to_item_resume() {
    let error = LinkedBytecodeCandidate::try_from_parts(parts_with_stream_next_resume(Some(
        InstructionIndex::new(1),
    )))
    .expect_err("item and natural-end resume targets must differ");
    assert!(matches!(
        error,
        LinkedBytecodeCandidateError::StreamNextResumeEndTargetsEqual {
            resume_site: 0,
            function,
            site,
            resume,
            end_resume,
        } if function == FunctionIndex::new(0)
            && site == InstructionIndex::new(0)
            && resume == InstructionIndex::new(1)
            && end_resume == InstructionIndex::new(1)
    ));
}
