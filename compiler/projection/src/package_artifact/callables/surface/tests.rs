use std::collections::BTreeSet;

use skiff_artifact_model::{
    FileIrRef, LiteralIr, NamedUnionBranchIr, PackageExportIndex, TypeDescriptorIr, TypeExport,
    TypeRefIr,
};
use skiff_compiler_core::{
    canonical_implementation_callable_source_path, implementation_package_callable_id,
    public_package_callable_id, ImplementationCallableKind,
};

use super::*;

#[test]
fn callable_identity_projection_stays_aligned_with_core_helpers(
) -> Result<(), crate::error::ProjectionError> {
    let package_id = "example.pkg";
    let module_path = "api";
    let executable_symbol = "api.Worker<T>.handle";
    let kind = ImplementationCallableKind::ImplMethod;
    let (source_path, implementation_id) = super::super::project_implementation_callable_identity(
        package_id,
        module_path,
        executable_symbol,
        kind,
    )?;
    assert_eq!(
        canonical_implementation_callable_source_path(module_path, executable_symbol, kind),
        Ok(source_path.clone())
    );
    assert_eq!(source_path, "api.Worker.handle");
    assert_eq!(
        implementation_package_callable_id(package_id, module_path, executable_symbol, kind),
        Ok(implementation_id.clone())
    );
    assert_eq!(
        implementation_id.as_str(),
        "pkg-callable:example.pkg:top-level:api.Worker.handle"
    );

    for (public_path, expected_bytes) in [
        ("run", "pkg-callable:example.pkg:run"),
        ("worker.handle", "pkg-callable:example.pkg:worker.handle"),
    ] {
        let projected_id =
            super::super::signatures::project_public_callable_id(package_id, public_path)?;
        assert_eq!(
            public_package_callable_id(package_id, public_path),
            Ok(projected_id.clone())
        );
        assert_eq!(projected_id.as_str(), expected_bytes);
        assert_ne!(projected_id, implementation_id);
    }

    match super::super::signatures::project_public_callable_id(package_id, "") {
        Err(crate::error::ProjectionError::InvalidPackageArtifact { message }) => {
            assert_eq!(
                message,
                "package example.pkg artifact projection: package callable public path must not be empty"
            );
        }
        Ok(_) => panic!("an empty public path must fail projection"),
    }
    Ok(())
}

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
        actor: None,
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
