use std::sync::Arc;

use skiff_runtime_linked_program::{
    ExprRefIr, FileAddr, LinkedExprIr, LinkedFileUnit, LinkedNamedUnionBranch,
    LinkedNominalTypeRefBase, LinkedTypeDescriptor, LinkedTypeRef, PackageRefIr, PackageSymbolRef,
    TypeAddr, TypeDeclIr, UnitAddr,
};
use skiff_runtime_loader::RuntimeAssemblyLoader;

use super::fixtures::CycleFixture;

fn native(name: &str) -> LinkedTypeRef {
    LinkedTypeRef::Native {
        name: name.to_string(),
        args: Vec::new(),
    }
}

fn addr(package_slot: usize, type_index: usize) -> TypeAddr {
    TypeAddr {
        unit: UnitAddr::Package(package_slot),
        file: FileAddr::LoadedFileIndex(0),
        type_index,
    }
}

fn declaration(name: &str, type_params: &[&str], descriptor: LinkedTypeDescriptor) -> TypeDeclIr {
    TypeDeclIr {
        name: name.to_string(),
        descriptor,
        type_params: type_params
            .iter()
            .map(|parameter| (*parameter).to_string())
            .collect(),
        implements: Vec::new(),
        source_span: None,
    }
}

fn append_wrap(file: &mut LinkedFileUnit, value: u32, type_ref: LinkedTypeRef) {
    file.executables[0]
        .body
        .expressions
        .push(LinkedExprIr::RepresentationWrap {
            value: ExprRefIr { expression: value },
            type_ref,
        });
}

fn relink(
    mutate: impl FnOnce(&mut Vec<Vec<Arc<LinkedFileUnit>>>),
) -> anyhow::Result<Vec<Vec<Arc<LinkedFileUnit>>>> {
    let fixture = CycleFixture::new();
    let hydrated = RuntimeAssemblyLoader::new(&fixture.resolver).load(fixture.assembly)?;
    let candidate = crate::assembly::link_runtime_assembly(hydrated)?;
    let mut files = candidate
        .execution_image()
        .code_slots()
        .iter()
        .map(|code| code.files().to_vec())
        .collect::<Vec<_>>();
    mutate(&mut files);
    crate::assembly_execution::relink_execution_files_for_test(
        candidate.shared_image().as_ref(),
        &files,
    )
}

fn relink_shared_wrap(
    prepare: impl FnOnce(&mut LinkedFileUnit),
    type_ref: LinkedTypeRef,
) -> anyhow::Result<Vec<Vec<Arc<LinkedFileUnit>>>> {
    relink(|files| {
        let shared = Arc::make_mut(&mut files[0][0]);
        prepare(shared);
        append_wrap(shared, 0, type_ref);
    })
}

#[test]
fn representation_wrap_links_plain_generic_nested_and_external_targets_exactly() {
    let linked = relink(|files| {
        let (shared_slots, helper_slots) = files.split_at_mut(1);
        let shared = Arc::make_mut(&mut shared_slots[0][0]);
        let helper = Arc::make_mut(&mut helper_slots[0][0]);

        helper.types[0] = declaration(
            "ExternalRepresentation",
            &[],
            LinkedTypeDescriptor::Representation {
                representation: native("bytes"),
            },
        );
        shared.types.push(declaration(
            "PlainRepresentation",
            &[],
            LinkedTypeDescriptor::Representation {
                representation: native("string"),
            },
        ));
        shared.types.push(declaration(
            "GenericRepresentation",
            &["T"],
            LinkedTypeDescriptor::Representation {
                representation: LinkedTypeRef::TypeParam {
                    name: "T".to_string(),
                },
            },
        ));
        shared.types.push(declaration(
            "OuterRepresentation",
            &["T"],
            LinkedTypeDescriptor::Representation {
                representation: LinkedTypeRef::TypeParam {
                    name: "T".to_string(),
                },
            },
        ));

        let generic_string = LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address { addr: addr(0, 2) },
            arguments: vec![native("string")],
        };
        let generic_number = LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address { addr: addr(0, 2) },
            arguments: vec![native("number")],
        };
        let nested = LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address { addr: addr(0, 3) },
            arguments: vec![generic_string.clone()],
        };

        append_wrap(shared, 40, LinkedTypeRef::Address { addr: addr(0, 1) });
        append_wrap(shared, 41, generic_string);
        append_wrap(shared, 42, generic_number);
        append_wrap(shared, 43, nested);
        append_wrap(shared, 44, LinkedTypeRef::Address { addr: addr(1, 0) });
    })
    .expect("all exact representation targets must link");

    let expressions = &linked[0][0].executables[0].body.expressions;
    let wraps = &expressions[expressions.len() - 5..];
    let expected = [
        (40, LinkedTypeRef::Address { addr: addr(0, 1) }),
        (
            41,
            LinkedTypeRef::AppliedNominal {
                base: LinkedNominalTypeRefBase::Address { addr: addr(0, 2) },
                arguments: vec![native("string")],
            },
        ),
        (
            42,
            LinkedTypeRef::AppliedNominal {
                base: LinkedNominalTypeRefBase::Address { addr: addr(0, 2) },
                arguments: vec![native("number")],
            },
        ),
        (
            43,
            LinkedTypeRef::AppliedNominal {
                base: LinkedNominalTypeRefBase::Address { addr: addr(0, 3) },
                arguments: vec![LinkedTypeRef::AppliedNominal {
                    base: LinkedNominalTypeRefBase::Address { addr: addr(0, 2) },
                    arguments: vec![native("string")],
                }],
            },
        ),
        (44, LinkedTypeRef::Address { addr: addr(1, 0) }),
    ];

    for (wrap, (expected_value, expected_type)) in wraps.iter().zip(expected) {
        let LinkedExprIr::RepresentationWrap { value, type_ref } = wrap else {
            panic!("expected representation wrap")
        };
        assert_eq!(value.expression, expected_value);
        assert_eq!(type_ref, &expected_type);
    }
    let LinkedExprIr::RepresentationWrap {
        type_ref: string_target,
        ..
    } = &wraps[1]
    else {
        unreachable!()
    };
    let LinkedExprIr::RepresentationWrap {
        type_ref: number_target,
        ..
    } = &wraps[2]
    else {
        unreachable!()
    };
    let LinkedExprIr::RepresentationWrap {
        type_ref: external_target,
        ..
    } = &wraps[4]
    else {
        unreachable!()
    };
    assert_ne!(string_target, number_target);
    assert_ne!(string_target, external_target);
}

#[test]
fn representation_wrap_rejects_every_non_representation_declaration_kind() {
    let cases = [
        (
            "record",
            LinkedTypeDescriptor::Record {
                fields: Default::default(),
            },
        ),
        (
            "union",
            LinkedTypeDescriptor::Union {
                branches: vec![LinkedNamedUnionBranch::Literal {
                    value: skiff_runtime_linked_program::LiteralIr::String {
                        value: "branch".to_string(),
                    },
                }],
            },
        ),
        (
            "alias",
            LinkedTypeDescriptor::Alias {
                target: native("string"),
            },
        ),
        ("interface", LinkedTypeDescriptor::Interface),
    ];

    for (kind, descriptor) in cases {
        let error = relink_shared_wrap(
            |shared| shared.types[0].descriptor = descriptor,
            LinkedTypeRef::Address { addr: addr(0, 0) },
        )
        .expect_err("non-representation target must fail closed");
        assert!(
            format!("{error:#}").contains(kind),
            "unexpected {kind} error: {error:#}"
        );
    }
}

#[test]
fn representation_wrap_rejects_wrong_arity_owner_and_residual_type_params() {
    let error = relink_shared_wrap(
        |shared| {
            shared.types.push(declaration(
                "GenericRepresentation",
                &["T"],
                LinkedTypeDescriptor::Representation {
                    representation: LinkedTypeRef::TypeParam {
                        name: "T".to_string(),
                    },
                },
            ));
        },
        LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address { addr: addr(0, 1) },
            arguments: vec![native("string"), native("number")],
        },
    )
    .expect_err("wrong representation arity must fail closed");
    assert!(
        format!("{error:#}").contains("arity 2, expected 1"),
        "unexpected arity error: {error:#}"
    );

    let error = relink_shared_wrap(
        |shared| {
            shared.types[0] = declaration(
                "GenericRepresentation",
                &["T"],
                LinkedTypeDescriptor::Representation {
                    representation: LinkedTypeRef::TypeParam {
                        name: "T".to_string(),
                    },
                },
            );
        },
        LinkedTypeRef::AppliedNominal {
            base: LinkedNominalTypeRefBase::Address { addr: addr(0, 0) },
            arguments: vec![LinkedTypeRef::TypeParam {
                name: "Unclosed".to_string(),
            }],
        },
    )
    .expect_err("residual type parameter must fail closed");
    assert!(
        format!("{error:#}").contains("unresolved type parameter"),
        "unexpected type parameter error: {error:#}"
    );

    let error = relink_shared_wrap(
        |_| {},
        LinkedTypeRef::TypeParam {
            name: "Unclosed".to_string(),
        },
    )
    .expect_err("direct residual type parameter must fail closed");
    assert!(
        format!("{error:#}").contains("unresolved type parameter"),
        "unexpected direct type parameter error: {error:#}"
    );

    let error = relink_shared_wrap(
        |_| {},
        LinkedTypeRef::DbObjectSymbol {
            symbol: skiff_runtime_linked_program::ServiceSymbolRef {
                module_path: "shared.main".to_string(),
                symbol: "LocalRecord".to_string(),
            },
        },
    )
    .expect_err("db object symbol must not masquerade as a nominal representation target");
    assert!(
        format!("{error:#}").contains("plain or applied nominal"),
        "unexpected db object target error: {error:#}"
    );

    let error = relink_shared_wrap(
        |_| {},
        LinkedTypeRef::Address {
            addr: addr(usize::MAX, 0),
        },
    )
    .expect_err("wrong package owner must fail closed");
    assert!(
        format!("{error:#}").contains("package code slot"),
        "unexpected owner error: {error:#}"
    );

    let error = relink_shared_wrap(
        |_| {},
        LinkedTypeRef::PackageSymbol {
            symbol: PackageSymbolRef {
                package: PackageRefIr::Dependency {
                    dependency_ref: "helper".to_string(),
                },
                symbol_path: "ExternalRepresentation".to_string(),
                abi_expectation: Some("wrong-local-abi".to_string()),
            },
        },
    )
    .expect_err("wrong external package ABI owner must fail closed");
    assert!(
        format!("{error:#}").contains("local ABI expectation mismatches"),
        "unexpected external owner error: {error:#}"
    );

    let error = relink_shared_wrap(
        |_| {},
        LinkedTypeRef::PackageSchema {
            package_id: "example.models".to_string(),
            stable_schema_key: "Representation".to_string(),
            package_schema_type_id: skiff_artifact_model::PackageSchemaTypeId::new(
                "schema:representation",
            ),
        },
    )
    .expect_err("PackageSchema owner must not enter representation wrap");
    assert!(
        format!("{error:#}").contains("plain or applied nominal"),
        "unexpected PackageSchema error: {error:#}"
    );
}
