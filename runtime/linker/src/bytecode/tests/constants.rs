use skiff_artifact_model::{LiteralIr, Opcode, TypeRefIr};
use skiff_runtime_linked_bytecode::{
    FrozenConstantNodeIndex, LinkedConstantReference, LinkedFrozenConstantValue,
    LinkedInstructionTarget, LinkedValueDropPlan, LinkedValueTransferPlan,
};

use crate::bytecode::{
    link_deployment_execution_image, BytecodeLinkError, BytecodeLinkLimit, BytecodeLinkLocation,
    BytecodeLinkObligation, DeploymentExecutionImageError,
};

use super::{
    execution_limits,
    fixtures::{
        ConstantProgram, Fixture, RepresentationLiteralCase, HELPER_FUNCTION, ROOT_FUNCTION,
    },
    generous_execution_limits, generous_limits,
};

mod multi_package;

#[test]
fn package_global_number_literal_links_exact_authority_and_const_use() {
    let fixture = Fixture::constant(ConstantProgram::Number);
    let hydrated = fixture.hydrate();
    let image = link_deployment_execution_image(hydrated, &generous_execution_limits()).unwrap();

    assert_eq!(image.constants().len(), 1);
    assert_eq!(image.constant_roots().len(), 1);
    assert_eq!(image.frozen_constant_nodes().len(), 1);
    let constant = &image.constants()[0];
    assert_eq!(constant.index().get(), 0);
    assert_eq!(
        constant.origin().package_build_id(),
        &fixture.package_reference.package_build_id
    );
    assert_eq!(constant.origin().artifact_index().get(), 0);
    assert!(constant.origin().specialization().is_none());
    assert!(matches!(
        constant.reference(),
        LinkedConstantReference::LocalNode { node } if node.get() == 0
    ));
    assert_eq!(
        constant.plan(),
        &LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::Trivial,
        }
    );

    let root = &image.constant_roots()[0];
    assert_eq!(
        root.owner_package_build_id(),
        &fixture.package_reference.package_build_id
    );
    assert_eq!(root.symbol_path().as_str(), "fixture.answer");
    assert_eq!(root.constant(), constant.index());
    let node = &image.frozen_constant_nodes()[0];
    assert!(node.origin().specialization().is_none());
    assert!(matches!(
        node.value(),
        LinkedFrozenConstantValue::Literal(LiteralIr::Number { value })
            if value == &serde_json::Number::from(42)
    ));

    let type_position = usize::try_from(constant.ty().get()).unwrap();
    let ty = &image.types()[type_position];
    assert_eq!(ty.type_ref(), &TypeRefIr::builtin("number"));
    assert!(ty.origin().specialization().is_none());

    let function = image
        .functions()
        .iter()
        .find(|function| function.key().artifact_function_key().as_str() == ROOT_FUNCTION)
        .unwrap();
    assert_eq!(function.instructions()[0].opcode(), Opcode::Const);
    assert_eq!(
        function.instructions()[0].resolved_operands()[0].target(),
        LinkedInstructionTarget::Constant(constant.index())
    );
    assert_eq!(function.stack_map().entries()[1].stack_before().len(), 1);
    assert_eq!(
        function.stack_map().entries()[1].stack_before()[0].ty(),
        constant.ty()
    );
}

#[test]
fn package_global_string_literal_links_its_exact_compiler_lifecycle() {
    let fixture = Fixture::constant(ConstantProgram::LiteralString);
    let hydrated = fixture.hydrate();
    let image = link_deployment_execution_image(hydrated, &generous_execution_limits()).unwrap();
    assert_eq!(image.constants().len(), 1);
    assert_eq!(
        image.constants()[0].plan(),
        &LinkedValueTransferPlan::SnapshotShare {
            drop: LinkedValueDropPlan::SnapshotRelease,
        }
    );
    assert!(matches!(
        image.frozen_constant_nodes()[0].value(),
        LinkedFrozenConstantValue::Literal(LiteralIr::String { value }) if value == "ready"
    ));
}

#[test]
fn representation_literal_uses_exact_direct_artifact_rows_after_reorder_and_duplication() {
    for (case, representation_artifact_index, physical_artifact_index) in [
        (RepresentationLiteralCase::Exact, 1, 2),
        (RepresentationLiteralCase::ReorderedRows, 2, 0),
        (RepresentationLiteralCase::DuplicatePayloadRows, 2, 3),
    ] {
        let fixture = Fixture::representation_literal(case);
        let image =
            link_deployment_execution_image(fixture.hydrate(), &generous_execution_limits())
                .expect("exact representation literal fact should link");
        let owner = image.constants()[0].ty();
        let carrier = image
            .type_representation_carrier(owner)
            .expect("constant owner exposes its exact linked carrier fact");
        let representation = &image.types()[carrier.representation_type().get() as usize];
        let physical = &image.types()[carrier.physical_carrier_type().get() as usize];
        assert_eq!(representation.type_ref(), &TypeRefIr::builtin("integer"));
        assert_eq!(physical.type_ref(), &TypeRefIr::builtin("number"));
        assert_eq!(
            representation.origin().artifact_index().get(),
            representation_artifact_index,
        );
        assert_eq!(
            physical.origin().artifact_index().get(),
            physical_artifact_index,
        );
        assert!(representation.origin().specialization().is_none());
        assert!(physical.origin().specialization().is_none());
    }
}

#[test]
fn representation_literal_rejects_missing_or_inconsistent_exact_fact() {
    let missing = Fixture::representation_literal(RepresentationLiteralCase::MissingFact);
    assert!(matches!(
        link_deployment_execution_image(missing.hydrate(), &generous_execution_limits()),
        Err(DeploymentExecutionImageError::Link(
            BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ConstantInitializationPlan,
                ..
            }
        ))
    ));

    for case in [
        RepresentationLiteralCase::WrongDescriptor,
        RepresentationLiteralCase::WrongPayload,
        RepresentationLiteralCase::WrongPhysicalCarrier,
    ] {
        let fixture = Fixture::representation_literal(case);
        assert!(matches!(
            link_deployment_execution_image(fixture.hydrate(), &generous_execution_limits()),
            Err(DeploymentExecutionImageError::Link(
                BytecodeLinkError::UnsatisfiedObligation {
                    obligation: BytecodeLinkObligation::ConcreteTypeAndShapeTables,
                    ..
                }
            ))
        ));
    }
}

#[test]
fn every_supported_scalar_literal_uses_its_exact_type_and_lifecycle() {
    for (program, literal, name, drop) in [
        (
            ConstantProgram::Null,
            LiteralIr::Null,
            "null",
            LinkedValueDropPlan::Trivial,
        ),
        (
            ConstantProgram::Bool,
            LiteralIr::Bool { value: true },
            "bool",
            LinkedValueDropPlan::Trivial,
        ),
        (
            ConstantProgram::Number,
            LiteralIr::Number {
                value: serde_json::Number::from(42),
            },
            "number",
            LinkedValueDropPlan::Trivial,
        ),
    ] {
        let fixture = Fixture::constant(program);
        let hydrated = fixture.hydrate();
        let image =
            link_deployment_execution_image(hydrated, &generous_execution_limits()).unwrap();
        let constant = &image.constants()[0];
        let type_position = usize::try_from(constant.ty().get()).unwrap();
        assert_eq!(
            image.types()[type_position].type_ref(),
            &TypeRefIr::builtin(name)
        );
        assert_eq!(
            constant.plan(),
            &LinkedValueTransferPlan::SnapshotShare { drop }
        );
        assert!(matches!(
            image.frozen_constant_nodes()[0].value(),
            LinkedFrozenConstantValue::Literal(actual) if actual == &literal
        ));
        let function = image
            .functions()
            .iter()
            .find(|function| function.key().artifact_function_key().as_str() == ROOT_FUNCTION)
            .unwrap();
        assert_eq!(
            function.stack_map().entries()[1].stack_before()[0].plan(),
            constant.plan()
        );
    }
}

#[test]
fn composite_graph_kinds_link_exact_authority_and_values() {
    for program in [
        ConstantProgram::Array,
        ConstantProgram::Record,
        ConstantProgram::Representation,
        ConstantProgram::Implementation,
    ] {
        let fixture = Fixture::constant(program);
        let hydrated = fixture.hydrate();
        let image =
            link_deployment_execution_image(hydrated, &generous_execution_limits()).unwrap();
        assert_eq!(image.constants().len(), 1);
        assert_eq!(image.constant_roots().len(), 1);
        let constant = &image.constants()[0];
        assert!(image.constant_heap().get(constant.index()).is_some());
        let root = &image.constant_roots()[0];
        assert_eq!(root.constant(), constant.index());
        match program {
            ConstantProgram::Array => {
                assert_eq!(image.frozen_constant_nodes().len(), 2);
                let node = &image.frozen_constant_nodes()[1];
                assert!(matches!(
                    node.value(),
                    LinkedFrozenConstantValue::Array { children }
                        if children.as_ref() == [FrozenConstantNodeIndex::new(0)]
                ));
                let array_type = &image.types()[constant.ty().get() as usize];
                assert_eq!(
                    array_type.type_ref(),
                    &TypeRefIr::Builtin {
                        name: "Array".to_string(),
                        args: vec![TypeRefIr::builtin("string")],
                    }
                );
            }
            ConstantProgram::Record => {
                assert_eq!(image.frozen_constant_nodes().len(), 2);
                let node = &image.frozen_constant_nodes()[1];
                assert!(matches!(
                    node.value(),
                    LinkedFrozenConstantValue::Record { shape, children }
                        if shape.get() == 0
                            && children.as_ref() == [FrozenConstantNodeIndex::new(0)]
                ));
                let shape = &image.shapes()[0];
                assert_eq!(shape.nominal_type(), constant.ty());
                assert_eq!(shape.fields().len(), 1);
            }
            ConstantProgram::Representation => {
                assert_eq!(image.frozen_constant_nodes().len(), 2);
                let node = &image.frozen_constant_nodes()[1];
                assert!(matches!(
                    node.value(),
                    LinkedFrozenConstantValue::Representation { ty, value }
                        if *ty == constant.ty() && *value == FrozenConstantNodeIndex::new(0)
                ));
                let carrier = image
                    .type_representation_carrier(constant.ty())
                    .expect("representation constant exposes its exact carrier");
                assert_eq!(
                    image.types()[carrier.physical_carrier_type().get() as usize].type_ref(),
                    &TypeRefIr::builtin("number")
                );
            }
            ConstantProgram::Implementation => {
                assert_eq!(image.frozen_constant_nodes().len(), 3);
                let node = &image.frozen_constant_nodes()[2];
                let LinkedFrozenConstantValue::Implementation { record, behaviors } = node.value()
                else {
                    panic!("implementation constant must retain an Implementation graph node");
                };
                assert_eq!(record.get(), 1);
                assert_eq!(behaviors.len(), 1);
                assert_eq!(
                    behaviors[0].artifact_function_key().as_str(),
                    HELPER_FUNCTION
                );
                let function = &image.functions()[behaviors[0].function().get() as usize];
                assert_eq!(
                    function.key().artifact_function_key().as_str(),
                    HELPER_FUNCTION
                );
            }
            _ => unreachable!("positive composite graph fixture"),
        }
    }
}

#[test]
fn composite_graph_malformed_or_missing_plans_fail_closed() {
    for program in [
        ConstantProgram::RecordWrongFieldPlan,
        ConstantProgram::RepresentationMissingCarrier,
        ConstantProgram::ImplementationMissingReceiver,
        ConstantProgram::AmbiguousArray,
    ] {
        let fixture = Fixture::constant(program);
        let hydrated = fixture.hydrate();
        let error = link_deployment_execution_image(hydrated, &generous_execution_limits())
            .expect_err("malformed constant graph must fail closed");
        assert!(
            matches!(
                error,
                DeploymentExecutionImageError::Link(BytecodeLinkError::UnsatisfiedObligation {
                    obligation: BytecodeLinkObligation::ConstantInitializationPlan,
                    location: BytecodeLinkLocation::Constant { .. },
                    ..
                })
            ),
            "unexpected malformed graph error: {error:?}"
        );
    }
}

#[test]
fn anonymous_literal_rows_link_while_package_symbol_rows_remain_fail_closed() {
    let anonymous = Fixture::constant(ConstantProgram::Anonymous);
    let hydrated = anonymous.hydrate();
    let image = link_deployment_execution_image(hydrated, &generous_execution_limits()).unwrap();
    assert_eq!(image.constants().len(), 1);
    assert!(image.constant_roots().is_empty());
    assert!(matches!(
        image.constants()[0].reference(),
        LinkedConstantReference::LocalNode { .. }
    ));

    let package_symbol = Fixture::package_symbol_constant();
    let hydrated = package_symbol.hydrate();
    assert!(matches!(
        link_deployment_execution_image(hydrated, &generous_execution_limits()),
        Err(DeploymentExecutionImageError::Link(
            BytecodeLinkError::ImplementationUnavailable {
                obligation: BytecodeLinkObligation::ConstantInitializationPlan,
                location: BytecodeLinkLocation::Package { .. },
            }
        ))
    ));
}

#[test]
fn literal_carrier_and_from_type_must_match_exactly() {
    for program in [
        ConstantProgram::WrongCarrier,
        ConstantProgram::LiteralMismatch,
        ConstantProgram::WrongPlan,
        ConstantProgram::WrongStringPlan,
    ] {
        let fixture = Fixture::constant(program);
        let hydrated = fixture.hydrate();
        assert!(matches!(
            link_deployment_execution_image(hydrated, &generous_execution_limits()),
            Err(DeploymentExecutionImageError::Link(
                BytecodeLinkError::UnsatisfiedObligation {
                    obligation: BytecodeLinkObligation::ConstantInitializationPlan,
                    location: BytecodeLinkLocation::Constant { node_index: 0, .. },
                    ..
                }
            ))
        ));
    }
}

#[test]
fn constant_graph_node_and_edge_limits_are_aggregate_gates() {
    let number = Fixture::constant(ConstantProgram::Number);
    let number_hydrated = number.hydrate();
    let mut node_limits = generous_limits();
    node_limits.max_constant_graph_nodes = 0;
    assert!(matches!(
        link_deployment_execution_image(number_hydrated, &execution_limits(node_limits)),
        Err(DeploymentExecutionImageError::Link(
            BytecodeLinkError::LimitExceeded {
                limit: BytecodeLinkLimit::ConstantGraphNodes,
                actual: 1,
                max: 0,
                ..
            }
        ))
    ));

    let array = Fixture::constant(ConstantProgram::Array);
    let array_hydrated = array.hydrate();
    let mut edge_limits = generous_limits();
    edge_limits.max_constant_graph_edges = 0;
    assert!(matches!(
        link_deployment_execution_image(array_hydrated, &execution_limits(edge_limits)),
        Err(DeploymentExecutionImageError::Link(
            BytecodeLinkError::LimitExceeded {
                limit: BytecodeLinkLimit::ConstantGraphEdges,
                actual: 1,
                max: 0,
                ..
            }
        ))
    ));
}
