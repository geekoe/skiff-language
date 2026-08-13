use std::collections::BTreeMap;

use skiff_artifact_model::{ValueDropPlan, ValueTransferPlan};

use super::*;

#[test]
fn phase_1_bytecode_admission_rejects_string_shape_with_typed_error() {
    let error = compile_phase_1_source(
        "example.com/bytecode-phase1-string",
        "function run() -> string { return \"disabled\" }\n",
    )
    .unwrap_err();

    let PackageCompileError::BytecodeEmission {
        source:
            crate::BytecodeEmissionError::UnsupportedPhase1Capability {
                capability,
                module_path,
                function_key,
                location,
            },
    } = error
    else {
        panic!("expected typed Phase 1 capability rejection, got {error:?}");
    };
    assert_eq!(capability, crate::Phase1UnsupportedCapability::ValueShape);
    assert_eq!(module_path, "main");
    assert_eq!(function_key.as_deref(), Some("main::run"));
    assert_eq!(location, "return type");
}

#[test]
fn phase_1_bytecode_admission_blocks_public_emitter_string_array_bypass() {
    let error = compile_phase_1_source(
        "example.com/bytecode-phase1-string-array",
        "function run() -> Array<string> { return [] }\n",
    )
    .unwrap_err();

    let PackageCompileError::BytecodeEmission {
        source:
            crate::BytecodeEmissionError::UnsupportedConstruct {
                function_key,
                construct,
                location,
            },
    } = error
    else {
        panic!("expected typed Phase 2 array rejection, got {error:?}");
    };
    assert_eq!(construct, "phase 2 record/array value shape");
    assert_eq!(function_key, "main::run");
    assert_eq!(location, " return type element type (ValueShape)");
}

#[test]
fn phase_1_bytecode_admission_rejects_tail_local_call_with_typed_error() {
    let error = compile_phase_1_source(
        "example.com/bytecode-phase1-tail",
        "function helper(value: number) -> number { return value + 1 }\nfunction run(value: number) -> number { return helper(value) }\n",
    )
    .unwrap_err();

    let PackageCompileError::BytecodeEmission {
        source:
            crate::BytecodeEmissionError::UnsupportedPhase1Capability {
                capability,
                module_path,
                function_key,
                location,
            },
    } = error
    else {
        panic!("expected typed Phase 1 capability rejection, got {error:?}");
    };
    assert_eq!(capability, crate::Phase1UnsupportedCapability::TailCall);
    assert_eq!(module_path, "main");
    assert_eq!(function_key.as_deref(), Some("main::run"));
    assert_eq!(location, "statement 0");
}

#[test]
fn phase_2_bytecode_admission_persists_exact_source_plans_in_frame_layouts() {
    let output = compile_phase_1_source(
        "example.com/bytecode-phase2-exact-plans",
        "function helper(value: number) -> number { return value + 5 }\nfunction run(value: number) -> number { final result = helper(value) if result == 7 { return result - 4 } return 0 }\n",
    )
    .expect("scalar/local-call fixture compiles through the exact plan authority");
    let handoff = output.bytecode_handoff().expect("enabled bytecode handoff");
    let number_plan = ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::Trivial,
    };

    let run = &handoff.artifact().image.functions["main::run"];
    assert_eq!(
        run.frame_layout.slot_plans,
        vec![number_plan.clone(), number_plan.clone()]
    );
    assert_eq!(run.frame_layout.result_plans, vec![number_plan.clone()]);

    let helper = &handoff.artifact().image.functions["main::helper"];
    assert_eq!(helper.frame_layout.slot_plans, vec![number_plan.clone()]);
    assert_eq!(helper.frame_layout.result_plans, vec![number_plan]);
}

#[test]
fn phase_2_bytecode_admission_publishes_nested_record_array_fixture() {
    let output = compile_phase_1_source(
        "example.com/bytecode-phase2-record-array",
        "type Inner {\n  x: number,\n  tags: Array<number>,\n}\ntype Payload {\n  inner: Inner,\n  rows: Array<Inner>,\n}\nfunction stamp(value: Payload) -> Payload { return value }\nfunction run(seed: number) -> Payload {\n  final rows = [Inner { x: seed, tags: [1, 2] }]\n  final payload = Payload { inner: Inner { x: seed, tags: [1, 2] }, rows: rows }\n  final carried = stamp(payload)\n  return carried\n}\n",
    )
    .expect("nested record/array construction and transfer admit and emit");
    let handoff = output.bytecode_handoff().expect("enabled bytecode handoff");
    let view = skiff_artifact_model::bytecode::structurally_validate(handoff.artifact())
        .expect("accepted record/array artifact remains structurally valid");
    assert!(view
        .functions()
        .iter()
        .any(|function| function.function_key == "main::run"));
    assert_eq!(
        handoff.artifact().image.functions["main::run"]
            .frame_layout
            .slot_plans
            .iter()
            .filter(|plan| matches!(
                plan,
                ValueTransferPlan::SnapshotShare {
                    drop: ValueDropPlan::SnapshotRelease,
                }
            ))
            .count(),
        3,
        "record/array slots carry the exact snapshot-release source plans"
    );
}

#[test]
fn phase_2_bytecode_admission_rejects_record_with_string_field() {
    let error = compile_phase_1_source(
        "example.com/bytecode-phase2-string-field",
        "type Bad {\n  label: string,\n}\nfunction run(value: number) -> Bad {\n  final item = Bad { label: \"unplanned\" }\n  return item\n}\n",
    )
    .unwrap_err();

    let PackageCompileError::BytecodeEmission {
        source:
            crate::BytecodeEmissionError::UnsupportedConstruct {
                function_key,
                construct,
                location,
            },
    } = error
    else {
        panic!("expected the stable Phase 2 nested-shape rejection, got {error:?}");
    };
    assert_eq!(construct, "phase 2 record/array value shape");
    assert_eq!(function_key, "main::run");
    assert_eq!(location, " return type field `label` (ValueShape)");
}

#[test]
fn phase_2_bytecode_admission_crosses_effects_for_nested_writable_path() {
    let error = compile_phase_1_source(
        "example.com/bytecode-phase2-writable-mutation",
        "type Inner {\n  x: number,\n  tags: Array<number>,\n}\ntype Payload {\n  inner: Inner,\n  rows: Array<Inner>,\n}\ntype Probe {\n  original: Payload,\n  mutated: Payload,\n}\nfunction stamp(value: Payload) -> Payload { return value }\nfunction run(seed: number) -> Probe {\n  final a = Payload {\n    inner: Inner { x: seed, tags: [1, 2] },\n    rows: [],\n  }\n  var b = a\n  b.inner.x = 2\n  final carried = stamp(a)\n  return Probe { original: carried, mutated: b }\n}\n",
    )
    .unwrap_err();

    // The nested writable path now crosses the callable-effects boundary. The
    // next compiler-side gate is lowering-owned source-event availability for
    // the mutation expression, outside the C2 effects write boundary.
    let PackageCompileError::BytecodeEmission {
        source:
            crate::BytecodeEmissionError::Phase1SourceEventsUnavailable {
                module_path,
                function_key,
                reason: crate::MirSourceEventUnavailableReason::SourceEventNotRepresentable { .. },
            },
    } = error
    else {
        panic!("expected the post-effects source-event boundary, got {error:?}");
    };
    assert_eq!(module_path, "main");
    assert_eq!(function_key, "main::run");
}

fn compile_phase_1_source(
    package_id: &str,
    text: &str,
) -> Result<PackageCompileOutput, PackageCompileError> {
    let repository_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("compiler manifest must have a repository parent")
        .to_path_buf();
    let platform_sources =
        crate::CompilerPlatformSources::new(&repository_root).expect("repository platform sources");
    let temp = std::env::temp_dir().join(format!(
        "skiff-phase1-admission-{}-{}",
        std::process::id(),
        package_id
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character
                } else {
                    '-'
                }
            })
            .collect::<String>(),
    ));
    std::fs::create_dir_all(&temp).expect("create temporary source root");
    let source_path = temp.join("main.skiff");
    std::fs::write(&source_path, text).expect("write temporary source");
    let source_tree = crate::SourceTree {
        root: temp.clone(),
        sources: vec![crate::SourceTreeFile {
            module_path: "main".to_string(),
            file_path: std::path::PathBuf::from("main.skiff"),
            is_test_file: false,
            byte_len: text.len() as u64,
        }],
    };
    let compiler_source = skiff_compiler_source::source_graph::CompilerSourceFile::parse(
        std::path::PathBuf::from("main.skiff"),
        "main".to_string(),
        false,
        false,
        text.to_string(),
        source_path.display().to_string(),
    )
    .expect("parse Phase 1 source fixture");
    let package = crate::PackageSourceInput::new(
        crate::PublicationManifest::new(
            skiff_compiler_core::id::PublicationId::parse(package_id)
                .expect("valid fixture package id"),
            "1.0.0".to_string(),
            skiff_compiler_input::PublicationApiSpec::empty(),
            Vec::new(),
            crate::ManifestProvenance {
                owner: crate::ManifestOwner::UserOrBuiltinPackage,
                path: std::path::PathBuf::new(),
                synthetic: true,
            },
        ),
        source_tree,
        crate::PublicationSourceGraph::from_compiler_sources(vec![compiler_source]),
        Vec::new(),
    );
    let aliases = BTreeMap::new();
    let result = compile_package(PackageCompileInput::new(
        &platform_sources,
        &package,
        &aliases,
        package_id,
        true,
    ));
    std::fs::remove_dir_all(temp).expect("remove temporary source root");
    result
}
