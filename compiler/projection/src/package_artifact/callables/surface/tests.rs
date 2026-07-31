use std::collections::BTreeSet;

use skiff_artifact_model::{
    FileIrRef, LiteralIr, NamedUnionBranchIr, PackageExportIndex, TypeDescriptorIr, TypeExport,
    TypeRefIr,
};

use super::*;

#[test]
fn package_schema_public_generic_local_abi_keeps_declaration_kinds_and_named_branches() {
    let record = TypeDescriptorIr::Record {
        fields: BTreeMap::from([(
            "value".to_string(),
            TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
        )]),
    };
    let representation = TypeDescriptorIr::Representation {
        representation: TypeRefIr::TypeParam {
            name: "T".to_string(),
        },
    };
    let named_union = TypeDescriptorIr::Union {
        branches: vec![
            NamedUnionBranchIr::ConcreteNominal {
                nominal_type: TypeRefIr::ServiceSymbol {
                    symbol: skiff_artifact_model::ServiceSymbolRef {
                        module_path: "models".to_string(),
                        symbol: "Ready".to_string(),
                    },
                },
            },
            NamedUnionBranchIr::SyntheticDiscriminator {
                payload_type: TypeRefIr::TypeParam {
                    name: "T".to_string(),
                },
                discriminator_field: "kind".to_string(),
                discriminator_value: "retry".to_string(),
            },
            NamedUnionBranchIr::Literal {
                value: LiteralIr::String {
                    value: "cancelled".to_string(),
                },
            },
        ],
    };
    let export = |type_index, symbol: &str, descriptor, type_params| TypeExport {
        file: FileIrRef::new("file", "models"),
        type_index,
        symbol: symbol.to_string(),
        is_interface: false,
        descriptor: Some(descriptor),
        type_params,
        interface_methods: Vec::new(),
    };
    let interface_methods = vec![skiff_artifact_model::InterfaceMethodSignature {
        name: "read".to_string(),
        type_params: Vec::new(),
        params: vec![skiff_artifact_model::FunctionTypeParamIr {
            name: "fallback".to_string(),
            ty: TypeRefIr::TypeParam {
                name: "T".to_string(),
            },
        }],
        return_type: TypeRefIr::TypeParam {
            name: "T".to_string(),
        },
        is_native: false,
        is_provider: false,
        is_static: false,
        implicit_self: None,
    }];
    let mut interface = export(
        3,
        "Reader",
        TypeDescriptorIr::Interface,
        vec!["T".to_string()],
    );
    interface.is_interface = true;
    interface.interface_methods = interface_methods.clone();
    let projected = ProjectedPackageExportLinks {
        exports: PackageExportIndex {
            types: BTreeMap::from([
                (
                    "example.pkg/RecordBox".to_string(),
                    export(
                        0,
                        "RecordBox",
                        record.clone(),
                        vec!["T".to_string(), "U".to_string()],
                    ),
                ),
                (
                    "example.pkg/RepresentationBox".to_string(),
                    export(
                        1,
                        "RepresentationBox",
                        representation.clone(),
                        vec!["T".to_string()],
                    ),
                ),
                (
                    "example.pkg/Choice".to_string(),
                    export(
                        2,
                        "Choice",
                        named_union.clone(),
                        vec!["Left".to_string(), "Right".to_string()],
                    ),
                ),
                ("example.pkg/Reader".to_string(), interface),
            ]),
            ..PackageExportIndex::default()
        },
        public_instances: Vec::new(),
        alias_types: BTreeSet::new(),
    };

    let symbols = project_non_callable_symbols(&projected).unwrap();
    for (path, expected_descriptor, expected_type_params) in [
        (
            "example.pkg/RecordBox",
            record,
            vec!["T".to_string(), "U".to_string()],
        ),
        (
            "example.pkg/RepresentationBox",
            representation,
            vec!["T".to_string()],
        ),
        (
            "example.pkg/Choice",
            named_union,
            vec!["Left".to_string(), "Right".to_string()],
        ),
        (
            "example.pkg/Reader",
            TypeDescriptorIr::Interface,
            vec!["T".to_string()],
        ),
    ] {
        let PackageLocalAbiSymbol::Type {
            descriptor,
            type_params,
            ..
        } = &symbols[path]
        else {
            panic!("{path} must remain a typed package-local declaration");
        };
        assert_eq!(descriptor, &expected_descriptor);
        assert_eq!(type_params, &expected_type_params);
    }
    let PackageLocalAbiSymbol::Type {
        is_interface,
        interface_methods: actual_methods,
        ..
    } = &symbols["example.pkg/Reader"]
    else {
        unreachable!("Reader type checked above");
    };
    assert!(*is_interface);
    assert_eq!(actual_methods, &interface_methods);
}
