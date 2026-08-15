//! Bytecode v7 manifest ownership and cross-row structural invariants.

use crate::bytecode::dto::{
    BytecodeFunctionOrigin, BytecodePoolEntry, BytecodeRelocation, CallLoanBinding,
    LocalInterfaceMethod, LocalInterfaceRef, ParameterSlotDecl, SourceMapEntry,
    WritablePathSegment,
};

use super::*;

fn bind_helper_receiver(artifact: &mut BytecodeArtifact) {
    let helper = artifact
        .image
        .functions
        .get_mut("module::helper")
        .expect("helper fixture");
    helper.self_type_ref = Some(0);
    helper.frame_layout.parameter_slots = vec![ParameterSlotDecl {
        slot: 0,
        mode: crate::ParamModeIr::Value,
        plan: snapshot_share(),
        dense_record_shape_ref: None,
    }];

    let main = artifact
        .image
        .functions
        .get_mut("module::main")
        .expect("main fixture");
    let BytecodeRelocation::LocalExecutableRef { specialization, .. } = &mut main.relocations[0]
    else {
        unreachable!()
    };
    specialization.concrete_receiver = Some(string_type());
}

fn local_interface_relocation(target: &str) -> BytecodeRelocation {
    BytecodeRelocation::LocalInterfaceRef {
        interface: LocalInterfaceRef {
            interface: crate::InterfaceInstantiationRef {
                interface_abi_id: "interface:reader".to_string(),
                canonical_type_args: Vec::new(),
            },
            concrete_type: string_type(),
            methods: vec![LocalInterfaceMethod {
                slot: 0,
                method_name: "read".to_string(),
                method_abi_id: "method:reader:read".to_string(),
                signature: crate::InterfaceMethodSlotSignatureIr {
                    params: Vec::new(),
                    return_type: string_type(),
                },
                effects: analyzed_no_effects(),
                function_key: target.to_string(),
                receiver_call_abi: crate::ReceiverCallAbi::ExplicitSelfFirst,
            }],
        },
    }
}

#[test]
fn manifest_header_is_pinned_to_every_exact_semantic_authority() {
    for (field, corrupt) in [
        (
            "nativeValueLifecycleRegistry",
            fn_corrupt_native_registry as fn(&mut BytecodeArtifact),
        ),
        ("valueLifecyclePolicy", fn_corrupt_lifecycle_policy),
        ("hostEffectRegistry", fn_corrupt_host_registry),
        ("intrinsicRegistry", fn_corrupt_intrinsic_registry),
        (
            "platformErrorProjectionRegistry",
            fn_corrupt_platform_error_projection_registry,
        ),
    ] {
        let mut artifact = canonical_artifact();
        corrupt(&mut artifact);
        let error = assert_rejected(&artifact);
        assert!(matches!(error, StructuralValidationError::Header { .. }));
        assert!(error.to_string().contains(field), "{field}: {error}");
    }
}

fn fn_corrupt_native_registry(artifact: &mut BytecodeArtifact) {
    artifact
        .native_value_lifecycle_registry
        .fingerprint
        .push_str(":corrupt");
}

fn fn_corrupt_lifecycle_policy(artifact: &mut BytecodeArtifact) {
    artifact
        .value_lifecycle_policy
        .fingerprint
        .push_str(":corrupt");
}

fn fn_corrupt_host_registry(artifact: &mut BytecodeArtifact) {
    artifact
        .host_effect_registry
        .fingerprint
        .push_str(":corrupt");
}

fn fn_corrupt_intrinsic_registry(artifact: &mut BytecodeArtifact) {
    artifact.intrinsic_registry.fingerprint.push_str(":corrupt");
}

fn fn_corrupt_platform_error_projection_registry(artifact: &mut BytecodeArtifact) {
    artifact.platform_error_projection_registry = serde_json::from_value(serde_json::json!({
        "registryId": crate::PLATFORM_ERROR_PROJECTION_REGISTRY_ID,
        "registryVersion": crate::PLATFORM_ERROR_PROJECTION_REGISTRY_VERSION,
        "fingerprint": format!("sha256:{}", "0".repeat(64)),
    }))
    .expect("historical registry reference has valid general shape");
}

#[test]
fn executable_coordinate_wire_is_path_free_required_and_strict() {
    let coordinate = executable_coordinate(7);
    let value = serde_json::to_value(&coordinate).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "fileIrIdentity": "file-ir:module",
            "modulePath": "module",
            "executableIndex": 7
        })
    );
    assert_eq!(
        serde_json::from_value::<crate::PackageExecutableCoordinate>(value.clone()).unwrap(),
        coordinate
    );
    for field in ["fileIrIdentity", "modulePath", "executableIndex"] {
        let mut missing = value.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<crate::PackageExecutableCoordinate>(missing).is_err());
    }
    for forbidden in ["artifactPath", "sourceAstHash", "packageBuildId"] {
        let mut invalid = value.clone();
        invalid[forbidden] = serde_json::json!("forbidden");
        assert!(serde_json::from_value::<crate::PackageExecutableCoordinate>(invalid).is_err());
    }
}

#[test]
fn validated_view_retains_semantic_authority_origin_and_receiver_facts() {
    let mut artifact = canonical_artifact();
    bind_helper_receiver(&mut artifact);
    let view = structurally_validate(&artifact).expect("receiver-bound fixture");
    assert_eq!(
        view.native_value_lifecycle_registry(),
        crate::native_value_lifecycle_registry_identity()
    );
    assert_eq!(
        view.value_lifecycle_policy(),
        crate::value_lifecycle_policy_identity()
    );
    assert_eq!(
        view.host_effect_registry(),
        crate::host_effect_registry_identity()
    );
    assert_eq!(
        view.intrinsic_registry(),
        crate::intrinsic_registry_identity()
    );
    let helper = view
        .functions()
        .iter()
        .find(|function| function.function_key == "module::helper")
        .expect("validated helper");
    assert_eq!(
        helper.origin,
        artifact.image.functions["module::helper"].origin
    );
    assert_eq!(helper.self_type_ref, Some(0));
}

#[test]
fn executable_and_synthetic_origins_are_unique_and_owner_linked() {
    let mut duplicate = canonical_artifact();
    duplicate
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .origin = BytecodeFunctionOrigin::Executable {
        executable: executable_coordinate(0),
    };
    assert!(assert_rejected(&duplicate)
        .to_string()
        .contains("duplicates function"));

    let mut orphan = canonical_artifact();
    orphan
        .image
        .functions
        .get_mut("module::main$callback0")
        .unwrap()
        .origin = BytecodeFunctionOrigin::SyntheticCallback {
        owner: executable_coordinate(99),
        site_ordinal: 0,
    };
    assert!(assert_rejected(&orphan)
        .to_string()
        .contains("owner has no ordinary executable origin"));

    let mut duplicate_site = canonical_artifact();
    let mut callback = callback_function();
    callback.function_key = "module::main$callback1".to_string();
    duplicate_site
        .image
        .functions
        .insert(callback.function_key.clone(), callback);
    assert!(assert_rejected(&duplicate_site)
        .to_string()
        .contains("synthetic callback origin duplicates"));
}

#[test]
fn origins_reject_noncanonical_coordinates_and_synthetic_receivers() {
    let mut wrong_prefix = canonical_artifact();
    let BytecodeFunctionOrigin::Executable { executable } = &mut wrong_prefix
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .origin
    else {
        unreachable!()
    };
    executable.module_path = "other".to_string();
    assert!(assert_rejected(&wrong_prefix)
        .to_string()
        .contains("canonical module prefix"));

    let mut synthetic_receiver = canonical_artifact();
    synthetic_receiver
        .image
        .functions
        .get_mut("module::main$callback0")
        .unwrap()
        .self_type_ref = Some(0);
    assert!(assert_rejected(&synthetic_receiver)
        .to_string()
        .contains("synthetic callback must not declare selfTypeRef"));
}

#[test]
fn receiver_layout_and_local_specialization_presence_are_exact() {
    let mut missing_layout = canonical_artifact();
    missing_layout
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .self_type_ref = Some(0);
    assert!(assert_rejected(&missing_layout)
        .to_string()
        .contains("incoming parameter ordinal 0"));

    let mut unexpected_receiver = canonical_artifact();
    let BytecodeRelocation::LocalExecutableRef { specialization, .. } = &mut unexpected_receiver
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations[0]
    else {
        unreachable!()
    };
    specialization.concrete_receiver = Some(string_type());
    assert!(assert_rejected(&unexpected_receiver)
        .to_string()
        .contains("concreteReceiver presence"));

    let mut exact = canonical_artifact();
    bind_helper_receiver(&mut exact);
    assert_validates(&exact);
}

#[test]
fn callback_and_interface_rows_only_target_their_declared_function_kinds() {
    let mut bad_callback_relocation = canonical_artifact();
    let BytecodeRelocation::SyntheticCallbackRef { function_key } = &mut bad_callback_relocation
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations[3]
    else {
        unreachable!()
    };
    *function_key = "module::helper".to_string();
    assert!(assert_rejected(&bad_callback_relocation)
        .to_string()
        .contains("must target a synthetic callback function"));

    let mut bad_capture = canonical_artifact();
    let BytecodePoolEntry::CallbackCaptureLayout(layout) =
        &mut bad_capture.image.pools.callback_capture[0]
    else {
        unreachable!()
    };
    layout.function_key = "module::helper".to_string();
    assert!(assert_rejected(&bad_capture)
        .to_string()
        .contains("must target a synthetic callback function"));

    let mut bad_interface = canonical_artifact();
    bad_interface
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations
        .push(local_interface_relocation("module::helper"));
    assert!(assert_rejected(&bad_interface)
        .to_string()
        .contains("target must be receiver-bound"));

    let mut exact = canonical_artifact();
    bind_helper_receiver(&mut exact);
    exact
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .relocations
        .push(local_interface_relocation("module::helper"));
    assert_validates(&exact);
}

#[test]
fn canonical_constant_roots_reject_aliases_and_unqualified_names() {
    let mut unqualified = canonical_artifact();
    unqualified
        .image
        .constant_roots
        .insert("constant".to_string(), 0);
    assert!(assert_rejected(&unqualified)
        .to_string()
        .contains("canonical module-qualified"));

    let mut alias = canonical_artifact();
    alias
        .image
        .constant_roots
        .insert("module.alias".to_string(), 0);
    assert!(assert_rejected(&alias)
        .to_string()
        .contains("aliases constants pool row"));
}

#[test]
fn writable_locals_are_canonical_bounded_and_never_parameters() {
    let mut duplicate = canonical_artifact();
    duplicate
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .writable_local_slots = vec![1, 1];
    assert!(assert_rejected(&duplicate)
        .to_string()
        .contains("strictly increasing"));

    let mut parameter = canonical_artifact();
    parameter
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .writable_local_slots = vec![0, 1];
    assert!(assert_rejected(&parameter)
        .to_string()
        .contains("incoming parameter slot"));

    let mut out_of_bounds = canonical_artifact();
    out_of_bounds
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .writable_local_slots = vec![4];
    assert!(assert_rejected(&out_of_bounds)
        .to_string()
        .contains("out of bounds"));
}

#[test]
fn parameter_dense_record_shape_ref_is_nullable_but_bounded_and_kind_checked() {
    let mut present = canonical_artifact();
    present
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .parameter_slots[0]
        .dense_record_shape_ref = Some(0);
    assert_validates(&present);

    let mut out_of_bounds = canonical_artifact();
    out_of_bounds
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .frame_layout
        .parameter_slots[0]
        .dense_record_shape_ref = Some(u32::MAX);
    assert!(assert_rejected(&out_of_bounds)
        .to_string()
        .contains("denseRecordShapeRef"));

    let mut wrong_kind = present;
    wrong_kind.image.pools.shapes[0] = type_entry(string_type());
    assert!(assert_rejected(&wrong_kind)
        .to_string()
        .contains("incompatible entry kind"));
}

#[test]
fn call_loan_layouts_require_nonempty_ordered_exact_writable_roots() {
    let mut empty = canonical_artifact();
    empty
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .call_loan_layouts[0]
        .loans
        .clear();
    assert!(assert_rejected(&empty)
        .to_string()
        .contains("loans must not be empty"));

    let mut duplicate_parameter = canonical_artifact();
    let layout = &mut duplicate_parameter
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .call_loan_layouts[0];
    layout.loans.push(layout.loans[0].clone());
    assert!(assert_rejected(&duplicate_parameter)
        .to_string()
        .contains("parameterOrdinal"));

    let mut undeclared_root = canonical_artifact();
    undeclared_root
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .call_loan_layouts[0]
        .loans[0]
        .root_slot = 2;
    assert!(assert_rejected(&undeclared_root)
        .to_string()
        .contains("not a declared writable local"));

    let mut bad_path = canonical_artifact();
    bad_path
        .image
        .functions
        .get_mut("module::main")
        .unwrap()
        .call_loan_layouts[0]
        .loans[0]
        .writable_path_ref = 99;
    assert!(assert_rejected(&bad_path)
        .to_string()
        .contains("does not select a WritablePath"));

    let mut wrong_root_type = canonical_artifact();
    let main = wrong_root_type
        .image
        .functions
        .get_mut("module::main")
        .unwrap();
    main.frame_layout.writable_local_slots = vec![1, 2];
    main.call_loan_layouts[0].loans[0].root_slot = 2;
    assert!(assert_rejected(&wrong_root_type)
        .to_string()
        .contains("does not match writable path rootTypeRef"));
}

#[test]
fn call_local_inout_checks_table_and_selector_count_without_guessing_values() {
    let mut artifact = canonical_artifact();
    artifact.image.pools.types.push(BytecodePoolEntry::TypeRef {
        ty: TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![string_type()],
        },
        representation_carrier: None,
        plan: ValueTransferPlan::SnapshotShare {
            drop: ValueDropPlan::SnapshotRelease,
        },
    });
    let BytecodePoolEntry::WritablePath(path) = &mut artifact.image.pools.writable_paths[0] else {
        unreachable!()
    };
    path.root_type_ref = 2;
    path.leaf_type_ref = 0;
    path.segments = vec![WritablePathSegment::ArrayIndex {
        selector_ordinal: 0,
        element_type_ref: 0,
    }];
    let main = artifact.image.functions.get_mut("module::main").unwrap();
    main.frame_layout.slot_type_refs[1] = 2;
    main.exception_regions[0].catch_slot_type_ref = 2;

    let helper = artifact.image.functions.get_mut("module::helper").unwrap();
    helper.words = vec![0x26, 0, 1, 0, 0, 0x25];
    helper.relocations = vec![BytecodeRelocation::LocalExecutableRef {
        function_key: "module::helper".to_string(),
        specialization: crate::bytecode::dto::BytecodeSpecialization {
            type_arguments: vec![string_type()],
            concrete_receiver: None,
        },
    }];
    helper.frame_layout.slot_type_refs[0] = 2;
    helper.frame_layout.writable_local_slots = vec![0];
    helper.call_loan_layouts = vec![crate::bytecode::dto::CallLoanLayout {
        loans: vec![CallLoanBinding {
            parameter_ordinal: 0,
            root_slot: 0,
            writable_path_ref: 0,
        }],
    }];
    helper.statement_entries[0].attribution_id = StatementAttributionId::Expression {
        expression_index: 0,
        occurrence_ordinal: 0,
    };
    helper.source_map = vec![SourceMapEntry {
        start_pc: 0,
        end_pc: 5,
        site: crate::InstructionSourceSite::Synthetic {
            reason: crate::SyntheticInstructionSiteReason::CompilerGeneratedWrapper,
        },
    }];
    assert_validates(&artifact);

    let mut insufficient = artifact.clone();
    insufficient
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .words[2] = 0;
    assert!(assert_rejected(&insufficient)
        .to_string()
        .contains("smaller than call loan selector count"));

    let mut missing_source = artifact.clone();
    missing_source
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .source_map
        .clear();
    assert!(assert_rejected(&missing_source)
        .to_string()
        .contains("requires exactly one source/synthetic site"));

    let mut missing_layout = artifact;
    missing_layout
        .image
        .functions
        .get_mut("module::helper")
        .unwrap()
        .words[4] = 1;
    assert!(assert_rejected(&missing_layout)
        .to_string()
        .contains("callLoanLayouts table"));
}
