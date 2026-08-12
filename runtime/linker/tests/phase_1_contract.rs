#[path = "phase_1_contract/support.rs"]
mod support;

use std::sync::Arc;

use skiff_artifact_identity::{ArtifactIdentityError, ValidatedBytecodeArtifact};
use skiff_artifact_model::{
    BytecodeDecodeError, BytecodeRelocation, InstructionSourceSite, Opcode, ParamModeIr,
    PendingEffectCategory, StatementAttributionClass, StructuralValidationError, TypeRefIr,
};
use skiff_compiler::{
    BytecodeEmissionError, CompilerPlatformSources, PackageCompileError,
    Phase1UnsupportedCapability,
};
use skiff_runtime_linked_bytecode::{
    LinkedFunction, LinkedInstruction, LinkedInstructionTarget, TypeIndex,
};
use skiff_runtime_linker::{
    link_deployment_execution_image, BytecodeLinkError, BytecodeLinkLocation,
    DeploymentExecutionImage, DeploymentExecutionImageError, Phase1LinkedCapability,
};
use skiff_test_runner::canonical_package::{compile_package_project, CanonicalPackageProjectError};

use support::{
    package_source, production_sized_execution_limits, repository_root, PublishedService,
    TempRoot, PHASE_1_SCALAR_LOCAL_SOURCE,
};

#[test]
fn unsupported_typed_source_is_owned_by_phase_1_compiler_admission() {
    let source = r#"function run() -> string { return "disabled" }"#;
    let source_root = package_source("unsupported-source", source, false);
    let artifact_root = TempRoot::create("phase-1-contract-artifact-unsupported-source");
    let platform = CompilerPlatformSources::new(&repository_root()).expect("open platform source");
    let error = compile_package_project(&platform, source_root.path(), artifact_root.path())
        .expect_err("Phase 1 compiler admission must reject string values before emission");
    let CanonicalPackageProjectError::Compile(PackageCompileError::BytecodeEmission { source }) =
        error
    else {
        panic!("unsupported typed source must be owned by bytecode emission: {error:?}");
    };
    let BytecodeEmissionError::UnsupportedPhase1Capability {
        capability,
        module_path,
        function_key,
        location,
    } = source
    else {
        panic!("unsupported source must retain its typed Phase 1 owner: {source:?}");
    };
    assert_eq!(capability, Phase1UnsupportedCapability::ValueShape);
    assert_eq!(module_path, "main");
    assert_eq!(function_key.as_deref(), Some("main::run"));
    assert_eq!(location, "return type");
}

#[test]
fn scalar_local_source_facts_survive_canonical_publication_loader_and_link() {
    let fixture = PublishedService::build_from_source(
        "scalar-local-producer-consumer",
        PHASE_1_SCALAR_LOCAL_SOURCE,
    );
    let artifact = fixture.bytecode.artifact();
    let artifact_run = artifact
        .image
        .functions
        .get("main::run")
        .expect("compiler emits the source run function");
    let artifact_helper = artifact
        .image
        .functions
        .get("main::helper")
        .expect("compiler emits the source helper function");

    assert_eq!(artifact_run.relocations.len(), 1);
    let BytecodeRelocation::LocalExecutableRef {
        function_key,
        specialization,
    } = &artifact_run.relocations[0]
    else {
        panic!(
            "the source direct-local call must remain a typed local relocation: {:?}",
            artifact_run.relocations[0]
        );
    };
    assert_eq!(function_key, "main::helper");
    assert!(specialization.type_arguments.is_empty());
    assert_eq!(specialization.concrete_receiver, None);

    assert_eq!(artifact_helper.frame_layout.slot_count, 1);
    assert_eq!(artifact_helper.frame_layout.parameter_slots.len(), 1);
    assert_eq!(artifact_helper.frame_layout.parameter_slots[0].slot, 0);
    assert_eq!(
        artifact_helper.frame_layout.parameter_slots[0].mode,
        ParamModeIr::Value
    );
    assert_eq!(artifact_helper.frame_layout.result_count, 1);
    assert_eq!(artifact_run.frame_layout.slot_count, 2);
    assert_eq!(artifact_run.frame_layout.parameter_slots.len(), 1);
    assert_eq!(artifact_run.frame_layout.parameter_slots[0].slot, 0);
    assert_eq!(
        artifact_run.frame_layout.parameter_slots[0].mode,
        ParamModeIr::Value
    );
    assert_eq!(artifact_run.frame_layout.result_count, 1);
    assert_eq!(
        artifact_run.frame_layout.slot_type_refs,
        vec![
            artifact_helper.frame_layout.slot_type_refs[0],
            artifact_helper.frame_layout.slot_type_refs[0],
        ],
        "the source local result slot retains the exact scalar type"
    );
    assert_eq!(
        artifact_run.frame_layout.result_type_refs,
        artifact_helper.frame_layout.result_type_refs
    );

    let image = Arc::new(
        link_deployment_execution_image(fixture.hydrated(), &production_sized_execution_limits())
            .expect("the admitted scalar/local source fixture must become an execution image"),
    );
    assert_eq!(image.functions().len(), 2);
    let linked_run = linked_function(&image, "main::run");
    let linked_helper = linked_function(&image, "main::helper");

    let call = only_instruction(linked_run, Opcode::CallLocal);
    assert_eq!(
        call.operands(),
        &[0, 1, 1],
        "relocation index, argument count, and result count are exact"
    );
    assert_eq!(call.resolved_operands().len(), 1);
    assert_eq!(call.resolved_operands()[0].operand_ordinal(), 0);
    assert_eq!(
        call.resolved_operands()[0].target(),
        LinkedInstructionTarget::Function(linked_helper.index()),
        "the linker resolves the source helper symbol to the exact helper function"
    );

    assert_eq!(linked_helper.frame().slot_types().len(), 1);
    assert_eq!(linked_helper.frame().parameters().len(), 1);
    assert_eq!(linked_helper.frame().parameters()[0].slot().get(), 0);
    assert_eq!(
        linked_helper.frame().parameters()[0].mode(),
        ParamModeIr::Value
    );
    assert_eq!(linked_helper.frame().result_types().len(), 1);
    assert_eq!(linked_run.frame().slot_types().len(), 2);
    assert_eq!(linked_run.frame().parameters().len(), 1);
    assert_eq!(linked_run.frame().parameters()[0].slot().get(), 0);
    assert_eq!(
        linked_run.frame().parameters()[0].mode(),
        ParamModeIr::Value
    );
    assert_eq!(linked_run.frame().result_types().len(), 1);
    assert_eq!(
        linked_run.frame().slot_types()[0],
        linked_run.frame().slot_types()[1],
        "the run parameter and source local use one specialization-owned scalar type"
    );
    assert_eq!(
        linked_run.frame().slot_types()[0],
        linked_run.frame().result_types()[0],
        "the run result retains its source-owned scalar type"
    );
    assert_eq!(
        linked_helper.frame().slot_types()[0],
        linked_helper.frame().result_types()[0],
        "the helper result retains its source-owned scalar type"
    );
    assert_linked_type_handoff(
        &image,
        linked_run,
        linked_run.frame().slot_types()[0],
        artifact_run.frame_layout.slot_type_refs[0],
    );
    assert_linked_type_handoff(
        &image,
        linked_helper,
        linked_helper.frame().slot_types()[0],
        artifact_helper.frame_layout.slot_type_refs[0],
    );

    assert_eq!(image.ingress_bindings().len(), 1);
    let (ingress, gateway_identity) = fixture.http_gateway_lookup();
    let handler = image
        .http_gateway_entry(&ingress, &gateway_identity)
        .expect("canonical HTTP publication resolves its exact gateway entry");
    assert_eq!(handler.function(), linked_run.index());
    assert_eq!(
        handler.signature().parameter_types(),
        &linked_run.frame().slot_types()[..1]
    );
    assert_eq!(handler.signature().parameter_modes(), &[ParamModeIr::Value]);
    assert_eq!(
        handler.signature().parameter_plans(),
        &linked_run.frame().slot_plans()[..1]
    );
    assert_eq!(
        handler.signature().result_types(),
        linked_run.frame().result_types()
    );
    assert_eq!(
        handler.signature().result_plans(),
        linked_run.frame().result_plans()
    );

    assert_frame_slot_use(linked_run, Opcode::StoreSlot, 1);
    assert_frame_slot_use(linked_run, Opcode::LoadSlot, 1);
    assert_required_opcodes(
        linked_helper,
        &[Opcode::LoadSlot, Opcode::Const, Opcode::Add, Opcode::Return],
    );
    assert_required_opcodes(
        linked_run,
        &[
            Opcode::CallLocal,
            Opcode::StoreSlot,
            Opcode::Equal,
            Opcode::JumpIfFalse,
            Opcode::Subtract,
            Opcode::Return,
        ],
    );
    let (branch_ordinal, branch) = linked_run
        .instructions()
        .iter()
        .enumerate()
        .find(|(_, instruction)| instruction.opcode() == Opcode::JumpIfFalse)
        .expect("source if emits one conditional branch");
    let LinkedInstructionTarget::Branch(branch_target) = branch.resolved_operands()[0].target()
    else {
        panic!("conditional branch must retain a typed instruction target");
    };
    assert!(branch_target.get() as usize > branch_ordinal);
    assert!((branch_target.get() as usize) < linked_run.instructions().len());

    let validated_run = validated_function(&fixture, "main::run");
    let validated_helper = validated_function(&fixture, "main::helper");
    assert_statement_handoff(validated_run, linked_run);
    assert_statement_handoff(validated_helper, linked_helper);
}

#[test]
fn malformed_word_is_owned_by_bounded_structural_admission() {
    let fixture = PublishedService::build("malformed-structural");
    let mut artifact = fixture.bytecode.artifact().clone();
    let (function_key, function) = artifact
        .image
        .functions
        .iter_mut()
        .next()
        .expect("production compiler emits at least one function");
    let function_key = function_key.clone();
    function.words = vec![0xffff_ffff];
    function.statement_entries.clear();
    function.source_map.clear();

    let error = ValidatedBytecodeArtifact::admit(artifact)
        .expect_err("bounded artifact admission must reject an unknown opcode word");
    assert!(matches!(
        error,
        ArtifactIdentityError::InvalidBytecodeStructural(
            StructuralValidationError::Decode {
                function_key: actual_function,
                error: BytecodeDecodeError::UnknownOpcode {
                    pc: 0,
                    word: 0xffff_ffff,
                },
            },
        ) if actual_function == function_key
    ));
}

#[test]
fn bytecode_content_identity_mismatch_is_owned_by_artifact_admission() {
    let fixture = PublishedService::build("identity-mismatch");
    let mut artifact = fixture.bytecode.artifact().clone();
    let computed = artifact.bytecode_identity.clone();
    let last = artifact
        .bytecode_identity
        .pop()
        .expect("canonical identity is non-empty");
    artifact
        .bytecode_identity
        .push(if last == '0' { '1' } else { '0' });
    let declared = artifact.bytecode_identity.clone();

    let error = ValidatedBytecodeArtifact::admit(artifact)
        .expect_err("content drift must not retain the declared bytecode identity");
    assert!(matches!(
        error,
        ArtifactIdentityError::BytecodeIdentityMismatch {
            declared: actual_declared,
            computed: actual_computed,
        } if actual_declared == declared && actual_computed == computed
    ));
}

#[test]
fn reachable_pending_effect_is_rejected_by_the_link_capability_owner() {
    let fixture = PublishedService::build("reachable-effect");
    let (hydrated, package, function_key) = fixture.with_pending_effect("::run");
    let error = link_deployment_execution_image(hydrated, &production_sized_execution_limits())
        .expect_err("a reachable host effect must not enter the Phase 1 executable closure");
    let DeploymentExecutionImageError::Link(error) = error else {
        panic!("reachable host effect failed at the wrong boundary: {error}");
    };
    assert_eq!(
        error,
        BytecodeLinkError::UnsupportedPhase1Capability {
            capability: Phase1LinkedCapability::PendingEffect(PendingEffectCategory::HostEffect,),
            location: BytecodeLinkLocation::Function {
                package: Box::new(package),
                function_key,
            },
        },
    );
}

#[test]
fn design_dependent_unreachable_pending_effect_is_not_a_raw_artifact_scan() {
    let fixture = PublishedService::build("unreachable-effect");
    let (hydrated, _package, unsupported_function) = fixture.with_pending_effect("::unused");

    let image = link_deployment_execution_image(hydrated, &production_sized_execution_limits())
        .expect("an unreachable disabled function must not replace entry-closure admission");
    assert!(image.functions().iter().all(|function| {
        function.key().artifact_function_key().as_str() != unsupported_function
    }));
}

fn linked_function<'a>(image: &'a DeploymentExecutionImage, key: &str) -> &'a LinkedFunction {
    image
        .functions()
        .iter()
        .find(|function| function.key().artifact_function_key().as_str() == key)
        .unwrap_or_else(|| panic!("linked candidate contains {key}"))
}

fn validated_function<'a>(
    fixture: &'a PublishedService,
    key: &str,
) -> &'a skiff_artifact_model::ValidatedFunction {
    fixture
        .bytecode
        .view()
        .functions()
        .iter()
        .find(|function| function.function_key == key)
        .unwrap_or_else(|| panic!("validated artifact contains {key}"))
}

fn only_instruction(function: &LinkedFunction, opcode: Opcode) -> &LinkedInstruction {
    let mut matching = function
        .instructions()
        .iter()
        .filter(|instruction| instruction.opcode() == opcode);
    let instruction = matching
        .next()
        .unwrap_or_else(|| panic!("{:?} contains {opcode:?}", function.key()));
    assert!(
        matching.next().is_none(),
        "{:?} contains exactly one {opcode:?}",
        function.key()
    );
    instruction
}

fn assert_frame_slot_use(function: &LinkedFunction, opcode: Opcode, expected_slot: u32) {
    let instruction = function
        .instructions()
        .iter()
        .find(|instruction| {
            instruction.opcode() == opcode && instruction.operands() == [expected_slot]
        })
        .unwrap_or_else(|| {
            panic!(
                "{:?} contains {opcode:?} for frame slot {expected_slot}",
                function.key()
            )
        });
    assert_eq!(instruction.resolved_operands().len(), 1);
    assert_eq!(instruction.resolved_operands()[0].operand_ordinal(), 0);
    assert_eq!(
        instruction.resolved_operands()[0].target(),
        LinkedInstructionTarget::FrameSlot(skiff_runtime_linked_bytecode::FrameSlotIndex::new(
            expected_slot
        ))
    );
}

fn assert_required_opcodes(function: &LinkedFunction, required: &[Opcode]) {
    for opcode in required {
        assert!(
            function
                .instructions()
                .iter()
                .any(|instruction| instruction.opcode() == *opcode),
            "{:?} retains source-required opcode {opcode:?}",
            function.key()
        );
    }
    assert!(
        function
            .instructions()
            .iter()
            .all(|instruction| instruction.opcode() != Opcode::TailCallLocal),
        "the accepted fixture must remain an ordinary direct local call"
    );
}

fn assert_linked_type_handoff(
    image: &DeploymentExecutionImage,
    function: &LinkedFunction,
    linked_type: TypeIndex,
    artifact_type_ref: u32,
) {
    let entry = image
        .types()
        .get(linked_type.get() as usize)
        .expect("frame type index resolves in the linked image");
    assert_eq!(entry.index(), linked_type);
    assert_eq!(
        entry.origin().package_build_id(),
        function.key().package_build_id()
    );
    assert_eq!(entry.origin().artifact_index().get(), artifact_type_ref);
    assert_eq!(entry.origin().specialization(), Some(function.key()));
    assert_eq!(entry.type_ref(), &TypeRefIr::builtin("number"));
}

fn assert_statement_handoff(
    artifact: &skiff_artifact_model::ValidatedFunction,
    linked: &LinkedFunction,
) {
    assert_eq!(artifact.instructions.len(), linked.instructions().len());
    for (expected, actual) in artifact.instructions.iter().zip(linked.instructions()) {
        assert_eq!(actual.artifact_pc(), expected.pc);
        assert_eq!(actual.opcode(), expected.descriptor.kind);
        assert_eq!(actual.operands(), expected.operand_words);
    }

    assert_eq!(
        artifact.statement_entries.len(),
        linked.statement_entries().len()
    );
    for (expected, actual) in artifact
        .statement_entries
        .iter()
        .zip(linked.statement_entries())
    {
        let expected_instruction = artifact
            .instructions
            .iter()
            .position(|instruction| instruction.pc == expected.pc)
            .expect("structural admission anchors every statement row at an instruction");
        assert_eq!(actual.instruction().get() as usize, expected_instruction);
        assert_eq!(actual.sequence_ordinal(), expected.sequence_ordinal);
        assert_eq!(actual.attribution_id(), expected.attribution_id);
        assert_eq!(actual.site(), &expected.site);
    }

    let source_owned = artifact
        .statement_entries
        .iter()
        .filter(|entry| matches!(entry.site, InstructionSourceSite::Source { .. }))
        .collect::<Vec<_>>();
    assert!(
        !source_owned.is_empty(),
        "{} must retain source-owned statement rows",
        artifact.function_key
    );
    assert!(
        source_owned
            .iter()
            .any(|entry| { entry.attribution_id.class() == StatementAttributionClass::Statement }),
        "{} must retain a source-owned statement attribution",
        artifact.function_key
    );
}
