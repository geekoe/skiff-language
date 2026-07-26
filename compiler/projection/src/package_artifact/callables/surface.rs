use std::collections::BTreeMap;

use skiff_artifact_model::{
    OperationTargetRef, PackageCallableId, PackageCallableSignature, PackageExportIndex,
    PackageImplementationLinks, PackageLocalAbiSymbol, PackageTypeRef,
};
use skiff_compiler_projection_input::ProjectionPackageCallableSignatureFacts;

use crate::{
    error::ProjectionError,
    package_artifact::{api_exports::PackageExports, export_links::ProjectedPackageExportLinks},
};

use super::signatures;

pub(super) struct LocalCallableSurface {
    pub public_symbols: BTreeMap<String, PackageLocalAbiSymbol>,
    pub implementation_links: PackageImplementationLinks,
    pub callables: Vec<CanonicalCallable>,
}

pub(super) struct CanonicalCallable {
    pub public_path: String,
    pub callable_id: PackageCallableId,
    pub owner_module: String,
    pub executable_index: u32,
    pub signature: PackageCallableSignature,
    pub target: OperationTargetRef,
}

pub(super) struct CallableTarget {
    pub public_path: String,
    pub callable_id: PackageCallableId,
    pub owner_module: String,
    pub executable_index: u32,
    pub target: OperationTargetRef,
}

pub(super) fn project_local_surface(
    package_id: &str,
    api_exports: &PackageExports,
    exports: &ProjectedPackageExportLinks,
    signatures: &ProjectionPackageCallableSignatureFacts,
) -> Result<LocalCallableSurface, ProjectionError> {
    let mut implementation_links = PackageImplementationLinks::from_exports(&exports.exports);
    for instance in &exports.public_instances {
        if implementation_links
            .constants
            .insert(instance.public_path.clone(), instance.receiver.clone())
            .is_some()
        {
            return Err(ProjectionError::InvalidPackageArtifact {
                message: format!(
                    "public instance {} receiver link conflicts with an exported constant",
                    instance.public_path
                ),
            });
        }
    }
    let mut public_symbols = project_non_callable_symbols(exports)?;
    let mut callable_targets = signatures::package_callable_targets(package_id, exports);
    signatures::add_direct_impl_method_targets(
        package_id,
        api_exports,
        &exports.exports,
        &mut callable_targets,
    );
    let callables =
        signatures::attach_canonical_signatures(package_id, signatures, callable_targets)?;
    add_public_instance_symbols(exports, &callables, &mut public_symbols)?;
    Ok(LocalCallableSurface {
        public_symbols,
        implementation_links,
        callables,
    })
}

fn project_non_callable_symbols(
    projected: &ProjectedPackageExportLinks,
) -> Result<BTreeMap<String, PackageLocalAbiSymbol>, ProjectionError> {
    let mut symbols = BTreeMap::new();
    let exports: &PackageExportIndex = &projected.exports;
    for (public_path, export) in &exports.types {
        let descriptor =
            export
                .descriptor
                .clone()
                .ok_or_else(|| ProjectionError::InvalidPackageArtifact {
                    message: format!("package type export {public_path} has no typed descriptor"),
                })?;
        insert_public_symbol(
            &mut symbols,
            public_path.clone(),
            PackageLocalAbiSymbol::Type {
                local_type_id: format!("type:{public_path}"),
                descriptor,
                is_alias: projected.alias_types.contains(public_path),
                is_interface: export.is_interface,
                type_params: export.type_params.clone(),
                interface_methods: export.interface_methods.clone(),
            },
        )?;
    }
    for (public_path, export) in &exports.constants {
        insert_public_symbol(
            &mut symbols,
            public_path.clone(),
            PackageLocalAbiSymbol::Constant {
                const_id: format!("const:{public_path}"),
                ty: PackageTypeRef::Local {
                    local_type: export.ty.clone(),
                },
            },
        )?;
    }
    Ok(symbols)
}

fn add_public_instance_symbols(
    exports: &ProjectedPackageExportLinks,
    callables: &[CanonicalCallable],
    public_symbols: &mut BTreeMap<String, PackageLocalAbiSymbol>,
) -> Result<(), ProjectionError> {
    let callable_ids = callables
        .iter()
        .map(|callable| (callable.public_path.as_str(), callable.callable_id.clone()))
        .collect::<BTreeMap<_, _>>();
    for instance in &exports.public_instances {
        let methods = instance
            .methods
            .iter()
            .map(|method| {
                let callable_id = callable_ids
                    .get(method.public_path.as_str())
                    .cloned()
                    .ok_or_else(|| ProjectionError::InvalidPackageArtifact {
                        message: format!(
                            "public instance {} method {} has no Local ABI callable",
                            instance.public_path, method.public_path
                        ),
                    })?;
                Ok((method.name.clone(), callable_id))
            })
            .collect::<Result<BTreeMap<_, _>, ProjectionError>>()?;
        insert_public_symbol(
            public_symbols,
            instance.public_path.clone(),
            PackageLocalAbiSymbol::PublicInstance {
                instance_id: instance.public_path.clone(),
                declared_receiver_type: instance.declared_receiver_type.clone(),
                interfaces: instance.interfaces.clone(),
                methods,
            },
        )?;
    }
    Ok(())
}

pub(super) fn insert_public_symbol(
    symbols: &mut BTreeMap<String, PackageLocalAbiSymbol>,
    public_path: String,
    symbol: PackageLocalAbiSymbol,
) -> Result<(), ProjectionError> {
    if symbols.insert(public_path.clone(), symbol).is_some() {
        return Err(ProjectionError::InvalidPackageArtifact {
            message: format!("duplicate PackageLocalAbi public path {public_path}"),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
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
            may_suspend: false,
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
}
