use skiff_artifact_identity::validate_bytecode_identity;
use skiff_artifact_model::{
    opcode_table_fingerprint, BlockIr, BytecodePoolEntry, ConstIr, ExecutableBody, ExprIr,
    ExprRefIr, FileIrUnit, FrozenConstantNode, LiteralIr, StmtIr, StmtRefIr, TypeDeclIr,
    TypeDescriptorIr, TypeRefIr,
};
use skiff_compiler_emission::{
    emit_bytecode_artifact, BytecodeEmissionError, BytecodeValueTransferPlans,
};
use skiff_compiler_lowering::{
    mir::{MirConst, MirUnit},
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
    emit_bytecode_artifact(
        units,
        bundles,
        &BytecodeValueTransferPlans::default(),
        &opcode_table_fingerprint(),
    )
    .expect("constant-only image emits")
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
    let BytecodePoolEntry::FrozenConstantRef { node_index } = &artifact.image.pools.constants[0]
    else {
        panic!("constants pool is homogeneous")
    };
    assert_eq!(*node_index, 0);
    assert!(matches!(
        artifact.image.frozen_constant_graph.nodes.as_slice(),
        [FrozenConstantNode::Literal {
            literal: LiteralIr::Number { .. }
        }]
    ));
    validate_bytecode_identity(&artifact).expect("emitter success includes C9");
}

#[test]
fn local_constant_types_are_qualified_by_their_exact_module_owner() {
    fn owner(module_path: &str) -> (MirUnit, FrozenConstantBundle) {
        let mut file_ir = FileIrUnit::empty(module_path, "source-hash");
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
            "wrapped",
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
        mir_and_bundle(&file_ir)
    }

    let (alpha_mir, alpha_bundle) = owner("alpha");
    let (zeta_mir, zeta_bundle) = owner("zeta");
    let artifact = emit_constants(&[zeta_mir, alpha_mir], &[zeta_bundle, alpha_bundle]);

    let pooled_types = artifact
        .image
        .pools
        .types
        .iter()
        .map(|entry| match entry {
            BytecodePoolEntry::TypeRef { ty } => ty,
            _ => panic!("types pool is homogeneous"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        pooled_types,
        vec![
            &TypeRefIr::PublicationType {
                module_path: "alpha".to_string(),
                type_index: 0,
            },
            &TypeRefIr::PublicationType {
                module_path: "zeta".to_string(),
                type_index: 0,
            },
        ]
    );

    let graph_type_refs = artifact
        .image
        .frozen_constant_graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            FrozenConstantNode::TypeRef { type_ref } => Some(*type_ref),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(graph_type_refs, vec![0, 1]);
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
    let error = emit_bytecode_artifact(
        &[mir],
        &[],
        &BytecodeValueTransferPlans::default(),
        &opcode_table_fingerprint(),
    )
    .expect_err("bundle coverage is mandatory even for an empty unit");
    assert!(matches!(
        error,
        BytecodeEmissionError::MissingConstantBundle { module_path }
            if module_path == "missing"
    ));
}
