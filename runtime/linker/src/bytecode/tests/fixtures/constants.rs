use std::collections::BTreeMap;

use skiff_artifact_identity::ValidatedBytecodeArtifact;
use skiff_artifact_model::{
    BytecodeArtifact, BytecodeConstantRef, BytecodePoolEntry, ConstExport, FileIrRef,
    FrozenBehaviorBinding, FrozenConstantGraph, FrozenConstantNode, LiteralIr,
    PackageLocalAbiSymbol, PackageRefIr, PackageSymbolRef, PackageTypeRef, ShapeDeclaration,
    ShapeFieldDeclaration, TypeDescriptorIr, TypeRefIr, ValueDropPlan, ValueTransferPlan,
};

use super::{
    DependencyBuildPin, Fixture, NormalizationDependency, RootProgram, DEPENDENCY_ALIAS,
    HELPER_FUNCTION,
};

const CONSTANT_ROOT: &str = "fixture.answer";
const REPRESENTATION_TYPE: &str = "fixture.Duration";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bytecode::tests) enum RepresentationLiteralCase {
    Exact,
    MissingFact,
    WrongDescriptor,
    WrongPayload,
    WrongPhysicalCarrier,
    ReorderedRows,
    DuplicatePayloadRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::bytecode::tests) enum ConstantProgram {
    Null,
    Bool,
    Number,
    String,
    LiteralString,
    LiteralMismatch,
    Array,
    Record,
    Representation,
    Implementation,
    PackageSymbol,
    Anonymous,
    WrongCarrier,
    WrongPlan,
    WrongStringPlan,
    RepresentationLiteral(RepresentationLiteralCase),
}

impl Fixture {
    pub(in crate::bytecode::tests) fn constant(program: ConstantProgram) -> Self {
        Self::new(RootProgram::Constant(program), false)
    }

    pub(in crate::bytecode::tests) fn two_package_constants(
        primary: ConstantProgram,
        dependency: ConstantProgram,
    ) -> Self {
        Self::new_with_options(
            RootProgram::Constant(primary),
            false,
            false,
            Some(NormalizationDependency {
                pin: DependencyBuildPin::Exact,
                conflict: None,
                constant: Some(dependency),
            }),
            false,
            None,
        )
    }

    pub(in crate::bytecode::tests) fn package_symbol_constant() -> Self {
        Self::two_package_constants(ConstantProgram::PackageSymbol, ConstantProgram::Number)
    }

    pub(in crate::bytecode::tests) fn representation_literal(
        case: RepresentationLiteralCase,
    ) -> Self {
        Self::constant(ConstantProgram::RepresentationLiteral(case))
    }
}

pub(super) fn populate_bytecode(artifact: &mut BytecodeArtifact, program: ConstantProgram) {
    if let ConstantProgram::RepresentationLiteral(case) = program {
        populate_representation_literal(artifact, case);
        return;
    }
    let (nodes, root_node, ty, plan) = match program {
        ConstantProgram::Null => literal_parts(LiteralIr::Null, TypeRefIr::builtin("null")),
        ConstantProgram::Bool => {
            literal_parts(LiteralIr::Bool { value: true }, TypeRefIr::builtin("bool"))
        }
        ConstantProgram::Number | ConstantProgram::Anonymous => literal_parts(
            LiteralIr::Number {
                value: serde_json::Number::from(42),
            },
            TypeRefIr::builtin("number"),
        ),
        ConstantProgram::String => literal_parts(
            LiteralIr::String {
                value: "ready".to_string(),
            },
            TypeRefIr::builtin("string"),
        ),
        ConstantProgram::LiteralString => {
            let literal = LiteralIr::String {
                value: "ready".to_string(),
            };
            let ty = TypeRefIr::Literal {
                value: literal.clone(),
            };
            (
                vec![FrozenConstantNode::Literal { literal }],
                0,
                ty.clone(),
                exact_fixture_plan(&ty),
            )
        }
        ConstantProgram::LiteralMismatch => {
            let ty = TypeRefIr::Literal {
                value: LiteralIr::String {
                    value: "other".to_string(),
                },
            };
            (
                vec![FrozenConstantNode::Literal {
                    literal: LiteralIr::String {
                        value: "ready".to_string(),
                    },
                }],
                0,
                ty.clone(),
                exact_fixture_plan(&ty),
            )
        }
        ConstantProgram::Array => {
            let ty = TypeRefIr::Builtin {
                name: "Array".to_string(),
                args: vec![TypeRefIr::builtin("string")],
            };
            (
                vec![
                    FrozenConstantNode::Literal {
                        literal: LiteralIr::String {
                            value: "item".to_string(),
                        },
                    },
                    FrozenConstantNode::Array { children: vec![0] },
                ],
                1,
                ty.clone(),
                exact_fixture_plan(&ty),
            )
        }
        ConstantProgram::Record => composite_parts(
            vec![
                number_node(),
                FrozenConstantNode::Record {
                    shape_index: 0,
                    children: vec![0],
                },
            ],
            1,
        ),
        ConstantProgram::Representation => composite_parts(
            vec![
                number_node(),
                FrozenConstantNode::Representation {
                    type_ref: 0,
                    value: 0,
                },
            ],
            1,
        ),
        ConstantProgram::Implementation => composite_parts(
            vec![
                number_node(),
                FrozenConstantNode::Record {
                    shape_index: 0,
                    children: vec![0],
                },
                FrozenConstantNode::Implementation {
                    record: 1,
                    behaviors: vec![FrozenBehaviorBinding {
                        function_key: HELPER_FUNCTION.to_string(),
                    }],
                },
            ],
            2,
        ),
        ConstantProgram::PackageSymbol => composite_parts(Vec::new(), 0),
        ConstantProgram::WrongCarrier => literal_parts(
            LiteralIr::Number {
                value: serde_json::Number::from(42),
            },
            TypeRefIr::builtin("bool"),
        ),
        ConstantProgram::WrongPlan => {
            let (nodes, root, ty, _) = literal_parts(
                LiteralIr::Number {
                    value: serde_json::Number::from(42),
                },
                TypeRefIr::builtin("number"),
            );
            (
                nodes,
                root,
                ty,
                ValueTransferPlan::FromType {
                    ty: TypeRefIr::builtin("bool"),
                },
            )
        }
        ConstantProgram::WrongStringPlan => (
            vec![FrozenConstantNode::Literal {
                literal: LiteralIr::String {
                    value: "ready".to_string(),
                },
            }],
            0,
            TypeRefIr::builtin("string"),
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial,
            },
        ),
        ConstantProgram::RepresentationLiteral(_) => unreachable!(),
    };

    let type_plan = exact_fixture_plan(&ty);
    artifact.image.pools.types = vec![BytecodePoolEntry::TypeRef {
        ty,
        representation_carrier: None,
        plan: type_plan,
    }];
    artifact.image.pools.shapes = if matches!(
        program,
        ConstantProgram::Record | ConstantProgram::Implementation
    ) {
        vec![BytecodePoolEntry::ShapeRef {
            shape: ShapeDeclaration {
                type_ref: 0,
                plan: ValueTransferPlan::SnapshotShare {
                    drop: skiff_artifact_model::ValueDropPlan::SnapshotRelease,
                },
                privileged_affine_composite: None,
                fields: vec![ShapeFieldDeclaration {
                    name: "value".to_string(),
                    type_ref: 0,
                    plan: ValueTransferPlan::SnapshotShare {
                        drop: skiff_artifact_model::ValueDropPlan::Trivial,
                    },
                }],
            },
        }]
    } else {
        Vec::new()
    };
    artifact.image.pools.constants = vec![BytecodePoolEntry::ConstantRef {
        reference: if program == ConstantProgram::PackageSymbol {
            BytecodeConstantRef::PackageSymbol {
                symbol: PackageSymbolRef {
                    package: PackageRefIr::Dependency {
                        dependency_ref: DEPENDENCY_ALIAS.to_string(),
                    },
                    symbol_path: CONSTANT_ROOT.to_string(),
                    abi_expectation: None,
                },
            }
        } else {
            BytecodeConstantRef::LocalNode {
                node_index: root_node,
            }
        },
        type_ref: 0,
        plan,
    }];
    // Artifact admission permits only LocalNode rows in `constant_roots`.
    // PackageSymbol is therefore necessarily anonymous at the linker gate.
    artifact.image.constant_roots = if matches!(
        program,
        ConstantProgram::Anonymous | ConstantProgram::PackageSymbol
    ) {
        BTreeMap::new()
    } else {
        BTreeMap::from([(CONSTANT_ROOT.to_string(), 0)])
    };
    artifact.image.frozen_constant_graph = FrozenConstantGraph { nodes };
}

pub(super) fn representation_type_symbol(
    program: RootProgram,
) -> Option<(String, PackageLocalAbiSymbol)> {
    let RootProgram::Constant(ConstantProgram::RepresentationLiteral(case)) = program else {
        return None;
    };
    let (descriptor, is_alias) = match case {
        RepresentationLiteralCase::WrongDescriptor => (
            TypeDescriptorIr::Alias {
                target: TypeRefIr::builtin("integer"),
            },
            true,
        ),
        RepresentationLiteralCase::WrongPayload => (
            TypeDescriptorIr::Representation {
                representation: TypeRefIr::builtin("string"),
            },
            false,
        ),
        _ => (
            TypeDescriptorIr::Representation {
                representation: TypeRefIr::builtin("integer"),
            },
            false,
        ),
    };
    Some((
        REPRESENTATION_TYPE.to_string(),
        PackageLocalAbiSymbol::Type {
            local_type_id: "type:fixture.Duration".to_string(),
            descriptor,
            is_alias,
            is_interface: false,
            type_params: Vec::new(),
            interface_methods: Vec::new(),
            actor: None,
        },
    ))
}

fn populate_representation_literal(
    artifact: &mut BytecodeArtifact,
    case: RepresentationLiteralCase,
) {
    let owner = TypeRefIr::PackageSymbol {
        symbol: PackageSymbolRef {
            package: PackageRefIr::PackageId {
                package_id: "example.bytecode-link".to_string(),
            },
            symbol_path: REPRESENTATION_TYPE.to_string(),
            abi_expectation: None,
        },
    };
    let plan = ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::Trivial,
    };
    let physical = if case == RepresentationLiteralCase::WrongPhysicalCarrier {
        TypeRefIr::builtin("bool")
    } else {
        TypeRefIr::builtin("number")
    };
    let representation = if case == RepresentationLiteralCase::WrongPayload {
        TypeRefIr::builtin("string")
    } else {
        TypeRefIr::builtin("integer")
    };
    let (owner_index, representation_index, physical_index, types) = match case {
        RepresentationLiteralCase::ReorderedRows => (
            1,
            2,
            0,
            vec![
                BytecodePoolEntry::TypeRef {
                    ty: physical,
                    representation_carrier: None,
                    plan: plan.clone(),
                },
                BytecodePoolEntry::TypeRef {
                    ty: owner,
                    representation_carrier: Some(
                        skiff_artifact_model::RepresentationCarrierDeclaration {
                            representation_type_ref: 2,
                            physical_carrier_type_ref: 0,
                        },
                    ),
                    plan: plan.clone(),
                },
                BytecodePoolEntry::TypeRef {
                    ty: representation,
                    representation_carrier: None,
                    plan: plan.clone(),
                },
            ],
        ),
        RepresentationLiteralCase::DuplicatePayloadRows => (
            0,
            2,
            3,
            vec![
                BytecodePoolEntry::TypeRef {
                    ty: owner,
                    representation_carrier: Some(
                        skiff_artifact_model::RepresentationCarrierDeclaration {
                            representation_type_ref: 2,
                            physical_carrier_type_ref: 3,
                        },
                    ),
                    plan: plan.clone(),
                },
                BytecodePoolEntry::TypeRef {
                    ty: representation,
                    representation_carrier: None,
                    plan: plan.clone(),
                },
                BytecodePoolEntry::TypeRef {
                    ty: TypeRefIr::builtin("integer"),
                    representation_carrier: None,
                    plan: plan.clone(),
                },
                BytecodePoolEntry::TypeRef {
                    ty: physical,
                    representation_carrier: None,
                    plan: plan.clone(),
                },
            ],
        ),
        _ => (
            0,
            1,
            2,
            vec![
                BytecodePoolEntry::TypeRef {
                    ty: owner,
                    representation_carrier: (case != RepresentationLiteralCase::MissingFact)
                        .then_some(skiff_artifact_model::RepresentationCarrierDeclaration {
                            representation_type_ref: 1,
                            physical_carrier_type_ref: 2,
                        }),
                    plan: plan.clone(),
                },
                BytecodePoolEntry::TypeRef {
                    ty: representation,
                    representation_carrier: None,
                    plan: plan.clone(),
                },
                BytecodePoolEntry::TypeRef {
                    ty: physical,
                    representation_carrier: None,
                    plan: plan.clone(),
                },
            ],
        ),
    };
    artifact.image.pools.types = types;
    artifact.image.pools.constants = vec![BytecodePoolEntry::ConstantRef {
        reference: BytecodeConstantRef::LocalNode { node_index: 0 },
        type_ref: owner_index,
        plan,
    }];
    artifact.image.constant_roots = BTreeMap::from([(CONSTANT_ROOT.to_string(), 0)]);
    artifact.image.frozen_constant_graph = FrozenConstantGraph {
        nodes: vec![number_node()],
    };
    debug_assert_ne!(owner_index, representation_index);
    debug_assert_ne!(owner_index, physical_index);
}

pub(super) fn implementation_symbols(
    bytecode: &ValidatedBytecodeArtifact,
    package_id: &str,
) -> BTreeMap<String, PackageLocalAbiSymbol> {
    implementation_links(bytecode)
        .into_iter()
        .map(|(source_path, export)| {
            (
                source_path.clone(),
                PackageLocalAbiSymbol::Constant {
                    const_id: format!("pkg-const:{package_id}:top-level:{source_path}"),
                    ty: PackageTypeRef::Local {
                        local_type: export.ty,
                    },
                },
            )
        })
        .collect()
}

fn exact_fixture_plan(ty: &TypeRefIr) -> ValueTransferPlan {
    match ty {
        TypeRefIr::Builtin { name, .. }
            if matches!(
                name.as_str(),
                "null" | "bool" | "number" | "integer" | "Date"
            ) =>
        {
            ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::Trivial,
            }
        }
        _ => ValueTransferPlan::SnapshotShare {
            drop: ValueDropPlan::SnapshotRelease,
        },
    }
}

fn literal_parts(
    literal: LiteralIr,
    ty: TypeRefIr,
) -> (Vec<FrozenConstantNode>, u32, TypeRefIr, ValueTransferPlan) {
    (
        vec![FrozenConstantNode::Literal { literal }],
        0,
        ty.clone(),
        exact_fixture_plan(&ty),
    )
}

fn composite_parts(
    nodes: Vec<FrozenConstantNode>,
    root: u32,
) -> (Vec<FrozenConstantNode>, u32, TypeRefIr, ValueTransferPlan) {
    let ty = TypeRefIr::builtin("number");
    (nodes, root, ty.clone(), exact_fixture_plan(&ty))
}

fn number_node() -> FrozenConstantNode {
    FrozenConstantNode::Literal {
        literal: LiteralIr::Number {
            value: serde_json::Number::from(42),
        },
    }
}

pub(super) fn implementation_links(
    bytecode: &ValidatedBytecodeArtifact,
) -> BTreeMap<String, ConstExport> {
    bytecode
        .view()
        .constant_roots()
        .iter()
        .map(|(source_path, pool_index)| {
            let pool_position =
                usize::try_from(*pool_index).expect("validated constant pool index must fit usize");
            let BytecodePoolEntry::ConstantRef { type_ref, .. } = bytecode
                .view()
                .pools()
                .constants
                .get(pool_position)
                .expect("validated constant root must select a pool row")
            else {
                panic!("validated constant root must select a ConstantRef row")
            };
            let type_position =
                usize::try_from(*type_ref).expect("validated type pool index must fit usize");
            let BytecodePoolEntry::TypeRef { ty, .. } = bytecode
                .view()
                .pools()
                .types
                .get(type_position)
                .expect("validated constant type row must exist")
            else {
                panic!("validated constant type row must be a TypeRef")
            };
            let symbol = source_path
                .strip_prefix("fixture.")
                .expect("fixture constant root must use its exact module prefix")
                .to_string();
            (
                source_path.clone(),
                ConstExport {
                    file: FileIrRef::new("file-ir:fixture", "fixture"),
                    const_index: *pool_index,
                    symbol,
                    ty: ty.clone(),
                },
            )
        })
        .collect()
}
