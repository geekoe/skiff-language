use std::path::PathBuf;

use skiff_artifact_model::{
    bytecode::BoundedDecoder, BytecodePoolEntry, BytecodeRelocation, CallableEffectSummary,
    HostEffectExecutorIdentity, Opcode, PendingEffectCategory, PrivilegedAffineCompositeIdentity,
    ResourceDropPlan, ResumeResultMaterialization, ValueDropPlan, ValueTransferPlan,
};
use skiff_compiler_lowering::{
    mir::source_program::{lower_single_source_program, SingleSourceProgram},
    Bounds, ConstEvaluator, LoweredPackage,
};

use crate::bytecode::{
    emitter::emit_bytecode_artifact_unchecked, plans::derive_test_bytecode_value_transfer_plans,
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

fn emit_unchecked_source(source: &str) -> skiff_artifact_model::BytecodeArtifact {
    let lowered = lower(source);
    let plans = derive_test_bytecode_value_transfer_plans(lowered.mir_units())
        .expect("test source has complete lifecycle plans");
    let bundles = lowered
        .file_ir_units()
        .iter()
        .map(|unit| {
            ConstEvaluator::new(Bounds::default())
                .evaluate_unit(unit)
                .expect("test source constants evaluate")
        })
        .collect::<Vec<_>>();
    emit_bytecode_artifact_unchecked(lowered.mir_units(), &bundles, &plans)
        .expect("crate-owned emission accepts structurally valid MIR")
}

#[test]
fn stream_for_in_backedges_drop_only_live_iteration_items() {
    const SOURCE: &str = r#"
function normal(input: Stream<bytes>) -> void {
  for chunk in input {
  }
}

function continued(input: Stream<bytes>) -> void {
  for chunk in input {
    continue
  }
}

function broken(input: Stream<bytes>) -> void {
  for chunk in input {
    break
  }
}
"#;
    let artifact = emit_unchecked_source(SOURCE);
    let lowered = lower(SOURCE);

    for symbol in ["main.normal", "main.continued", "main.broken"] {
        let mir = lowered.mir_units()[0]
            .functions
            .iter()
            .find(|function| function.symbol == symbol)
            .unwrap_or_else(|| panic!("missing MIR function {symbol}"));
        let item_slot = mir
            .slots
            .iter()
            .find(|slot| slot.name == "chunk")
            .expect("for-in item slot exists")
            .slot;
        let function_key = symbol.replace('.', "::");
        let function = artifact
            .image
            .functions
            .get(&function_key)
            .unwrap_or_else(|| panic!("missing emitted function {function_key}"));
        let decoded = BoundedDecoder::new()
            .decode_function(&function.words)
            .expect("emitted loop decodes");
        let item_drops_before_jump = decoded
            .instructions
            .windows(2)
            .filter(|pair| {
                pair[0].descriptor.kind == Opcode::Drop
                    && pair[0].operand(0) == item_slot
                    && pair[1].descriptor.kind == Opcode::Jump
            })
            .count();
        assert_eq!(
            item_drops_before_jump, 1,
            "{symbol} must drop its live item exactly once before leaving the iteration"
        );
        let endpoint_slot = decoded
            .instructions
            .iter()
            .find(|instruction| instruction.descriptor.kind == Opcode::StreamNext)
            .expect("stream loop emits StreamNext")
            .operand(0);
        assert_eq!(
            decoded
                .instructions
                .iter()
                .filter(|instruction| {
                    instruction.descriptor.kind == Opcode::Drop
                        && instruction.operand(0) == endpoint_slot
                })
                .count(),
            1,
            "{symbol} must release the stream endpoint exactly once at loop continuation"
        );
    }
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
        let trivial = matches!(
            ty,
            skiff_artifact_model::TypeRefIr::Builtin { name, args }
                if args.is_empty()
                    && matches!(name.as_str(), "bool" | "integer" | "null" | "number")
        ) || matches!(
            ty,
            skiff_artifact_model::TypeRefIr::PackageSymbol { symbol }
                if symbol.symbol_path == "std.time.Duration"
        );
        if matches!(
            ty,
            skiff_artifact_model::TypeRefIr::PackageSymbol { symbol }
                if symbol.symbol_path == "std.http.HttpClientStreamHandle"
        ) {
            return Ok(ValueTransferPlan::FromType { ty: ty.clone() });
        }
        Ok(ValueTransferPlan::SnapshotShare {
            drop: if trivial {
                ValueDropPlan::Trivial
            } else {
                ValueDropPlan::SnapshotRelease
            },
        })
    })
    .expect("the injected source authority covers every materialized HTTP result field");
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

    let request_function = artifact
        .image
        .functions
        .get("main::request")
        .expect("ignored unary response still emits one request function");
    let decoded_request = BoundedDecoder::new()
        .decode_function(&request_function.words)
        .expect("request function remains decodable");
    assert!(decoded_request
        .instructions
        .iter()
        .all(|instruction| instruction.descriptor.kind != Opcode::GetDenseField));
    let request_resume = artifact
        .image
        .pools
        .resume
        .iter()
        .find_map(|entry| match entry {
            BytecodePoolEntry::ResumeDescriptor(descriptor)
                if descriptor.function_key == "main::request" =>
            {
                Some(descriptor)
            }
            _ => None,
        })
        .expect("request call owns one exact resume descriptor");
    let [Some(ResumeResultMaterialization::DenseRecord { shape_ref })] =
        request_resume.result_materializations.as_slice()
    else {
        panic!("ignored unary response must still carry one exact dense materialization shape")
    };
    let BytecodePoolEntry::ShapeRef { shape } = &artifact.image.pools.shapes[*shape_ref as usize]
    else {
        panic!("dense result materialization must reference a shape row")
    };
    let result_type = request_resume.result_type_refs[0];
    let BytecodePoolEntry::TypeRef { ty: result_ty } =
        &artifact.image.pools.types[result_type as usize]
    else {
        panic!("resume result must reference a type row")
    };
    let BytecodePoolEntry::TypeRef { ty: nominal_ty } =
        &artifact.image.pools.types[shape.type_ref as usize]
    else {
        panic!("materialization shape must reference a nominal type row")
    };
    assert_eq!(result_ty, nominal_ty);
    assert_eq!(
        shape
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["body", "headers", "status"]
    );
    assert_eq!(shape.plan, request_resume.result_plans[0]);
    assert!(shape.privileged_affine_composite.is_none());
}

#[test]
fn mixed_sleep_and_http_require_both_exact_effect_categories() {
    let lowered = lower(
        r#"
import std

function mixed(input: std.http.HttpClientRequest) -> void {
  std.time.sleep(Duration.milliseconds(1))
  std.http.request(input)
}
"#,
    );
    let CallableEffectSummary::Analyzed { effects } =
        &lowered.mir_units()[0].functions[0].effect_summary
    else {
        panic!("real source carries analyzed effects")
    };
    assert_eq!(
        effects
            .pending_effect_categories
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            PendingEffectCategory::NativeCall,
            PendingEffectCategory::HostEffect,
        ])
    );
    admit_phase_1_bytecode_mir(lowered.mir_units())
        .expect("mixed source owns both exact producer categories");

    let mut wrong = lowered.mir_units().to_vec();
    let CallableEffectSummary::Analyzed { effects } = &mut wrong[0].functions[0].effect_summary
    else {
        panic!("real source carries analyzed effects")
    };
    effects.pending_effect_categories = vec![PendingEffectCategory::NativeCall];
    let error = admit_phase_1_bytecode_mir(&wrong)
        .expect_err("mixed source missing HostEffect authority must fail closed");
    assert!(matches!(
        error,
        BytecodeEmissionError::UnsupportedPhase1Capability {
            capability: Phase1UnsupportedCapability::PendingEffect,
            ..
        }
    ));
}

#[test]
fn http_executor_rejects_native_call_category_laundering() {
    let lowered = lower(
        r#"
import std
function request(input: std.http.HttpClientRequest) -> void {
  std.http.request(input)
}
"#,
    );
    let mut wrong = lowered.mir_units().to_vec();
    let CallableEffectSummary::Analyzed { effects } = &mut wrong[0].functions[0].effect_summary
    else {
        panic!("real source carries analyzed effects")
    };
    effects.pending_effect_categories = vec![PendingEffectCategory::NativeCall];
    let error = admit_phase_1_bytecode_mir(&wrong)
        .expect_err("HTTP cannot borrow Sleep's NativeCall category");
    assert!(matches!(
        error,
        BytecodeEmissionError::UnsupportedPhase1Capability {
            capability: Phase1UnsupportedCapability::PendingEffect,
            ..
        }
    ));
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
    assert_eq!(
        shape.plan,
        ValueTransferPlan::MoveOnly {
            drop: ValueDropPlan::RecursiveShape { shape_ref },
        }
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

#[test]
fn stream_result_without_gateway_authority_fails_before_emit_shape_matters() {
    for item in ["string", "bytes"] {
        let lowered = lower(&format!(
            "function run() -> Stream<{item}> {{ return null }}"
        ));
        let error = admit_phase_1_bytecode_mir(lowered.mir_units())
            .expect_err("a stream signature without gateway authority must fail closed");
        assert!(matches!(
            error,
            BytecodeEmissionError::UnsupportedPhase1Capability {
                capability: Phase1UnsupportedCapability::Stream,
                location,
                ..
            } if location.contains("lacks exact canonical gateway authority")
        ));
    }
}
