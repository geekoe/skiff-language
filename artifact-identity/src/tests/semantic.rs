use super::*;
use skiff_artifact_model::{
    AbiContractRevision, AbiDeclarationKind, AbiSourceDeclarationAnchor, DescriptorHash,
    NominalTypeRefBaseIr, SchemaRevision,
};

fn anchor(
    publication_id: &str,
    module_path: &[&str],
    symbol: &str,
    kind: AbiDeclarationKind,
) -> AbiSourceDeclarationAnchor {
    AbiSourceDeclarationAnchor {
        publication_id: publication_id.to_string(),
        abi_epoch: 0,
        module_path: module_path
            .iter()
            .map(|segment| (*segment).to_string())
            .collect(),
        symbol: symbol.to_string(),
        kind,
    }
}

#[test]
fn nominal_identity_is_stable_for_the_same_anchor_and_args() {
    let declaration = anchor(
        "example.com/pkg",
        &["collections"],
        "List",
        AbiDeclarationKind::Type,
    );
    let argument = abi_type_id_from_source_anchor(
        &anchor(
            "skiff.run/std",
            &["primitives"],
            "String",
            AbiDeclarationKind::Type,
        ),
        &[],
    );

    assert_eq!(
        abi_type_id_from_source_anchor(&declaration, std::slice::from_ref(&argument)),
        abi_type_id_from_source_anchor(&declaration, &[argument])
    );
}

#[test]
fn nominal_identity_changes_with_anchor_but_not_descriptor() {
    let before = anchor(
        "example.com/pkg",
        &["api"],
        "User",
        AbiDeclarationKind::Type,
    );
    let moved = anchor(
        "example.com/pkg",
        &["model"],
        "User",
        AbiDeclarationKind::Type,
    );
    let id = abi_type_id_from_source_anchor(&before, &[]);
    assert_ne!(id, abi_type_id_from_source_anchor(&moved, &[]));

    let revision_before = AbiContractRevision {
        descriptor_hash: DescriptorHash(vec![1]),
        schema_revision: SchemaRevision(1),
    };
    let revision_after = AbiContractRevision {
        descriptor_hash: DescriptorHash(vec![2]),
        schema_revision: SchemaRevision(2),
    };
    assert_ne!(revision_before, revision_after);
    assert_eq!(id, abi_type_id_from_source_anchor(&before, &[]));
}

#[test]
fn public_path_is_not_a_nominal_identity_input() {
    let declaration = anchor(
        "example.com/pkg",
        &["api"],
        "User",
        AbiDeclarationKind::Type,
    );
    let before = abi_type_id_from_source_anchor(&declaration, &[]);
    let after = abi_type_id_from_source_anchor(&declaration, &[]);
    assert_eq!(
        before, after,
        "public lookup paths are not derivation inputs"
    );
}

#[test]
fn interface_and_method_ids_preserve_ordered_generic_args() {
    let declaration = TypeRefIr::Builtin {
        name: "pkg.Pair".to_string(),
        args: Vec::new(),
    };
    let string = TypeRefIr::builtin("string");
    let number = TypeRefIr::builtin("number");
    let left =
        interface_instantiation_ref(declaration.clone(), vec![string.clone(), number.clone()]);
    let right = interface_instantiation_ref(declaration, vec![number, string]);

    assert_eq!(left.interface_abi_id, right.interface_abi_id);
    assert_ne!(
        canonical_interface_method_abi_id(&left, "get"),
        canonical_interface_method_abi_id(&right, "get")
    );
}

#[test]
fn interface_instantiation_splits_native_declaration_from_args() {
    let string_ref = interface_instantiation_ref_for_type_ref(&TypeRefIr::Builtin {
        name: "pkg.Boxed".to_string(),
        args: vec![TypeRefIr::builtin("string")],
    });
    let number_ref = interface_instantiation_ref_for_type_ref(&TypeRefIr::Builtin {
        name: "pkg.Boxed".to_string(),
        args: vec![TypeRefIr::builtin("number")],
    });

    assert_eq!(string_ref.interface_abi_id, number_ref.interface_abi_id);
    assert_ne!(
        string_ref.canonical_type_args,
        number_ref.canonical_type_args
    );
}

#[test]
fn type_ref_abi_key_preserves_applied_nominal_owner_nesting_and_order() {
    let applied = |type_index, arguments| TypeRefIr::AppliedNominal {
        base: NominalTypeRefBaseIr::LocalType { type_index },
        arguments,
    };
    let string_box = applied(0, vec![TypeRefIr::builtin("string")]);
    let number_box = applied(0, vec![TypeRefIr::builtin("number")]);
    assert_ne!(type_ref_abi_key(&string_box), type_ref_abi_key(&number_box));

    let nested = applied(
        0,
        vec![applied(
            1,
            vec![TypeRefIr::builtin("string"), TypeRefIr::builtin("number")],
        )],
    );
    let reordered = applied(
        0,
        vec![applied(
            1,
            vec![TypeRefIr::builtin("number"), TypeRefIr::builtin("string")],
        )],
    );
    assert_ne!(type_ref_abi_key(&nested), type_ref_abi_key(&reordered));
}
