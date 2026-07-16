use skiff_artifact_model::{
    FileIrUnit, InterfaceInstantiationRef, PackageRefIr, PackageSymbolRef, TypeDescriptorIr,
    TypeRefIr,
};

pub(crate) fn file_ir_units_reference_package<'a>(
    file_ir_units: impl IntoIterator<Item = &'a FileIrUnit>,
    package_id: &str,
) -> bool {
    file_ir_units
        .into_iter()
        .any(|file| file_unit_references_package(file, package_id))
}

fn file_unit_references_package(file: &FileIrUnit, package_id: &str) -> bool {
    file.external_refs
        .package_symbols
        .iter()
        .any(|symbol| package_symbol_references_package(symbol, package_id))
        || file.type_table.iter().any(|ty| {
            type_descriptor_references_package(&ty.descriptor, package_id)
                || ty
                    .implements
                    .iter()
                    .any(|implemented| type_ref_references_package(implemented, package_id))
        })
        || file.executables.iter().any(|executable| {
            executable
                .params
                .iter()
                .any(|param| type_ref_references_package(&param.ty, package_id))
                || type_ref_references_package(&executable.return_type, package_id)
                || executable
                    .self_type
                    .as_ref()
                    .is_some_and(|ty| type_ref_references_package(ty, package_id))
        })
}

fn type_descriptor_references_package(descriptor: &TypeDescriptorIr, package_id: &str) -> bool {
    match descriptor {
        TypeDescriptorIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_references_package(field, package_id)),
        TypeDescriptorIr::Alias { target } => type_ref_references_package(target, package_id),
        TypeDescriptorIr::Union { variants } => variants
            .iter()
            .any(|variant| type_ref_references_package(variant, package_id)),
        TypeDescriptorIr::Native { .. } => false,
    }
}

fn type_ref_references_package(ty: &TypeRefIr, package_id: &str) -> bool {
    match ty {
        TypeRefIr::PackageSymbol { symbol } => {
            package_symbol_references_package(symbol, package_id)
        }
        TypeRefIr::Native { args, .. } => args
            .iter()
            .any(|arg| type_ref_references_package(arg, package_id)),
        TypeRefIr::Record { fields } => fields
            .values()
            .any(|field| type_ref_references_package(field, package_id)),
        TypeRefIr::Union { items } => items
            .iter()
            .any(|item| type_ref_references_package(item, package_id)),
        TypeRefIr::Nullable { inner } => type_ref_references_package(inner, package_id),
        TypeRefIr::AnyInterface { interface } => {
            interface_instantiation_references_package(interface, package_id)
        }
        TypeRefIr::Function {
            params,
            return_type,
        } => {
            params
                .iter()
                .any(|param| type_ref_references_package(&param.ty, package_id))
                || type_ref_references_package(return_type, package_id)
        }
        TypeRefIr::LocalType { .. }
        | TypeRefIr::PublicationType { .. }
        | TypeRefIr::ServiceSymbol { .. }
        | TypeRefIr::DbObjectSymbol { .. }
        | TypeRefIr::Literal { .. }
        | TypeRefIr::TypeParam { .. } => false,
    }
}

fn interface_instantiation_references_package(
    interface: &InterfaceInstantiationRef,
    package_id: &str,
) -> bool {
    serde_json::from_str::<TypeRefIr>(&interface.interface_abi_id)
        .ok()
        .is_some_and(|ty| type_ref_references_package(&ty, package_id))
        || interface
            .canonical_type_args
            .iter()
            .any(|arg| type_ref_references_package(arg, package_id))
}

fn package_symbol_references_package(symbol: &PackageSymbolRef, package_id: &str) -> bool {
    match &symbol.package {
        PackageRefIr::PackageId {
            package_id: candidate,
        } => candidate == package_id,
        PackageRefIr::Dependency { .. } => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use skiff_artifact_model::{
        FunctionTypeParamIr, InterfaceInstantiationRef, PackageRefIr, PackageSymbolRef, TypeDeclIr,
        TypeDescriptorIr, TypeRefIr,
    };

    use super::*;

    const TARGET: &str = "example.com/target";

    #[test]
    fn finds_direct_external_package_symbol_for_requested_package() {
        let mut file = FileIrUnit::empty("api", "source");
        file.external_refs.package_symbols.push(package_symbol(
            PackageRefIr::PackageId {
                package_id: TARGET.to_string(),
            },
            "Thing",
        ));

        assert!(file_ir_units_reference_package([&file], TARGET));
        assert!(!file_ir_units_reference_package(
            [&file],
            "example.com/other"
        ));
    }

    #[test]
    fn finds_package_reference_nested_in_nominal_type_closure() {
        let mut file = FileIrUnit::empty("api", "source");
        file.type_table.push(TypeDeclIr {
            name: "Envelope".to_string(),
            descriptor: TypeDescriptorIr::Record {
                fields: BTreeMap::from([(
                    "callback".to_string(),
                    TypeRefIr::Function {
                        params: vec![FunctionTypeParamIr {
                            name: "input".to_string(),
                            ty: TypeRefIr::native("string"),
                        }],
                        return_type: Box::new(TypeRefIr::Nullable {
                            inner: Box::new(package_type(TARGET, "Result")),
                        }),
                    },
                )]),
            },
            type_params: Vec::new(),
            discriminator: None,
            implements: vec![TypeRefIr::AnyInterface {
                interface: InterfaceInstantiationRef {
                    interface_abi_id: "opaque-interface".to_string(),
                    canonical_type_args: vec![package_type(TARGET, "Config")],
                },
            }],
            source_span: None,
        });

        assert!(file_ir_units_reference_package([&file], TARGET));
    }

    #[test]
    fn dependency_alias_refs_do_not_claim_a_concrete_package_id() {
        let mut file = FileIrUnit::empty("api", "source");
        file.external_refs.package_symbols.push(package_symbol(
            PackageRefIr::Dependency {
                dependency_ref: "target".to_string(),
            },
            "Thing",
        ));

        assert!(!file_ir_units_reference_package([&file], TARGET));
    }

    fn package_type(package_id: &str, symbol: &str) -> TypeRefIr {
        TypeRefIr::PackageSymbol {
            symbol: package_symbol(
                PackageRefIr::PackageId {
                    package_id: package_id.to_string(),
                },
                symbol,
            ),
        }
    }

    fn package_symbol(package: PackageRefIr, symbol: &str) -> PackageSymbolRef {
        PackageSymbolRef {
            package,
            symbol_path: symbol.to_string(),
            abi_expectation: None,
        }
    }
}
