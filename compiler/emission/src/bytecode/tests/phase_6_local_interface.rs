use std::path::PathBuf;

use skiff_artifact_model::{
    BoxSourceIr, BytecodeRelocation, CallableEffectSummary, ExprIr, TypeRefIr,
};
use skiff_compiler_lowering::{
    mir::source_program::{lower_single_source_program, SingleSourceProgram},
    mir::{MirStmt, MirStmtKind},
    Bounds, ConstEvaluator,
};

use crate::bytecode::plans::derive_test_bytecode_value_transfer_plans;
use crate::{
    admit_phase_1_bytecode_mir, emit_bytecode_artifact, BytecodeEmissionError,
    Phase1UnsupportedCapability,
};

const PACKAGE_ID: &str = "example.com/phase6-local-interface";
const SOURCE: &str = r#"
interface Reader {
  function label(self: Self) -> number
}

type Impl implements Reader {
  value: number,
}

type Impl2 implements Reader {
  value: number,
}

impl Impl {
  function label() -> number {
    return 1
  }
}

impl Impl2 {
  function label() -> number {
    return 2
  }
}

function run(seed: number) -> number {
  final first = Impl { value: 1 } as Reader
  final second = Impl2 { value: 2 } as Reader
  final firstValue = first.label()
  final secondValue = second.label()
  if firstValue == 1 {
    if secondValue == 2 {
      return seed
    }
  }
  return seed + 1
}
"#;

const STRING_SOURCE: &str = r#"
interface Reader {
  function label(self: Self) -> string
}

type Impl implements Reader {
  value: string,
}

impl Impl {
  function label() -> string {
    return self.value
  }
}

function run(seed: number) -> number {
  final reader = Impl { value: "interface-ok" } as Reader
  return reader.label().length()
}
"#;

fn lower(source: &str) -> skiff_compiler_lowering::LoweredPackage {
    let platform_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    lower_single_source_program(SingleSourceProgram {
        platform_root: &platform_root,
        package_id: PACKAGE_ID,
        module_path: "main",
        relative_path: "main.skiff",
        source,
    })
    .expect("local interface source lowers through the production source/MIR API")
}

fn emitted_artifact() -> skiff_artifact_model::BytecodeArtifact {
    let lowered = lower(SOURCE);
    let admitted = admit_phase_1_bytecode_mir(lowered.mir_units())
        .expect("exact local interface facts pass admission");
    let plans = derive_test_bytecode_value_transfer_plans(lowered.mir_units())
        .expect("plans derive exactly");
    let bundles = lowered
        .file_ir_units()
        .iter()
        .map(|unit| {
            ConstEvaluator::new(Bounds::default())
                .evaluate_unit(unit)
                .expect("local interface constants evaluate")
        })
        .collect::<Vec<_>>();
    emit_bytecode_artifact(&admitted, &bundles, &plans).expect("local interface emits")
}

fn admit_and_emit_source(source: &str) -> skiff_artifact_model::BytecodeArtifact {
    let lowered = lower(source);
    let admitted = admit_phase_1_bytecode_mir(lowered.mir_units())
        .expect("phase 6 core blocker source passes admission");
    let plans = derive_test_bytecode_value_transfer_plans(lowered.mir_units())
        .expect("phase 6 core blocker plans derive exactly");
    let bundles = lowered
        .file_ir_units()
        .iter()
        .map(|unit| {
            ConstEvaluator::new(Bounds::default())
                .evaluate_unit(unit)
                .expect("phase 6 core blocker constants evaluate")
        })
        .collect::<Vec<_>>();
    emit_bytecode_artifact(&admitted, &bundles, &plans).expect("phase 6 core blocker emits")
}

#[test]
fn db_abort_transaction_value_with_throw_emits() {
    admit_and_emit_source(
        r#"
type Doc {
  id: string,
  value: string,
}

db object Doc {
  primary key(id)
}

type Failure {
  message: string,
}

function run(seed: number) -> number {
  db transaction value {
    db insert Doc { id = "phase6-db-abort" value = "bad" }
    throw Failure { message: "abort" }
    0
  }
}
"#,
    );
}

#[test]
fn recoverable_restore_db_require_record_with_string_fields_emits() {
    admit_and_emit_source(
        r#"
type Doc {
  id: string,
  value: string,
}

db object Doc {
  primary key(id)
}

function restore(seed: number) -> number {
  final found = db require Doc("phase6-key")
  return found.value.length()
}
"#,
    );
}

#[test]
fn service_throw_record_payload_emits() {
    admit_and_emit_source(
        r#"
type Failure {
  message: string,
}

function fail() -> void {
  throw Failure { message: "service-throw" }
}
"#,
    );
}

#[test]
fn local_interface_admission_emits_exact_local_and_requirement_tables() {
    let artifact = emitted_artifact();
    let mut local_methods = Vec::new();
    let mut requirement_methods = Vec::new();
    for function in artifact.image.functions.values() {
        for relocation in &function.relocations {
            match relocation {
                BytecodeRelocation::LocalInterfaceRef { interface } => {
                    local_methods.extend(interface.methods.iter().cloned());
                }
                BytecodeRelocation::InterfaceRequirementRef { methods, .. } => {
                    requirement_methods.extend(methods.iter().cloned());
                }
                _ => {}
            }
        }
    }
    assert_eq!(local_methods.len(), 2);
    assert_eq!(requirement_methods.len(), 2);
    let local = &local_methods[0];
    let requirement = &requirement_methods[0];
    assert_eq!(local.method_abi_id, requirement.method_abi_id);
    assert_eq!(local.signature, requirement.signature);
    assert_eq!(local.effects, requirement.effects);
    assert!(matches!(
        local.effects,
        CallableEffectSummary::Analyzed { .. }
    ));
    assert!(!local.function_key.is_empty());
    assert!(local_methods
        .iter()
        .all(|method| method.method_name == "label"));
    assert!(
        requirement_methods
            .iter()
            .any(|requirement| requirement.signature.params[0].ty
                == local_methods[1].signature.params[0].ty),
        "each call requirement retains the exact concrete local table signature"
    );
}

#[test]
fn local_interface_impl_emits_exact_string_return_and_receiver_field_layout() {
    let lowered = lower(STRING_SOURCE);
    let admitted = admit_phase_1_bytecode_mir(lowered.mir_units())
        .expect("exact local interface string facts pass admission");
    let facts = admitted
        .machine_carriers()
        .function("main::Impl.label")
        .expect("local interface implementation carrier row is present");
    assert_eq!(
        facts
            .result()
            .expect("local interface implementation has a non-void result")
            .ty(),
        &TypeRefIr::builtin("string")
    );
    let receiver_shape = facts
        .slot_shape(0)
        .expect("local interface receiver slot has an analyzed field layout");
    assert_eq!(
        receiver_shape.owner(),
        &TypeRefIr::LocalType { type_index: 0 }
    );
    assert_eq!(
        receiver_shape
            .fields()
            .get("value")
            .expect("receiver field layout carries `value`")
            .ty(),
        &TypeRefIr::builtin("string")
    );

    let plans = derive_test_bytecode_value_transfer_plans(lowered.mir_units())
        .expect("local interface string plans derive exactly");
    let bundles = lowered
        .file_ir_units()
        .iter()
        .map(|unit| {
            ConstEvaluator::new(Bounds::default())
                .evaluate_unit(unit)
                .expect("local interface string constants evaluate")
        })
        .collect::<Vec<_>>();
    emit_bytecode_artifact(&admitted, &bundles, &plans)
        .expect("exact local interface string shapes emit");
}

#[test]
fn local_interface_declaration_without_exact_table_fails_closed() {
    let lowered = lower(
        r#"
interface Reader {
  function label(self: Self) -> number
}

type Impl implements Reader {
  value: number,
}

impl Impl {
  function label() -> number {
    return 1
  }
}

function run(seed: number) -> number {
  return seed
}
"#,
    );
    let error = admit_phase_1_bytecode_mir(lowered.mir_units())
        .expect_err("a local interface conformance without an exact table must fail closed");
    assert!(matches!(
        error,
        BytecodeEmissionError::UnsupportedPhase1Capability {
            capability: Phase1UnsupportedCapability::Interface,
            ..
        }
    ));
}

#[test]
fn drifted_and_ambiguous_local_interface_facts_fail_closed() {
    let units = lower(SOURCE).mir_units().to_vec();
    let mut drifted = units.clone();
    for unit in &mut drifted {
        for function in &mut unit.functions {
            for expression in &mut function.expressions {
                if let ExprIr::InterfaceBox {
                    source: BoxSourceIr::Local { method_table, .. },
                    ..
                } = &mut expression.expression
                {
                    method_table.slots[0].signature.return_type = TypeRefIr::builtin("bool");
                }
            }
        }
    }
    let error = admit_phase_1_bytecode_mir(&drifted)
        .expect_err("drifted local interface signature must fail admission");
    assert!(
        error.to_string().contains("signature drifts"),
        "unexpected drift error: {error}"
    );

    let mut ambiguous = units;
    let mut second_box = None;
    let mut first_box_type = None;
    for unit in &ambiguous {
        for function in &unit.functions {
            for expression in &function.expressions {
                if let ExprIr::InterfaceBox {
                    source: BoxSourceIr::Local { concrete_type, .. },
                    ..
                } = &expression.expression
                {
                    if first_box_type.is_none() {
                        first_box_type = Some(concrete_type.clone());
                    } else if first_box_type.as_ref() != Some(concrete_type) {
                        second_box = Some(expression.index);
                    }
                }
            }
        }
    }
    let second_box = second_box.expect("the fixture contains a second local box");
    let mut mutated = false;
    for unit in &mut ambiguous {
        for function in &mut unit.functions {
            for block in &mut function.blocks {
                for statement in &mut block.statements {
                    if let MirStmtKind::InitSlot { slot, value } = &mut statement.kind {
                        let original_value = *value;
                        let original_slot = *slot;
                        *value = skiff_artifact_model::ExprRefIr {
                            expression: second_box,
                        };
                        block.statements.push(MirStmt {
                            statement_index: u32::MAX,
                            span: None,
                            kind: MirStmtKind::InitSlot {
                                slot: original_slot,
                                value: original_value,
                            },
                        });
                        mutated = true;
                        break;
                    }
                }
            }
        }
    }
    assert!(
        mutated,
        "the ambiguous fixture mutation must reach a slot initializer"
    );
    let error = admit_phase_1_bytecode_mir(&ambiguous)
        .expect_err("ambiguous local interface ABI facts must fail admission");
    assert!(
        error.to_string().contains("ambiguous"),
        "unexpected ambiguity error: {error}"
    );
}
