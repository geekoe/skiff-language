//! Structural closure tests for compiler-owned representation carrier facts.

use super::*;

fn fact(
    representation_type_ref: u32,
    physical_carrier_type_ref: u32,
) -> crate::bytecode::dto::RepresentationCarrierDeclaration {
    crate::bytecode::dto::RepresentationCarrierDeclaration {
        representation_type_ref,
        physical_carrier_type_ref,
    }
}

fn representation_artifact() -> BytecodeArtifact {
    let mut artifact = canonical_artifact();
    artifact
        .image
        .pools
        .types
        .push(type_entry(TypeRefIr::builtin("integer")));
    artifact.image.pools.types.push(BytecodePoolEntry::TypeRef {
        ty: TypeRefIr::builtin("fixtureRepresentation"),
        representation_carrier: Some(fact(2, 1)),
        plan: snapshot_share(),
    });
    artifact
}

fn carrier_mut(
    artifact: &mut BytecodeArtifact,
) -> &mut crate::bytecode::dto::RepresentationCarrierDeclaration {
    let BytecodePoolEntry::TypeRef {
        representation_carrier: Some(declaration),
        ..
    } = &mut artifact.image.pools.types[3]
    else {
        unreachable!("representation fixture owns one carrier declaration")
    };
    declaration
}

#[test]
fn representation_carrier_accepts_exact_bounded_one_layer_closure() {
    let artifact = representation_artifact();
    assert_validates(&artifact);

    let value = serde_json::to_value(&artifact.image.pools.types[3]).unwrap();
    assert_eq!(
        value["representationCarrier"],
        serde_json::json!({
            "representationTypeRef": 2,
            "physicalCarrierTypeRef": 1,
        })
    );
    let decoded: BytecodePoolEntry = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, artifact.image.pools.types[3]);
}

#[test]
fn representation_carrier_wire_requires_both_exact_refs() {
    let value = serde_json::to_value(&representation_artifact().image.pools.types[3]).unwrap();
    for field in ["representationTypeRef", "physicalCarrierTypeRef"] {
        let mut missing = value.clone();
        missing["representationCarrier"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        let error = serde_json::from_value::<BytecodePoolEntry>(missing)
            .expect_err("representation carrier refs are required")
            .to_string();
        assert!(error.contains("missing field"), "{field}: {error}");
    }

    let mut unknown = value;
    unknown["representationCarrier"]["legacyCarrier"] = serde_json::json!(1);
    let error = serde_json::from_value::<BytecodePoolEntry>(unknown)
        .expect_err("unknown representation carrier fields must fail closed")
        .to_string();
    assert!(error.contains("unknown field"), "{error}");
}

#[test]
fn representation_carrier_rejects_out_of_bounds_and_wrong_kind_refs() {
    for field in ["representationTypeRef", "physicalCarrierTypeRef"] {
        let mut artifact = representation_artifact();
        let out_of_bounds = artifact.image.pools.types.len() as u32;
        let declaration = carrier_mut(&mut artifact);
        match field {
            "representationTypeRef" => declaration.representation_type_ref = out_of_bounds,
            "physicalCarrierTypeRef" => declaration.physical_carrier_type_ref = out_of_bounds,
            _ => unreachable!(),
        }
        let error = assert_rejected(&artifact);
        assert!(
            error.to_string().contains(field) && error.to_string().contains("out of bounds"),
            "{field}: {error}"
        );
    }

    let mut wrong_kind = representation_artifact();
    wrong_kind.image.pools.types[2] = BytecodePoolEntry::ShapeRef {
        shape: ShapeDeclaration {
            type_ref: 0,
            plan: snapshot_share(),
            privileged_affine_composite: None,
            fields: Vec::new(),
        },
    };
    let error = assert_rejected(&wrong_kind);
    assert!(
        error
            .to_string()
            .contains("image.pools.types[2] has incompatible entry kind"),
        "{error}"
    );
}

#[test]
fn representation_carrier_rejects_self_and_aliased_refs() {
    for field in ["representationTypeRef", "physicalCarrierTypeRef"] {
        let mut artifact = representation_artifact();
        let declaration = carrier_mut(&mut artifact);
        match field {
            "representationTypeRef" => declaration.representation_type_ref = 3,
            "physicalCarrierTypeRef" => declaration.physical_carrier_type_ref = 3,
            _ => unreachable!(),
        }
        let error = assert_rejected(&artifact);
        assert!(
            error.to_string().contains(field) && error.to_string().contains("self-reference"),
            "{field}: {error}"
        );
    }

    let mut aliased = representation_artifact();
    carrier_mut(&mut aliased).representation_type_ref = 1;
    let error = assert_rejected(&aliased);
    assert!(
        error.to_string().contains(
            "representationTypeRef and physicalCarrierTypeRef must reference distinct TypeRef rows"
        ),
        "{error}"
    );
}

#[test]
fn representation_carrier_rejects_nested_and_cyclic_facts() {
    let mut nested = representation_artifact();
    let BytecodePoolEntry::TypeRef {
        representation_carrier,
        ..
    } = &mut nested.image.pools.types[2]
    else {
        unreachable!("type pool is homogeneous")
    };
    *representation_carrier = Some(fact(0, 1));
    let error = assert_rejected(&nested);
    assert!(
        error.to_string().contains("one-layer TypeRef row"),
        "{error}"
    );

    let mut nested_physical = representation_artifact();
    let BytecodePoolEntry::TypeRef {
        representation_carrier,
        ..
    } = &mut nested_physical.image.pools.types[1]
    else {
        unreachable!("type pool is homogeneous")
    };
    *representation_carrier = Some(fact(0, 2));
    let error = assert_rejected(&nested_physical);
    assert!(
        error.to_string().contains("physicalCarrierTypeRef")
            && error.to_string().contains("one-layer TypeRef row"),
        "{error}"
    );

    let mut cycle = representation_artifact();
    let BytecodePoolEntry::TypeRef {
        representation_carrier,
        ..
    } = &mut cycle.image.pools.types[2]
    else {
        unreachable!("type pool is homogeneous")
    };
    *representation_carrier = Some(fact(3, 0));
    let error = assert_rejected(&cycle);
    assert!(
        error.to_string().contains("one-layer TypeRef row"),
        "{error}"
    );
}

#[test]
fn representation_carrier_requires_exact_plan_consistency() {
    let mut wrong_representation_plan = representation_artifact();
    let BytecodePoolEntry::TypeRef {
        plan: entry_plan, ..
    } = &mut wrong_representation_plan.image.pools.types[2]
    else {
        unreachable!("type pool is homogeneous")
    };
    *entry_plan = plan(ValueTransferPlanKind::MoveOnly);
    let error = assert_rejected(&wrong_representation_plan);
    assert!(
        error
            .to_string()
            .contains("representationTypeRef plan must exactly match"),
        "{error}"
    );

    let mut wrong_physical_plan = representation_artifact();
    let BytecodePoolEntry::TypeRef {
        plan: entry_plan, ..
    } = &mut wrong_physical_plan.image.pools.types[1]
    else {
        unreachable!("type pool is homogeneous")
    };
    *entry_plan = plan(ValueTransferPlanKind::MoveOnly);
    let error = assert_rejected(&wrong_physical_plan);
    assert!(
        error
            .to_string()
            .contains("physicalCarrierTypeRef plan must exactly match"),
        "{error}"
    );
}
