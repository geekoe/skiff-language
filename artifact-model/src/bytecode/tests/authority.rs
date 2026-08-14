//! Canonical execution authority projection tests: function stream item
//! authority and exact intrinsic result/resume contracts.

use crate::bytecode::authority::{FunctionStreamEndContract, IntrinsicResumeContract};
use crate::bytecode::dto::{
    BytecodeFunctionOrigin, BytecodeIntrinsicRef, BytecodePoolEntry, BytecodeRelocation,
    FrameLayout, HostEffectSignature, IntrinsicReference, ResumeDescriptor, ResumeErrorMode,
    StatementAttributionId, StatementEntry, ValueDropPlan, ValueTransferPlan,
};

use super::*;

fn stream_producer_artifact() -> BytecodeArtifact {
    let mut artifact = canonical_artifact();
    let stream_index = artifact.image.pools.types.len() as u32;
    artifact.image.pools.types.push(BytecodePoolEntry::TypeRef {
        ty: TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![string_type()],
        },
    });
    artifact
        .image
        .pools
        .resume
        .push(BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
            function_key: "module::producer".to_string(),
            site_pc: 0,
            resume_pc: 2,
            end_resume_pc: None,
            expected_stack_height_before_result: 0,
            result_type_refs: Vec::new(),
            result_plans: Vec::new(),
            result_materializations: Vec::new(),
            error_mode: ResumeErrorMode::RaiseAtSite,
        }));

    let mut producer = callback_function();
    producer.function_key = "module::producer".to_string();
    producer.origin = BytecodeFunctionOrigin::Executable {
        executable: executable_coordinate(2),
    };
    producer.words = vec![0x61, 1, 0x25];
    producer.relocations = Vec::new();
    producer.frame_layout = FrameLayout {
        slot_count: 0,
        slot_type_refs: Vec::new(),
        parameter_slots: Vec::new(),
        writable_local_slots: Vec::new(),
        result_count: 0,
        result_type_refs: Vec::new(),
        result_plans: Vec::new(),
        stream_result_type_ref: Some(stream_index),
        slot_plans: Vec::new(),
    };
    producer.max_operand_depth = 1;
    producer.effect_summary_ref = crate::PackageCallableId::new("operation:module:producer");
    producer.exception_regions = Vec::new();
    producer.active_regions = Vec::new();
    producer.switch_tables = Vec::new();
    producer.statement_entries = vec![StatementEntry {
        pc: 0,
        sequence_ordinal: 0,
        attribution_id: StatementAttributionId::Generated { ordinal: 0 },
        site: statement_synthetic_site(),
    }];
    producer.source_map = vec![source_map_synthetic(0, 2)];
    artifact
        .image
        .functions
        .insert("module::producer".to_string(), producer);
    artifact
}

fn stream_consumer_artifact() -> BytecodeArtifact {
    let mut artifact = canonical_artifact();
    artifact
        .image
        .pools
        .resume
        .push(BytecodePoolEntry::ResumeDescriptor(ResumeDescriptor {
            function_key: "module::consumer".to_string(),
            site_pc: 0,
            resume_pc: 3,
            end_resume_pc: Some(4),
            expected_stack_height_before_result: 0,
            result_type_refs: vec![0],
            result_plans: vec![snapshot_share()],
            result_materializations: vec![None],
            error_mode: ResumeErrorMode::RaiseAtSite,
        }));

    let mut consumer = callback_function();
    consumer.function_key = "module::consumer".to_string();
    consumer.origin = BytecodeFunctionOrigin::Executable {
        executable: executable_coordinate(2),
    };
    consumer.words = vec![0x60, 0, 1, 0x08, 0x25];
    consumer.relocations = Vec::new();
    consumer.frame_layout = FrameLayout {
        slot_count: 1,
        slot_type_refs: vec![0],
        parameter_slots: Vec::new(),
        writable_local_slots: Vec::new(),
        result_count: 0,
        result_type_refs: Vec::new(),
        result_plans: Vec::new(),
        stream_result_type_ref: None,
        slot_plans: vec![snapshot_share()],
    };
    consumer.max_operand_depth = 1;
    consumer.effect_summary_ref = crate::PackageCallableId::new("operation:module:consumer");
    consumer.exception_regions = Vec::new();
    consumer.active_regions = Vec::new();
    consumer.switch_tables = Vec::new();
    consumer.statement_entries = vec![StatementEntry {
        pc: 0,
        sequence_ordinal: 0,
        attribution_id: StatementAttributionId::Generated { ordinal: 0 },
        site: statement_synthetic_site(),
    }];
    consumer.source_map = vec![source_map_synthetic(0, 4)];
    artifact
        .image
        .functions
        .insert("module::consumer".to_string(), consumer);
    artifact
}

fn intrinsic_reference() -> IntrinsicReference {
    let entry = crate::intrinsic_registry()
        .entries()
        .iter()
        .find(|entry| {
            matches!(
                &entry.target,
                crate::BytecodeIntrinsicRef::Static { canonical_key, .. }
                    if canonical_key == "core.array.empty"
            )
        })
        .expect("intrinsic registry contains core.array.empty");
    IntrinsicReference {
        target: entry.target.clone(),
        signature: HostEffectSignature {
            parameter_types: Vec::new(),
            parameter_modes: Vec::new(),
            parameter_plans: Vec::new(),
            result_types: vec![TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![string_type()],
            }],
            result_plans: vec![ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            }],
            effects: entry.signature.effects.clone(),
        },
    }
}

fn artifact_with_intrinsic() -> BytecodeArtifact {
    let mut artifact = canonical_artifact();
    artifact
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations
        .push(BytecodeRelocation::IntrinsicRef {
            intrinsic: intrinsic_reference(),
        });
    artifact
}

#[test]
fn validated_view_derives_exact_function_stream_item_authority() {
    let artifact = stream_producer_artifact();
    assert_validates(&artifact);
    let view = structurally_validate(&artifact).unwrap();

    let stream_items = view.function_stream_items();
    assert_eq!(stream_items.len(), 1);
    assert_eq!(stream_items[0].function_key, "module::producer");
    assert_eq!(stream_items[0].authority.item_type, string_type());
    assert_eq!(
        stream_items[0].authority.end,
        FunctionStreamEndContract::NormalExit
    );

    let emit = view
        .resume_sites()
        .iter()
        .find(|site| site.function_key == "module::producer")
        .expect("producer emit_stream resume site");
    let item = emit
        .stream_item
        .as_ref()
        .expect("emit_stream item authority");
    assert_eq!(item.item_type, string_type());

    let resume_authority = emit.result_authority();
    assert_eq!(
        resume_authority
            .stream_item
            .as_ref()
            .expect("resume result authority stream item")
            .item_type,
        string_type()
    );
}

#[test]
fn emit_stream_requires_the_function_stream_authority() {
    let mut artifact = stream_producer_artifact();
    artifact
        .image
        .functions
        .get_mut("module::producer")
        .unwrap()
        .frame_layout
        .stream_result_type_ref = None;
    let error = assert_rejected(&artifact);
    assert!(error
        .to_string()
        .contains("exact function stream item authority"));
}

#[test]
fn ordinary_result_frame_is_not_derived_as_stream_producer() {
    let artifact = canonical_artifact();
    assert_validates(&artifact);
    let view = structurally_validate(&artifact).unwrap();

    assert_eq!(view.function_stream_items().len(), 0);
}

#[test]
fn stream_producer_requires_zero_return_arity_and_exact_stream_type() {
    let mut wrong_type = stream_producer_artifact();
    wrong_type
        .image
        .functions
        .get_mut("module::producer")
        .unwrap()
        .frame_layout
        .stream_result_type_ref = Some(0);
    let error = assert_rejected(&wrong_type);
    assert!(
        error.to_string().contains("must select Stream<T>"),
        "{error}"
    );

    let mut non_zero_return = stream_producer_artifact();
    let stream_index = non_zero_return
        .image
        .functions
        .get_mut("module::producer")
        .unwrap()
        .frame_layout
        .stream_result_type_ref
        .expect("producer authority");
    let frame = &mut non_zero_return
        .image
        .functions
        .get_mut("module::producer")
        .unwrap()
        .frame_layout;
    frame.result_count = 1;
    frame.result_type_refs = vec![stream_index];
    frame.result_plans = vec![snapshot_share()];
    let error = assert_rejected(&non_zero_return);
    assert!(
        error.to_string().contains("resultCount must be 0"),
        "{error}"
    );
}

#[test]
fn stream_return_type_requires_explicit_producer_declaration() {
    let mut artifact = canonical_artifact();
    let stream_index = artifact.image.pools.types.len() as u32;
    artifact.image.pools.types.push(BytecodePoolEntry::TypeRef {
        ty: TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![string_type()],
        },
    });
    artifact
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .result_type_refs[0] = stream_index;

    let error = assert_rejected(&artifact);
    assert!(
        error
            .to_string()
            .contains("must declare frameLayout.streamResultTypeRef"),
        "{error}"
    );
}

#[test]
fn stream_next_exposes_item_and_natural_end_resume_paths() {
    let artifact = stream_consumer_artifact();
    assert_validates(&artifact);
    let view = structurally_validate(&artifact).unwrap();

    let site = view
        .resume_sites()
        .iter()
        .find(|site| site.function_key == "module::consumer")
        .expect("stream_next resume site");
    assert_eq!(site.resume_pc, 3);
    assert_eq!(site.end_resume_pc, Some(4));
    assert_eq!(site.stream_item, None);

    let authority = site.result_authority();
    assert_eq!(authority.end_resume_pc, Some(4));
    assert_eq!(authority.result_type_refs, vec![0]);
}

#[test]
fn stream_next_requires_explicit_end_resume_pc() {
    let mut artifact = stream_consumer_artifact();
    let BytecodePoolEntry::ResumeDescriptor(descriptor) = &mut artifact.image.pools.resume[1]
    else {
        unreachable!("stream consumer resume descriptor")
    };
    descriptor.end_resume_pc = None;

    let error = assert_rejected(&artifact);
    assert!(
        error.to_string().contains("requires endResumePc"),
        "{error}"
    );
}

#[test]
fn non_stream_resume_cannot_declare_end_resume_pc() {
    let mut artifact = canonical_artifact();
    let BytecodePoolEntry::ResumeDescriptor(descriptor) = &mut artifact.image.pools.resume[0]
    else {
        unreachable!("canonical resume descriptor")
    };
    descriptor.end_resume_pc = Some(27);

    let error = assert_rejected(&artifact);
    assert!(
        error.to_string().contains("only valid for StreamNext"),
        "{error}"
    );
}

#[test]
fn stream_next_end_resume_pc_must_be_distinct_instruction_header() {
    let mut duplicate = stream_consumer_artifact();
    let BytecodePoolEntry::ResumeDescriptor(descriptor) = &mut duplicate.image.pools.resume[1]
    else {
        unreachable!("stream consumer resume descriptor")
    };
    descriptor.end_resume_pc = Some(3);
    let error = assert_rejected(&duplicate);
    assert!(
        error.to_string().contains("must differ from item resumePc"),
        "{error}"
    );

    let mut not_header = stream_consumer_artifact();
    let BytecodePoolEntry::ResumeDescriptor(descriptor) = &mut not_header.image.pools.resume[1]
    else {
        unreachable!("stream consumer resume descriptor")
    };
    descriptor.end_resume_pc = Some(2);
    let error = assert_rejected(&not_header);
    assert!(
        error.to_string().contains("is not an instruction header"),
        "{error}"
    );
}

#[test]
fn validated_view_derives_never_resume_contract_for_synchronous_intrinsic() {
    let artifact = artifact_with_intrinsic();
    assert_validates(&artifact);
    let view = structurally_validate(&artifact).unwrap();

    let contracts = view.intrinsic_contracts();
    assert_eq!(contracts.len(), 1);
    assert_eq!(contracts[0].function_key, "module::main");
    assert_eq!(contracts[0].plan.resume, IntrinsicResumeContract::Never);
    assert_eq!(contracts[0].plan.result_types.len(), 1);
    assert_eq!(contracts[0].plan.result_plans.len(), 1);
    assert!(matches!(
        contracts[0].target,
        BytecodeIntrinsicRef::Static { .. }
    ));
}

#[test]
fn pending_intrinsic_without_exact_resume_contract_fails_closed() {
    let mut artifact = artifact_with_intrinsic();
    let BytecodeRelocation::IntrinsicRef { intrinsic } = artifact
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations
        .last_mut()
        .unwrap()
    else {
        unreachable!();
    };
    intrinsic.signature.effects = crate::CallableMayEffects {
        escapes_caller_value: false,
        requires_same_heap_identity: false,
        invokes_unknown_target: false,
        may_pending: true,
        pending_effect_categories: vec![crate::PendingEffectCategory::NativeCall],
        inout_path_effects: Vec::new(),
    };
    let error = assert_rejected(&artifact);
    assert!(error.to_string().contains("must be NoPending"));
}
