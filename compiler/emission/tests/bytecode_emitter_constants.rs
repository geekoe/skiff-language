use std::collections::BTreeMap;

use skiff_artifact_identity::validate_bytecode_identity;
use skiff_artifact_model::{
    BlockIr, BytecodeConstantRef, BytecodePoolEntry, CallableEffectSummary, ConstIr,
    ExecutableBody, ExprIr, ExprRefIr, FileIrUnit, FrozenConstantNode, LiteralIr,
    PackageCallableId, StmtIr, StmtRefIr, TypeDeclIr, TypeDescriptorIr, TypeRefIr,
    ValueTransferPlan,
};
use skiff_compiler_emission::{
    emit_bytecode_artifact, BytecodeEmissionError, BytecodeValueTransferPlans,
    FunctionValueTransferPlans,
};
use skiff_compiler_lowering::{
    mir::{MirConst, MirExecutableKind, MirFunction, MirLiveness, MirUnit},
    Bounds, ConstEvaluator, FrozenConstantBundle,
};

fn expression(index: u32) -> ExprRefIr {
    ExprRefIr { expression: index }
}

fn constant(name: &str, ty: TypeRefIr, expressions: Vec<ExprIr>) -> ConstIr {
    let root = u32::try_from(expressions.len() - 1).expect("small test expression table");
    ConstIr {
        name: name.to_string(),
        ty,
        body: ExecutableBody {
            blocks: vec![BlockIr {
                label: "entry".to_string(),
                statements: vec![StmtRefIr { statement: 0 }],
            }],
            statements: vec![StmtIr::Return {
                value: Some(expression(root)),
            }],
            expressions,
        },
        source_span: None,
    }
}

fn mir_and_bundle(file_ir: &FileIrUnit) -> (MirUnit, FrozenConstantBundle) {
    let bundle = ConstEvaluator::new(Bounds::default())
        .evaluate_unit(file_ir)
        .expect("test constants evaluate");
    let mir = MirUnit {
        module_path: file_ir.module_path.clone(),
        external_refs: file_ir.external_refs.clone(),
        source_map: file_ir.source_map.clone(),
        type_table: file_ir.type_table.clone(),
        link_targets: file_ir.link_targets.clone(),
        constants: file_ir
            .constants
            .iter()
            .enumerate()
            .map(|(index, constant)| MirConst {
                index: u32::try_from(index).expect("small test constant table"),
                symbol: format!("{}.{}", file_ir.module_path, constant.name),
                ty: constant.ty.clone(),
                source_span: constant.source_span.clone(),
            })
            .collect(),
        functions: Vec::new(),
    };
    (mir, bundle)
}

fn emit_constants(
    units: &[MirUnit],
    bundles: &[FrozenConstantBundle],
) -> skiff_artifact_model::BytecodeArtifact {
    let plans = explicit_constant_plans(units);
    emit_bytecode_artifact(units, bundles, &plans).expect("constant-only image emits")
}

fn explicit_constant_plans(units: &[MirUnit]) -> BytecodeValueTransferPlans {
    let constants = units
        .iter()
        .flat_map(|unit| &unit.constants)
        .map(|constant| {
            (
                constant.symbol.clone(),
                ValueTransferPlan::FromType {
                    ty: constant.ty.clone(),
                },
            )
        })
        .collect();
    BytecodeValueTransferPlans::new(BTreeMap::new(), constants)
}

fn empty_function(module_path: &str, declaration: &str) -> MirFunction {
    MirFunction {
        executable_index: 0,
        symbol: format!("{module_path}.{declaration}"),
        kind: MirExecutableKind::Function,
        type_params: Vec::new(),
        params: Vec::new(),
        return_type: TypeRefIr::builtin("void"),
        self_type: None,
        slots: Vec::new(),
        expressions: Vec::new(),
        blocks: Vec::new(),
        regions: Vec::new(),
        statements: Vec::new(),
        liveness: MirLiveness::default(),
        effect_summary_ref: PackageCallableId::new(format!("callable:{module_path}:{declaration}")),
        effect_summary: CallableEffectSummary::analysis_pending(),
        source_span: None,
    }
}

#[test]
fn literal_bundle_becomes_one_checked_constant_pool_root() {
    let mut file_ir = FileIrUnit::empty("sample", "source-hash");
    file_ir.constants.push(constant(
        "answer",
        TypeRefIr::builtin("number"),
        vec![ExprIr::Literal {
            value: LiteralIr::Number {
                value: serde_json::Number::from(42),
            },
        }],
    ));
    let (mir, bundle) = mir_and_bundle(&file_ir);

    let artifact = emit_constants(&[mir], &[bundle]);

    assert_eq!(artifact.image.pools.constants.len(), 1);
    let BytecodePoolEntry::ConstantRef {
        reference: BytecodeConstantRef::LocalNode { node_index },
        type_ref,
        plan,
    } = &artifact.image.pools.constants[0]
    else {
        panic!("constants pool is homogeneous")
    };
    assert_eq!(*node_index, 0);
    assert_eq!(*type_ref, 0);
    assert_eq!(
        plan,
        &ValueTransferPlan::FromType {
            ty: TypeRefIr::builtin("number"),
        }
    );
    assert_eq!(artifact.image.constant_roots["sample.answer"], 0);
    assert!(matches!(
        artifact.image.frozen_constant_graph.nodes.as_slice(),
        [FrozenConstantNode::Literal {
            literal: LiteralIr::Number { .. }
        }]
    ));
    validate_bytecode_identity(&artifact).expect("emitter success includes C9");
}

#[test]
fn array_bundle_relocates_children_and_keeps_one_canonical_root() {
    let mut file_ir = FileIrUnit::empty("arrays", "source-hash");
    file_ir.constants.push(constant(
        "values",
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![TypeRefIr::builtin("string")],
        },
        vec![
            ExprIr::Literal {
                value: LiteralIr::String {
                    value: "a".to_string(),
                },
            },
            ExprIr::Literal {
                value: LiteralIr::String {
                    value: "b".to_string(),
                },
            },
            ExprIr::ArrayLiteral {
                items: vec![expression(0), expression(1)],
            },
        ],
    ));
    let (mir, bundle) = mir_and_bundle(&file_ir);

    let artifact = emit_constants(&[mir], &[bundle]);

    assert_eq!(artifact.image.constant_roots["arrays.values"], 0);
    assert!(matches!(
        artifact.image.frozen_constant_graph.nodes.as_slice(),
        [
            FrozenConstantNode::Literal { .. },
            FrozenConstantNode::Literal { .. },
            FrozenConstantNode::Array { children }
        ] if children == &[0, 1]
    ));
    validate_bytecode_identity(&artifact).expect("array-only v4 artifact is admissible");
}

#[test]
fn a_constant_without_an_explicit_plan_fails_before_graph_emission() {
    let mut file_ir = FileIrUnit::empty("unplanned", "source-hash");
    file_ir.constants.push(constant(
        "value",
        TypeRefIr::builtin("string"),
        vec![ExprIr::Literal {
            value: LiteralIr::String {
                value: "value".to_string(),
            },
        }],
    ));
    let (mir, bundle) = mir_and_bundle(&file_ir);

    let error = emit_bytecode_artifact(&[mir], &[bundle], &BytecodeValueTransferPlans::empty())
        .expect_err("constant plans cannot be inferred from the declared type");
    assert!(matches!(
        error,
        BytecodeEmissionError::MissingConstantValueTransferPlan { symbol }
            if symbol == "unplanned.value"
    ));
}

#[test]
fn an_unowned_constant_plan_cannot_be_ignored() {
    let plans = BytecodeValueTransferPlans::new(
        BTreeMap::new(),
        BTreeMap::from([(
            "unknown.value".to_string(),
            ValueTransferPlan::FromType {
                ty: TypeRefIr::builtin("string"),
            },
        )]),
    );

    let error = emit_bytecode_artifact(&[], &[], &plans)
        .expect_err("extra constant plan must fail exact coverage");
    assert!(matches!(
        error,
        BytecodeEmissionError::UnexpectedConstantValueTransferPlan { symbol }
            if symbol == "unknown.value"
    ));
}

#[test]
fn a_deferred_constant_plan_must_name_the_exact_constant_type() {
    let mut file_ir = FileIrUnit::empty("mismatched", "source-hash");
    file_ir.constants.push(constant(
        "value",
        TypeRefIr::builtin("number"),
        vec![ExprIr::Literal {
            value: LiteralIr::Number {
                value: serde_json::Number::from(1),
            },
        }],
    ));
    let (mir, bundle) = mir_and_bundle(&file_ir);
    let plans = BytecodeValueTransferPlans::new(
        BTreeMap::new(),
        BTreeMap::from([(
            "mismatched.value".to_string(),
            ValueTransferPlan::FromType {
                ty: TypeRefIr::builtin("string"),
            },
        )]),
    );

    let error = emit_bytecode_artifact(&[mir], &[bundle], &plans)
        .expect_err("FromType cannot describe a different value type");
    assert!(matches!(
        error,
        BytecodeEmissionError::ConstantValueTransferPlanTypeMismatch { symbol }
            if symbol == "mismatched.value"
    ));
}

#[test]
fn representation_constants_remain_gated_on_the_v4_producer_contract() {
    let mut file_ir = FileIrUnit::empty("wrapped", "source-hash");
    file_ir.type_table.push(TypeDeclIr {
        name: "Wrapped".to_string(),
        descriptor: TypeDescriptorIr::Representation {
            representation: TypeRefIr::builtin("string"),
        },
        type_params: Vec::new(),
        implements: Vec::new(),
        source_span: None,
    });
    file_ir.constants.push(constant(
        "value",
        TypeRefIr::LocalType { type_index: 0 },
        vec![
            ExprIr::Literal {
                value: LiteralIr::String {
                    value: "value".to_string(),
                },
            },
            ExprIr::RepresentationWrap {
                value: expression(0),
                type_ref: TypeRefIr::LocalType { type_index: 0 },
            },
        ],
    ));
    let (mir, bundle) = mir_and_bundle(&file_ir);
    let plans = explicit_constant_plans(std::slice::from_ref(&mir));

    let error = emit_bytecode_artifact(&[mir], &[bundle], &plans)
        .expect_err("representation facts cannot enter bytecode before the v4 producer seam");
    assert!(matches!(
        error,
        BytecodeEmissionError::UnsupportedConstantNode {
            construct: "Representation",
            ..
        }
    ));
}

#[test]
fn record_shape_without_nominal_field_facts_fails_closed() {
    fn owner(module_path: &str) -> (MirUnit, FrozenConstantBundle) {
        let mut file_ir = FileIrUnit::empty(module_path, "source-hash");
        file_ir.type_table = vec![
            TypeDeclIr {
                name: "Inner".to_string(),
                descriptor: TypeDescriptorIr::Representation {
                    representation: TypeRefIr::builtin("string"),
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            },
            TypeDeclIr {
                name: "Container".to_string(),
                descriptor: TypeDescriptorIr::Record {
                    fields: BTreeMap::from([(
                        "value".to_string(),
                        TypeRefIr::LocalType { type_index: 0 },
                    )]),
                },
                type_params: Vec::new(),
                implements: Vec::new(),
                source_span: None,
            },
        ];
        file_ir.constants.push(constant(
            "container",
            TypeRefIr::LocalType { type_index: 1 },
            vec![
                ExprIr::Literal {
                    value: LiteralIr::String {
                        value: "payload".to_string(),
                    },
                },
                ExprIr::Construct {
                    type_ref: TypeRefIr::LocalType { type_index: 1 },
                    fields: BTreeMap::from([("value".to_string(), expression(0))]),
                },
            ],
        ));
        mir_and_bundle(&file_ir)
    }

    let (alpha_mir, alpha_bundle) = owner("alpha_shape");
    let (zeta_mir, zeta_bundle) = owner("zeta_shape");
    let units = [zeta_mir, alpha_mir];
    let bundles = [zeta_bundle, alpha_bundle];
    let plans = explicit_constant_plans(&units);
    let error = emit_bytecode_artifact(&units, &bundles, &plans)
        .expect_err("record constants require nominal field and lifecycle facts");
    assert!(matches!(
        error,
        BytecodeEmissionError::UnsupportedConstantNode {
            construct: "Record",
            ..
        }
    ));
}

#[test]
fn constant_declaration_order_does_not_change_the_image_identity() {
    fn owner(constants: Vec<ConstIr>) -> (MirUnit, FrozenConstantBundle) {
        let mut file_ir = FileIrUnit::empty("stable", "source-hash");
        file_ir.constants = constants;
        mir_and_bundle(&file_ir)
    }
    let first = constant(
        "first",
        TypeRefIr::builtin("string"),
        vec![ExprIr::Literal {
            value: LiteralIr::String {
                value: "a".to_string(),
            },
        }],
    );
    let second = constant(
        "second",
        TypeRefIr::builtin("string"),
        vec![ExprIr::Literal {
            value: LiteralIr::String {
                value: "b".to_string(),
            },
        }],
    );
    let (forward_mir, forward_bundle) = owner(vec![first.clone(), second.clone()]);
    let (reverse_mir, reverse_bundle) = owner(vec![second, first]);

    let forward = emit_constants(&[forward_mir], &[forward_bundle]);
    let reverse = emit_constants(&[reverse_mir], &[reverse_bundle]);

    assert_eq!(forward.bytecode_identity, reverse.bytecode_identity);
    assert_eq!(forward.image, reverse.image);
}

#[test]
fn a_mir_unit_without_its_owned_bundle_fails_closed() {
    let file_ir = FileIrUnit::empty("missing", "source-hash");
    let (mir, _) = mir_and_bundle(&file_ir);
    let error = emit_bytecode_artifact(&[mir], &[], &BytecodeValueTransferPlans::empty())
        .expect_err("bundle coverage is mandatory even for an empty unit");
    assert!(matches!(
        error,
        BytecodeEmissionError::MissingConstantBundle { module_path }
            if module_path == "missing"
    ));
}

#[test]
fn a_function_without_explicit_transfer_plans_fails_before_body_emission() {
    let file_ir = FileIrUnit::empty("planned", "source-hash");
    let (mut mir, bundle) = mir_and_bundle(&file_ir);
    mir.functions.push(empty_function("planned", "run"));

    let error = emit_bytecode_artifact(&[mir], &[bundle], &BytecodeValueTransferPlans::empty())
        .expect_err("the emitter cannot infer transfer plans");
    assert!(matches!(
        error,
        BytecodeEmissionError::MissingValueTransferPlans { function_key }
            if function_key == "planned::run"
    ));
}

#[test]
fn an_empty_function_cannot_bypass_the_fail_closed_body_gate() {
    let file_ir = FileIrUnit::empty("gated", "source-hash");
    let (mut mir, bundle) = mir_and_bundle(&file_ir);
    mir.functions.push(empty_function("gated", "run"));
    let transfer_plans = BytecodeValueTransferPlans::new(
        BTreeMap::from([(
            "gated::run".to_string(),
            FunctionValueTransferPlans {
                slot_plans: Vec::new(),
                result_plans: Vec::new(),
            },
        )]),
        BTreeMap::new(),
    );

    let error = emit_bytecode_artifact(&[mir], &[bundle], &transfer_plans)
        .expect_err("no MIR function is silently omitted");
    assert!(matches!(
        error,
        BytecodeEmissionError::UnsupportedConstruct { function_key, .. }
            if function_key == "gated::run"
    ));
}
