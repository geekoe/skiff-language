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
    for ty in [
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
    ] {
        artifact
            .image
            .pools
            .types
            .push(BytecodePoolEntry::TypeRef { ty });
    }
    let shape_ref = artifact.image.pools.shapes.len() as u32;
    artifact
        .image
        .pools
        .shapes
        .push(BytecodePoolEntry::ShapeRef {
            shape: ShapeDeclaration {
                type_ref: handle_type_ref,
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
fn ordinary_shape_cannot_embed_an_affine_resource_field() {
    let mut artifact = canonical_artifact();
    let BytecodePoolEntry::ShapeRef { shape } = &mut artifact.image.pools.shapes[0] else {
        unreachable!("canonical shape")
    };
    shape.fields[0].plan = ValueTransferPlan::AffineResource {
        drop: ResourceDropPlan::ResourceTableRelease,
    };
    assert!(assert_rejected(&artifact)
        .to_string()
        .contains("ordinary shape may not contain an affine resource field"));
}
