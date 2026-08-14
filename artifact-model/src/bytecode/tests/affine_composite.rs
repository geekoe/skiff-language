//! Structural authority for the one registry-owned record with an affine child.

use super::*;

fn package_type(symbol_path: &str) -> TypeRefIr {
    TypeRefIr::PackageSymbol {
        symbol: crate::PackageSymbolRef {
            package: crate::PackageRefIr::PackageId {
                package_id: "skiff.run/std".to_string(),
            },
            symbol_path: symbol_path.to_string(),
            abi_expectation: None,
        },
    }
}

fn affine_take_artifact() -> BytecodeArtifact {
    let mut artifact = canonical_artifact();
    let handle_type_ref = artifact.image.pools.types.len() as u32;
    let shape_ref = artifact.image.pools.shapes.len() as u32;
    for (ordinal, ty) in [
        package_type("std.http.HttpClientStreamHandle"),
        TypeRefIr::Builtin {
            name: "Stream".to_string(),
            args: vec![TypeRefIr::builtin("bytes")],
        },
        TypeRefIr::Builtin {
            name: "Array".to_string(),
            args: vec![package_type("std.http.HttpHeader")],
        },
        TypeRefIr::builtin("integer"),
    ]
    .into_iter()
    .enumerate()
    {
        let plan = match ordinal {
            0 => ValueTransferPlan::MoveOnly {
                drop: ValueDropPlan::RecursiveShape { shape_ref },
            },
            1 => ValueTransferPlan::AffineResource {
                drop: ResourceDropPlan::ResourceTableRelease,
            },
            2 => ValueTransferPlan::SnapshotShare {
                drop: ValueDropPlan::SnapshotRelease,
            },
            _ => snapshot_share(),
        };
        artifact.image.pools.types.push(BytecodePoolEntry::TypeRef {
            ty,
            representation_carrier: None,
            plan,
        });
    }
    artifact
        .image
        .pools
        .shapes
        .push(BytecodePoolEntry::ShapeRef {
            shape: ShapeDeclaration {
                type_ref: handle_type_ref,
                plan: ValueTransferPlan::MoveOnly {
                    drop: ValueDropPlan::RecursiveShape { shape_ref },
                },
                privileged_affine_composite: Some(
                    crate::PrivilegedAffineCompositeIdentity::HttpClientStreamHandle,
                ),
                fields: vec![
                    ShapeFieldDeclaration {
                        name: "body".to_string(),
                        type_ref: handle_type_ref + 1,
                        plan: ValueTransferPlan::AffineResource {
                            drop: ResourceDropPlan::ResourceTableRelease,
                        },
                    },
                    ShapeFieldDeclaration {
                        name: "headers".to_string(),
                        type_ref: handle_type_ref + 2,
                        plan: ValueTransferPlan::SnapshotShare {
                            drop: ValueDropPlan::SnapshotRelease,
                        },
                    },
                    ShapeFieldDeclaration {
                        name: "status".to_string(),
                        type_ref: handle_type_ref + 3,
                        plan: snapshot_share(),
                    },
                ],
            },
        });

    let mut function = callback_function();
    function.function_key = "module::affine_take".to_string();
    function.origin = BytecodeFunctionOrigin::Executable {
        executable: executable_coordinate(2),
    };
    function.words = vec![0x07, 0, 0x44, shape_ref, 0, 0x08, 0x25];
    function.frame_layout = FrameLayout {
        slot_count: 1,
        slot_type_refs: vec![handle_type_ref],
        parameter_slots: Vec::new(),
        writable_local_slots: Vec::new(),
        result_count: 0,
        result_type_refs: Vec::new(),
        result_plans: Vec::new(),
        stream_result_type_ref: None,
        slot_plans: vec![ValueTransferPlan::MoveOnly {
            drop: ValueDropPlan::RecursiveShape { shape_ref },
        }],
    };
    function.max_operand_depth = 1;
    function.effect_summary_ref = crate::PackageCallableId::new("operation:module:affine_take");
    function.statement_entries.clear();
    function.source_map.clear();
    artifact
        .image
        .functions
        .insert(function.function_key.clone(), function);
    artifact
}

#[test]
fn take_dense_field_requires_exact_privileged_affine_field() {
    let artifact = affine_take_artifact();
    assert_validates(&artifact);

    let mut shared = artifact.clone();
    shared
        .image
        .functions
        .get_mut("module::affine_take")
        .unwrap()
        .words[2] = 0x41;
    assert!(assert_rejected(&shared)
        .to_string()
        .contains("GetDenseField may not share a privileged affine field"));

    let mut wrong_field = artifact;
    wrong_field
        .image
        .functions
        .get_mut("module::affine_take")
        .unwrap()
        .words[4] = 1;
    assert!(assert_rejected(&wrong_field)
        .to_string()
        .contains("TakeDenseField requires the exact affine-take field"));
}

#[test]
fn ordinary_shape_fields_require_exact_non_recursive_snapshot_plans() {
    let mut artifact = canonical_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = &mut artifact.image.pools.shapes[0] else {
        unreachable!("canonical shape")
    };
    shape.fields[0].plan = ValueTransferPlan::AffineResource {
        drop: ResourceDropPlan::ResourceTableRelease,
    };
    assert!(assert_rejected(&artifact)
        .to_string()
        .contains("ordinary shape fields require explicit non-recursive SnapshotShare plans"));

    let mut deferred = canonical_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = &mut deferred.image.pools.shapes[0] else {
        unreachable!("canonical shape")
    };
    shape.fields[0].plan = ValueTransferPlan::FromType { ty: number_type() };
    assert!(assert_rejected(&deferred)
        .to_string()
        .contains("ordinary shape fields require explicit non-recursive SnapshotShare plans"));
}

#[test]
fn shape_roots_require_exact_compiler_owned_lifecycle_plans() {
    let mut ordinary = canonical_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = &mut ordinary.image.pools.shapes[0] else {
        unreachable!("canonical shape")
    };
    shape.plan = ValueTransferPlan::FromType {
        ty: TypeRefIr::Record {
            fields: BTreeMap::from([("value".to_string(), number_type())]),
        },
    };
    assert!(assert_rejected(&ordinary)
        .to_string()
        .contains("ordinary shape requires an explicit SnapshotShare/SnapshotRelease root plan"));

    let mut privileged = affine_take_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = privileged
        .image
        .pools
        .shapes
        .last_mut()
        .expect("privileged shape row")
    else {
        unreachable!("shape pool is homogeneous")
    };
    shape.plan = ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::SnapshotRelease,
    };
    let error = assert_rejected(&privileged);
    assert!(
        error
            .to_string()
            .contains("plan must exactly match its referenced privileged shape root"),
        "{error}"
    );
}

#[test]
fn privileged_type_plan_must_match_its_exact_shape_root() {
    let mut wrong_plan = affine_take_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = wrong_plan
        .image
        .pools
        .shapes
        .last()
        .expect("privileged shape row")
    else {
        unreachable!("shape pool is homogeneous")
    };
    let type_ref = shape.type_ref as usize;
    let BytecodePoolEntry::TypeRef { plan, .. } = &mut wrong_plan.image.pools.types[type_ref]
    else {
        unreachable!("type pool is homogeneous")
    };
    *plan = ValueTransferPlan::SnapshotShare {
        drop: ValueDropPlan::SnapshotRelease,
    };
    assert!(assert_rejected(&wrong_plan)
        .to_string()
        .contains("plan must exactly match its owning TypeRef plan"));

    let mut wrong_shape = affine_take_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = wrong_shape
        .image
        .pools
        .shapes
        .last()
        .expect("privileged shape row")
    else {
        unreachable!("shape pool is homogeneous")
    };
    let type_ref = shape.type_ref as usize;
    let BytecodePoolEntry::TypeRef { plan, .. } = &mut wrong_shape.image.pools.types[type_ref]
    else {
        unreachable!("type pool is homogeneous")
    };
    *plan = ValueTransferPlan::MoveOnly {
        drop: ValueDropPlan::RecursiveShape { shape_ref: 0 },
    };
    assert!(assert_rejected(&wrong_shape)
        .to_string()
        .contains("recursive MoveOnly shape lacks privileged affine composite authority"));

    let mut out_of_bounds = affine_take_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = out_of_bounds
        .image
        .pools
        .shapes
        .last()
        .expect("privileged shape row")
    else {
        unreachable!("shape pool is homogeneous")
    };
    let type_ref = shape.type_ref as usize;
    let BytecodePoolEntry::TypeRef { plan, .. } = &mut out_of_bounds.image.pools.types[type_ref]
    else {
        unreachable!("type pool is homogeneous")
    };
    *plan = ValueTransferPlan::MoveOnly {
        drop: ValueDropPlan::RecursiveShape {
            shape_ref: u32::MAX,
        },
    };
    let error = assert_rejected(&out_of_bounds);
    assert!(
        error.to_string().contains("out of bounds of shapes pool"),
        "{error}"
    );
}

#[test]
fn one_type_row_cannot_claim_two_privileged_shape_roots() {
    let mut artifact = affine_take_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = artifact
        .image
        .pools
        .shapes
        .last()
        .expect("privileged shape row")
    else {
        unreachable!("shape pool is homogeneous")
    };
    let mut duplicate = shape.clone();
    let duplicate_ref = artifact.image.pools.shapes.len() as u32;
    duplicate.plan = ValueTransferPlan::MoveOnly {
        drop: ValueDropPlan::RecursiveShape {
            shape_ref: duplicate_ref,
        },
    };
    artifact
        .image
        .pools
        .shapes
        .push(BytecodePoolEntry::ShapeRef { shape: duplicate });
    assert!(assert_rejected(&artifact)
        .to_string()
        .contains("plan must exactly match its owning TypeRef plan"));
}
