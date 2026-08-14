use std::path::PathBuf;

use skiff_artifact_model::{
    bytecode::BoundedDecoder, BytecodePoolEntry, BytecodeRelocation, HostEffectExecutorIdentity,
    Opcode, PrivilegedAffineCompositeIdentity, ResourceDropPlan, ValueDropPlan, ValueTransferPlan,
};
use skiff_compiler_lowering::{
    mir::source_program::{lower_single_source_program, SingleSourceProgram},
    Bounds, ConstEvaluator, LoweredPackage,
};

use crate::{
    admit_phase_1_bytecode_mir, derive_bytecode_value_transfer_plans, emit_bytecode_artifact,
    BytecodeEmissionError, Phase1UnsupportedCapability,
};

const PACKAGE_ID: &str = "example.com/phase5-host-effect-source";

fn lower(source: &str) -> LoweredPackage {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    lower_single_source_program(SingleSourceProgram {
        platform_root: &platform_root,
        package_id: PACKAGE_ID,
        module_path: "main",
        relative_path: "main.skiff",
        source,
    })
    .expect("real Phase 5 source lowers through the production source/MIR API")
}

#[test]
fn exact_registry_executors_flow_from_real_source_to_public_emission() {
    let lowered = lower(
        r#"
import std

function sleeping() -> void {
  std.time.sleep(Duration.milliseconds(1))
}

function request(input: std.http.HttpClientRequest) -> void {
  std.http.request(input)
}

function stream(input: std.http.HttpClientRequest) -> void {
  std.http.stream(input)
}
"#,
    );
    let admitted = admit_phase_1_bytecode_mir(lowered.mir_units())
        .expect("the three registry-owned executor identities are admitted");
    let plans = derive_bytecode_value_transfer_plans(&admitted, |_module_path, ty| {
        Ok(ValueTransferPlan::FromType { ty: ty.clone() })
    })
    .expect("the public plan boundary receives exact source types");
    let bundles = lowered
        .file_ir_units()
        .iter()
        .map(|unit| {
            ConstEvaluator::new(Bounds::default())
                .evaluate_unit(unit)
                .expect("constant-free source has an exact empty bundle")
        })
        .collect::<Vec<_>>();
    let artifact = emit_bytecode_artifact(&admitted, &bundles, &plans)
        .expect("public emission consumes the admission proof");

    let mut effects = artifact
        .image
        .functions
        .values()
        .flat_map(|function| &function.relocations)
        .filter_map(|relocation| match relocation {
            BytecodeRelocation::HostEffectRef(effect) => Some(effect),
            _ => None,
        })
        .collect::<Vec<_>>();
    effects.sort_by(|left, right| {
        left.target
            .binding_key
            .as_deref()
            .cmp(&right.target.binding_key.as_deref())
    });
    assert_eq!(
        effects
            .iter()
            .map(|effect| effect.target.binding_key.as_deref().unwrap())
            .collect::<Vec<_>>(),
        vec![
            "std.http.client.request",
            "std.http.client.stream",
            "std.time.sleep",
        ]
    );
    let identities = effects
        .iter()
        .map(|effect| {
            skiff_artifact_model::host_effect_registry()
                .entries()
                .iter()
                .find(|entry| {
                    effect.target.binding_key.as_deref() == Some(entry.binding_key.as_str())
                })
                .and_then(|entry| entry.executor_identity)
                .expect("every emitted call resolves to a closed executor identity")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        identities,
        vec![
            HostEffectExecutorIdentity::HttpClientRequest,
            HostEffectExecutorIdentity::HttpClientStream,
            HostEffectExecutorIdentity::Sleep,
        ]
    );
    assert_eq!(
        artifact
            .image
            .functions
            .values()
            .map(|function| {
                BoundedDecoder::new()
                    .decode_function(&function.words)
                    .expect("public emission produces decodable wordcode")
            })
            .flat_map(|function| function.instructions)
            .filter(|instruction| instruction.descriptor.kind == Opcode::InvokeHost)
            .count(),
        3
    );
}

#[test]
fn exact_stream_body_flows_from_real_source_to_affine_take_and_recursive_drop() {
    let lowered = lower(
        r#"
import std

function consume(input: std.http.HttpClientRequest) -> void {
  final response = std.http.stream(input)
  for chunk in response.body {
  }
}
"#,
    );
    let admitted = admit_phase_1_bytecode_mir(lowered.mir_units())
        .expect("exact host result/body/for-in MIR facts are admitted");
    let plans = derive_bytecode_value_transfer_plans(&admitted, |_module_path, ty| {
        Ok(ValueTransferPlan::FromType { ty: ty.clone() })
    })
    .expect("source plans defer only the pool-local privileged shape binding");
    let bundles = lowered
        .file_ir_units()
        .iter()
        .map(|unit| {
            ConstEvaluator::new(Bounds::default())
                .evaluate_unit(unit)
                .expect("constant-free source has an exact empty bundle")
        })
        .collect::<Vec<_>>();
    let artifact = emit_bytecode_artifact(&admitted, &bundles, &plans)
        .expect("public emission closes the exact privileged shape authority");
    let function = artifact
        .image
        .functions
        .get("main::consume")
        .expect("real source function is published under its canonical key");
    let decoded = BoundedDecoder::new()
        .decode_function(&function.words)
        .expect("affine source emits decodable wordcode");
    let take = decoded
        .instructions
        .windows(2)
        .find(|pair| {
            pair[0].descriptor.kind == Opcode::TakeSlot
                && pair[1].descriptor.kind == Opcode::TakeDenseField
        })
        .expect("body projection is one adjacent consume-whole affine take");
    let shape_ref = take[1].operand(0);
    assert_eq!(take[1].operand(1), 0, "body is exact dense ordinal zero");
    let Some(BytecodePoolEntry::ShapeRef { shape }) =
        artifact.image.pools.shapes.get(shape_ref as usize)
    else {
        panic!("TakeDenseField does not reference a shape declaration")
    };
    assert_eq!(
        shape.privileged_affine_composite,
        Some(PrivilegedAffineCompositeIdentity::HttpClientStreamHandle)
    );
    assert_eq!(shape.fields[0].name, "body");
    assert_eq!(
        shape.fields[0].plan,
        ValueTransferPlan::AffineResource {
            drop: ResourceDropPlan::ResourceTableRelease,
        }
    );
    assert!(function.frame_layout.slot_plans.iter().any(|plan| {
        matches!(
            plan,
            ValueTransferPlan::MoveOnly {
                drop: ValueDropPlan::RecursiveShape {
                    shape_ref: root_shape
                }
            } if *root_shape == shape_ref
        )
    }));
    assert!(decoded
        .instructions
        .iter()
        .any(|instruction| instruction.descriptor.kind == Opcode::StreamNext));
}

#[test]
fn a_second_real_source_body_take_fails_before_emission() {
    let lowered = lower(
        r#"
import std

function consume(input: std.http.HttpClientRequest) -> void {
  final response = std.http.stream(input)
  for first in response.body {
  }
  for second in response.body {
  }
}
"#,
    );
    let error = admit_phase_1_bytecode_mir(lowered.mir_units())
        .expect_err("a second affine take must not acquire compiler authority");
    assert!(matches!(
        error,
        BytecodeEmissionError::UnsupportedPhase1Capability {
            capability: Phase1UnsupportedCapability::HostTarget,
            ..
        }
    ));
}

#[test]
fn registry_rows_without_executor_identity_fail_before_value_shape_admission() {
    for (label, source, rejected_binding) in [
        (
            "sse",
            r#"
import std
function run(input: std.http.HttpClientRequest) -> void {
  std.http.sse(input)
}
"#,
            "std.http.client.sse",
        ),
        (
            "date-now",
            r#"
function run() -> void {
  Date.now()
}
"#,
            "core.date.now",
        ),
    ] {
        let lowered = lower(source);
        let error = admit_phase_1_bytecode_mir(lowered.mir_units())
            .expect_err("a registry row without executor identity stays fail closed");
        assert!(
            matches!(
                &error,
                BytecodeEmissionError::UnsupportedPhase1Capability {
                    capability: Phase1UnsupportedCapability::HostTarget,
                    ..
                }
            ),
            "{label}: {error:?}"
        );
        assert!(
            error.to_string().contains(rejected_binding),
            "{label}: {error}"
        );
    }
}

#[test]
fn an_http_type_without_an_exact_executor_operand_grants_no_admission() {
    let lowered = lower(
        r#"
import std
function unused(input: std.http.HttpClientRequest) -> void {
}
"#,
    );
    let error = admit_phase_1_bytecode_mir(lowered.mir_units())
        .expect_err("an HTTP type shape alone cannot grant admission");
    assert!(matches!(
        error,
        BytecodeEmissionError::UnsupportedPhase1Capability {
            capability: Phase1UnsupportedCapability::ValueShape,
            ..
        }
    ));
}

#[test]
fn generic_string_and_stream_shapes_remain_outside_the_exact_registry_slice() {
    for source in [
        "function run(value: string) -> void { }",
        "function run(value: Stream<number>) -> void { }",
    ] {
        let lowered = lower(source);
        let error = admit_phase_1_bytecode_mir(lowered.mir_units())
            .expect_err("type shape alone stays fail closed");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::ValueShape
                    | Phase1UnsupportedCapability::Stream,
                ..
            }
        ));
    }
}
